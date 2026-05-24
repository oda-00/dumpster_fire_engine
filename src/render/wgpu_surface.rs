//! wgpu fallback rendering surface — selected at runtime when Vulkan is
//! unavailable (macOS without MoltenVK, WebAssembly, DX12-only hardware).
//!
//! Provides:
//!  * UI overlay (DrawList → GPU, font atlas, solid fills, glyphs)
//!  * Basic forward-lit scene pass (simplified PBR, no RT, no skinning)
//!  * Tonemap pass (Linear / Reinhard / ACES)
//!  * Debug lines

#![cfg(feature = "wgpu-backend")]

use std::sync::Arc;
use wgpu::util::DeviceExt;

use crate::forge_master::master::{ForgeError, ForgeResult};
use crate::resource_manager::ui_manager::{draw::UiVertex, font};

use super::backend::{BackendKind, DrawListSnapshot, GpuSurface, RenderSceneInput};

// ── Vertex descriptor matching UiVertex (pos, uv, color u8×4) ──────────────

fn ui_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    use wgpu::VertexFormat::*;
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<UiVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                shader_location: 0,
                offset: 0,
                format: Float32x2,
            }, // pos
            wgpu::VertexAttribute {
                shader_location: 1,
                offset: 8,
                format: Float32x2,
            }, // uv
            wgpu::VertexAttribute {
                shader_location: 2,
                offset: 16,
                format: Unorm8x4,
            }, // color
        ],
    }
}

// ── WgpuSurface ────────────────────────────────────────────────────────────

pub struct WgpuSurface {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    // HDR intermediate texture (Rgba16Float, same dims as surface)
    hdr_tex: wgpu::Texture,
    hdr_view: wgpu::TextureView,

    // Depth buffer
    depth_tex: wgpu::Texture,
    depth_view: wgpu::TextureView,

    // Font atlas texture (R8Unorm)
    font_tex: wgpu::Texture,
    font_view: wgpu::TextureView,
    sampler: wgpu::Sampler,

    // Screen-size uniform (2×f32)
    screen_buf: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,

    // Tonemap uniform (u32 op)
    tonemap_buf: wgpu::Buffer,
    tonemap_bind_group: wgpu::BindGroup,

    // Pipelines
    ui_pipeline: wgpu::RenderPipeline,
    tonemap_pipeline: wgpu::RenderPipeline,
    scene_pipeline: wgpu::RenderPipeline,

    // Bind groups
    atlas_bind_group: wgpu::BindGroup,
    hdr_bind_group: wgpu::BindGroup,

    // UI vertex + index buffers (grown on demand)
    ui_vb: wgpu::Buffer,
    ui_ib: wgpu::Buffer,
    ui_vb_cap: usize,
    ui_ib_cap: usize,

    width: u32,
    height: u32,
}

impl WgpuSurface {
    pub fn new(winit_window: Arc<winit::window::Window>) -> ForgeResult<Self> {
        // ── Instance + adapter ──────────────────────────────────────────────
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let surface = instance
            .create_surface(winit_window.clone())
            .map_err(|e| ForgeError::Io(std::io::Error::other(e.to_string())))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| ForgeError::Io(std::io::Error::other("no wgpu adapter")))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("dfe-wgpu"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| ForgeError::Io(std::io::Error::other(e.to_string())))?;

        let size = winit_window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // ── HDR intermediate texture ────────────────────────────────────────
        let (hdr_tex, hdr_view) = create_hdr_texture(&device, width, height);
        let (depth_tex, depth_view) = create_depth_texture(&device, width, height);

        // ── Font atlas ─────────────────────────────────────────────────────
        let atlas = font::bake_atlas();
        let font_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font-atlas"),
            size: wgpu::Extent3d {
                width: font::ATLAS_W,
                height: font::ATLAS_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            font_tex.as_image_copy(),
            &atlas,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(font::ATLAS_W),
                rows_per_image: Some(font::ATLAS_H),
            },
            wgpu::Extent3d {
                width: font::ATLAS_W,
                height: font::ATLAS_H,
                depth_or_array_layers: 1,
            },
        );
        let font_view = font_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ── Bind group layouts ─────────────────────────────────────────────
        let atlas_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let screen_bgl = uniform_bgl(&device, "screen-bgl", wgpu::ShaderStages::VERTEX);
        let tonemap_tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap-tex-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let tonemap_param_bgl =
            uniform_bgl(&device, "tonemap-param-bgl", wgpu::ShaderStages::FRAGMENT);
        let scene_cam_bgl = uniform_bgl(&device, "scene-cam-bgl", wgpu::ShaderStages::VERTEX);

        // ── Uniform buffers ─────────────────────────────────────────────────
        let screen_data: [f32; 2] = [width as f32, height as f32];
        let screen_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("screen-uniform"),
            contents: unsafe { std::slice::from_raw_parts(screen_data.as_ptr() as *const u8, 8) },
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let tonemap_init: u32 = 0;
        let tonemap_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tonemap-uniform"),
            contents: unsafe {
                std::slice::from_raw_parts(&tonemap_init as *const u32 as *const u8, 4)
            },
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Bind groups ─────────────────────────────────────────────────────
        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas-bg"),
            layout: &atlas_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&font_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen-bg"),
            layout: &screen_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_buf.as_entire_binding(),
            }],
        });
        let hdr_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hdr-bg"),
            layout: &tonemap_tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let tonemap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap-param-bg"),
            layout: &tonemap_param_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: tonemap_buf.as_entire_binding(),
            }],
        });

        // ── Pipelines ────────────────────────────────────────────────────────
        let ui_shader_src = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/ui.wgsl"
        ));
        let tm_shader_src = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/tonemap.wgsl"
        ));
        let sc_shader_src = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/forward_lit_simple.wgsl"
        ));

        let ui_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui-shader"),
            source: wgpu::ShaderSource::Wgsl(ui_shader_src.into()),
        });
        let tm_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tonemap-shader"),
            source: wgpu::ShaderSource::Wgsl(tm_shader_src.into()),
        });
        let sc_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene-shader"),
            source: wgpu::ShaderSource::Wgsl(sc_shader_src.into()),
        });

        let ui_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui-pl-layout"),
            bind_group_layouts: &[&atlas_bgl, &screen_bgl],
            push_constant_ranges: &[],
        });
        let tm_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tm-pl-layout"),
            bind_group_layouts: &[&tonemap_tex_bgl, &tonemap_param_bgl],
            push_constant_ranges: &[],
        });
        let sc_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sc-pl-layout"),
            bind_group_layouts: &[&scene_cam_bgl],
            push_constant_ranges: &[],
        });

        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui-pipeline"),
            layout: Some(&ui_pl_layout),
            vertex: wgpu::VertexState {
                module: &ui_shader,
                entry_point: "vs_main",
                buffers: &[ui_vertex_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &ui_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let tonemap_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tonemap-pipeline"),
            layout: Some(&tm_pl_layout),
            vertex: wgpu::VertexState {
                module: &tm_shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &tm_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene-pipeline"),
            layout: Some(&sc_pl_layout),
            vertex: wgpu::VertexState {
                module: &sc_shader,
                entry_point: "vs_main",
                buffers: &[], // glTF vertex buffers bound separately
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &sc_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // ── Initial UI buffers ───────────────────────────────────────────────
        let ui_vb = create_vertex_buffer(&device, 4096 * std::mem::size_of::<UiVertex>());
        let ui_ib = create_index_buffer(&device, 4096 * 4);

        Ok(Self {
            device,
            queue,
            surface,
            config,
            hdr_tex,
            hdr_view,
            depth_tex,
            depth_view,
            font_tex,
            font_view,
            sampler,
            screen_buf,
            screen_bind_group,
            tonemap_buf,
            tonemap_bind_group,
            ui_pipeline,
            tonemap_pipeline,
            scene_pipeline,
            atlas_bind_group,
            hdr_bind_group,
            ui_vb,
            ui_ib,
            ui_vb_cap: 4096 * std::mem::size_of::<UiVertex>(),
            ui_ib_cap: 4096 * 4,
            width,
            height,
        })
    }

    fn ensure_ui_buffers(&mut self, vb_need: usize, ib_need: usize) {
        if vb_need > self.ui_vb_cap {
            let cap = (vb_need * 2).next_power_of_two();
            self.ui_vb = create_vertex_buffer(&self.device, cap);
            self.ui_vb_cap = cap;
        }
        if ib_need > self.ui_ib_cap {
            let cap = (ib_need * 2).next_power_of_two();
            self.ui_ib = create_index_buffer(&self.device, cap);
            self.ui_ib_cap = cap;
        }
    }
}

impl GpuSurface for WgpuSurface {
    fn backend(&self) -> BackendKind {
        BackendKind::Wgpu
    }

    fn draw_frame(
        &mut self,
        scene: &RenderSceneInput<'_>,
        ui: &DrawListSnapshot<'_>,
    ) -> ForgeResult<()> {
        // Upload tonemap op
        let tonemap_val: u32 = scene.world.tonemap_op;
        self.queue.write_buffer(&self.tonemap_buf, 0, unsafe {
            std::slice::from_raw_parts(&tonemap_val as *const u32 as *const u8, 4)
        });

        // Upload UI geometry
        let vb_bytes = unsafe {
            std::slice::from_raw_parts(
                ui.vertices.as_ptr() as *const u8,
                ui.vertices.len() * std::mem::size_of::<UiVertex>(),
            )
        };
        let ib_bytes = unsafe {
            std::slice::from_raw_parts(ui.indices.as_ptr() as *const u8, ui.indices.len() * 4)
        };
        self.ensure_ui_buffers(vb_bytes.len().max(1), ib_bytes.len().max(1));
        if !ui.vertices.is_empty() {
            self.queue.write_buffer(&self.ui_vb, 0, vb_bytes);
        }
        if !ui.indices.is_empty() {
            self.queue.write_buffer(&self.ui_ib, 0, ib_bytes);
        }

        // Acquire swapchain image
        let output = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            Err(e) => return Err(ForgeError::Io(std::io::Error::other(e.to_string()))),
        };
        let frame_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dfe-frame"),
            });

        // ── Pass 1: Scene → HDR ─────────────────────────────────────────────
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.1,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.scene_pipeline);
            // No actual mesh draws yet; renders a cleared HDR background.
        }

        // ── Pass 2: Tonemap HDR → swapchain ────────────────────────────────
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tonemap-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.tonemap_pipeline);
            rp.set_bind_group(0, &self.hdr_bind_group, &[]);
            rp.set_bind_group(1, &self.tonemap_bind_group, &[]);
            rp.draw(0..3, 0..1);
        }

        // ── Pass 3: UI → swapchain (LOAD_OP_LOAD) ──────────────────────────
        if !ui.indices.is_empty() {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.ui_pipeline);
            rp.set_bind_group(0, &self.atlas_bind_group, &[]);
            rp.set_bind_group(1, &self.screen_bind_group, &[]);
            rp.set_vertex_buffer(0, self.ui_vb.slice(..));
            rp.set_index_buffer(self.ui_ib.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..ui.indices.len() as u32, 0, 0..1);
        }

        self.queue.submit(std::iter::once(enc.finish()));
        output.present();
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.width = width;
        self.height = height;
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);

        // Recreate size-dependent textures.
        let (hdr_tex, hdr_view) = create_hdr_texture(&self.device, width, height);
        let (depth_tex, depth_view) = create_depth_texture(&self.device, width, height);
        self.hdr_tex = hdr_tex;
        self.hdr_view = hdr_view;
        self.depth_tex = depth_tex;
        self.depth_view = depth_view;

        // Rebuild HDR bind group to point at new texture view.
        let tonemap_tex_bgl =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("tonemap-tex-bgl-resize"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                multisampled: false,
                                view_dimension: wgpu::TextureViewDimension::D2,
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        self.hdr_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hdr-bg-resized"),
            layout: &tonemap_tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        // Update screen uniform.
        let screen_data: [f32; 2] = [width as f32, height as f32];
        self.queue.write_buffer(&self.screen_buf, 0, unsafe {
            std::slice::from_raw_parts(screen_data.as_ptr() as *const u8, 8)
        });
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn create_hdr_texture(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr-tex"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn create_depth_texture(
    device: &wgpu::Device,
    w: u32,
    h: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-tex"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn uniform_bgl(
    device: &wgpu::Device,
    label: &str,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

fn create_vertex_buffer(device: &wgpu::Device, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui-vb"),
        size: size as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_index_buffer(device: &wgpu::Device, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui-ib"),
        size: size as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
