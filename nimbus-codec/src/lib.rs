//! # nimbus-codec
//!
//! rkyv-based codec for zero-copy RPC message framing.
//!
//! This crate provides:
//! - `NimbusCodec` - Length-prefixed frame encoder/decoder
//! - `AlignedBufferPool` - Pool for aligned buffers (required for rkyv zero-copy)
//!
//! ## Frame Format
//!
//! ```text
//! +----------------+------------------+
//! | Length (4 LE)  | Payload (N bytes)|
//! +----------------+------------------+
//! ```
//!
//! The length is a 32-bit little-endian integer specifying the payload size.

mod aligned;
mod frame;

pub use aligned::{AlignedBufferPool, PooledBuffer};
pub use frame::NimbusCodec;

// Re-export for convenience
pub use rkyv::util::AlignedVec;
