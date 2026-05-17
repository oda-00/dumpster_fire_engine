//! Pre-import GLB / glTF JSON rewriter.
//!
//! The `gltf` crate's deserializer requires every animation channel target
//! to have a numeric `node`. `KHR_animation_pointer` removes that — the
//! channel points at a JSON pointer string instead — so files using it
//! deserialize-fail with `missing field 'node'`.
//!
//! This module patches the GLB's JSON chunk: every channel that carries a
//! `KHR_animation_pointer` extension but no `target.node` gets a sentinel
//! `node: 0` injected. The pointer string itself is preserved in the
//! channel's `extensions` map, so the asset loader can fish it out later
//! and stash it in `Animation.pointer_channels` (instead of treating it
//! as a normal node-targeted channel).

use serde_json::{Map, Value};
use thin_vec::ThinVec;

use crate::animation::AnimPointerChannel;

const GLB_MAGIC: u32 = 0x46546C67;
const GLB_VERSION: u32 = 2;
const CHUNK_JSON: u32 = 0x4E4F534A;
const CHUNK_BIN:  u32 = 0x004E4942;

/// Per-animation rewrite output. `pointers` lists the channels that got
/// patched as `KHR_animation_pointer` ones; `patched_channel_indices` lists
/// the original channel positions (so the asset loader can skip them when
/// building the regular `AnimChannel` list).
#[derive(Debug, Clone, Default)]
pub struct PointerPatch {
    pub pointers:                ThinVec<AnimPointerChannel>,
    pub patched_channel_indices: ThinVec<u32>,
}

/// Returns `Some(new_glb_bytes, per_animation_patch)` if any channel was
/// patched, otherwise `None`.
pub fn rewrite_animation_pointer(
    bytes: &[u8],
) -> Option<(Vec<u8>, Vec<PointerPatch>)> {
    if bytes.len() < 28 { return None; }
    if u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != GLB_MAGIC {
        return rewrite_plain_json(bytes);
    }
    if u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != GLB_VERSION {
        return None;
    }

    let json_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    if u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) != CHUNK_JSON {
        return None;
    }
    let json_start = 20;
    let json_end   = json_start + json_len;
    if json_end > bytes.len() { return None; }
    let json_str = std::str::from_utf8(&bytes[json_start..json_end]).ok()?;

    let (patched_json, pointers) = patch_json(json_str)?;

    // BIN chunk (optional) starts right after the JSON chunk header.
    let bin_payload: Option<&[u8]> = if json_end + 8 <= bytes.len() {
        let bin_len = u32::from_le_bytes([
            bytes[json_end], bytes[json_end + 1], bytes[json_end + 2], bytes[json_end + 3],
        ]) as usize;
        let bin_kind = u32::from_le_bytes([
            bytes[json_end + 4], bytes[json_end + 5], bytes[json_end + 6], bytes[json_end + 7],
        ]);
        if bin_kind == CHUNK_BIN {
            let start = json_end + 8;
            let end   = start + bin_len;
            if end <= bytes.len() { Some(&bytes[start..end]) } else { None }
        } else { None }
    } else { None };

    let mut json_bytes = patched_json.into_bytes();
    while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }

    let bin_total = bin_payload.map(|p| {
        let mut n = p.len();
        while n % 4 != 0 { n += 1; }
        n
    }).unwrap_or(0);

    let total = 12 + 8 + json_bytes.len() + if bin_payload.is_some() { 8 + bin_total } else { 0 };

    let mut out = Vec::with_capacity(total);
    let push = |o: &mut Vec<u8>, v: u32| o.extend_from_slice(&v.to_le_bytes());
    push(&mut out, GLB_MAGIC);
    push(&mut out, GLB_VERSION);
    push(&mut out, total as u32);
    push(&mut out, json_bytes.len() as u32);
    push(&mut out, CHUNK_JSON);
    out.extend_from_slice(&json_bytes);
    if let Some(bin) = bin_payload {
        push(&mut out, bin_total as u32);
        push(&mut out, CHUNK_BIN);
        out.extend_from_slice(bin);
        while out.len() < total { out.push(0); }
    }

    Some((out, pointers))
}

/// Non-GLB JSON path — used by `import` over a `.gltf` file. We just rewrite
/// the JSON string in place and let the caller swap the bytes.
fn rewrite_plain_json(bytes: &[u8]) -> Option<(Vec<u8>, Vec<PointerPatch>)> {
    let s = std::str::from_utf8(bytes).ok()?;
    let (patched, pointers) = patch_json(s)?;
    Some((patched.into_bytes(), pointers))
}

fn patch_json(json_str: &str) -> Option<(String, Vec<PointerPatch>)> {
    let mut v: Value = serde_json::from_str(json_str).ok()?;
    let mut per_anim: Vec<PointerPatch> = Vec::new();
    let mut any_patched = false;

    let Some(anims) = v.get_mut("animations").and_then(|v| v.as_array_mut()) else {
        return None;
    };

    for anim in anims.iter_mut() {
        let mut patch = PointerPatch::default();
        let Some(channels) = anim.get_mut("channels").and_then(|v| v.as_array_mut()) else {
            per_anim.push(patch);
            continue;
        };
        for (ch_idx, ch) in channels.iter_mut().enumerate() {
            let Some(ch_obj) = ch.as_object_mut() else { continue };
            let sampler = ch_obj.get("sampler").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

            let Some(target) = ch_obj.get_mut("target") else { continue };
            let Some(t_obj) = target.as_object_mut() else { continue };

            if t_obj.contains_key("node") { continue; }

            let ptr = t_obj
                .get("extensions")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("KHR_animation_pointer"))
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("pointer"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());

            let Some(ptr) = ptr else { continue };
            t_obj.insert("node".to_owned(), Value::from(0u64));
            // The path on a KHR_animation_pointer channel is "pointer",
            // which fails the gltf crate's enum validation when we later
            // read it back. Force it to a benign variant; the channel is
            // routed via `pointer_channels` regardless, and the loader
            // strips it from the regular channel list.
            t_obj.insert("path".to_owned(), Value::from("translation"));
            any_patched = true;
            patch.pointers.push(AnimPointerChannel { pointer: ptr, sampler });
            patch.patched_channel_indices.push(ch_idx as u32);
        }
        per_anim.push(patch);
    }

    if !any_patched { return None; }
    Some((serde_json::to_string(&v).ok()?, per_anim))
}

/// Pluck `Map<String, Value>` from the root extensions for inspection — used
/// by tests / tooling that want to peek without re-parsing the file.
pub fn root_extensions(json_str: &str) -> Option<Map<String, Value>> {
    let v: Value = serde_json::from_str(json_str).ok()?;
    v.get("extensions").and_then(|v| v.as_object()).cloned()
}
