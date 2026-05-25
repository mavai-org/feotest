//! Code expansion for `#[probabilistic_test]`.
//!
//! The attribute lowers a test function into a single-criterion service
//! contract and runs it through the contract-driven probabilistic test path.
//! The function body is the criterion's postcondition: a `-> bool` body passes
//! on `true`, and a `-> Result<(), ContractViolation>` body passes on `Ok(())`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemFn, ReturnType};

use crate::parse::{Approach, PTestAttrs};

/// Expands the `#[probabilistic_test(...)]` attribute into a `#[test]` function.
pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let attrs: PTestAttrs = syn::parse2(attr)?;
    let func: ItemFn = syn::parse2(item)?;
    let approach = attrs.detect_approach()?;

    let returns_bool = validate_function_signature(&func)?;

    let fn_name = &func.sig.ident;
    let fn_name_str = fn_name.to_string();
    let fn_body = &func.block;
    let fn_output = &func.sig.output;
    let fn_vis = &func.vis;

    // The body becomes a local function judging one response. It takes the
    // input string; a parameterless test body simply ignores it.
    let body_param = func
        .sig
        .inputs
        .first()
        .map_or_else(|| quote! { _input: &str }, |param| quote! { #param });

    let judge = build_judge(returns_bool);
    let criterion_kind = build_criterion_kind(approach, &attrs);
    let samples = attrs.samples.unwrap();
    let run_options = build_run_options(&attrs);
    let (spec_load, baseline_call) = build_baseline(&attrs);

    Ok(quote! {
        #[test]
        #fn_vis fn #fn_name() {
            fn __feotest_body(#body_param) #fn_output #fn_body

            struct __FeotestContract;

            impl feotest::service_contract::ServiceContract for __FeotestContract {
                type Input = String;
                type Output = String;

                fn id(&self) -> &str {
                    #fn_name_str
                }

                fn invoke(
                    &self,
                    input: &String,
                    _cost: &mut feotest::controls::Cost,
                ) -> ::core::result::Result<String, feotest::model::Defect> {
                    ::core::result::Result::Ok(input.clone())
                }

                fn criteria(&self) -> feotest::criteria::Criteria<String> {
                    feotest::criteria::Criteria::of([
                        #criterion_kind
                            .name("result")
                            .satisfies(
                                "result",
                                |__response: &String| -> feotest::model::Outcome { #judge },
                            )
                            .build(),
                    ])
                }
            }

            #spec_load

            let __feotest_inputs = vec!["default".to_string()];
            let __feotest_result = feotest::ptest::ProbabilisticTest::for_contract(__FeotestContract)
                .inputs(&__feotest_inputs)
                .samples(#samples)
                #run_options
                #baseline_call
                .run();

            assert!(
                __feotest_result.passed(),
                "probabilistic test '{}' did not pass: {:?}",
                #fn_name_str,
                __feotest_result.verdict_record().verdict(),
            );
        }
    })
}

/// Maps the body's result onto the criterion's pass/fail outcome: a `bool`
/// body passes on `true`, any other (treated as `Result`) on `Ok(())`.
fn build_judge(returns_bool: bool) -> TokenStream {
    if returns_bool {
        quote! {
            if __feotest_body(__response) {
                ::core::result::Result::Ok(())
            } else {
                ::core::result::Result::Err(feotest::model::ContractViolation::new(
                    "result",
                    "test body returned false",
                ))
            }
        }
    } else {
        quote! { __feotest_body(__response) }
    }
}

/// The single criterion's target: an explicit threshold is a normative rate; a
/// baseline-backed test derives its target empirically.
fn build_criterion_kind(approach: Approach, attrs: &PTestAttrs) -> TokenStream {
    match approach {
        Approach::ThresholdFirst => {
            let threshold = attrs.threshold.unwrap();
            quote! { feotest::criteria::Criteria::<String>::meeting().pass_rate(#threshold) }
        }
        Approach::SampleSizeFirst => {
            quote! { feotest::criteria::Criteria::<String>::empirical().pass_rate() }
        }
    }
}

/// The optional builder calls (confidence, intent, threshold origin, contract
/// reference) for the contract test, in a fixed order.
fn build_run_options(attrs: &PTestAttrs) -> TokenStream {
    let confidence = attrs
        .confidence
        .map(|c| quote! { .confidence(#c) })
        .unwrap_or_default();
    let intent = match attrs.intent.as_deref() {
        Some("smoke") => quote! { .smoke() },
        _ => TokenStream::new(),
    };
    let origin = attrs.threshold_origin.as_deref().map_or_else(
        TokenStream::new,
        |origin| {
            let origin_expr = threshold_origin_expr(origin);
            quote! { .threshold_origin(#origin_expr) }
        },
    );
    let contract_ref = attrs
        .contract_ref
        .as_ref()
        .map(|cref| quote! { .contract_ref(#cref) })
        .unwrap_or_default();
    quote! { #confidence #intent #origin #contract_ref }
}

/// The baseline-spec load statement and the `.baseline(..)` builder call for a
/// spec-backed test; both empty when no spec is declared.
fn build_baseline(attrs: &PTestAttrs) -> (TokenStream, TokenStream) {
    attrs.spec.as_ref().map_or_else(
        || (TokenStream::new(), TokenStream::new()),
        |spec_path| {
            (
                quote! {
                    let __feotest_spec = feotest::spec::SpecResolver::resolve_file(
                        concat!(env!("CARGO_MANIFEST_DIR"), "/", #spec_path)
                    ).expect("failed to load baseline spec");
                },
                quote! { .baseline(__feotest_spec) },
            )
        },
    )
}

/// Validates the function signature and reports whether it returns `bool`.
///
/// Accepts a `bool` body (passes on `true`) or any other return type, which is
/// treated as `Result<(), ContractViolation>` (passes on `Ok(())`). The
/// function must take zero or one parameter and must not be `async`.
fn validate_function_signature(func: &ItemFn) -> syn::Result<bool> {
    let returns_bool = match &func.sig.output {
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                &func.sig,
                "a probabilistic test body must return bool or Result<(), ContractViolation>",
            ));
        }
        ReturnType::Type(_, ty) => {
            matches!(ty.as_ref(), syn::Type::Path(tp) if tp.path.is_ident("bool"))
        }
    };

    if func.sig.inputs.len() > 1 {
        return Err(syn::Error::new_spanned(
            &func.sig.inputs,
            "a probabilistic test body takes zero or one parameter (input: &str)",
        ));
    }

    if func.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "a probabilistic test body cannot be async",
        ));
    }

    Ok(returns_bool)
}

/// Builds the `ThresholdOrigin` expression from a string value.
fn threshold_origin_expr(origin: &str) -> TokenStream {
    match origin {
        "sla" => quote! { feotest::model::ThresholdOrigin::Sla },
        "slo" => quote! { feotest::model::ThresholdOrigin::Slo },
        "policy" => quote! { feotest::model::ThresholdOrigin::Policy },
        "empirical" => quote! { feotest::model::ThresholdOrigin::Empirical },
        _ => quote! { feotest::model::ThresholdOrigin::Unspecified },
    }
}
