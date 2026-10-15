//! `#[derive(Reflect)]` — Unity-style serialized fields for Oxide types.
//!
//! On a struct with named fields it implements `oxide_core::reflect::Reflect`, exposing every
//! field to the Inspector, scene files and MCP tools. Field attributes:
//!
//! - `#[reflect(skip)]` — runtime-only state (Unity `[NonSerialized]`); the type must be `Default`.
//! - `#[reflect(range(min, max))]` — slider in the Inspector (Unity `[Range]`).
//! - `#[reflect(tooltip = "...")]` — hover help (Unity `[Tooltip]`).
//!
//! On an enum of unit variants it implements `FieldValue`, so the enum can be a field
//! (shown as a dropdown, stored by variant name).

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, LitStr, parse_macro_input};

/// Path to the `oxide_core` crate as seen from the crate being compiled.
fn core_path() -> TokenStream2 {
    match crate_name("oxide_core") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let id = Ident::new(&name, Span::call_site());
            quote!(::#id)
        }
        Err(_) => match crate_name("oxide_engine") {
            Ok(FoundCrate::Name(name)) => {
                let id = Ident::new(&name, Span::call_site());
                quote!(::#id::core)
            }
            _ => quote!(::oxide_core),
        },
    }
}

#[derive(Default)]
struct FieldAttrs {
    skip: bool,
    range: Option<(f64, f64)>,
    tooltip: Option<String>,
}

fn parse_attrs(attrs: &[syn::Attribute]) -> syn::Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("reflect")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                out.skip = true;
                Ok(())
            } else if meta.path.is_ident("range") {
                let content;
                syn::parenthesized!(content in meta.input);
                let min: syn::Expr = content.parse()?;
                content.parse::<syn::Token![,]>()?;
                let max: syn::Expr = content.parse()?;
                out.range = Some((lit_f64(&min)?, lit_f64(&max)?));
                Ok(())
            } else if meta.path.is_ident("tooltip") {
                let s: LitStr = meta.value()?.parse()?;
                out.tooltip = Some(s.value());
                Ok(())
            } else {
                Err(meta.error("expected `skip`, `range(min, max)` or `tooltip = \"...\"`"))
            }
        })?;
    }
    Ok(out)
}

fn lit_f64(e: &syn::Expr) -> syn::Result<f64> {
    match e {
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Float(f) => f.base10_parse(),
            syn::Lit::Int(i) => i.base10_parse::<i64>().map(|v| v as f64),
            _ => Err(syn::Error::new_spanned(e, "expected a number")),
        },
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) => lit_f64(expr).map(|v| -v),
        _ => Err(syn::Error::new_spanned(e, "expected a number")),
    }
}

#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let result = match &input.data {
        Data::Struct(s) => derive_struct(&input, &s.fields),
        Data::Enum(e) => derive_enum(&input, e),
        Data::Union(_) => Err(syn::Error::new_spanned(
            &input.ident,
            "Reflect cannot be derived for unions",
        )),
    };
    result.unwrap_or_else(|e| e.to_compile_error()).into()
}

fn derive_struct(input: &DeriveInput, fields: &Fields) -> syn::Result<TokenStream2> {
    let core = core_path();
    let ty = &input.ident;
    let ty_name = ty.to_string();
    let (impl_g, ty_g, where_g) = input.generics.split_for_impl();
    let Fields::Named(named) = fields else {
        if matches!(fields, Fields::Unit) {
            return Ok(quote! {
                impl #impl_g #core::reflect::Reflect for #ty #ty_g #where_g {
                    fn type_info(&self) -> #core::reflect::TypeInfo {
                        #core::reflect::TypeInfo { name: #ty_name, fields: ::std::vec::Vec::new() }
                    }
                    fn get_field(&self, _: &str) -> Option<#core::reflect::Value> { None }
                    fn set_field(&mut self, name: &str, _: #core::reflect::Value)
                        -> Result<(), #core::reflect::ReflectError> {
                        Err(#core::reflect::ReflectError::NoSuchField(name.to_owned()))
                    }
                }
            });
        }
        return Err(syn::Error::new_spanned(
            ty,
            "Reflect needs named fields (or a unit struct)",
        ));
    };

    let mut infos = Vec::new();
    let mut gets = Vec::new();
    let mut sets = Vec::new();
    for f in &named.named {
        let attrs = parse_attrs(&f.attrs)?;
        if attrs.skip {
            continue;
        }
        let id = f.ident.as_ref().expect("named field");
        let name = id.to_string();
        let range = match attrs.range {
            Some((a, b)) => quote!(Some((#a, #b))),
            None => quote!(None),
        };
        let tooltip = match &attrs.tooltip {
            Some(t) => quote!(Some(#t)),
            None => quote!(None),
        };
        infos.push(quote! {
            #core::reflect::FieldInfo {
                name: #name,
                kind: #core::reflect::field_kind(&self.#id),
                options: #core::reflect::field_options(&self.#id),
                type_name: #core::reflect::field_type_name(&self.#id),
                element: #core::reflect::field_element(&self.#id),
                range: #range,
                tooltip: #tooltip,
            }
        });
        gets.push(quote! { #name => Some(#core::reflect::FieldValue::to_value(&self.#id)), });
        sets.push(quote! {
            #name => {
                let expected = #core::reflect::field_kind(&self.#id);
                self.#id = #core::reflect::FieldValue::from_value(value).ok_or_else(|| {
                    #core::reflect::ReflectError::WrongType { field: name.to_owned(), expected }
                })?;
                Ok(())
            }
        });
    }

    Ok(quote! {
        impl #impl_g #core::reflect::Reflect for #ty #ty_g #where_g {
            fn type_info(&self) -> #core::reflect::TypeInfo {
                #core::reflect::TypeInfo { name: #ty_name, fields: vec![#(#infos),*] }
            }
            fn get_field(&self, name: &str) -> Option<#core::reflect::Value> {
                match name {
                    #(#gets)*
                    _ => None,
                }
            }
            fn set_field(&mut self, name: &str, value: #core::reflect::Value)
                -> Result<(), #core::reflect::ReflectError> {
                match name {
                    #(#sets)*
                    _ => Err(#core::reflect::ReflectError::NoSuchField(name.to_owned())),
                }
            }
        }
    })
}

fn derive_enum(input: &DeriveInput, data: &syn::DataEnum) -> syn::Result<TokenStream2> {
    let core = core_path();
    let ty = &input.ident;
    let ty_name = ty.to_string();
    let mut names = Vec::new();
    let mut idents = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                v,
                "Reflect enums may only have unit variants",
            ));
        }
        names.push(v.ident.to_string());
        idents.push(&v.ident);
    }
    Ok(quote! {
        impl #core::reflect::FieldValue for #ty {
            const KIND: #core::reflect::ValueKind = #core::reflect::ValueKind::Enum;
            const OPTIONS: &'static [&'static str] = &[#(#names),*];
            const TYPE_NAME: Option<&'static str> = Some(#ty_name);
            fn to_value(&self) -> #core::reflect::Value {
                #core::reflect::Value::Enum(match self { #(Self::#idents => #names,)* }.to_owned())
            }
            fn from_value(v: #core::reflect::Value) -> Option<Self> {
                match v {
                    #core::reflect::Value::Enum(s) | #core::reflect::Value::String(s) => match s.as_str() {
                        #(#names => Some(Self::#idents),)*
                        _ => None,
                    },
                    #core::reflect::Value::Int(i) => [#(Self::#idents),*].into_iter().nth(usize::try_from(i).ok()?),
                    _ => None,
                }
            }
        }
    })
}
