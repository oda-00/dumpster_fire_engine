use std::path::PathBuf;
use std::sync::Arc;
use thin_vec::ThinVec;

use ash::vk;
use crate::resource_manager::manager::{Handle, Id};

use super::ingot::Ingot;
use super::master::{ForgeMaster, ForgeResult};
use super::ore::{GpuMesh, GraphicsOreKind, IngotSpec, MAT4_IDENTITY, Ore, OreKind};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FrameTag;
pub type FrameHandle = Handle<FrameTag>;

pub struct FrameMarker;
pub type FrameId = Id<FrameMarker>;

pub struct FramePlan {
    pub id: FrameId,
    pub name: Arc<str>,
    pub ores: ThinVec<Ore>,
}

impl FramePlan {
    pub fn new(id: FrameId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name: name.into(),
            ores: ThinVec::new(),
        }
    }

    pub fn push(&mut self, ore: Ore) {
        self.ores.push(ore);
    }

    pub fn refine(self, master: &mut ForgeMaster) -> ForgeResult<Frame> {
        let mut frame = Frame::new(self.id, self.name);
        for ore in self.ores {
            frame.ingots.push(master.refine(ore)?);
        }
        Ok(frame)
    }
}

pub struct Frame {
    pub id: FrameId,
    pub name: Arc<str>,
    pub ingots: ThinVec<Ingot>,
}

impl Frame {
    pub fn new(id: FrameId, name: impl Into<Arc<str>>) -> Self {
        Self {
            id,
            name: name.into(),
            ingots: ThinVec::new(),
        }
    }

    pub fn add_ingot(&mut self, ingot: Ingot) {
        self.ingots.push(ingot);
    }

    pub fn manifest(&self) -> FrameManifest {
        FrameManifest {
            id: self.id,
            name: self.name.clone(),
            entries: self
                .ingots
                .iter()
                .map(|ingot| FrameEntry {
                    kind: ingot.kind,
                    byte_len: ingot.as_bytes().len() as u64,
                    save_path: ingot.save_path.clone(),
                })
                .collect(),
        }
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for ingot in &mut self.ingots {
            unsafe { ingot.destroy(device) };
        }
        self.ingots.clear();
    }
}

#[derive(Debug, Clone)]
pub struct FrameManifest {
    pub id: FrameId,
    pub name: Arc<str>,
    pub entries: ThinVec<FrameEntry>,
}

#[derive(Debug, Clone)]
pub struct FrameEntry {
    pub kind: OreKind,
    pub byte_len: u64,
    pub save_path: Option<PathBuf>,
}

pub fn ore_for_buffer(
    kind: OreKind,
    bytes: ThinVec<u8>,
    output_size: ash::vk::DeviceSize,
    workgroups: [u32; 3],
) -> Ore {
    Ore::new(
        kind,
        super::ore::OreInput::Bytes(bytes),
        IngotSpec::Buffer {
            size: output_size,
            save_path: None,
            extra_usage: ash::vk::BufferUsageFlags::empty(),
        },
        workgroups,
    )
}

// ── Graphics frames ────────────────────────────────────────────────────────
//
// The rasterization analog of FramePlan/Frame. A `GraphicsFramePlan` is just
// a draw-call description (vertex/instance counts, plus the graphics kind
// that picks which GraphicsForge/pipeline to bind). It carries no GPU
// resources, so "refining" it is a trivial conversion to a `GraphicsFrame`
// — the same shape, but stored on the Factory as the live draw list the
// window iterates each redraw.
//
// FrameId is shared with compute frames so factories can index by id across
// both kinds; the queue type on the parent Proto disambiguates.

#[derive(Debug, Clone)]
pub struct GraphicsFramePlan {
    pub id:             FrameId,
    pub name:           Arc<str>,
    pub kind:           GraphicsOreKind,
    /// Procedural vertex count (used when `mesh` is `None`).
    pub vertex_count:   u32,
    pub instance_count: u32,
    pub first_vertex:   u32,
    pub first_instance: u32,
    /// Uploaded mesh (ForwardLit draws). `None` = procedural / shader-generated.
    pub mesh: Option<Arc<GpuMesh>>,
    /// Column-major model-view-projection matrix sent as a push constant.
    /// Defaults to identity; only consumed by the ForwardLit pipeline.
    pub mvp: [f32; 16],
    /// Optional pre-resolved material descriptor set (set 1). `None` uses
    /// whatever was last bound (typically the dummy white material).
    pub material_set: Option<vk::DescriptorSet>,
    /// Optional alternative vertex buffer — overrides `mesh.vertex_buffer`
    /// when present. Used to feed a compute-shader-posed vertex stream
    /// (MorphBlend output) into the draw without re-uploading. The buffer
    /// must outlive the recorded command buffer; typically that means it's
    /// owned by the same FactoryMaster as the compute Ingot that produced
    /// it, and the compute factory is rebuilt every frame in lockstep.
    pub vertex_buffer_override: Option<vk::Buffer>,
    /// Optional per-vertex skin attributes (joints + weights) — only used
    /// by `SkinnedForwardLit` draws, bound at vertex binding 1.
    pub skin_vertex_buffer: Option<vk::Buffer>,
    /// Optional skin-palette descriptor set (set 2). Resolved from the
    /// SkinPalette compute Ingot's buffer each frame.
    pub skin_palette_set: Option<vk::DescriptorSet>,
}

impl GraphicsFramePlan {
    /// Procedural draw (no vertex buffer — shader generates verts).
    pub fn new(
        id:           FrameId,
        name:         impl Into<Arc<str>>,
        kind:         GraphicsOreKind,
        vertex_count: u32,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            vertex_count,
            instance_count: 1,
            first_vertex:   0,
            first_instance: 0,
            mesh: None,
            mvp:  MAT4_IDENTITY,
            material_set: None,
            vertex_buffer_override: None,
            skin_vertex_buffer: None,
            skin_palette_set: None,
        }
    }

    /// Indexed mesh draw (ForwardLit).
    pub fn new_mesh(
        id:   FrameId,
        name: impl Into<Arc<str>>,
        mesh: Arc<GpuMesh>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            kind:           GraphicsOreKind::ForwardLit,
            vertex_count:   0, // unused; index count comes from GpuMesh
            instance_count: 1,
            first_vertex:   0,
            first_instance: 0,
            mesh: Some(mesh),
            mvp:  MAT4_IDENTITY,
            material_set: None,
            vertex_buffer_override: None,
            skin_vertex_buffer: None,
            skin_palette_set: None,
        }
    }

    /// Promote a mesh draw to the SkinnedForwardLit pipeline. The caller
    /// must also attach the per-vertex skin buffer + palette descriptor
    /// via the dedicated builders below.
    pub fn with_kind(mut self, kind: GraphicsOreKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set the pre-resolved Vulkan descriptor set for the material (set 1).
    pub fn with_material_set(mut self, set: vk::DescriptorSet) -> Self {
        self.material_set = Some(set);
        self
    }

    /// Override the vertex buffer bound at draw time — used to substitute
    /// a compute-shader-posed buffer (MorphBlend output) for the rest-pose
    /// `GpuMesh` allocation. The index buffer still comes from the mesh.
    pub fn with_vertex_buffer_override(mut self, buffer: vk::Buffer) -> Self {
        self.vertex_buffer_override = Some(buffer);
        self
    }

    /// Bind a per-vertex skin attribute buffer at vertex binding 1
    /// (joints + weights — only consumed by SkinnedForwardLit).
    pub fn with_skin_vertex_buffer(mut self, buffer: vk::Buffer) -> Self {
        self.skin_vertex_buffer = Some(buffer);
        self
    }

    /// Bind the skin-palette descriptor set at set 2 (only consumed by
    /// SkinnedForwardLit). Typically created on-demand per frame from a
    /// SkinPalette compute Ingot's `result_buffer()`.
    pub fn with_skin_palette_set(mut self, set: vk::DescriptorSet) -> Self {
        self.skin_palette_set = Some(set);
        self
    }

    pub fn with_instances(mut self, instance_count: u32) -> Self {
        self.instance_count = instance_count;
        self
    }

    pub fn with_offsets(mut self, first_vertex: u32, first_instance: u32) -> Self {
        self.first_vertex   = first_vertex;
        self.first_instance = first_instance;
        self
    }

    /// Override the MVP push constant (column-major mat4).
    pub fn with_mvp(mut self, mvp: [f32; 16]) -> Self {
        self.mvp = mvp;
        self
    }

    /// "Refining" a graphics plan is a no-op other than the type flip — no
    /// compute dispatch, no GPU buffers — so this is infallible and doesn't
    /// touch the ForgeMaster.
    pub fn refine(self) -> GraphicsFrame {
        GraphicsFrame {
            id:             self.id,
            name:           self.name,
            kind:           self.kind,
            vertex_count:   self.vertex_count,
            instance_count: self.instance_count,
            first_vertex:   self.first_vertex,
            first_instance: self.first_instance,
            mesh:           self.mesh,
            mvp:            self.mvp,
            material_set:   self.material_set,
            vertex_buffer_override: self.vertex_buffer_override,
            skin_vertex_buffer:     self.skin_vertex_buffer,
            skin_palette_set:       self.skin_palette_set,
        }
    }
}

/// A live draw call stored on a [`Factory`].
///
/// `mesh = None`  → procedural (`cmd_draw`, vertex shader generates geometry).
/// `mesh = Some`  → indexed mesh (`cmd_bind_vertex_buffers` + `cmd_draw_indexed`).
///
/// The `Arc<GpuMesh>` keeps the GPU buffers alive for as long as the factory
/// lives. `Factory::destroy(device)` calls `Arc::try_unwrap` and then
/// `GpuMesh::destroy(device)` to properly free the Vulkan allocations.
#[derive(Debug, Clone)]
pub struct GraphicsFrame {
    pub id:             FrameId,
    pub name:           Arc<str>,
    pub kind:           GraphicsOreKind,
    pub vertex_count:   u32,
    pub instance_count: u32,
    pub first_vertex:   u32,
    pub first_instance: u32,
    pub mesh: Option<Arc<GpuMesh>>,
    pub mvp:  [f32; 16],
    pub material_set: Option<vk::DescriptorSet>,
    pub vertex_buffer_override: Option<vk::Buffer>,
    pub skin_vertex_buffer:     Option<vk::Buffer>,
    pub skin_palette_set:       Option<vk::DescriptorSet>,
}
