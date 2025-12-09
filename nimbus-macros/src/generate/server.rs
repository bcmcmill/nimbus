//! Generate server handler.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Result;

use crate::parse::{ServiceDefinition, StreamingMode};

/// Generate the server struct and implementation.
pub fn generate_server(service: &ServiceDefinition) -> Result<TokenStream> {
    let vis = &service.vis;
    let trait_name = &service.name;
    let server_name = format_ident!("{}Server", service.name);
    let _request_enum = format_ident!("{}Request", service.name);
    let response_enum = format_ident!("{}Response", service.name);
    let archived_request = format_ident!("Archived{}Request", service.name);

    let match_arms: Vec<TokenStream> = service
        .methods
        .iter()
        .map(|method| {
            let method_name = &method.name;
            let variant_name = to_pascal_case(&method.name.to_string());
            let variant_ident = format_ident!("{}", variant_name);

            // Generate argument extraction
            let arg_names: Vec<TokenStream> = method
                .args
                .iter()
                .map(|arg| {
                    let name = &arg.name;
                    quote! { #name }
                })
                .collect();

            let arg_types: Vec<TokenStream> = method
                .args
                .iter()
                .map(|arg| {
                    let ty = &arg.ty;
                    quote! { #ty }
                })
                .collect();

            match method.streaming {
                StreamingMode::Unary => {
                    let pattern = if method.args.is_empty() {
                        quote! { #archived_request::#variant_ident }
                    } else {
                        quote! { #archived_request::#variant_ident { #(#arg_names),* } }
                    };

                    // Need to deserialize archived args
                    let deserialized_args: Vec<TokenStream> = method
                        .args
                        .iter()
                        .zip(arg_types.iter())
                        .map(|(arg, ty)| {
                            let name = &arg.name;
                            quote! {
                                let #name: #ty = rkyv::deserialize(#name)
                                    .map_err(|e| nimbus_core::NimbusError::InvalidRequest(e.to_string()))?;
                            }
                        })
                        .collect();

                    let call = if method.args.is_empty() {
                        quote! { self.service.#method_name().await }
                    } else {
                        quote! { self.service.#method_name(#(#arg_names),*).await }
                    };

                    quote! {
                        #pattern => {
                            #(#deserialized_args)*
                            let result = #call;
                            #response_enum::#variant_ident(result)
                        }
                    }
                }
                StreamingMode::ServerStream => {
                    let item_ident = format_ident!("{}Item", variant_name);
                    let _end_ident = format_ident!("{}End", variant_name);

                    let pattern = if method.args.is_empty() {
                        quote! { #archived_request::#variant_ident }
                    } else {
                        quote! { #archived_request::#variant_ident { #(#arg_names),* } }
                    };

                    // For streaming, we'd need to return a stream
                    // This is a simplified version
                    quote! {
                        #pattern => {
                            // Streaming response - simplified for now
                            // In full implementation, this would yield multiple items
                            let _ = (#(#arg_names),*);
                            #response_enum::#item_ident(Err(Default::default()))
                        }
                    }
                }
                _ => {
                    // Placeholder for other streaming modes
                    let pattern = if method.args.is_empty() {
                        quote! { #archived_request::#variant_ident }
                    } else {
                        quote! { #archived_request::#variant_ident { .. } }
                    };

                    quote! {
                        #pattern => {
                            todo!("Streaming mode not yet implemented")
                        }
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        /// RPC server handler for #trait_name.
        #vis struct #server_name<S> {
            service: S,
        }

        impl<S> #server_name<S>
        where
            S: #trait_name + Send + Sync + 'static,
        {
            /// Create a new server with the given service implementation.
            pub fn new(service: S) -> Self {
                Self { service }
            }

            /// Handle a raw request (for integration with transport layer).
            pub async fn handle_raw(
                &self,
                request_bytes: &[u8],
            ) -> Result<Vec<u8>, nimbus_core::NimbusError> {
                // Access the archived request (zero-copy)
                let archived = rkyv::access::<#archived_request, rkyv::rancor::Error>(request_bytes)
                    .map_err(|e| nimbus_core::NimbusError::InvalidRequest(e.to_string()))?;

                // Dispatch to the appropriate method
                let response = self.handle_archived(archived).await?;

                // Serialize response
                let response_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&response)
                    .map_err(|e| nimbus_core::CodecError::Serialization(e.to_string()))?;

                Ok(response_bytes.to_vec())
            }

            /// Handle an archived request.
            async fn handle_archived(
                &self,
                request: &#archived_request,
            ) -> Result<#response_enum, nimbus_core::NimbusError> {
                let response = match request {
                    #(#match_arms)*
                };
                Ok(response)
            }
        }

        impl<S> nimbus_transport::RequestHandler for #server_name<S>
        where
            S: #trait_name + Send + Sync + 'static,
        {
            async fn handle(
                &self,
                request: rkyv::util::AlignedVec,
            ) -> Result<Vec<u8>, nimbus_core::NimbusError> {
                self.handle_raw(&request).await
            }
        }
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
