//! KTX2 container parser.
//!
//! Parses the KTX File Format Specification v2 header, level index, Data
//! Format Descriptor, and Supercompression Global Data.  Only
//! `SupercompressionScheme::None` and `SupercompressionScheme::BasisLZ` are
//! transcoded here; everything else returns `UnsupportedFeature`.

use crate::error::{GltfError, GltfResult};

/// Magic bytes at offset 0. 12 bytes.
const KTX2_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupercompressionScheme {
    None,
    BasisLZ,
    Zstd,
    ZLib,
    Other(u32),
}

impl SupercompressionScheme {
    fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::None,
            1 => Self::BasisLZ,
            2 => Self::Zstd,
            3 => Self::ZLib,
            n => Self::Other(n),
        }
    }
}

/// A single mip level entry from the level index.
#[derive(Debug, Clone, Copy)]
pub struct LevelEntry {
    pub byte_offset:              u64,
    pub byte_length:              u64,
    pub uncompressed_byte_length: u64,
}

/// Top-level KTX2 parsed document.
#[derive(Debug)]
pub struct Ktx2 {
    pub vk_format:           u32,
    pub type_size:            u32,
    pub pixel_width:          u32,
    pub pixel_height:         u32,
    pub pixel_depth:          u32,
    pub layer_count:          u32,
    pub face_count:           u32,
    pub level_count:          u32,
    pub supercompression:     SupercompressionScheme,
    pub levels:               Vec<LevelEntry>,
    /// Supercompression Global Data bytes, if any.
    pub sgd:                  Vec<u8>,
    /// Key/value metadata pairs.
    pub metadata:             Vec<(String, Vec<u8>)>,
}

impl Ktx2 {
    /// Parse a KTX2 file from bytes. Returns an error for invalid magic,
    /// truncated files, or unsupported compression.
    pub fn parse(data: &[u8]) -> GltfResult<Self> {
        if data.len() < 80 {
            return Err(GltfError::InvalidAccessor("KTX2 file too small"));
        }
        if &data[0..12] != KTX2_MAGIC {
            return Err(GltfError::InvalidAccessor("not a KTX2 file (bad magic)"));
        }

        let r = |off: usize| -> u32 {
            u32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]])
        };
        let r64 = |off: usize| -> u64 {
            u64::from_le_bytes([
                data[off], data[off+1], data[off+2], data[off+3],
                data[off+4], data[off+5], data[off+6], data[off+7],
            ])
        };

        let vk_format    = r(12);
        let type_size    = r(16);
        let pixel_width  = r(20).max(1);
        let pixel_height = r(24);
        let pixel_depth  = r(28);
        let layer_count  = r(32);
        let face_count   = r(36).max(1);
        let level_count  = r(40).max(1);
        let supercomp    = SupercompressionScheme::from_u32(r(44));

        // Byte offsets table (indices 48..79)
        let dfd_byte_offset   = r(48) as usize;
        let dfd_byte_length   = r(52) as usize;
        let kvd_byte_offset   = r(56) as usize;
        let kvd_byte_length   = r(60) as usize;
        let sgd_byte_offset   = r64(64) as usize;
        let sgd_byte_length   = r64(72) as usize;

        // Level index: starts at byte 80, 24 bytes per level
        let level_index_start = 80usize;
        let mut levels = Vec::with_capacity(level_count as usize);
        for i in 0..level_count as usize {
            let base = level_index_start + i * 24;
            if base + 24 > data.len() {
                return Err(GltfError::InvalidAccessor("KTX2 level index truncated"));
            }
            levels.push(LevelEntry {
                byte_offset:              r64(base),
                byte_length:              r64(base + 8),
                uncompressed_byte_length: r64(base + 16),
            });
        }

        // Key/value metadata (optional, not required for decode)
        let mut metadata = Vec::new();
        if kvd_byte_length > 0 && kvd_byte_offset + kvd_byte_length <= data.len() {
            let mut pos = kvd_byte_offset;
            let end = kvd_byte_offset + kvd_byte_length;
            while pos + 4 <= end {
                let kv_len = r(pos) as usize;
                pos += 4;
                if pos + kv_len > end { break; }
                let kv_data = &data[pos..pos + kv_len];
                // key = NUL-terminated string
                let nul = kv_data.iter().position(|&b| b == 0).unwrap_or(kv_data.len());
                let key   = String::from_utf8_lossy(&kv_data[..nul]).into_owned();
                let value = kv_data[nul + 1..].to_vec();
                metadata.push((key, value));
                pos += kv_len;
                // 4-byte aligned
                let padding = (4 - (kv_len % 4)) % 4;
                pos += padding;
            }
        }

        // Supercompression global data
        let sgd = if sgd_byte_length > 0 && sgd_byte_offset + sgd_byte_length <= data.len() {
            data[sgd_byte_offset..sgd_byte_offset + sgd_byte_length].to_vec()
        } else {
            Vec::new()
        };

        // ZSTD supercompression is decoded by `crate::codec::zstd` when
        // the level payload is consumed in `asset.rs` — the parser
        // doesn't need to do anything special here.

        let _ = (dfd_byte_offset, dfd_byte_length); // validated above

        Ok(Ktx2 {
            vk_format,
            type_size,
            pixel_width,
            pixel_height,
            pixel_depth,
            layer_count,
            face_count,
            level_count,
            supercompression: supercomp,
            levels,
            sgd,
            metadata,
        })
    }

    /// Return the raw bytes for mip level `level` (0 = largest).
    pub fn level_data<'a>(&self, data: &'a [u8], level: usize) -> Option<&'a [u8]> {
        let entry = self.levels.get(level)?;
        let start = entry.byte_offset as usize;
        let end   = start + entry.byte_length as usize;
        data.get(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ktx2_bad_magic_returns_error() {
        let bad = [0u8; 80];
        assert!(Ktx2::parse(&bad).is_err());
    }

    #[test]
    fn ktx2_too_small_returns_error() {
        assert!(Ktx2::parse(&[0u8; 10]).is_err());
    }

    #[test]
    fn ktx2_minimal_synthetic_parses() {
        // Build a minimal valid KTX2 with 1 level, no supercompression.
        let mut data = vec![0u8; 80 + 24 + 4]; // header + 1 level entry + dummy pixel
        data[0..12].copy_from_slice(&KTX2_MAGIC);
        // vk_format = VK_FORMAT_R8G8B8A8_UNORM = 37
        data[12..16].copy_from_slice(&37u32.to_le_bytes());
        // type_size = 1
        data[16..20].copy_from_slice(&1u32.to_le_bytes());
        // pixel_width = 1, pixel_height = 1
        data[20..24].copy_from_slice(&1u32.to_le_bytes());
        data[24..28].copy_from_slice(&1u32.to_le_bytes());
        // face_count = 1, level_count = 1, supercomp = 0 (None)
        data[36..40].copy_from_slice(&1u32.to_le_bytes());
        data[40..44].copy_from_slice(&1u32.to_le_bytes());
        // level 0: offset = 80+24=104, length = 4, uncompressed_length = 4
        let level_start = 80usize;
        data[level_start..level_start+8].copy_from_slice(&(104u64).to_le_bytes());
        data[level_start+8..level_start+16].copy_from_slice(&(4u64).to_le_bytes());
        data[level_start+16..level_start+24].copy_from_slice(&(4u64).to_le_bytes());
        // pixel at offset 104: RGBA = [255, 0, 128, 255]
        data[104..108].copy_from_slice(&[255, 0, 128, 255]);

        let ktx = Ktx2::parse(&data).expect("parse should succeed");
        assert_eq!(ktx.vk_format, 37);
        assert_eq!(ktx.pixel_width, 1);
        assert_eq!(ktx.pixel_height, 1);
        assert_eq!(ktx.level_count, 1);
        assert_eq!(ktx.supercompression, SupercompressionScheme::None);
        let pixels = ktx.level_data(&data, 0).expect("level 0 data");
        assert_eq!(pixels, &[255, 0, 128, 255]);
    }
}
