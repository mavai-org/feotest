//! Code expansion for `#[sentinel]`.
//!
//! The `#[sentinel]` attribute marks a struct as a reliability specification.
//! Expansion emits an `impl ReliabilitySpec` using the configured name and
//! description, plus an `inventory::submit!` registering a `SpecDescriptor`
//! that points at `StructName::default()` for construction.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Ident, ItemStruct, LitStr, Result, parse::Parser, parse2, spanned::Spanned};

/// Expands `#[sentinel(...)] struct S;` into the struct plus registration.
pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let attrs = SentinelAttrs::parse(attr)?;
    let item_struct: ItemStruct = parse2(item)?;

    let struct_ident = &item_struct.ident;
    let name_lit = attrs.name.unwrap_or_else(|| {
        LitStr::new(
            &to_snake_case(&struct_ident.to_string()),
            struct_ident.span(),
        )
    });
    let description_lit = attrs
        .description
        .unwrap_or_else(|| LitStr::new("", struct_ident.span()));

    // Generate a unique module name to encapsulate the submit! call without
    // polluting the surrounding namespace. Using the struct name avoids
    // collisions between multiple #[sentinel] structs in the same module.
    let submit_mod = Ident::new(
        &format!(
            "__feotest_sentinel_submit_{}",
            to_snake_case(&struct_ident.to_string())
        ),
        struct_ident.span(),
    );

    Ok(quote! {
        #item_struct

        impl ::feotest::sentinel::ReliabilitySpec for #struct_ident {
            fn name(&self) -> &'static str {
                #name_lit
            }

            fn description(&self) -> &'static str {
                #description_lit
            }

            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }
        }

        #[doc(hidden)]
        #[allow(non_snake_case, reason = "generated submission module")]
        mod #submit_mod {
            use super::#struct_ident;

            ::feotest::inventory::submit! {
                ::feotest::sentinel::SpecDescriptor {
                    name: #name_lit,
                    description: #description_lit,
                    constructor: || ::std::boxed::Box::new(<#struct_ident as ::core::default::Default>::default()),
                }
            }
        }
    })
}

#[derive(Default)]
struct SentinelAttrs {
    name: Option<LitStr>,
    description: Option<LitStr>,
}

impl SentinelAttrs {
    fn parse(attr: TokenStream) -> Result<Self> {
        if attr.is_empty() {
            return Ok(Self::default());
        }

        let mut out = Self::default();
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("name") {
                out.name = Some(meta.value()?.parse::<LitStr>()?);
                Ok(())
            } else if meta.path.is_ident("description") {
                out.description = Some(meta.value()?.parse::<LitStr>()?);
                Ok(())
            } else {
                Err(meta
                    .error("unknown argument to `#[sentinel]`; expected `name` or `description`"))
            }
        });
        parser.parse2(attr)?;
        Ok(out)
    }
}

fn to_snake_case(pascal: &str) -> String {
    let mut out = String::with_capacity(pascal.len() + 4);
    for (i, ch) in pascal.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Accepts only `ItemStruct`; rejects any other item kind with a clear
/// message. Used from the entry-point macro before invoking `expand`.
pub fn validate_is_struct(item: &TokenStream) -> Result<()> {
    match syn::parse2::<syn::Item>(item.clone())? {
        syn::Item::Struct(_) => Ok(()),
        other => Err(Error::new(
            other.span(),
            "`#[sentinel]` may only be applied to a struct",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::to_snake_case;

    #[test]
    fn snake_case_single_word() {
        assert_eq!(to_snake_case("Spec"), "spec");
    }

    #[test]
    fn snake_case_multi_word() {
        assert_eq!(to_snake_case("PaymentGateway"), "payment_gateway");
        assert_eq!(
            to_snake_case("ShoppingBasketReliability"),
            "shopping_basket_reliability"
        );
    }

    #[test]
    fn snake_case_single_letter_prefix() {
        assert_eq!(to_snake_case("ASpec"), "a_spec");
    }
}
