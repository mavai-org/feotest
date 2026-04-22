//! Code expansion for `#[use_case_factory]`.
//!
//! The attribute marks a method as producing a `UseCase`. For the current
//! scaffolding it does no runtime transformation — it passes the method
//! through unchanged — but it validates the return-type shape at
//! macro-expansion time so that misshapen factory methods are rejected
//! with a clear diagnostic rather than surfacing later as confusing type
//! errors in the surrounding machinery.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, ImplItemFn, ReturnType, Type, parse2, spanned::Spanned};

/// Expands `#[use_case_factory] fn ...` by validating the return type and
/// emitting the original method unchanged.
pub fn expand(attr: &TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(Error::new(
            attr.span(),
            "`#[use_case_factory]` does not accept arguments",
        ));
    }

    let method: ImplItemFn = parse2(item)?;
    validate_return_type(&method.sig.output)?;

    Ok(quote! { #method })
}

/// Accepts `-> impl <Trait>` or `-> Box<dyn <Trait>>`; rejects everything
/// else (including default / unit return, primitives, concrete types).
fn validate_return_type(ret: &ReturnType) -> syn::Result<()> {
    let ReturnType::Type(_, ty) = ret else {
        return Err(Error::new(
            ret.span(),
            "`#[use_case_factory]` methods must return `impl UseCase` or `Box<dyn UseCase>`",
        ));
    };

    match ty.as_ref() {
        Type::ImplTrait(_) => Ok(()),
        Type::Path(path) if is_box_of_dyn(path) => Ok(()),
        other => Err(Error::new(
            other.span(),
            "`#[use_case_factory]` methods must return `impl UseCase` or `Box<dyn UseCase>`",
        )),
    }
}

/// Structural check: is this path a `Box<dyn ...>`? We check the *last*
/// segment so `::std::boxed::Box<dyn T>` and `Box<dyn T>` are both accepted.
/// The inner type must be a `dyn Trait`; we do not attempt to resolve the
/// trait itself, leaving that to rustc.
fn is_box_of_dyn(path: &syn::TypePath) -> bool {
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    if last.ident != "Box" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return false;
    };
    args.args
        .iter()
        .any(|arg| matches!(arg, syn::GenericArgument::Type(Type::TraitObject(_))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn method(ts: TokenStream) -> ImplItemFn {
        parse2(ts).expect("parseable method")
    }

    #[test]
    fn accepts_impl_trait_return() {
        let m = method(quote! { fn f(&self) -> impl ::core::fmt::Debug { 1 } });
        assert!(validate_return_type(&m.sig.output).is_ok());
    }

    #[test]
    fn accepts_box_dyn_return() {
        let m = method(quote! { fn f(&self) -> Box<dyn ::core::fmt::Debug> { Box::new(1) } });
        assert!(validate_return_type(&m.sig.output).is_ok());
    }

    #[test]
    fn accepts_fully_qualified_box_dyn_return() {
        let m = method(
            quote! { fn f(&self) -> ::std::boxed::Box<dyn ::core::fmt::Debug> { Box::new(1) } },
        );
        assert!(validate_return_type(&m.sig.output).is_ok());
    }

    #[test]
    fn rejects_unit_return() {
        let m = method(quote! { fn f(&self) {} });
        assert!(validate_return_type(&m.sig.output).is_err());
    }

    #[test]
    fn rejects_concrete_type_return() {
        let m = method(quote! { fn f(&self) -> String { String::new() } });
        assert!(validate_return_type(&m.sig.output).is_err());
    }

    #[test]
    fn rejects_box_of_concrete_type() {
        // Box<String> has no `dyn` inside, so this should be rejected.
        let m = method(quote! { fn f(&self) -> Box<String> { Box::new(String::new()) } });
        assert!(validate_return_type(&m.sig.output).is_err());
    }
}
