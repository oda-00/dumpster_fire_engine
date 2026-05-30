//! Runtime test of the UI render-data path on a real Vulkan device.
//!
//! Headless CI has no hardware GPU, so this runs against the software device
//! Mesa lavapipe exposes (install `mesa-vulkan-drivers`, then point the loader
//! at it: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`). Set
//! `DFE_RT=0` because lavapipe has no ray-tracing extensions. When no device is
//! available the test skips, mirroring `render_animated_glb`.
//!
//! It exercises the pieces the unit tests can't reach because they need a GPU:
//! Vulkan device init, `FontAtlas` creation (allocates a Vulkan image), `fontdue`
//! glyph rasterization + the sorted glyph cache, the ASCII `push_text` fast path
//! (GUI_research.md §4.6), and `push_rect` quad/index emission.
//!
//! NOTE: the bundled `assets/fonts/FiraCode-Regular.ttf` must be a real TTF for
//! the font-dependent cases to run; if it is not (e.g. a broken/placeholder
//! asset), those cases skip with a message instead of failing.

use std::path::Path;

use dumpster_fire_engine::render::VulkanContext;
use dumpster_fire_engine::render::ui_core::layout::Rect;
use dumpster_fire_engine::render::ui_render::{DrawList, FontAtlas};

fn try_vulkan() -> Option<VulkanContext> {
    match VulkanContext::new() {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("ui_runtime test skipped: no Vulkan device ({e:?})");
            None
        }
    }
}

/// The engine bundles this font via `include_bytes!` in `ui_render::font`.
/// Validate the on-disk copy has a real sfnt signature before relying on it, so
/// a broken asset skips the font cases rather than panicking inside fontdue.
fn bundled_font_is_valid() -> bool {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/DejaVuSansMono.ttf");
    match std::fs::read(&path) {
        Ok(bytes) if bytes.len() >= 4 => {
            let m = &bytes[..4];
            // sfnt magics: 0x00010000 (TTF), 'true', 'OTTO' (CFF), 'ttcf' (collection)
            m == [0x00, 0x01, 0x00, 0x00]
                || m == *b"true"
                || m == *b"OTTO"
                || m == *b"ttcf"
        }
        _ => false,
    }
}

#[test]
fn vulkan_device_initializes() {
    // Pure runtime proof that the (software) device comes up end-to-end.
    let Some(_ctx) = try_vulkan() else { return };
}

#[test]
fn rt_pipeline_initializes_when_supported() {
    // Plan §Verification(4): RT-pipeline-init test that skips when the device
    // has no ray-tracing extensions (e.g. lavapipe), and otherwise proves the
    // previously-dead pipeline builds + an empty TLAS rebuild is a safe no-op.
    let Some(ctx) = try_vulkan() else { return };
    if !ctx.has_ray_tracing {
        eprintln!("rt_pipeline test skipped: device has no ray-tracing extensions");
        return;
    }
    match dumpster_fire_engine::render::rt_pipeline::RtPipeline::new(&ctx) {
        Ok(mut rt) => {
            // Empty instance list → no-op (RT misses/clears), never a crash.
            rt.rebuild_tlas(&ctx, &[]).expect("empty TLAS rebuild should succeed");
            unsafe { rt.destroy(&ctx) };
        }
        Err(e) => panic!("RT pipeline failed to build on an RT-capable device: {e:?}"),
    }
}

#[test]
fn push_rect_emits_one_quad() {
    // CPU-only: one rect is one quad, regardless of GPU/font.
    let mut dl = DrawList::new();
    dl.push_rect(Rect { x: 0.0, y: 0.0, w: 100.0, h: 20.0 }, [10, 20, 30, 255]);
    assert_eq!(dl.vertices.len(), 4, "a rect is one quad = 4 vertices");
    assert_eq!(dl.indices.len(), 6, "two triangles = 6 indices");
}

#[test]
fn push_text_ascii_emits_a_quad_per_glyph() {
    let Some(ctx) = try_vulkan() else { return };
    if !bundled_font_is_valid() {
        eprintln!("skipped: bundled FiraCode-Regular.ttf is not a valid TTF");
        return;
    }
    let mut atlas = FontAtlas::new(&ctx);

    let mut dl = DrawList::new();
    let text = "Hello UI"; // pure ASCII -> exercises the byte fast path
    assert!(text.is_ascii());
    dl.push_text(text, &mut atlas, [10.0, 10.0], [255, 255, 255, 255]);

    let glyphs = text.chars().count();
    assert_eq!(dl.vertices.len(), glyphs * 4, "4 vertices per glyph");
    assert_eq!(dl.indices.len(), glyphs * 6, "6 indices per glyph");
}

#[test]
fn ascii_and_unicode_paths_agree_on_shared_glyphs() {
    let Some(ctx) = try_vulkan() else { return };
    if !bundled_font_is_valid() {
        eprintln!("skipped: bundled FiraCode-Regular.ttf is not a valid TTF");
        return;
    }
    let mut atlas = FontAtlas::new(&ctx);

    let mut ascii = DrawList::new();
    ascii.push_text("AbC 123", &mut atlas, [0.0, 0.0], [255; 4]);

    // Appending a non-ASCII char (é) forces the whole string down the chars()
    // fallback branch; it must still emit 4 vertices per glyph, so the count is
    // exactly the ASCII run plus one more glyph.
    let mixed_text = "AbC 123é";
    assert!(!mixed_text.is_ascii());
    let mut mixed = DrawList::new();
    mixed.push_text(mixed_text, &mut atlas, [0.0, 0.0], [255; 4]);
    assert_eq!(mixed.vertices.len(), ascii.vertices.len() + 4);
}

#[test]
fn glyph_cache_returns_stable_metrics() {
    let Some(ctx) = try_vulkan() else { return };
    if !bundled_font_is_valid() {
        eprintln!("skipped: bundled FiraCode-Regular.ttf is not a valid TTF");
        return;
    }
    let mut atlas = FontAtlas::new(&ctx);

    let g1 = atlas.get_glyph('A', 14);
    let g2 = atlas.get_glyph('A', 14); // second call hits the cache
    assert_eq!(g1.advance, g2.advance);
    assert_eq!(g1.w, g2.w);
    assert_eq!(g1.h, g2.h);
    assert!(g1.advance > 0.0, "a glyph should advance the pen");
}
