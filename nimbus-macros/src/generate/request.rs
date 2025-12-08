//! Generate request and response enums.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Result;

use crate::parse::{ServiceDefinition, StreamingMode};

/// Generate the request enum for a service.
pub fn generate_request_enum(service: &ServiceDefinition) -> Result<TokenStream> {
    let vis = &service.vis;
    let enum_name = format_ident!("{}Request", service.name);

    let variants: Vec<TokenStream> = service
        .methods
        .iter()
        .map(|method| {
            let variant_name = to_pascal_case(&method.name.to_string());
            let variant_ident = format_ident!("{}", variant_name);

            if method.args.is_empty() {
                quote! { #variant_ident }
            } else {
                let fields: Vec<TokenStream> = method
                    .args
                    .iter()
                    .map(|arg| {
                        let name = &arg.name;
                        let ty = &arg.ty;
                        quote! { #name: #ty }
                    })
                    .collect();

                quote! {
                    #variant_ident {
                        #(#fields),*
                    }
                }
            }
        })
        .collect();

    Ok(quote! {
        /// Request enum for RPC calls.
        #[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
        #[rkyv(derive(Debug))]
        #vis enum #enum_name {
            #(#variants),*
        }
    })
}

/// Generate the response enum for a service.
pub fn generate_response_enum(service: &ServiceDefinition) -> Result<TokenStream> {
    let vis = &service.vis;
    let enum_name = format_ident!("{}Response", service.name);

    let mut variants: Vec<TokenStream> = Vec::new();

    for method in &service.methods {
        let variant_name = to_pascal_case(&method.name.to_string());
        let ok_type = &method.return_type.ok_type;
        let err_type = &method.return_type.err_type;

        match method.streaming {
            StreamingMode::Unary => {
                let variant_ident = format_ident!("{}", variant_name);
                variants.push(quote! {
                    #variant_ident(Result<#ok_type, #err_type>)
                });
            }
            StreamingMode::ServerStream => {
                // Server streaming has multiple response types
                let start_ident = format_ident!("{}Start", variant_name);
                let item_ident = format_ident!("{}Item", variant_name);
                let end_ident = format_ident!("{}End", variant_name);

                variants.push(quote! { #start_ident });
                variants.push(quote! { #item_ident(Result<#ok_type, #err_type>) });
                variants.push(quote! { #end_ident });
            }
            StreamingMode::ClientStream => {
                // Client streaming returns a single response
                let variant_ident = format_ident!("{}", variant_name);
                variants.push(quote! {
                    #variant_ident(Result<#ok_type, #err_type>)
                });
            }
            StreamingMode::Bidirectional => {
                // Bidirectional has streaming responses
                let start_ident = format_ident!("{}Start", variant_name);
                let item_ident = format_ident!("{}Item", variant_name);
                let end_ident = format_ident!("{}End", variant_name);

                variants.push(quote! { #start_ident });
                variants.push(quote! { #item_ident(Result<#ok_type, #err_type>) });
                variants.push(quote! { #end_ident });
            }
        }
    }

    Ok(quote! {
        /// Response enum for RPC calls.
        #[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
        #[rkyv(derive(Debug))]
        #vis enum #enum_name {
            #(#variants),*
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("add"), "Add");
        assert_eq!(to_pascal_case("get_user"), "GetUser");
        assert_eq!(to_pascal_case("list_all_items"), "ListAllItems");
    }
}
