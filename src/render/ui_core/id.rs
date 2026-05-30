use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU32;

use thin_vec::ThinVec;

use crate::resource_manager::manager::{Arena, Handle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetTag;
pub type WidgetId = Handle<WidgetTag>;
pub type WidgetArena = Arena<WidgetTag, super::widget::Widget>;

/// Stable path-based identity for immediate-mode widget lookup.
///
/// Each segment is a `&'static str` to avoid allocation during frame traversal.
/// The full path is only materialised into a `String` when doing a HashMap lookup.
#[derive(Clone, Debug, Default)]
pub struct WidgetIdPath(pub ThinVec<&'static str>);

impl WidgetIdPath {
    pub fn push(&mut self, seg: &'static str) {
        self.0.push(seg);
    }

    pub fn pop(&mut self) {
        self.0.pop();
    }
}

impl fmt::Display for WidgetIdPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            f.write_str(seg)?;
        }
        Ok(())
    }
}

impl WidgetIdPath {
    /// Stable integer identity for this path — the immediate-mode widget key.
    #[inline]
    pub fn key(&self) -> u64 {
        path_key(&self.0, "")
    }
}

/// FNV-1a hash of `stack` segments followed by a final `extra` segment, joined
/// by `/`. This is the immediate-mode widget identity key. Using a `u64` key
/// (stored in a sorted `ThinVec` looked up via `partition_point`) avoids the
/// per-frame `String` path allocation the builder used to pay for every widget
/// (see GUI_research.md §4.2 / docs/gui_research/asm/exp2_arena.rs). When `extra`
/// is empty it is omitted, so `path_key(stack, "")` hashes `stack` alone.
#[inline]
pub fn path_key(stack: &[&str], extra: &str) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    #[inline]
    fn mix(mut h: u64, bytes: &[u8]) -> u64 {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
        h
    }
    let mut h = OFFSET;
    let mut first = true;
    for seg in stack {
        if !first {
            h = mix(h, b"/");
        }
        h = mix(h, seg.as_bytes());
        first = false;
    }
    if !extra.is_empty() {
        if !first {
            h = mix(h, b"/");
        }
        h = mix(h, extra.as_bytes());
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_key_matches_joined_segments() {
        // key(stack + [name]) must equal hashing the full joined path once.
        let stack = ["root", "panel"];
        let combined = path_key(&["root", "panel", "button"], "");
        let incremental = path_key(&stack, "button");
        assert_eq!(combined, incremental);
    }

    #[test]
    fn path_key_distinguishes_paths() {
        // Same scope, different widget names → different keys.
        assert_ne!(path_key(&["panel"], "ok"), path_key(&["panel"], "cancel"));
        // Different scopes, same name → different keys.
        assert_ne!(path_key(&["a"], "x"), path_key(&["b"], "x"));
        // Note: path_key(&["a"], "b") == path_key(&["a","b"], "") by design —
        // the final segment is folded in identically (see the roundtrip test).
    }

    #[test]
    fn widget_id_path_key_uses_segments() {
        let p = WidgetIdPath({
            let mut v = ThinVec::new();
            v.push("root");
            v.push("panel");
            v
        });
        assert_eq!(p.key(), path_key(&["root", "panel"], ""));
    }
}
