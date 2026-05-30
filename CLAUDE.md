# Ray-Tracing Render Path + GUI/RT Compositing + UI Icon Graphics — Plan

> Finalized implementation plan. Builds on the editor + mesh-editing work and the
> `GUI_research.md` / `EDITOR_research.md` studies (feature branch / PR #45). This
> doc is the agreed plan for: (1) wiring the engine's unused ray-tracing pipeline
> into the frame and preferring it, (2) making the GUI composite correctly over a
> ray-traced scene, and (3) adding real open-source UI icon graphics — **without
> adding new crate dependencies**.

## Context

Two gaps surfaced while building the editor:

1. **Ray tracing is dead code.** The engine ships a complete RT pipeline
   (`src/render/rt_pipeline.rs`: SBT, a descriptor-set *layout* for
   TLAS / HDR-storage-image / lights, and `raygen` + `primary_chit` +
   `primary_miss` + `shadow_miss` shaders) and builds a BLAS per primitive
   (`gltf_assets.rs` → `LoadedGltf.blas`). **But** `Window::init_rt_pipeline` is
   never called, `cmd_trace_rays` appears nowhere, and no per-frame TLAS-instance
   list is ever built. Every frame goes through the forward **raster** pass; the
   `DFE_RT` env var only logs "using raster fallback". → The user wants RT
   **preferred**, with a ray-traced lighting mode.

2. **The GUI must support RT + real icons.** The UI overlay must composite over an
   RT-rendered scene, and the editor toolbar needs proper icons (20 Lucide SVGs are
   vendored at `assets/icons/lucide/` but not yet baked into the UI atlas).

**Outcome:** a `LightingMode::RayTraced` that renders the scene into the HDR target
via the RT pipeline (default-preferred when the device supports RT), the existing
tonemap + overlay path compositing the UI on top, and an icon-driven editor toolbar
baked from the open-source SVGs.

## Dependency philosophy (explicit)

This engine is deliberately low-dependency: a hand-coded VGA-style font, in-house UI
(`ui_core` / `ui_manager`), in-house glTF façade (`forge_gltf`) and mesh core
(`forge_mesh`). The existing external crates are the **unavoidable foundation layer**
you do not hand-roll — `ash` (Vulkan loader), `winit` (windowing), `glam` (SIMD
math), `rayon`, `gltf`, `image`, `fontdue`, `thin-vec`.

**This plan adds ZERO new crates:**
- **Ray tracing → no new crates.** All `ash` + the engine's own `rt_pipeline.rs` /
  `blas.rs`.
- **Icons → no new crates.** The SVGs are rasterized **offline, once**, with a
  *system* tool (e.g. `rsvg-convert` / ImageMagick — never a Cargo dependency), then
  embedded as a raw R8 alpha sheet via `include_bytes!` (exactly how the font ships
  today) or decoded with the **already-present `image` crate**.

## Part A — Research-doc expansion (web-sourced)

Add a **"Ray-traced rendering path"** section to `EDITOR_research.md`: Vulkan
hardware RT (`VK_KHR_acceleration_structure` + `ray_tracing_pipeline`), TLAS
**refit vs rebuild** (`MODE_UPDATE` for transform-only updates; BLAS `FAST_TRACE`,
TLAS `FAST_BUILD`), instance **culling into the TLAS**, **hybrid** raster+RT
(G-buffer → RT shadows/reflections → denoise; 1–2 spp), and how this engine's
specific dead pipeline gets wired. Cite Khronos hybrid best-practices, the nvpro
`vk_raytracing_tutorial_KHR`, the Vulkan RT tutorial, SaschaWillems samples, and
Blender Eevee-Next (interactive viewport RT).

Add a **"UI graphics: icons + RT compositing"** section to `GUI_research.md`:
open-source icon sources & licenses (Lucide ISC; Tabler / Heroicons / Ionicons MIT;
Material Symbols Apache-2.0; Kenney / OpenGameArt CC0; Iconify aggregator), the
zero-dep offline SVG→atlas pipeline, and how a UI overlay composites over an
HDR / ray-traced scene (final overlay pass, drawn last — the ImGui-over-RT model).

## Part B — Engine ray-tracing render path

Frame path today (`window.rs` `draw_frame`, `overlay.rs` `record`):
*raster scene → blit swapchain→HDR (`overlay.hdr_images`, `R16G16B16A16_SFLOAT`) →
tonemap pass (HDR→swapchain) → UI overlay pass (`LOAD_OP_LOAD`, drawn last).*
The UI already composites over an HDR scene image — so if **RT writes that HDR
image**, the UI sits over RT for free.

1. **Create the pipeline.** Call `Window::init_rt_pipeline(vulkan)` at graphics
   bring-up (after `init_overlay_pipeline`), guarded by `vulkan.has_ray_tracing`.
2. **HDR scene target + descriptor set.** Give the per-image HDR target `STORAGE`
   usage (extend `overlay.rs` image creation to
   `COLOR_ATTACHMENT | SAMPLED | STORAGE`); allocate the RT descriptor *set* in
   `RtPipeline` (pool + set) binding the TLAS, that HDR storage image, and a lights
   UBO.
3. **TLAS instances.** Add a renderer step that walks the scene draw list and, per
   mesh primitive, emits a `vk::AccelerationStructureInstanceKHR` (BLAS device
   address from `LoadedGltf.blas` + world 3×4 transform + instance index); call
   `rebuild_tlas` (use `MODE_UPDATE` refit when only transforms changed). Empty
   instance list → RT miss/clear (never a crash).
4. **Trace dispatch.** Add `RtPipeline::record_trace(cmd, extent, camera_push)`:
   bind pipeline + set, push camera, `cmd_trace_rays` into the HDR storage image,
   with image barriers (`GENERAL` for the trace → `SHADER_READ_ONLY` for tonemap).
5. **Unify the frame.** Add `LightingMode { Raster, RayTraced }` in `window.rs`:
   - `RayTraced` (default when `has_ray_tracing`): build instances → `rebuild_tlas`
     → `record_trace` into HDR → tonemap → UI overlay. Skip the raster scene pass +
     the swapchain→HDR blit.
   - `Raster`: the current path.
   - Any missing/failed RT resource → fall back to `Raster` (no regression).
6. **Prefer + toggle.** Default to `RayTraced` when supported; editor key `L` flips
   the mode (via an `AppCtx` setter); log the active mode.

## Part C — GUI rewritten to support ray tracing

The overlay/tonemap path already composites the UI over the HDR target; the change is
making that HDR target the **single compositing surface** for both raster and RT
(Part B steps 2 & 5), so the UI pass is identical regardless of how the scene was
produced. Verify the editor's gizmo / wireframe / mesh-edit overlays (drawn into the
same UI `DrawList`) still land on top in `RayTraced` mode.

## Part D — UI icon graphics (full scope, zero new crates)

1. **Bake offline.** Rasterize `assets/icons/lucide/*.svg` to small monochrome
   bitmaps with a system tool (one-time, not a Cargo dep), pack into a single R8
   alpha sheet, and commit the baked result (`assets/icons/ui_icons.{png,r8}`) plus a
   UV table.
2. **Atlas integration.** Extend `Atlas::build` / `bake_atlas`
   (`ui_manager/atlas.rs`) to append the icon sheet after the glyph region (grow
   atlas height), filling `icon_rects` + a named `IconId` enum
   (Move, RotateGizmo, Scale, Grid, Eye, Trash, Copy, Undo, Redo, Add, …). Embed the
   sheet via `include_bytes!` (raw R8 — no decoder, like the font) or decode with the
   existing `image` crate.
3. **Icon toolbar.** Replace text-label toolbar buttons in `src/bin/editor.rs` with
   icon draws (reuse the existing `glyph_rect`-style UV draw; icons tint via vertex
   color), covering the edit-mode/tools (E / G / R / S / snap / duplicate / frame /
   stats).

## Reuse (don't reinvent)
`overlay.rs` HDR images + tonemap + overlay passes; `RtPipeline::{new, rebuild_tlas}`
and the existing RT shaders/SBT; `LoadedGltf.blas` + `blas.rs`;
`vulkan.has_ray_tracing` + `rt_pipeline` / `rt_accel` loaders + `rt_props`;
`ForgeImage` / `ForgeBuffer`; the `Atlas` / `IconId` / `glyph_rect` icon path; the
editor viewport/gizmo overlay drawing.

## Critical files
- **RT:** `src/render/window.rs` (init call, `LightingMode`, frame branch),
  `src/render/rt_pipeline.rs` (descriptor set, `record_trace`, refit),
  `src/render/overlay.rs` (HDR `STORAGE` usage), a new renderer step building TLAS
  instances from the scene + `LoadedGltf.blas`, `src/render/app.rs`
  (`AppCtx` lighting-mode setter).
- **Icons:** offline-baked `assets/icons/ui_icons.{png,r8}` (system-tool rasterized,
  no Cargo dep), `ui_manager/{atlas.rs,icon.rs,font.rs}`, `src/bin/editor.rs` toolbar.
  No `Cargo.toml` changes.
- **Docs:** `EDITOR_research.md`, `GUI_research.md`.

## Verification
1. `cargo build --workspace` green (RT compiles; raster unaffected).
2. **Runtime smoke (lavapipe + Xvfb), WITHOUT `DFE_RT=0`:**
   `bash scripts/run-runtime-tests.sh --editor` — confirm the log shows
   "ray-tracing path enabled", `init_rt_pipeline` succeeds, the editor builds the
   TLAS and runs the loop in `RayTraced` mode **without crashing**; toggle `L` to
   confirm raster still works.
3. **Icon bake:** a unit test that the bake yields N icon rects with valid UVs;
   editor smoke shows the icon toolbar (no panic).
4. Extend `tests/ui_runtime.rs` with an RT-pipeline-init test (skips if no device).

## Risks / limits (explicit)
- **Headless cannot verify RT pixels.** lavapipe's RT *execution* is experimental;
  the smoke validates "compiles + initializes + traces without crashing", not image
  correctness — final RT visuals need an RT-capable GPU. Therefore **raster stays the
  guaranteed fallback**, and RT degrades to raster on any missing/failed resource (no
  regression to the working editor).
- TLAS instance gathering touches the scene→renderer data flow; build it additively
  (empty instance list → RT miss, not a crash).

## Scope note
Full scope — nothing cut. RT is full-RT-primary + ray-traced shadows (matching the
existing shaders); denoising / hybrid effects are a follow-up. Icons are baked at full
SVG fidelity. The only thing avoided is **new external crates**, per the engine's
dependency philosophy above.
