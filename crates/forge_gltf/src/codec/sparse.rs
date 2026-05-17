//! Sparse accessor walker for custom `_UNDERSCORED` vertex attributes.
//!
//! The `gltf` crate's typed readers already apply sparse overlays for the
//! built-in attribute types. For custom attributes we walk the raw accessor
//! JSON ourselves, applying the same spec algorithm:
//!
//! 1. If `accessor.bufferView` exists, copy that slice into the output buffer.
//!    Otherwise fill with zeros (spec §3.6.2.4).
//! 2. If `accessor.sparse` exists, overlay the `sparse.count` values at the
//!    positions given by `sparse.indices`.
//!
//! The output is a flat `Vec<u8>` sized `count * component_bytes * components`.

use crate::error::{GltfError, GltfResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentType {
    I8, U8, I16, U16, U32, F32,
}

impl ComponentType {
    pub fn byte_size(self) -> usize {
        match self {
            ComponentType::I8  | ComponentType::U8  => 1,
            ComponentType::I16 | ComponentType::U16 => 2,
            ComponentType::U32 | ComponentType::F32 => 4,
        }
    }

    pub fn from_gltf(ct: gltf::accessor::DataType) -> Self {
        match ct {
            gltf::accessor::DataType::I8  => ComponentType::I8,
            gltf::accessor::DataType::U8  => ComponentType::U8,
            gltf::accessor::DataType::I16 => ComponentType::I16,
            gltf::accessor::DataType::U16 => ComponentType::U16,
            gltf::accessor::DataType::U32 => ComponentType::U32,
            gltf::accessor::DataType::F32 => ComponentType::F32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementType {
    Scalar, Vec2, Vec3, Vec4, Mat2, Mat3, Mat4,
}

impl ElementType {
    pub fn component_count(self) -> usize {
        match self {
            ElementType::Scalar => 1,
            ElementType::Vec2   => 2,
            ElementType::Vec3   => 3,
            ElementType::Vec4   => 4,
            ElementType::Mat2   => 4,
            ElementType::Mat3   => 9,
            ElementType::Mat4   => 16,
        }
    }

    pub fn from_gltf(dt: gltf::accessor::Dimensions) -> Self {
        match dt {
            gltf::accessor::Dimensions::Scalar      => ElementType::Scalar,
            gltf::accessor::Dimensions::Vec2        => ElementType::Vec2,
            gltf::accessor::Dimensions::Vec3        => ElementType::Vec3,
            gltf::accessor::Dimensions::Vec4        => ElementType::Vec4,
            gltf::accessor::Dimensions::Mat2        => ElementType::Mat2,
            gltf::accessor::Dimensions::Mat3        => ElementType::Mat3,
            gltf::accessor::Dimensions::Mat4        => ElementType::Mat4,
        }
    }
}

/// Walk an accessor, applying sparse overlay if present.
/// Returns a flat byte buffer of `count * element_bytes` bytes.
pub fn resolve_accessor(
    accessor: &gltf::Accessor<'_>,
    buffers:  &[gltf::buffer::Data],
) -> GltfResult<Vec<u8>> {
    let count    = accessor.count();
    let ct       = ComponentType::from_gltf(accessor.data_type());
    let et       = ElementType::from_gltf(accessor.dimensions());
    let elem_bytes = ct.byte_size() * et.component_count();
    let total_bytes = count * elem_bytes;

    // Step 1: base data
    let mut out = vec![0u8; total_bytes];
    if let Some(view) = accessor.view() {
        let buf  = &buffers[view.buffer().index()];
        let stride = view.stride().unwrap_or(elem_bytes);
        let base = view.offset() + accessor.offset();
        for i in 0..count {
            let src = base + i * stride;
            let dst = i * elem_bytes;
            out[dst..dst + elem_bytes].copy_from_slice(&buf[src..src + elem_bytes]);
        }
    }

    // Step 2: sparse overlay
    if let Some(sparse) = accessor.sparse() {
        let sc = sparse.count();
        // Read indices
        let idx_view = sparse.indices().view();
        let idx_buf  = &buffers[idx_view.buffer().index()];
        let idx_off  = idx_view.offset() + sparse.indices().offset();
        let idx_type = sparse.indices().index_type();
        let idx_stride = match idx_type {
            gltf::accessor::sparse::IndexType::U8  => 1,
            gltf::accessor::sparse::IndexType::U16 => 2,
            gltf::accessor::sparse::IndexType::U32 => 4,
        };

        // Read values
        let val_view = sparse.values().view();
        let val_buf  = &buffers[val_view.buffer().index()];
        let val_off  = val_view.offset() + sparse.values().offset();

        for i in 0..sc {
            let raw_idx = &idx_buf[idx_off + i * idx_stride..];
            let idx = match idx_type {
                gltf::accessor::sparse::IndexType::U8  => raw_idx[0] as usize,
                gltf::accessor::sparse::IndexType::U16 => u16::from_le_bytes([raw_idx[0], raw_idx[1]]) as usize,
                gltf::accessor::sparse::IndexType::U32 => u32::from_le_bytes([raw_idx[0], raw_idx[1], raw_idx[2], raw_idx[3]]) as usize,
            };
            let src = val_off + i * elem_bytes;
            let dst = idx * elem_bytes;
            if dst + elem_bytes <= out.len() && src + elem_bytes <= val_buf.len() {
                out[dst..dst + elem_bytes].copy_from_slice(&val_buf[src..src + elem_bytes]);
            }
        }
    }

    Ok(out)
}

/// A custom (underscore-prefixed) vertex attribute resolved from an accessor.
#[derive(Debug, Clone)]
pub struct CustomAttribute {
    pub name:           String,
    pub component_type: ComponentType,
    pub element_type:   ElementType,
    pub count:          usize,
    pub data:           thin_vec::ThinVec<u8>,
    pub normalized:     bool,
}

/// Resolve a custom vertex attribute from a glTF primitive.
/// Returns `Err(SpecViolation)` if the accessor uses `U32` components
/// (forbidden for custom attributes per spec §3.7.2.1).
pub fn resolve_custom_attribute(
    name:     &str,
    accessor: &gltf::Accessor<'_>,
    buffers:  &[gltf::buffer::Data],
) -> GltfResult<CustomAttribute> {
    let ct = ComponentType::from_gltf(accessor.data_type());
    if ct == ComponentType::U32 {
        return Err(GltfError::SpecViolation(format!(
            "custom attribute `{name}` uses UNSIGNED_INT component type, which is forbidden"
        )));
    }
    let data = resolve_accessor(accessor, buffers)?;
    Ok(CustomAttribute {
        name:           name.to_owned(),
        component_type: ct,
        element_type:   ElementType::from_gltf(accessor.dimensions()),
        count:          accessor.count(),
        data:           data.into_iter().collect(),
        normalized:     accessor.normalized(),
    })
}
