//! Parsing utilities for the service macro.

use proc_macro2::TokenStream;
use syn::{
    Attribute, Error, FnArg, Ident, ItemTrait, Pat, PatType, Result, ReturnType, TraitItem,
    TraitItemFn, Type,
};

/// Arguments passed to the #[service] attribute.
#[derive(Debug, Default)]
pub struct ServiceArgs {
    /// Generate Clone impl for client.
    pub client_clone: bool,
}

impl ServiceArgs {
    pub fn parse(_attr: TokenStream) -> Self {
        // For now, use defaults. Can expand to parse attributes later.
        Self::default()
    }
}

/// Parsed service definition.
#[derive(Debug)]
pub struct ServiceDefinition {
    /// Visibility of the trait.
    pub vis: syn::Visibility,
    /// Name of the service trait.
    pub name: Ident,
    /// Parsed methods.
    pub methods: Vec<MethodDefinition>,
    /// Original trait item (for re-emission).
    pub original: ItemTrait,
}

/// Parsed method definition.
#[derive(Debug)]
pub struct MethodDefinition {
    /// Method name.
    pub name: Ident,
    /// Method arguments (excluding self).
    pub args: Vec<MethodArg>,
    /// Return type info.
    pub return_type: ReturnTypeInfo,
    /// Streaming mode.
    pub streaming: StreamingMode,
    /// Original method attributes.
    pub attrs: Vec<Attribute>,
    /// Whether the method takes &mut self.
    pub is_mut: bool,
}

/// A method argument.
#[derive(Debug)]
pub struct MethodArg {
    /// Argument name.
    pub name: Ident,
    /// Argument type.
    pub ty: Type,
    /// Whether this is a streaming argument.
    pub is_stream: bool,
}

/// Information about the return type.
#[derive(Debug)]
pub struct ReturnTypeInfo {
    /// The success type (T in Result<T, E>).
    pub ok_type: Type,
    /// The error type (E in Result<T, E>).
    pub err_type: Type,
    /// The full return type.
    pub full_type: Type,
}

/// Streaming mode for a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamingMode {
    /// Normal unary RPC.
    #[default]
    Unary,
    /// Server sends multiple responses.
    ServerStream,
    /// Client sends multiple requests.
    ClientStream,
    /// Both directions stream.
    Bidirectional,
}

impl ServiceDefinition {
    /// Parse a trait into a service definition.
    pub fn parse(item: ItemTrait) -> Result<Self> {
        let mut methods = Vec::new();

        for trait_item in &item.items {
            if let TraitItem::Fn(method) = trait_item {
                methods.push(MethodDefinition::parse(method)?);
            }
        }

        if methods.is_empty() {
            return Err(Error::new_spanned(
                &item,
                "service trait must have at least one method",
            ));
        }

        Ok(Self {
            vis: item.vis.clone(),
            name: item.ident.clone(),
            methods,
            original: item,
        })
    }
}

impl MethodDefinition {
    /// Parse a trait method into a method definition.
    pub fn parse(method: &TraitItemFn) -> Result<Self> {
        let sig = &method.sig;

        // Validate async
        if sig.asyncness.is_none() {
            return Err(Error::new_spanned(sig, "RPC methods must be async"));
        }

        // Check for self receiver
        let is_mut = match sig.inputs.first() {
            Some(FnArg::Receiver(recv)) => recv.mutability.is_some(),
            Some(FnArg::Typed(_)) => {
                return Err(Error::new_spanned(
                    sig,
                    "RPC methods must take &self or &mut self",
                ));
            }
            None => {
                return Err(Error::new_spanned(
                    sig,
                    "RPC methods must take &self or &mut self",
                ));
            }
        };

        // Parse streaming mode from attributes
        let streaming = Self::parse_streaming_mode(&method.attrs)?;

        // Parse arguments (skip self)
        let args = Self::parse_args(&sig.inputs)?;

        // Parse return type
        let return_type = Self::parse_return_type(&sig.output)?;

        Ok(Self {
            name: sig.ident.clone(),
            args,
            return_type,
            streaming,
            attrs: method.attrs.clone(),
            is_mut,
        })
    }

    fn parse_streaming_mode(attrs: &[Attribute]) -> Result<StreamingMode> {
        for attr in attrs {
            if attr.path().is_ident("server_stream") {
                return Ok(StreamingMode::ServerStream);
            }
            if attr.path().is_ident("client_stream") {
                return Ok(StreamingMode::ClientStream);
            }
            if attr.path().is_ident("bidirectional") {
                return Ok(StreamingMode::Bidirectional);
            }
        }
        Ok(StreamingMode::Unary)
    }

    fn parse_args(
        inputs: &syn::punctuated::Punctuated<FnArg, syn::token::Comma>,
    ) -> Result<Vec<MethodArg>> {
        let mut args = Vec::new();

        for input in inputs.iter().skip(1) {
            // Skip self
            if let FnArg::Typed(PatType { pat, ty, attrs, .. }) = input {
                let name = match pat.as_ref() {
                    Pat::Ident(ident) => ident.ident.clone(),
                    _ => {
                        return Err(Error::new_spanned(
                            pat,
                            "expected identifier pattern for argument",
                        ));
                    }
                };

                // Check for #[stream] attribute
                let is_stream = attrs.iter().any(|a| a.path().is_ident("stream"));

                args.push(MethodArg {
                    name,
                    ty: ty.as_ref().clone(),
                    is_stream,
                });
            }
        }

        Ok(args)
    }

    fn parse_return_type(output: &ReturnType) -> Result<ReturnTypeInfo> {
        let ty = match output {
            ReturnType::Default => {
                return Err(Error::new_spanned(
                    output,
                    "RPC methods must return Result<T, E>",
                ));
            }
            ReturnType::Type(_, ty) => ty.as_ref().clone(),
        };

        // Try to extract Result<T, E>
        if let Type::Path(type_path) = &ty {
            if let Some(segment) = type_path.path.segments.last() {
                if segment.ident == "Result" {
                    if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                        let mut types = args.args.iter().filter_map(|arg| {
                            if let syn::GenericArgument::Type(t) = arg {
                                Some(t.clone())
                            } else {
                                None
                            }
                        });

                        if let (Some(ok_type), Some(err_type)) = (types.next(), types.next()) {
                            return Ok(ReturnTypeInfo {
                                ok_type,
                                err_type,
                                full_type: ty,
                            });
                        }
                    }
                }
            }
        }

        Err(Error::new_spanned(
            output,
            "RPC methods must return Result<T, E>",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn parse_trait(tokens: proc_macro2::TokenStream) -> Result<ServiceDefinition> {
        let item: ItemTrait = syn::parse2(tokens)?;
        ServiceDefinition::parse(item)
    }

    #[test]
    fn test_parse_simple_service() {
        let service = parse_trait(quote! {
            pub trait Calculator {
                async fn add(&self, a: i32, b: i32) -> Result<i32, CalcError>;
            }
        })
        .unwrap();

        assert_eq!(service.name.to_string(), "Calculator");
        assert_eq!(service.methods.len(), 1);
        assert_eq!(service.methods[0].name.to_string(), "add");
        assert_eq!(service.methods[0].args.len(), 2);
    }

    #[test]
    fn test_parse_streaming_method() {
        let service = parse_trait(quote! {
            trait StreamService {
                #[server_stream]
                async fn list(&self, filter: Filter) -> Result<Item, Error>;
            }
        })
        .unwrap();

        assert_eq!(service.methods[0].streaming, StreamingMode::ServerStream);
    }

    #[test]
    fn test_non_async_method_error() {
        let result = parse_trait(quote! {
            trait BadService {
                fn sync_method(&self) -> Result<(), Error>;
            }
        });

        assert!(result.is_err());
    }

    #[test]
    fn test_no_result_return_error() {
        let result = parse_trait(quote! {
            trait BadService {
                async fn no_result(&self) -> String;
            }
        });

        assert!(result.is_err());
    }
}
