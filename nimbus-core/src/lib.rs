//! # nimbus-core
//!
//! Core types, traits, and error definitions for the Nimbus RPC framework.
//!
//! This crate provides:
//! - Error types (`NimbusError`, `TransportError`, `CodecError`)
//! - Request context (`Context`)
//! - Transport trait definitions
//! - RPC message envelope types

mod context;
mod error;
mod message;
mod transport;

pub use context::{Context, Metadata, TraceId};
pub use error::{CodecError, NimbusError, TransportError};
pub use message::{ArchivedMessageType, ArchivedRpcEnvelope, MessageType, RpcEnvelope};
pub use transport::{StreamHandle, Transport};
