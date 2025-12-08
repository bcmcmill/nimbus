//! Generate client stub.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Result;

use crate::parse::{ServiceArgs, ServiceDefinition, StreamingMode};

/// Generate the client struct and implementation.
pub fn generate_client(service: &ServiceDefinition, _args: &ServiceArgs) -> Result<TokenStream> {
    let vis = &service.vis;
    let trait_name = &service.name;
    let client_name = format_ident!("{}Client", service.name);
    let request_enum = format_ident!("{}Request", service.name);
    let response_enum = format_ident!("{}Response", service.name);

    let methods: Vec<TokenStream> = service
        .methods
        .iter()
        .map(|method| {
            let method_name = &method.name;
            let variant_name = to_pascal_case(&method.name.to_string());
            let variant_ident = format_ident!("{}", variant_name);

            let ok_type = &method.return_type.ok_type;
            let err_type = &method.return_type.err_type;

            // Generate argument list for method signature
            let args: Vec<TokenStream> = method
                .args
                .iter()
                .map(|arg| {
                    let name = &arg.name;
                    let ty = &arg.ty;
                    quote! { #name: #ty }
                })
                .collect();

            // Generate argument names for request construction
            let arg_names: Vec<TokenStream> = method
                .args
                .iter()
                .map(|arg| {
                    let name = &arg.name;
                    quote! { #name }
                })
                .collect();

            match method.streaming {
                StreamingMode::Unary => {
                    let request_construct = if method.args.is_empty() {
                        quote! { #request_enum::#variant_ident }
                    } else {
                        quote! { #request_enum::#variant_ident { #(#arg_names),* } }
                    };

                    quote! {
                        /// Call the #method_name RPC.
                        pub async fn #method_name(
                            &self,
                            ctx: nimbus_core::Context,
                            #(#args),*
                        ) -> Result<#ok_type, ClientError<#err_type>> {
                            let request = #request_construct;

                            // Serialize request
                            let request_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&request)
                                .map_err(|e| ClientError::Serialization(e.to_string()))?;

                            // Send via transport
                            let response_bytes = self.transport.call(&ctx, &request_bytes).await
                                .map_err(ClientError::Transport)?;

                            // Deserialize response
                            let archived = rkyv::access::<
                                <#response_enum as rkyv::Archive>::Archived,
                                rkyv::rancor::Error
                            >(&response_bytes)
                                .map_err(|e| ClientError::Deserialization(e.to_string()))?;

                            // Extract the result
                            match archived {
                                ArchivedResponse::#variant_ident(result) => {
                                    // Need to deserialize from archived
                                    let deserialized: Result<#ok_type, #err_type> =
                                        rkyv::deserialize::<Result<#ok_type, #err_type>, rkyv::rancor::Error>(result)
                                            .map_err(|e| ClientError::Deserialization(e.to_string()))?;
                                    deserialized.map_err(ClientError::Service)
                                }
                                _ => Err(ClientError::Protocol("unexpected response variant".into())),
                            }
                        }
                    }
                }
                StreamingMode::ServerStream => {
                    let item_ident = format_ident!("{}Item", variant_name);
                    let end_ident = format_ident!("{}End", variant_name);

                    let request_construct = if method.args.is_empty() {
                        quote! { #request_enum::#variant_ident }
                    } else {
                        quote! { #request_enum::#variant_ident { #(#arg_names),* } }
                    };

                    quote! {
                        /// Call the streaming #method_name RPC.
                        pub fn #method_name(
                            &self,
                            ctx: nimbus_core::Context,
                            #(#args),*
                        ) -> impl futures_core::Stream<Item = Result<#ok_type, ClientError<#err_type>>> + '_ {
                            async_stream::try_stream! {
                                let request = #request_construct;

                                let request_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&request)
                                    .map_err(|e| ClientError::Serialization(e.to_string()))?;

                                let stream = self.transport.open_stream(&ctx).await
                                    .map_err(ClientError::Transport)?;

                                // Send initial request
                                stream.sender.send(request_bytes.to_vec()).await
                                    .map_err(ClientError::Transport)?;

                                // Receive stream items
                                loop {
                                    match stream.receiver.recv().await.map_err(ClientError::Transport)? {
                                        Some(bytes) => {
                                            let archived = rkyv::access::<
                                                <#response_enum as rkyv::Archive>::Archived,
                                                rkyv::rancor::Error
                                            >(&bytes)
                                                .map_err(|e| ClientError::Deserialization(e.to_string()))?;

                                            match archived {
                                                ArchivedResponse::#item_ident(result) => {
                                                    let item: Result<#ok_type, #err_type> =
                                                        rkyv::deserialize(result)
                                                            .map_err(|e| ClientError::Deserialization(e.to_string()))?;
                                                    yield item.map_err(ClientError::Service)?;
                                                }
                                                ArchivedResponse::#end_ident => break,
                                                _ => Err(ClientError::Protocol("unexpected response".into()))?,
                                            }
                                        }
                                        None => break,
                                    }
                                }
                            }
                        }
                    }
                }
                StreamingMode::ClientStream | StreamingMode::Bidirectional => {
                    // For now, generate a TODO placeholder
                    quote! {
                        /// Streaming #method_name RPC (not yet fully implemented).
                        pub async fn #method_name(
                            &self,
                            _ctx: nimbus_core::Context,
                            #(#args),*
                        ) -> Result<#ok_type, ClientError<#err_type>> {
                            todo!("Client streaming not yet implemented")
                        }
                    }
                }
            }
        })
        .collect();

    let archived_response = format_ident!("Archived{}Response", service.name);

    Ok(quote! {
        /// Client error type.
        #[derive(Debug)]
        #vis enum ClientError<E> {
            /// Transport error.
            Transport(nimbus_core::NimbusError),
            /// Service returned an error.
            Service(E),
            /// Serialization error.
            Serialization(String),
            /// Deserialization error.
            Deserialization(String),
            /// Protocol error.
            Protocol(String),
        }

        impl<E: std::fmt::Display> std::fmt::Display for ClientError<E> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    Self::Transport(e) => write!(f, "transport error: {}", e),
                    Self::Service(e) => write!(f, "service error: {}", e),
                    Self::Serialization(e) => write!(f, "serialization error: {}", e),
                    Self::Deserialization(e) => write!(f, "deserialization error: {}", e),
                    Self::Protocol(e) => write!(f, "protocol error: {}", e),
                }
            }
        }

        impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for ClientError<E> {}

        /// RPC client for #trait_name.
        #vis struct #client_name<T> {
            transport: T,
        }

        impl<T> #client_name<T>
        where
            T: nimbus_core::Transport<Error = nimbus_core::NimbusError>,
        {
            /// Create a new client with the given transport.
            pub fn new(transport: T) -> Self {
                Self { transport }
            }

            /// Get a reference to the underlying transport.
            pub fn transport(&self) -> &T {
                &self.transport
            }

            // Use type alias for archived response
            type ArchivedResponse = #archived_response;

            #(#methods)*
        }

        // Type alias for archived response (used in methods)
        type ArchivedResponse = <#response_enum as rkyv::Archive>::Archived;
    })
}

/// Convert snake_case to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}
