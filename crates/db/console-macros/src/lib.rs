//! Derives for the deliberately small database-console registration surface.

use std::collections::BTreeSet;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::meta::ParseNestedMeta;
use syn::spanned::Spanned;
use syn::token::Paren;
use syn::{
    Attribute, Data, DeriveInput, Field, Fields, GenericArgument, Ident, LitStr, Path,
    PathArguments, PathSegment, Type, parse_macro_input,
};

/// Generates console metadata and dispatch for explicitly listed getters and field setters.
#[proc_macro_derive(ConsoleValue, attributes(console))]
pub fn derive_console_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_console_value(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generates a table registration that delegates storage work to an explicit adapter.
#[proc_macro_derive(ConsoleTable, attributes(console))]
pub fn derive_console_table(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_console_table(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[derive(Clone, Copy)]
enum ScalarKind {
    Bool,
    I64,
    U64,
    String,
    Bytes,
}

impl ScalarKind {
    fn parse(value: &LitStr) -> syn::Result<Self> {
        match value.value().as_str() {
            "bool" => Ok(Self::Bool),
            "i64" => Ok(Self::I64),
            "u64" => Ok(Self::U64),
            "string" => Ok(Self::String),
            "bytes" => Ok(Self::Bytes),
            _ => Err(syn::Error::new(
                value.span(),
                "scalar must be one of: bool, i64, u64, string, bytes",
            )),
        }
    }

    fn metadata(self) -> TokenStream2 {
        let variant = match self {
            Self::Bool => quote!(Bool),
            Self::I64 => quote!(I64),
            Self::U64 => quote!(U64),
            Self::String => quote!(String),
            Self::Bytes => quote!(Bytes),
        };
        quote!(::strata_db_console::ScalarType::#variant)
    }

    fn wrap_value(self, value: TokenStream2) -> TokenStream2 {
        let variant = match self {
            Self::Bool => quote!(Bool),
            Self::I64 => quote!(I64),
            Self::U64 => quote!(U64),
            Self::String => quote!(String),
            Self::Bytes => quote!(Bytes),
        };
        quote!(::strata_db_console::ConsoleScalar::#variant(#value))
    }

    fn read_argument(self, value: &Ident, target: &LitStr) -> TokenStream2 {
        match self {
            Self::Bool => quote!(#value.as_bool(#target)?),
            Self::I64 => quote!(#value.as_i64(#target)?),
            Self::U64 => quote!(#value.as_u64(#target)?),
            Self::String => quote!(#value.as_str(#target)?),
            Self::Bytes => quote!(#value.as_bytes(#target)?),
        }
    }

    fn read_owned(self, value: TokenStream2, target: &LitStr) -> TokenStream2 {
        match self {
            Self::Bool => quote!(#value.as_bool(#target)?),
            Self::I64 => quote!(#value.as_i64(#target)?),
            Self::U64 => quote!(#value.as_u64(#target)?),
            Self::String => quote!(#value.as_str(#target)?.to_owned()),
            Self::Bytes => quote!(#value.as_bytes(#target)?.to_vec()),
        }
    }
}

struct Getter {
    name: LitStr,
    scalar: ScalarKind,
    nullable: bool,
    source: GetterSource,
}

enum GetterSource {
    Field(Ident),
    Function(Path),
}

struct Setter {
    name: LitStr,
    scalar: ScalarKind,
    nullable: bool,
    function: SetterFunction,
}

enum SetterFunction {
    Convention(Ident),
    Explicit(Path),
}

struct Argument {
    name: LitStr,
    scalar: ScalarKind,
}

struct Modifier {
    name: LitStr,
    function: Path,
    arguments: Vec<Argument>,
}

#[derive(Default)]
struct ValueConfig {
    name: Option<LitStr>,
    getters: Vec<Getter>,
    setters: Vec<Setter>,
}

fn parse_console_attributes(
    attributes: &[Attribute],
    mut parse_item: impl FnMut(ParseNestedMeta<'_>) -> syn::Result<()>,
) -> syn::Result<()> {
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("console"))
    {
        attribute.parse_nested_meta(&mut parse_item)?;
    }
    Ok(())
}

fn parse_value_config(input: &DeriveInput) -> syn::Result<ValueConfig> {
    let mut config = ValueConfig::default();
    parse_console_attributes(&input.attrs, |meta| {
        if meta.path.is_ident("name") {
            config.name = Some(meta.value()?.parse()?);
            return Ok(());
        }
        if meta.path.is_ident("getter") {
            config.getters.push(parse_getter(meta)?);
            return Ok(());
        }
        Err(meta.error("unsupported ConsoleValue attribute"))
    })?;

    if let Data::Struct(data) = &input.data {
        for field in &data.fields {
            let (getter, setter) = parse_field_registration(field)?;
            if let Some(getter) = getter {
                config.getters.push(getter);
            }
            if let Some(setter) = setter {
                config.setters.push(setter);
            }
        }
    }

    reject_duplicate_names(config.getters.iter().map(|getter| &getter.name), "getter")?;
    reject_duplicate_names(config.setters.iter().map(|setter| &setter.name), "setter")?;
    Ok(config)
}

fn parse_field_registration(field: &Field) -> syn::Result<(Option<Getter>, Option<Setter>)> {
    let mut getter = None;
    let mut setter_function = None;
    let mut has_setter = false;
    for attribute in field
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("console"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("get") {
                if getter.is_some() {
                    return Err(meta.error("field getter declared more than once"));
                }
                getter = Some(parse_field_getter_options(field, meta)?);
            } else if meta.path.is_ident("set") {
                if has_setter {
                    return Err(meta.error("field setter declared more than once"));
                }
                has_setter = true;
                if meta.input.peek(Paren) {
                    meta.parse_nested_meta(|nested| {
                        if nested.path.is_ident("via") {
                            setter_function = Some(nested.value()?.parse()?);
                            Ok(())
                        } else {
                            Err(nested.error("field setter supports only 'via'"))
                        }
                    })?;
                }
            } else {
                return Err(meta.error("fields support only 'get' and 'set'"));
            }
            Ok(())
        })?;
    }

    if !has_setter {
        return Ok((getter, None));
    }
    let Some(getter) = getter else {
        return Err(syn::Error::new(
            field.span(),
            "a console field setter requires a getter",
        ));
    };
    if !matches!(&getter.source, GetterSource::Field(_)) {
        return Err(syn::Error::new(
            field.span(),
            "a console field setter requires a direct field getter",
        ));
    }
    let field_ident = field.ident.clone().ok_or_else(|| {
        syn::Error::new(
            field.span(),
            "field setters require a struct with named fields",
        )
    })?;
    let (scalar, nullable) = infer_scalar_type(&field.ty).ok_or_else(|| {
        syn::Error::new(
            field.ty.span(),
            "cannot infer console setter scalar from field type",
        )
    })?;
    let function = setter_function.map_or_else(
        || SetterFunction::Convention(format_ident!("set_{field_ident}")),
        SetterFunction::Explicit,
    );
    let setter = Setter {
        name: getter.name.clone(),
        scalar,
        nullable,
        function,
    };
    Ok((Some(getter), Some(setter)))
}

fn parse_field_getter_options(field: &Field, meta: ParseNestedMeta<'_>) -> syn::Result<Getter> {
    let field_ident = field.ident.clone().ok_or_else(|| {
        syn::Error::new(
            field.span(),
            "field getters require a struct with named fields",
        )
    })?;
    let inferred = infer_scalar_type(&field.ty);
    let mut name = None;
    let mut scalar = None;
    let mut nullable = inferred.map(|(_, nullable)| nullable).unwrap_or(false);
    let mut function = None;

    if meta.input.peek(Paren) {
        meta.parse_nested_meta(|nested| {
            if nested.path.is_ident("name") {
                name = Some(nested.value()?.parse()?);
            } else if nested.path.is_ident("scalar") {
                let value: LitStr = nested.value()?.parse()?;
                scalar = Some(ScalarKind::parse(&value)?);
            } else if nested.path.is_ident("nullable") {
                nullable = true;
            } else if nested.path.is_ident("via") {
                function = Some(nested.value()?.parse()?);
            } else {
                return Err(nested.error("unsupported field getter attribute"));
            }
            Ok(())
        })?;
    }

    let scalar = scalar
        .or_else(|| inferred.map(|(scalar, _)| scalar))
        .ok_or_else(|| {
            syn::Error::new(
                field.ty.span(),
                "cannot infer console scalar; add scalar = \"...\"",
            )
        })?;
    let source = function.map_or_else(
        || GetterSource::Field(field_ident.clone()),
        GetterSource::Function,
    );

    Ok(Getter {
        name: name.unwrap_or_else(|| LitStr::new(&field_ident.to_string(), field_ident.span())),
        scalar,
        nullable,
        source,
    })
}

fn infer_scalar_type(ty: &Type) -> Option<(ScalarKind, bool)> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident == "Option" {
        let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return None;
        };
        let Some(GenericArgument::Type(inner)) = arguments.args.first() else {
            return None;
        };
        return infer_scalar_type(inner).map(|(scalar, _)| (scalar, true));
    }

    let scalar = match segment.ident.to_string().as_str() {
        "bool" => ScalarKind::Bool,
        "i64" => ScalarKind::I64,
        "u64" => ScalarKind::U64,
        "String" => ScalarKind::String,
        "Vec" if is_vec_u8(segment) => ScalarKind::Bytes,
        _ => return None,
    };
    Some((scalar, false))
}

fn is_vec_u8(segment: &PathSegment) -> bool {
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    matches!(
        arguments.args.first(),
        Some(GenericArgument::Type(Type::Path(type_path)))
            if type_path.path.is_ident("u8")
    )
}

fn parse_getter(meta: ParseNestedMeta<'_>) -> syn::Result<Getter> {
    let mut name = None;
    let mut scalar = None;
    let mut nullable = false;
    let mut field = None;
    let mut function = None;

    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("name") {
            name = Some(nested.value()?.parse()?);
        } else if nested.path.is_ident("scalar") {
            let value: LitStr = nested.value()?.parse()?;
            scalar = Some(ScalarKind::parse(&value)?);
        } else if nested.path.is_ident("nullable") {
            nullable = true;
        } else if nested.path.is_ident("field") {
            field = Some(nested.value()?.parse()?);
        } else if nested.path.is_ident("via") {
            function = Some(nested.value()?.parse()?);
        } else {
            return Err(nested.error("unsupported getter attribute"));
        }
        Ok(())
    })?;

    let source = match (field, function) {
        (Some(field), None) => GetterSource::Field(field),
        (None, Some(function)) => GetterSource::Function(function),
        (Some(field), Some(_)) => {
            return Err(syn::Error::new(
                field.span(),
                "getter must use exactly one of 'field' or 'via'",
            ));
        }
        (None, None) => return Err(meta.error("getter requires either 'field' or 'via'")),
    };

    Ok(Getter {
        name: name.ok_or_else(|| meta.error("getter requires 'name'"))?,
        scalar: scalar.ok_or_else(|| meta.error("getter requires 'scalar'"))?,
        nullable,
        source,
    })
}

fn parse_modifier(meta: ParseNestedMeta<'_>) -> syn::Result<Modifier> {
    let mut name = None;
    let mut function = None;
    let mut arguments = Vec::new();

    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("name") {
            name = Some(nested.value()?.parse()?);
        } else if nested.path.is_ident("via") {
            function = Some(nested.value()?.parse()?);
        } else if nested.path.is_ident("argument") {
            arguments.push(parse_argument(nested)?);
        } else {
            return Err(nested.error("unsupported modifier attribute"));
        }
        Ok(())
    })?;

    reject_duplicate_names(
        arguments.iter().map(|argument| &argument.name),
        "modifier argument",
    )?;
    Ok(Modifier {
        name: name.ok_or_else(|| meta.error("modifier requires 'name'"))?,
        function: function.ok_or_else(|| meta.error("modifier requires 'via'"))?,
        arguments,
    })
}

fn parse_argument(meta: ParseNestedMeta<'_>) -> syn::Result<Argument> {
    let mut name = None;
    let mut scalar = None;
    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("name") {
            name = Some(nested.value()?.parse()?);
        } else if nested.path.is_ident("scalar") {
            let value: LitStr = nested.value()?.parse()?;
            scalar = Some(ScalarKind::parse(&value)?);
        } else {
            return Err(nested.error("unsupported modifier argument attribute"));
        }
        Ok(())
    })?;

    Ok(Argument {
        name: name.ok_or_else(|| meta.error("modifier argument requires 'name'"))?,
        scalar: scalar.ok_or_else(|| meta.error("modifier argument requires 'scalar'"))?,
    })
}

fn reject_duplicate_names<'a>(
    names: impl Iterator<Item = &'a LitStr>,
    kind: &str,
) -> syn::Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name.value()) {
            return Err(syn::Error::new(
                name.span(),
                format!("duplicate {kind} name '{}'", name.value()),
            ));
        }
    }
    Ok(())
}

fn expand_console_value(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "ConsoleValue does not yet support generic values",
        ));
    }

    let config = parse_value_config(input)?;
    let type_ident = &input.ident;
    let value_name = config
        .name
        .unwrap_or_else(|| LitStr::new(&type_ident.to_string(), type_ident.span()));

    let field_descriptors = config.getters.iter().map(|getter| {
        let name = &getter.name;
        let scalar = getter.scalar.metadata();
        let nullable = getter.nullable;
        let settable = config
            .setters
            .iter()
            .any(|setter| setter.name.value() == getter.name.value());
        quote! {
            ::strata_db_console::FieldDescriptor {
                name: #name,
                scalar_type: #scalar,
                nullable: #nullable,
                settable: #settable,
            }
        }
    });

    let getter_arms = config.getters.iter().map(|getter| {
        let name = &getter.name;
        let raw_value = match &getter.source {
            GetterSource::Field(field) => match getter.scalar {
                ScalarKind::String | ScalarKind::Bytes => quote!(self.#field.clone()),
                ScalarKind::Bool | ScalarKind::I64 | ScalarKind::U64 => quote!(self.#field),
            },
            GetterSource::Function(function) => quote!(#function(self)),
        };
        let value = if getter.nullable {
            let some_value = format_ident!("value");
            let wrapped = getter.scalar.wrap_value(quote!(#some_value));
            quote! {
                match #raw_value {
                    Some(#some_value) => #wrapped,
                    None => ::strata_db_console::ConsoleScalar::Null,
                }
            }
        } else {
            getter.scalar.wrap_value(raw_value)
        };
        quote!(#name => Ok(#value))
    });

    let setter_arms = config.setters.iter().map(|setter| {
        let name = &setter.name;
        let target = LitStr::new(&format!("field setter '{}'", name.value()), name.span());
        let read_value = setter.scalar.read_owned(quote!(value), &target);
        let parsed_value = if setter.nullable {
            quote! {
                match value {
                    ::strata_db_console::ConsoleScalar::Null => None,
                    _ => Some(#read_value),
                }
            }
        } else {
            read_value
        };
        let call = match &setter.function {
            SetterFunction::Convention(function) => quote!(self.#function(__console_value)),
            SetterFunction::Explicit(function) => quote!(#function(self, __console_value)),
        };
        quote! {
            #name => {
                let __console_value = #parsed_value;
                let () = #call;
                Ok(())
            }
        }
    });
    let readonly_arms = config.getters.iter().filter_map(|getter| {
        if config
            .setters
            .iter()
            .any(|setter| setter.name.value() == getter.name.value())
        {
            return None;
        }
        let name = &getter.name;
        Some(quote! {
            #name => Err(::strata_db_console::ConsoleError::ReadOnlyField {
                value: Self::__STRATA_DB_CONSOLE_METADATA.name,
                field: field.to_owned(),
            })
        })
    });

    Ok(quote! {
        impl #type_ident {
            const __STRATA_DB_CONSOLE_METADATA: ::strata_db_console::ValueMetadata =
                ::strata_db_console::ValueMetadata {
                    name: #value_name,
                    fields: &[#(#field_descriptors),*],
                };
        }

        impl ::strata_db_console::ConsoleValue for #type_ident {
            fn metadata(&self) -> &'static ::strata_db_console::ValueMetadata {
                &Self::__STRATA_DB_CONSOLE_METADATA
            }

            fn get(
                &self,
                field: &str,
            ) -> ::strata_db_console::ConsoleResult<::strata_db_console::ConsoleScalar> {
                match field {
                    #(#getter_arms,)*
                    _ => Err(::strata_db_console::ConsoleError::UnknownField {
                        value: Self::__STRATA_DB_CONSOLE_METADATA.name,
                        field: field.to_owned(),
                    }),
                }
            }

            fn set(
                &mut self,
                field: &str,
                value: &::strata_db_console::ConsoleScalar,
            ) -> ::strata_db_console::ConsoleResult<()> {
                match field {
                    #(#setter_arms,)*
                    #(#readonly_arms,)*
                    _ => Err(::strata_db_console::ConsoleError::UnknownField {
                        value: Self::__STRATA_DB_CONSOLE_METADATA.name,
                        field: field.to_owned(),
                    }),
                }
            }

            fn as_any(&self) -> &dyn ::std::any::Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn ::std::any::Any {
                self
            }
        }

        impl ::strata_db_console::RegisteredConsoleValue for #type_ident {
            fn value_metadata() -> &'static ::strata_db_console::ValueMetadata {
                &Self::__STRATA_DB_CONSOLE_METADATA
            }
        }
    })
}

#[derive(Default)]
struct TableConfig {
    name: Option<LitStr>,
    aliases: Vec<LitStr>,
    key: Option<ScalarKind>,
    schema: Option<Type>,
    value: Option<Type>,
    adapter: Option<Path>,
    parse_key: Option<Path>,
    render_key: Option<Path>,
    map_value: Option<Path>,
    unmap_value: Option<Path>,
    modifiers: Vec<Modifier>,
}

fn parse_table_config(input: &DeriveInput) -> syn::Result<TableConfig> {
    let mut config = TableConfig::default();
    parse_console_attributes(&input.attrs, |meta| {
        if meta.path.is_ident("name") {
            config.name = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("alias") {
            config.aliases.push(meta.value()?.parse()?);
        } else if meta.path.is_ident("key") {
            let value: LitStr = meta.value()?.parse()?;
            config.key = Some(ScalarKind::parse(&value)?);
        } else if meta.path.is_ident("schema") {
            config.schema = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("value") {
            config.value = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("adapter") {
            config.adapter = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("parse_key") {
            config.parse_key = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("render_key") {
            config.render_key = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("map_value") {
            config.map_value = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("unmap_value") {
            config.unmap_value = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("modifier") {
            config.modifiers.push(parse_modifier(meta)?);
        } else {
            return Err(meta.error("unsupported ConsoleTable attribute"));
        }
        Ok(())
    })?;
    reject_duplicate_names(config.aliases.iter(), "table alias")?;
    reject_duplicate_names(
        config.modifiers.iter().map(|modifier| &modifier.name),
        "modifier",
    )?;
    Ok(config)
}

fn only_unnamed_field(input: &DeriveInput) -> syn::Result<&Field> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "ConsoleTable requires a one-field tuple struct",
        ));
    };
    let Fields::Unnamed(fields) = &data.fields else {
        return Err(syn::Error::new(
            data.fields.span(),
            "ConsoleTable requires a one-field tuple struct",
        ));
    };
    if fields.unnamed.len() != 1 {
        return Err(syn::Error::new(
            fields.span(),
            "ConsoleTable requires a one-field tuple struct",
        ));
    }
    Ok(&fields.unnamed[0])
}

fn expand_console_table(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "ConsoleTable does not yet support generic registrations",
        ));
    }
    only_unnamed_field(input)?;
    let config = parse_table_config(input)?;
    let type_ident = &input.ident;
    let name = config
        .name
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'name'"))?;
    let aliases = config.aliases;
    let key_type = config
        .key
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'key'"))?
        .metadata();
    let schema = config
        .schema
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'schema'"))?;
    let value = config
        .value
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'value'"))?;
    let adapter = config
        .adapter
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'adapter'"))?;
    let parse_key = config
        .parse_key
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'parse_key'"))?;
    let render_key = config
        .render_key
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'render_key'"))?;
    let map_value = config
        .map_value
        .ok_or_else(|| syn::Error::new(input.span(), "ConsoleTable requires 'map_value'"))?;
    let modifier_descriptors = config.modifiers.iter().map(|modifier| {
        let name = &modifier.name;
        let arguments = modifier.arguments.iter().map(|argument| {
            let name = &argument.name;
            let scalar = argument.scalar.metadata();
            quote! {
                ::strata_db_console::ArgumentDescriptor {
                    name: #name,
                    scalar_type: #scalar,
                }
            }
        });
        quote! {
            ::strata_db_console::ModifierDescriptor {
                name: #name,
                arguments: &[#(#arguments),*],
            }
        }
    });
    let modifier_arms = config.modifiers.iter().map(|modifier| {
        let modifier_name = &modifier.name;
        let function = &modifier.function;
        let argument_count = modifier.arguments.len();
        let argument_values = modifier
            .arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let argument_ident = format_ident!("__console_argument_{index}");
                let argument_name = argument.name.value();
                let target = LitStr::new(
                    &format!(
                        "modifier '{}' argument '{argument_name}'",
                        modifier_name.value()
                    ),
                    argument.name.span(),
                );
                let read = argument.scalar.read_argument(&argument_ident, &target);
                quote! {
                    let #argument_ident = &arguments[#index];
                    let #argument_ident = #read;
                }
            })
            .collect::<Vec<_>>();
        let argument_idents = (0..argument_count)
            .map(|index| format_ident!("__console_argument_{index}"))
            .collect::<Vec<_>>();

        quote! {
            #modifier_name => {
                if arguments.len() != #argument_count {
                    return Err(::strata_db_console::ConsoleError::invalid_input(
                        "modifier arguments",
                        format!(
                            "modifier '{}' expected {} arguments, got {}",
                            #modifier_name,
                            #argument_count,
                            arguments.len(),
                        ),
                    ));
                }
                #(#argument_values)*
                #function(value, #(#argument_idents),*)
            }
        }
    });

    let base_adapter = quote! {
        #adapter::<#schema, #value>::new(
            #name,
            &[#(#aliases),*],
            #key_type,
            self.0.clone(),
            #parse_key,
            #render_key,
            #map_value,
        )
    };
    let make_adapter = config.unmap_value.as_ref().map_or_else(
        || base_adapter.clone(),
        |unmap_value| quote!(#base_adapter.with_unmap_value(#unmap_value)),
    );

    let write_methods = config.unmap_value.is_some().then(|| {
        quote! {
            fn stage_set(
                &self,
                key: &::strata_db_console::ConsoleScalar,
                field: &str,
                value: &::strata_db_console::ConsoleScalar,
            ) -> ::strata_db_console::ConsoleResult<
                ::std::boxed::Box<dyn ::strata_db_console::StagedWrite>,
            > {
                let adapter = #make_adapter;
                adapter.stage_write(key, format!("set {field}"), |record| {
                    ::strata_db_console::ConsoleValue::set(record, field, value)
                })
            }

            fn stage_modify(
                &self,
                key: &::strata_db_console::ConsoleScalar,
                modifier: &str,
                arguments: &[::strata_db_console::ConsoleScalar],
            ) -> ::strata_db_console::ConsoleResult<
                ::std::boxed::Box<dyn ::strata_db_console::StagedWrite>,
            > {
                let adapter = #make_adapter;
                adapter.stage_write(key, format!("modify {modifier}"), |record| {
                    Self::__console_modify_value(record, modifier, arguments)
                })
            }
        }
    });

    Ok(quote! {
        impl #type_ident {
            fn __console_modify_value(
                value: &mut #value,
                modifier: &str,
                arguments: &[::strata_db_console::ConsoleScalar],
            ) -> ::strata_db_console::ConsoleResult<()> {
                match modifier {
                    #(#modifier_arms,)*
                    _ => Err(::strata_db_console::ConsoleError::UnknownModifier {
                        console_source: #name,
                        modifier: modifier.to_owned(),
                    }),
                }
            }
        }

        impl ::strata_db_console::ConsoleTable for #type_ident {
            fn name(&self) -> &'static str {
                #name
            }

            fn aliases(&self) -> &'static [&'static str] {
                &[#(#aliases),*]
            }

            fn key_type(&self) -> ::strata_db_console::ScalarType {
                #key_type
            }

            fn value_metadata(&self) -> &'static ::strata_db_console::ValueMetadata {
                <#value as ::strata_db_console::RegisteredConsoleValue>::value_metadata()
            }

            fn modifiers(&self) -> &'static [::strata_db_console::ModifierDescriptor] {
                &[#(#modifier_descriptors),*]
            }

            fn get(
                &self,
                key: &::strata_db_console::ConsoleScalar,
            ) -> ::strata_db_console::ConsoleResult<Option<::strata_db_console::RecordHandle>> {
                let adapter = #make_adapter;
                ::strata_db_console::ConsoleTable::get(&adapter, key)
            }

            fn scan(
                &self,
                direction: ::strata_db_console::ScanDirection,
            ) -> ::strata_db_console::ConsoleResult<::strata_db_console::RecordStream> {
                let adapter = #make_adapter;
                ::strata_db_console::ConsoleTable::scan(&adapter, direction)
            }

            fn modify(
                &self,
                record: &mut ::strata_db_console::RecordHandle,
                modifier: &str,
                arguments: &[::strata_db_console::ConsoleScalar],
            ) -> ::strata_db_console::ConsoleResult<()> {
                if record.table() != #name {
                    return Err(::strata_db_console::ConsoleError::invalid_input(
                        "modifier record",
                        format!(
                            "expected a '{}' record, got '{}'",
                            #name,
                            record.table(),
                        ),
                    ));
                }
                let value = record.downcast_mut::<#value>().ok_or_else(|| {
                    ::strata_db_console::ConsoleError::read(
                        #name,
                        "record value has an unexpected concrete type",
                    )
                })?;
                Self::__console_modify_value(value, modifier, arguments)
            }

            #write_methods
        }
    })
}

#[cfg(test)]
mod tests {
    use super::expand_console_value;

    #[test]
    fn rejects_duplicate_getter_names() {
        let input = syn::parse_quote! {
            #[console(
                getter(name = "value", scalar = "u64", field = first),
                getter(name = "value", scalar = "u64", field = second)
            )]
            struct Duplicate { first: u64, second: u64 }
        };

        let error = expand_console_value(&input).expect_err("duplicate getter must fail");
        assert!(error.to_string().contains("duplicate getter name 'value'"));
    }
}
