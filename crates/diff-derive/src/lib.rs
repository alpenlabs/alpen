use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod diffable;

#[proc_macro_derive(DaDiff, attributes(diff_override))]
pub fn derive_diffable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    diffable::generate_diffable(&input).into()
}
