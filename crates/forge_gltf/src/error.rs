//! Error surface used by every loader entry point.

use std::fmt;

#[derive(Debug)]
pub enum GltfError {
    Io(gltf::Error),
    NoPrimitives,
    NoPositions,
    InvalidAccessor(&'static str),
    UnsupportedComponent(&'static str),
    /// The file declares `asset.version` that is not `"2.*"`, or
    /// `asset.minVersion` is higher than 2.0.
    UnsupportedVersion(String),
    /// A required extension listed in `extensionsRequired` is not implemented.
    UnsupportedExtension(String),
    /// The file violates a normative MUST in the glTF 2.0 spec.
    SpecViolation(String),
    /// An optional feature (e.g. KTX2 zstd supercompression) is not yet
    /// implemented; loading continues with a degraded path.
    UnsupportedFeature(String),
}

impl fmt::Display for GltfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GltfError::Io(e)                    => write!(f, "gltf I/O error: {e}"),
            GltfError::NoPrimitives             => write!(f, "glTF file has no mesh primitives"),
            GltfError::NoPositions              => write!(f, "glTF primitive has no POSITION accessor"),
            GltfError::InvalidAccessor(s)       => write!(f, "glTF invalid accessor: {s}"),
            GltfError::UnsupportedComponent(s)  => write!(f, "glTF unsupported component type: {s}"),
            GltfError::UnsupportedVersion(s)    => write!(f, "glTF unsupported version: {s}"),
            GltfError::UnsupportedExtension(s)  => write!(f, "glTF required extension not supported: {s}"),
            GltfError::SpecViolation(s)         => write!(f, "glTF spec violation: {s}"),
            GltfError::UnsupportedFeature(s)    => write!(f, "glTF feature not yet implemented: {s}"),
        }
    }
}

impl std::error::Error for GltfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self { GltfError::Io(e) => Some(e), _ => None }
    }
}

impl From<gltf::Error> for GltfError {
    fn from(e: gltf::Error) -> Self { GltfError::Io(e) }
}

pub type GltfResult<T> = Result<T, GltfError>;
