//! # nimbus-macros
//!
//! Proc macros for the Nimbus RPC framework.
//!
//! This crate provides the `#[service]` attribute macro for defining RPC services.
//!
//! ## Overview
//!
//! The `#[service]` macro transforms a trait into a complete RPC service with:
//! - Request/Response enums (rkyv-serializable)
//! - Client stub with async methods
//! - Server handler for dispatching requests
//!
//! ## Generated Items
//!
//! For a trait named `MyService`, the macro generates:
//! - `MyServiceRequest` - Enum of all request variants
//! - `MyServiceResponse` - Enum of all response variants
//! - `MyServiceClient<T>` - Client stub
//! - `MyServiceServer<S>` - Server handler

mod generate;
mod parse;

use proc_macro::TokenStream;
use syn::{ItemTrait, parse_macro_input};

/// Define an RPC service from a trait.
///
/// ## Usage
///
/// Apply this attribute to a trait to generate RPC infrastructure.
/// The macro generates request/response enums and client/server stubs.
///
/// ## Generated Items
///
/// For a trait named `MyService`, the following items are generated:
///
/// - `MyServiceRequest` - Enum of all request variants (rkyv-serializable)
/// - `MyServiceResponse` - Enum of all response variants (rkyv-serializable)
/// - `MyServiceClient<T>` - Client stub implementing the trait methods
/// - `MyServiceServer<S>` - Server handler that dispatches to trait implementation
///
/// ## Streaming Methods
///
/// Streaming methods can be declared with attributes:
/// - `#[server_stream]` - Server returns multiple responses
/// - `#[client_stream]` - Client sends multiple requests
/// - `#[bidirectional]` - Both sides stream
#[proc_macro_attribute]
pub fn service(attr: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse::ServiceArgs::parse(attr.into());
    let item = parse_macro_input!(input as ItemTrait);

    match generate::generate_service(args, item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
