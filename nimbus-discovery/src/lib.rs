//! # nimbus-discovery
//!
//! Service discovery for the Nimbus RPC framework.
//!
//! This crate provides:
//! - `Resolver` trait for custom discovery backends
//! - `DnsResolver` for DNS-based service discovery (SRV records)
//! - `RegistryClient` for HTTP-based registry discovery
//! - Caching and health checking

mod resolver;

#[cfg(feature = "dns")]
mod dns;

#[cfg(feature = "registry")]
mod registry;

pub use resolver::{Endpoint, ResolveError, Resolver, StaticResolver};

#[cfg(feature = "dns")]
pub use dns::DnsResolver;

#[cfg(feature = "registry")]
pub use registry::RegistryClient;
