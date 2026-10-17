//! `#[derive(Reflect)]` — Unity-style serialized fields for Oxide types.
//!
//! On a struct with named fields it implements `oxide_core::reflect::Reflect`, exposing every
//! field to the Inspector, scene files and MCP tools. Field attributes:
//!
//! - `#[reflect(skip)]` — runtime-only state (Unity `[NonSerialized]`); the type must be `Default`.
//! - `#[reflect(range(min, max))]` — slider in the Inspector (Unity `[Range]`).
//! - `#[reflect(tooltip = "...")]` — hover help (Unity `[Tooltip]`).
//! - `#[reflect(header = "...")]` — a section title above the field (Unity `[Header]`).
//! - `#[reflect(hide)]` — saved but not shown (Unity `[HideInInspector]`).
//! - `#[reflect(min = 0)]` — lowest value (Unity `[Min]`).
//! - `#[reflect(space)]` / `#[reflect(space = 20)]` — gap above the field (Unity `[Space]`).
//! - `#[reflect(multiline)]` / `#[reflect(multiline = 5)]` — text box (Unity `[TextArea]`).
//! - `#[reflect(hdr)]` — HDR colour picker (Unity `[ColorUsage(hdr: true)]`).
//! - `#[reflect(delayed)]` — applies when editing ends (Unity `[Delayed]`).
//! - `#[reflect(label = "...")]` — shown name (Unity `[InspectorName]`).
//! - `#[reflect(formerly = "old_name")]` — also reads data saved under an old field name
//!   (Unity `[FormerlySerializedAs]`); repeatable.
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
        // Game code sees the engine through `oxide_engine` or, in a project's Scripts,
        // the `oxide_dylib` library (usually renamed to `oxide`).
        Err(_) => match crate_name("oxide_engine")
            .or_else(|_| crate_name("oxide_dylib"))
            .or_else(|_| crate_name("oxide_player"))
        {
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
    header: Option<String>,
    hide: bool,
    min: Option<f64>,
    space: f32,
    multiline: Option<u32>,
    hdr: bool,
    delayed: bool,
    label: Option<String>,
    formerly: Vec<String>,
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
            } else if meta.path.is_ident("header") {
                let s: LitStr = meta.value()?.parse()?;
                out.header = Some(s.value());
                Ok(())
            } else if meta.path.is_ident("label") {
                let s: LitStr = meta.value()?.parse()?;
                out.label = Some(s.value());
                Ok(())
            } else if meta.path.is_ident("formerly") {
                let s: LitStr = meta.value()?.parse()?;
                out.formerly.push(s.value());
                Ok(())
            } else if meta.path.is_ident("hide") {
                out.hide = true;
                Ok(())
            } else if meta.path.is_ident("hdr") {
                out.hdr = true;
                Ok(())
            } else if meta.path.is_ident("delayed") {
                out.delayed = true;
                Ok(())
            } else if meta.path.is_ident("min") {
                let e: syn::Expr = meta.value()?.parse()?;
                out.min = Some(lit_f64(&e)?);
                Ok(())
            } else if meta.path.is_ident("space") {
                out.space = if meta.input.peek(syn::Token![=]) {
                    let e: syn::Expr = meta.value()?.parse()?;
                    lit_f64(&e)? as f32
                } else {
                    8.0
                };
                Ok(())
            } else if meta.path.is_ident("multiline") {
                out.multiline = Some(if meta.input.peek(syn::Token![=]) {
                    let e: syn::Expr = meta.value()?.parse()?;
                    lit_f64(&e)? as u32
                } else {
                    3
                });
                Ok(())
            } else {
                Err(meta.error(
                    "expected `skip`, `range(min, max)`, `tooltip`, `header`, `hide`, `min`, \
                     `space`, `multiline`, `hdr`, `delayed`, `label` or `formerly`",
                ))
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

/// Checks the struct's `#[reflect(...)]` attributes (`default` is accepted for older code;
/// `Default` is now found automatically).
fn check_struct_attrs(input: &DeriveInput) -> syn::Result<()> {
    for attr in &input.attrs {
        if attr.path().is_ident("reflect") {
            attr.parse_nested_meta(|m| {
                if m.path.is_ident("default") {
                    Ok(())
                } else {
                    Err(m.error("expected `default` on a struct"))
                }
            })?;
        }
    }
    Ok(())
}

fn derive_struct(input: &DeriveInput, fields: &Fields) -> syn::Result<TokenStream2> {
    let core = core_path();
    check_struct_attrs(input)?;
    // Built from `Default` when the type has one (nested structs arriving whole, in lists).
    let from_default = quote! {
        #[allow(unused_imports)]
        use #core::reflect::__private::{NoDefault as _, ViaDefault as _};
        let mut s: Self = (&#core::reflect::__private::Maker::<Self>(::core::marker::PhantomData)).make()?;
        #core::reflect::FieldValue::set_from(&mut s, v).then_some(s)
    };
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
        let opt_str = |v: &Option<String>| match v {
            Some(t) => quote!(Some(#t)),
            None => quote!(None),
        };
        let header = opt_str(&attrs.header);
        let label = opt_str(&attrs.label);
        let min = match attrs.min {
            Some(m) => quote!(Some(#m)),
            None => quote!(None),
        };
        let multiline = match attrs.multiline {
            Some(m) => quote!(Some(#m)),
            None => quote!(None),
        };
        let (hide, space, hdr, delayed) = (attrs.hide, attrs.space, attrs.hdr, attrs.delayed);
        let aliases = &attrs.formerly;
        infos.push(quote! {
            #core::reflect::FieldInfo {
                name: #name,
                kind: #core::reflect::field_kind(&self.#id),
                options: #core::reflect::field_options(&self.#id),
                type_name: #core::reflect::field_type_name(&self.#id),
                element: #core::reflect::field_element(&self.#id),
                range: #range,
                tooltip: #tooltip,
                nullable: #core::reflect::field_nullable(&self.#id),
                new_item: #core::reflect::field_new_item(&self.#id),
                nested: #core::reflect::field_nested(&self.#id),
                ui: #core::reflect::FieldUi {
                    header: #header,
                    hide: #hide,
                    min: #min,
                    space: #space,
                    multiline: #multiline,
                    hdr: #hdr,
                    delayed: #delayed,
                    label: #label,
                },
            }
        });
        gets.push(quote! { #name => Some(#core::reflect::FieldValue::to_value(&self.#id)), });
        sets.push(quote! {
            #name #(| #aliases)* => {
                let expected = #core::reflect::field_kind(&self.#id);
                if #core::reflect::FieldValue::set_from(&mut self.#id, value) {
                    Ok(())
                } else {
                    Err(#core::reflect::ReflectError::WrongType { field: name.to_owned(), expected })
                }
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

        /// Nested use as a field of another reflected type (serialized as an object).
        impl #impl_g #core::reflect::FieldValue for #ty #ty_g #where_g {
            const KIND: #core::reflect::ValueKind = #core::reflect::ValueKind::Object;
            const TYPE_NAME: Option<&'static str> = Some(#ty_name);
            fn to_value(&self) -> #core::reflect::Value {
                #core::reflect::Value::Object(
                    #core::reflect::Reflect::fields(self).into_iter().map(|(k, v)| (k.to_string(), v)).collect()
                )
            }
            fn from_value(v: #core::reflect::Value) -> Option<Self> {
                #from_default
            }
            fn default_value() -> Option<#core::reflect::Value> {
                #[allow(unused_imports)]
                use #core::reflect::__private::{NoDefault as _, ViaDefault as _};
                let s: Option<Self> = (&#core::reflect::__private::Maker::<Self>(::core::marker::PhantomData)).make();
                s.map(|s| #core::reflect::FieldValue::to_value(&s))
            }
            fn nested_info() -> Option<#core::reflect::TypeInfo> {
                #[allow(unused_imports)]
                use #core::reflect::__private::{NoDefault as _, ViaDefault as _};
                let s: Option<Self> = (&#core::reflect::__private::Maker::<Self>(::core::marker::PhantomData)).make();
                s.map(|s| #core::reflect::Reflect::type_info(&s))
            }
            fn set_from(&mut self, v: #core::reflect::Value) -> bool {
                match v {
                    #core::reflect::Value::Object(map) => map
                        .into_iter()
                        .all(|(k, v)| #core::reflect::Reflect::set_field(self, &k, v).is_ok()),
                    _ => false,
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
            fn default_value() -> Option<#core::reflect::Value> {
                Self::OPTIONS.first().map(|n| #core::reflect::Value::Enum((*n).to_owned()))
            }
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
