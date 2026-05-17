//! KHR_gaussian_splatting extension data.
//!
//! Each "splat set" stores a buffer of structured records — position
//! (`vec3`), scale (`vec3`), rotation (`vec4` quat), colour (`vec4`),
//! opacity (`f32`), and up to 16 spherical-harmonics coefficient vectors
//! (`vec3` each). Renderers turn each record into a screen-space billboard
//! quad with EWA-filtered alpha so the splat's covariance ellipsoid
//! shades correctly through the camera.
//!
//! The extension is a Khronos baseline (Feb 2026) — ratification is
//! pending. Parsing the data and surfacing it via `GltfAsset` is enough
//! for downstream renderers to begin work; the actual sort+billboard+
//! compositing pipeline lives in the engine bridge.

use thin_vec::ThinVec;

/// One Gaussian splat set referenced by the asset's primary scene.
/// Every component-stream is the same length; absent streams are empty
/// (consumers fill defaults).
#[derive(Debug, Clone, Default)]
pub struct GaussianSplatSet {
    /// Optional friendly name from the glTF JSON.
    pub name: Option<String>,
    /// Per-splat world-space anchor positions.
    pub positions: ThinVec<[f32; 3]>,
    /// Per-splat covariance scales (one per axis).
    pub scales: ThinVec<[f32; 3]>,
    /// Per-splat orientation quaternions (x, y, z, w).
    pub rotations: ThinVec<[f32; 4]>,
    /// Per-splat base colour (premultiplied RGBA).
    pub colors: ThinVec<[f32; 4]>,
    /// Per-splat single-channel opacity. Multiplied with `colors.a` at
    /// composite time; the spec keeps it separate so authoring tools can
    /// edit it independently of the colour stream.
    pub opacities: ThinVec<f32>,
    /// Spherical-harmonics coefficients. Outer length = up to 16 (the
    /// SH-3 band count); inner length = `positions.len()`; each entry is
    /// one `vec3` directional radiance contribution per splat. Empty
    /// outer means "no SH" (lambertian-only splat).
    pub sh: ThinVec<ThinVec<[f32; 3]>>,
}

impl GaussianSplatSet {
    /// Number of splats — derived from the longest populated stream so a
    /// partial set (positions only, say) still reports correctly.
    pub fn len(&self) -> usize {
        self.positions.len()
            .max(self.scales.len())
            .max(self.rotations.len())
            .max(self.colors.len())
            .max(self.opacities.len())
    }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

/// Extract `KHR_gaussian_splatting` sets from a glTF document. Each
/// matching node carries a `splat` extension with attribute-style
/// accessor references; we walk those and pull the per-stream values.
/// External-buffer reads use `buffer_data` so this works for both `.glb`
/// (everything inline) and `.gltf` + sibling `.bin` files.
pub fn extract_splats(
    doc:         &gltf::Document,
    buffer_data: &[gltf::buffer::Data],
) -> ThinVec<GaussianSplatSet> {
    let mut out = ThinVec::new();
    for node in doc.nodes() {
        let Some(ext) = node.extension_value("KHR_gaussian_splatting") else { continue };
        let Some(obj) = ext.as_object() else { continue };
        let Some(attrs) = obj.get("attributes").and_then(|a| a.as_object()) else { continue };

        let mut set = GaussianSplatSet {
            name: node.name().map(str::to_owned),
            ..Default::default()
        };

        if let Some(idx) = attrs.get("POSITION").and_then(|v| v.as_u64()) {
            set.positions = read_vec3_accessor(doc, buffer_data, idx as usize);
        }
        if let Some(idx) = attrs.get("_SCALE").and_then(|v| v.as_u64()) {
            set.scales = read_vec3_accessor(doc, buffer_data, idx as usize);
        }
        if let Some(idx) = attrs.get("_ROTATION").and_then(|v| v.as_u64()) {
            set.rotations = read_vec4_accessor(doc, buffer_data, idx as usize);
        }
        if let Some(idx) = attrs.get("_COLOR").and_then(|v| v.as_u64()) {
            set.colors = read_vec4_accessor(doc, buffer_data, idx as usize);
        }
        if let Some(idx) = attrs.get("_OPACITY").and_then(|v| v.as_u64()) {
            set.opacities = read_f32_accessor(doc, buffer_data, idx as usize);
        }
        for sh_band in 0..16 {
            let key = format!("_SH{sh_band}");
            if let Some(idx) = attrs.get(&key).and_then(|v| v.as_u64()) {
                let band = read_vec3_accessor(doc, buffer_data, idx as usize);
                if !band.is_empty() {
                    set.sh.push(band);
                }
            }
        }

        if !set.is_empty() { out.push(set); }
    }
    out
}

// ─── Accessor helpers ──────────────────────────────────────────────────────
//
// glTF accessors point into buffer views which point into buffers. The
// gltf crate's `Reader` API would normally handle this, but it specializes
// on primitive types not raw accessor reads; for the splat extension's
// custom attribute names we walk the offsets ourselves.

fn accessor_bytes<'a>(
    doc:    &gltf::Document,
    bufs:   &'a [gltf::buffer::Data],
    idx:    usize,
) -> Option<(&'a [u8], usize, usize)> {
    // Returns (bytes, count, component_count).
    let acc = doc.accessors().nth(idx)?;
    let view = acc.view()?;
    let buf = &bufs[view.buffer().index()].0;
    let off = view.offset() + acc.offset();
    let len = view.length();
    let bytes = buf.get(off..off + len)?;
    let count = acc.count();
    let nc = match acc.dimensions() {
        gltf::accessor::Dimensions::Vec2   => 2,
        gltf::accessor::Dimensions::Vec3   => 3,
        gltf::accessor::Dimensions::Vec4   => 4,
        gltf::accessor::Dimensions::Mat2   => 4,
        gltf::accessor::Dimensions::Mat3   => 9,
        gltf::accessor::Dimensions::Mat4   => 16,
        gltf::accessor::Dimensions::Scalar => 1,
    };
    Some((bytes, count, nc))
}

fn read_f32_accessor(
    doc: &gltf::Document,
    bufs: &[gltf::buffer::Data],
    idx: usize,
) -> ThinVec<f32> {
    let Some((bytes, count, _)) = accessor_bytes(doc, bufs, idx) else { return ThinVec::new() };
    let mut out = ThinVec::with_capacity(count);
    for i in 0..count {
        let off = i * 4;
        if off + 4 > bytes.len() { break; }
        out.push(f32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]));
    }
    out
}

fn read_vec3_accessor(
    doc: &gltf::Document,
    bufs: &[gltf::buffer::Data],
    idx: usize,
) -> ThinVec<[f32; 3]> {
    let Some((bytes, count, _)) = accessor_bytes(doc, bufs, idx) else { return ThinVec::new() };
    let mut out = ThinVec::with_capacity(count);
    for i in 0..count {
        let off = i * 12;
        if off + 12 > bytes.len() { break; }
        let x = f32::from_le_bytes([bytes[off    ], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let y = f32::from_le_bytes([bytes[off + 4], bytes[off + 5], bytes[off + 6], bytes[off + 7]]);
        let z = f32::from_le_bytes([bytes[off + 8], bytes[off + 9], bytes[off +10], bytes[off +11]]);
        out.push([x, y, z]);
    }
    out
}

fn read_vec4_accessor(
    doc: &gltf::Document,
    bufs: &[gltf::buffer::Data],
    idx: usize,
) -> ThinVec<[f32; 4]> {
    let Some((bytes, count, _)) = accessor_bytes(doc, bufs, idx) else { return ThinVec::new() };
    let mut out = ThinVec::with_capacity(count);
    for i in 0..count {
        let off = i * 16;
        if off + 16 > bytes.len() { break; }
        let x = f32::from_le_bytes([bytes[off    ], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let y = f32::from_le_bytes([bytes[off + 4], bytes[off + 5], bytes[off + 6], bytes[off + 7]]);
        let z = f32::from_le_bytes([bytes[off + 8], bytes[off + 9], bytes[off +10], bytes[off +11]]);
        let w = f32::from_le_bytes([bytes[off+12], bytes[off +13], bytes[off +14], bytes[off +15]]);
        out.push([x, y, z, w]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_is_empty() {
        let set = GaussianSplatSet::default();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn set_len_uses_longest_populated_stream() {
        let mut set = GaussianSplatSet::default();
        set.positions = thin_vec::thin_vec![[0.0, 0.0, 0.0]; 5];
        set.opacities = thin_vec::thin_vec![1.0; 7]; // longer
        assert_eq!(set.len(), 7);
        assert!(!set.is_empty());
    }
}
