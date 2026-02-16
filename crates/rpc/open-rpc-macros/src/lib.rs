//! OpenRPC proc-macro helpers, vendored and adapted from
//! [sui-open-rpc-macros](https://github.com/MystenLabs/sui/tree/main/crates/sui-open-rpc-macros).
//!
//! SPDX-License-Identifier: Apache-2.0
#![allow(
    unreachable_pub,
    clippy::absolute_paths,
    reason = "vendored from sui-open-rpc-macros"
)]

use derive_syn_parse::Parse;
use itertools::Itertools;
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2, TokenTree};
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{
    parse,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    spanned::Spanned,
    token::Paren,
    Attribute, GenericArgument, LitStr, PatType, Path, PathArguments, Token, TraitItem, Type,
};
use unescape::unescape;

/// Add a `[TraitName]OpenRpc` struct providing access to an OpenRPC doc builder.
///
/// This proc macro must be used in conjunction with `jsonrpsee_proc_macro::rpc`.
///
/// # Example
///
/// ```ignore
/// #[open_rpc(namespace = "strata", tag = "Full Node")]
/// #[rpc(server, namespace = "strata")]
/// pub trait OLFullNodeRpc {
///     #[method(name = "getRawBlocksRange")]
///     async fn get_raw_blocks_range(&self, start: u64, end: u64) -> RpcResult<Vec<Block>>;
/// }
/// ```
///
/// This generates a `OLFullNodeRpcOpenRpc` struct with a `module_doc()` method
/// that returns an `strata_open_rpc::Module`.
#[proc_macro_attribute]
pub fn open_rpc(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr: OpenRpcAttributes = parse_macro_input!(attr);

    let mut trait_data: syn::ItemTrait = syn::parse(item).unwrap();
    let rpc_definition = parse_rpc_method(&mut trait_data).unwrap();

    let namespace = attr
        .find_attr("namespace")
        .map(|str| str.value())
        .unwrap_or_default();

    let tag = attr.find_attr("tag").to_quote();

    let methods = rpc_definition.methods.iter().filter_map(|method| {
        if method.deprecated {
            return None;
        }
        let name = &method.name;
        let deprecated = method.deprecated;
        let doc = &method.doc;
        let mut inputs = Vec::new();
        for (name, ty, description) in &method.params {
            let (ty, required) = extract_type_from_option(ty.clone());
            let description = if let Some(description) = description {
                quote! {Some(#description.to_string())}
            } else {
                quote! {None}
            };

            inputs.push(quote! {
                let des = builder.create_content_descriptor::<#ty>(#name, None, #description, #required);
                inputs.push(des);
            })
        }
        let returns_ty = if let Some(ty) = &method.returns {
            let (ty, required) = extract_type_from_option(ty.clone());
            let name = quote! {#ty}.to_string();
            quote! {Some(builder.create_content_descriptor::<#ty>(#name, None, None, #required));}
        } else {
            quote! {None;}
        };

        if method.is_pubsub {
            Some(quote! {
                let mut inputs: Vec<strata_open_rpc::ContentDescriptor> = Vec::new();
                #(#inputs)*
                let result = #returns_ty
                builder.add_subscription(#namespace, #name, inputs, result, #doc, #tag, #deprecated);
            })
        } else {
            Some(quote! {
                let mut inputs: Vec<strata_open_rpc::ContentDescriptor> = Vec::new();
                #(#inputs)*
                let result = #returns_ty
                builder.add_method(#namespace, #name, inputs, result, #doc, #tag, #deprecated);
            })
        }
    }).collect::<Vec<_>>();

    let open_rpc_name = quote::format_ident!("{}OpenRpc", &rpc_definition.name);

    quote! {
        #trait_data
        pub struct #open_rpc_name;
        impl #open_rpc_name {
            pub fn module_doc() -> strata_open_rpc::Module {
                let mut builder = strata_open_rpc::RpcModuleDocBuilder::default();
                #(#methods)*
                builder.build()
            }
        }
    }
    .into()
}

trait OptionalQuote {
    fn to_quote(&self) -> TokenStream2;
}

impl OptionalQuote for Option<LitStr> {
    fn to_quote(&self) -> TokenStream2 {
        if let Some(value) = self {
            quote!(Some(#value.to_string()))
        } else {
            quote!(None)
        }
    }
}

struct RpcDefinition {
    name: Ident,
    methods: Vec<Method>,
}

struct Method {
    name: String,
    params: Vec<(String, Type, Option<String>)>,
    returns: Option<Type>,
    doc: String,
    is_pubsub: bool,
    deprecated: bool,
}

fn parse_rpc_method(trait_data: &mut syn::ItemTrait) -> Result<RpcDefinition, syn::Error> {
    let mut methods = Vec::new();
    for trait_item in &mut trait_data.items {
        if let TraitItem::Method(method) = trait_item {
            let doc = extract_doc_comments(&method.attrs).to_string();
            let params: Vec<_> = method
                .sig
                .inputs
                .iter_mut()
                .filter_map(|arg| {
                    match arg {
                        syn::FnArg::Receiver(_) => None,
                        syn::FnArg::Typed(arg) => {
                            let description =
                                if let Some(description) = arg.attrs.iter().position(|a| a.path.is_ident("doc")) {
                                    let doc = extract_doc_comments(&arg.attrs);
                                    arg.attrs.remove(description);
                                    Some(doc)
                                } else {
                                    None
                                };
                            match *arg.pat.clone() {
                                syn::Pat::Ident(name) => {
                                    Some(get_type(arg).map(|ty| (name.ident.to_string(), ty, description)))
                                }
                                syn::Pat::Wild(wild) => Some(Err(syn::Error::new(
                                    wild.underscore_token.span(),
                                    "Method argument names must be valid Rust identifiers; got `_` instead",
                                ))),
                                _ => Some(Err(syn::Error::new(
                                    arg.span(),
                                    "Unexpected method signature input",
                                ))),
                            }
                        }
                    }
                })
                .collect::<Result<_, _>>()?;

            let (method_name, returns, is_pubsub, deprecated) =
                if let Some(attr) = find_attr(&mut method.attrs, "method") {
                    let token: TokenStream = attr.tokens.clone().into();
                    let returns = match &method.sig.output {
                        syn::ReturnType::Default => None,
                        syn::ReturnType::Type(_, output) => extract_type_from(output, "RpcResult"),
                    };
                    let attributes = parse::<Attributes>(token)?;
                    let method_name = attributes.get_value("name");
                    let deprecated = attributes.find("deprecated").is_some();
                    (method_name, returns, false, deprecated)
                } else if let Some(attr) = find_attr(&mut method.attrs, "subscription") {
                    let token: TokenStream = attr.tokens.clone().into();
                    let attributes = parse::<Attributes>(token)?;
                    let name = attributes.get_value("name");
                    let type_ = attributes
                        .find("item")
                        .expect("Subscription should have an [item] attribute")
                        .type_
                        .clone()
                        .expect("[item] attribute should have a value");
                    let deprecated = attributes.find("deprecated").is_some();
                    (name, Some(type_), true, deprecated)
                } else {
                    panic!("Unknown method name")
                };

            methods.push(Method {
                name: method_name,
                params,
                returns,
                doc,
                is_pubsub,
                deprecated,
            });
        }
    }
    Ok(RpcDefinition {
        name: trait_data.ident.clone(),
        methods,
    })
}

fn extract_type_from(ty: &Type, from_ty: &str) -> Option<Type> {
    fn path_is(path: &Path, from_ty: &str) -> bool {
        path.leading_colon.is_none()
            && path.segments.len() == 1
            && path.segments.iter().next().unwrap().ident == from_ty
    }

    if let Type::Path(p) = ty {
        if p.qself.is_none() && path_is(&p.path, from_ty) {
            if let PathArguments::AngleBracketed(a) = &p.path.segments[0].arguments {
                if let Some(GenericArgument::Type(ty)) = a.args.first() {
                    return Some(ty.clone());
                }
            }
        }
    }
    None
}

fn extract_type_from_option(ty: Type) -> (Type, bool) {
    if let Some(ty) = extract_type_from(&ty, "Option") {
        (ty, false)
    } else {
        (ty, true)
    }
}

fn get_type(pat_type: &mut PatType) -> Result<Type, syn::Error> {
    Ok(
        if let Some((pos, attr)) = pat_type
            .attrs
            .iter()
            .find_position(|a| a.path.is_ident("schemars"))
        {
            let attribute = parse::<NamedAttribute>(attr.tokens.clone().into())?;
            let stream = syn::parse_str(&attribute.value.value())?;
            let tokens = respan_token_stream(stream, attribute.value.span());
            let path = syn::parse2(tokens)?;
            pat_type.attrs.remove(pos);
            path
        } else {
            pat_type.ty.as_ref().clone()
        },
    )
}

fn find_attr<'a>(attrs: &'a mut [Attribute], ident: &str) -> Option<&'a mut Attribute> {
    attrs.iter_mut().find(|a| a.path.is_ident(ident))
}

fn respan_token_stream(stream: TokenStream2, span: Span) -> TokenStream2 {
    stream
        .into_iter()
        .map(|mut token| {
            if let TokenTree::Group(g) = &mut token {
                *g = proc_macro2::Group::new(g.delimiter(), respan_token_stream(g.stream(), span));
            }
            token.set_span(span);
            token
        })
        .collect()
}

/// Extract doc comments from `#[doc = "..."]` attributes.
fn extract_doc_comments(attrs: &[Attribute]) -> String {
    let mut s = String::new();
    let mut sep = "";
    for attr in attrs {
        if !attr.path.is_ident("doc") {
            continue;
        }

        let Ok(syn::Meta::NameValue(meta)) = attr.parse_meta() else {
            continue;
        };

        let syn::Lit::Str(lit) = &meta.lit else {
            continue;
        };

        let token = lit.value();
        let line = token.strip_prefix(' ').unwrap_or(&token).trim_end();

        if line.is_empty() {
            s.push_str("\n\n");
            sep = "";
        } else {
            s.push_str(sep);
            sep = " ";
        }

        s.push_str(line);
    }

    unescape(&s).unwrap_or_else(|| panic!("Cannot unescape doc comments : [{s}]"))
}

#[derive(Parse)]
struct OpenRpcAttributes {
    #[parse_terminated(OpenRpcAttribute::parse)]
    fields: Punctuated<OpenRpcAttribute, Token![,]>,
}

impl OpenRpcAttributes {
    fn find_attr(&self, name: &str) -> Option<LitStr> {
        self.fields
            .iter()
            .find(|attr| attr.label == name)
            .map(|attr| attr.value.clone())
    }
}

#[derive(Parse)]
struct OpenRpcAttribute {
    label: Ident,
    _eq_token: Token![=],
    value: syn::LitStr,
}

#[derive(Parse)]
struct NamedAttribute {
    #[paren]
    _paren_token: Paren,
    #[inside(_paren_token)]
    _ident: Ident,
    #[inside(_paren_token)]
    _eq_token: Token![=],
    #[inside(_paren_token)]
    value: syn::LitStr,
}

struct Attributes {
    pub attrs: Punctuated<Attr, syn::token::Comma>,
}

impl Attributes {
    pub fn find(&self, attr_name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|attr| attr.key == attr_name)
    }

    pub fn get_value(&self, attr_name: &str) -> String {
        self.attrs
            .iter()
            .find(|attr| attr.key == attr_name)
            .unwrap_or_else(|| panic!("Method should have a [{attr_name}] attribute."))
            .value
            .as_ref()
            .unwrap_or_else(|| panic!("[{attr_name}] attribute should have a value"))
            .value()
    }
}

impl Parse for Attributes {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        let _paren = syn::parenthesized!(content in input);
        let attrs = content.parse_terminated(Attr::parse)?;
        Ok(Self { attrs })
    }
}

struct Attr {
    pub key: Ident,
    pub token: Option<TokenStream2>,
    pub value: Option<syn::LitStr>,
    pub type_: Option<Type>,
}

impl ToTokens for Attr {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        tokens.append(self.key.clone());
        if let Some(token) = &self.token {
            tokens.extend(token.to_token_stream());
        }
        if let Some(value) = &self.value {
            tokens.append(value.token());
        }
        if let Some(type_) = &self.type_ {
            tokens.extend(type_.to_token_stream());
        }
    }
}

impl Parse for Attr {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let key = input.parse()?;
        let token = if input.peek(Token!(=)) {
            Some(input.parse::<Token!(=)>()?.to_token_stream())
        } else if input.peek(Token!(<=)) {
            Some(input.parse::<Token!(<=)>()?.to_token_stream())
        } else {
            None
        };

        let value = if token.is_some() && input.peek(syn::LitStr) {
            Some(input.parse::<syn::LitStr>()?)
        } else {
            None
        };

        let type_ = if token.is_some() && input.peek(syn::Ident) {
            Some(input.parse::<Type>()?)
        } else {
            None
        };

        Ok(Self {
            key,
            token,
            value,
            type_,
        })
    }
}
