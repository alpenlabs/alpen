use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Fields, Path};

fn get_diff_override(attrs: &[Attribute]) -> Option<Path> {
    for attr in attrs {
        if attr.path().is_ident("diff_override") {
            // Parse the attribute like #[diff(MyType)]
            if let Ok(path) = attr.parse_args::<syn::Path>() {
                return Some(path);
            }
        }
    }
    None
}

pub fn generate_diffable(input: &DeriveInput) -> TokenStream {
    let name = &input.ident;
    let vis = &input.vis;
    let diff_name = format_ident!("{}Diff", name);

    // Copy generics and where clause
    let generics = &input.generics;
    let (_, ty_generics, where_clause) = generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => panic!("Diffable only supports named structs"),
        },
        _ => panic!("Diffable can only be used with structs"),
    };

    let mut diff_fields = Vec::new();

    for field in fields {
        let fname = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let diff_field_name = format_ident!("{}_diff", fname);

        match get_diff_override(&field.attrs) {
            Some(path) => {
                diff_fields.push(quote! {
                    #diff_field_name: #path
                });
            }
            // If no diff override is found, use RegisterDiff(or error out making it mandatory?)
            None => {
                diff_fields.push(quote! {
                    #diff_field_name: ::strata_da_lib::diff::RegisterDiff<#field_ty>
                });

                // let err = Error::new(
                //     field.ident.as_ref().unwrap().span(),
                //     format!("Missing #[diff(...)] attribute on field `{}`", fname),
                // );
                // return err.to_compile_error();
            }
        }
    }

    quote! {
        #[derive(Debug, Clone)]
        #vis struct #diff_name #ty_generics #where_clause {
            #(pub #diff_fields),*
        }
    }
}
