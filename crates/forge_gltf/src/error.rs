//! Error surface used by every loader entry point. Wraps `gltf::Error` and
//! adds variants for the structural failures we surface (missing primitives,
//! missing required attributes, out-of-range accessor references).

use std::fmt;

#[derive(Debug)]
pub enum GltfError {
    Io(gltf::Error),
    NoPrimitives,
    NoPositions,
    InvalidAccessor(&'static str),
    UnsupportedComponent(&'static str),
}

impl fmt::Display for GltfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GltfError::Io(e)                 => write!(f, "gltf I/O error: {e}"),
            GltfError::NoPrimitives          => write!(f, "glTF file has no mesh primitives"),
            GltfError::NoPositions           => write!(f, "glTF primitive has no POSITION accessor"),
            GltfError::InvalidAccessor(s)    => write!(f, "glTF invalid accessor: {s}"),
            GltfError::UnsupportedComponent(s) => {
                write!(f, "glTF unsupported component type: {s}")
            }
        }
    }
}

impl std::error::Error for GltfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GltfError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<gltf::Error> for GltfError {
    fn from(e: gltf::Error) -> Self { GltfError::Io(e) }
}

pub type GltfResult<T> = Result<T, GltfError>;
