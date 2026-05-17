//! Hand-rolled decoders for every compressed / extension format that
//! KHR_* extensions bring into glTF 2.0.
//!
//! No external crate for any of these: see `Cargo.toml` for the policy.

pub mod basisu_etc1s;
pub mod basisu_uastc;
pub mod bc;
pub mod draco;
pub mod extensions;
pub mod ktx2;
pub mod meshopt;
pub mod mikktspace;
pub mod sparse;
pub mod webp;
