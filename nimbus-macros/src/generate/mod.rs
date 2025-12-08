//! Code generation for the service macro.

mod client;
mod request;
mod server;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemTrait, Result};

use crate::parse::{ServiceArgs, ServiceDefinition};

/// Generate all code for a service trait.
pub fn generate_service(args: ServiceArgs, item: ItemTrait) -> Result<TokenStream> {
    let service = ServiceDefinition::parse(item)?;

    let original_trait = generate_trait(&service);
    let request_enum = request::generate_request_enum(&service)?;
    let response_enum = request::generate_response_enum(&service)?;
    let client = client::generate_client(&service, &args)?;
    let server = server::generate_server(&service)?;

    Ok(quote! {
        #original_trait
        #request_enum
        #response_enum
        #client
        #server
    })
}

/// Re-emit the original trait with minor modifications.
fn generate_trait(service: &ServiceDefinition) -> TokenStream {
    let vis = &service.vis;
    let name = &service.name;
    let items = &service.original.items;
    let attrs = &service.original.attrs;

    quote! {
        #(#attrs)*
        #vis trait #name: Send + Sync {
            #(#items)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::ServiceArgs;

    #[test]
    fn test_generate_service() {
        let item: ItemTrait = syn::parse_quote! {
            pub trait Calculator {
                async fn add(&self, a: i32, b: i32) -> Result<i32, CalcError>;
            }
        };

        let result = generate_service(ServiceArgs::default(), item);
        assert!(result.is_ok());
    }
}
