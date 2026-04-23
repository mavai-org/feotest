//! Code expansion for `#[probabilistic_test]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemFn, ReturnType};

use crate::parse::{
    Approach, PTestAttrs, ParsedDuration, ParsedPacing, parse_duration_str, parse_pacing_str,
};

/// Expands the `#[probabilistic_test(...)]` attribute into a `#[test]` function.
pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let attrs: PTestAttrs = syn::parse2(attr)?;
    let func: ItemFn = syn::parse2(item)?;
    let approach = attrs.detect_approach()?;

    validate_function_signature(&func)?;

    let fn_name = &func.sig.ident;
    let fn_name_str = fn_name.to_string();
    let fn_body = &func.block;
    let fn_vis = &func.vis;

    // Extract the inner function's parameters and return type
    let has_input_param = !func.sig.inputs.is_empty();

    // Build the trial wrapper
    let trial_fn = if has_input_param {
        quote! {
            fn __feotest_trial_fn(input: &str) -> bool #fn_body
        }
    } else {
        quote! {
            fn __feotest_trial_fn(_input: &str) -> bool {
                let __feotest_inner = move || -> bool #fn_body;
                __feotest_inner()
            }
        }
    };

    // Build spec resolution (for spec-based approaches)
    let spec_resolution = build_spec_resolution(&attrs);

    // Build validation call
    let validation = build_validation(&attrs, &fn_name_str);

    // Build approach configuration
    let approach_expr = build_approach(approach, &attrs);

    // Build optional builder calls
    let optional_calls = build_optional_calls(&attrs);

    // Build execution config if time_budget or pacing is set
    let exec_config = build_execution_config(&attrs);

    Ok(quote! {
        #[test]
        #fn_vis fn #fn_name() {
            #trial_fn

            let __feotest_inputs = vec!["default".to_string()];

            let __feotest_trial_wrapper = |input: &str| -> feotest::model::TrialOutcome {
                let __feotest_start = std::time::Instant::now();
                if __feotest_trial_fn(input) {
                    feotest::model::TrialOutcome::success(__feotest_start.elapsed())
                } else {
                    feotest::model::TrialOutcome::failure(
                        feotest::model::ContractViolation::new(
                            "probabilistic_test",
                            "trial returned false",
                        ),
                        __feotest_start.elapsed(),
                    )
                }
            };

            #spec_resolution

            #validation

            let __feotest_result = feotest::ptest::ProbabilisticTestBuilder::builder()
                .use_case_id(#fn_name_str)
                .use_case(|| ())
                .inputs(&__feotest_inputs)
                .trial(|(): &(), input: &str| __feotest_trial_wrapper(input))
                #approach_expr
                #optional_calls
                #exec_config
                .build()
                .run();

            assert!(
                __feotest_result.passed(),
                "Probabilistic test '{}' failed: {:?}",
                #fn_name_str,
                __feotest_result.verdict_record().verdict()
            );
        }
    })
}

/// Validates that the function signature is compatible with the macro.
fn validate_function_signature(func: &ItemFn) -> syn::Result<()> {
    // Must return bool (or have no explicit return type, treated as -> ())
    match &func.sig.output {
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                &func.sig,
                "probabilistic test function must return bool",
            ));
        }
        ReturnType::Type(_, ty) => {
            // Check if the return type is `bool`
            if let syn::Type::Path(tp) = ty.as_ref() {
                if tp.path.is_ident("bool") {
                    // ok
                } else {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "probabilistic test function must return bool",
                    ));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    ty,
                    "probabilistic test function must return bool",
                ));
            }
        }
    }

    // Must have 0 or 1 parameters
    if func.sig.inputs.len() > 1 {
        return Err(syn::Error::new_spanned(
            &func.sig.inputs,
            "probabilistic test function must take 0 or 1 parameter (input: &str)",
        ));
    }

    // Must not be async
    if func.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "probabilistic test functions cannot be async",
        ));
    }

    Ok(())
}

/// Builds the spec resolution code for spec-based approaches.
fn build_spec_resolution(attrs: &PTestAttrs) -> TokenStream {
    attrs
        .spec
        .as_ref()
        .map_or_else(TokenStream::new, |spec_path| {
            quote! {
                let __feotest_spec = feotest::spec::SpecResolver::resolve_file(
                    concat!(env!("CARGO_MANIFEST_DIR"), "/", #spec_path)
                ).expect("failed to load baseline spec");
            }
        })
}

/// Helper to generate an `Option<T>` token stream from an `Option<T>`.
fn option_expr<T: quote::ToTokens>(opt: Option<T>) -> TokenStream {
    opt.map_or_else(|| quote! { None }, |v| quote! { Some(#v) })
}

/// Builds the runtime validation call.
fn build_validation(attrs: &PTestAttrs, test_name: &str) -> TokenStream {
    let samples_expr = option_expr(attrs.samples);
    let threshold_expr = option_expr(attrs.threshold);
    let confidence_expr = option_expr(attrs.confidence);
    let mde_expr = option_expr(attrs.min_detectable_effect);
    let power_expr = option_expr(attrs.power);

    let origin_expr = build_threshold_origin_expr(attrs.threshold_origin.as_deref());

    let (has_baseline, baseline_rate) = if attrs.spec.is_some() {
        (
            quote! { true },
            quote! { Some(__feotest_spec.statistics.success_rate.observed) },
        )
    } else {
        (quote! { false }, quote! { None })
    };

    quote! {
        feotest::ptest::validation::validate(
            &feotest::ptest::validation::MacroConfig {
                test_name: #test_name.to_string(),
                samples: #samples_expr,
                threshold: #threshold_expr,
                confidence: #confidence_expr,
                min_detectable_effect: #mde_expr,
                power: #power_expr,
                threshold_origin: #origin_expr,
                has_baseline: #has_baseline,
                baseline_rate: #baseline_rate,
            }
        );
    }
}

/// Builds the approach expression for the builder.
fn build_approach(approach: Approach, attrs: &PTestAttrs) -> TokenStream {
    match approach {
        Approach::ThresholdFirst => {
            let samples = attrs.samples.unwrap();
            let threshold = attrs.threshold.unwrap();
            quote! {
                .approach(feotest::ptest::builder::ThresholdApproach::ThresholdFirst {
                    samples: #samples,
                    min_pass_rate: #threshold,
                })
            }
        }
        Approach::SampleSizeFirst => {
            let samples = attrs.samples.unwrap();
            let confidence = attrs.confidence.unwrap();
            quote! {
                .approach(feotest::ptest::builder::ThresholdApproach::SampleSizeFirst {
                    samples: #samples,
                    confidence: #confidence,
                })
                .baseline_spec(__feotest_spec)
            }
        }
        Approach::ConfidenceFirst => {
            let confidence = attrs.confidence.unwrap();
            let mde = attrs.min_detectable_effect.unwrap();
            let power = attrs.power.unwrap();
            quote! {
                .approach(feotest::ptest::builder::ThresholdApproach::ConfidenceFirst {
                    confidence: #confidence,
                    min_detectable_effect: #mde,
                    power: #power,
                })
                .baseline_spec(__feotest_spec)
            }
        }
    }
}

/// Builds optional builder method calls.
fn build_optional_calls(attrs: &PTestAttrs) -> TokenStream {
    let mut calls = TokenStream::new();

    if let Some(intent) = &attrs.intent {
        let intent_expr = if intent == "smoke" {
            quote! { feotest::model::TestIntent::Smoke }
        } else {
            quote! { feotest::model::TestIntent::Verification }
        };
        calls.extend(quote! { .intent(#intent_expr) });
    }

    if attrs.threshold_origin.is_some() {
        let origin_expr = build_threshold_origin_expr(attrs.threshold_origin.as_deref());
        calls.extend(quote! { .threshold_origin(#origin_expr) });
    }

    if let Some(contract_ref) = &attrs.contract_ref {
        calls.extend(quote! { .contract_ref(#contract_ref) });
    }

    if let Some(transparent) = attrs.transparent_stats {
        calls.extend(quote! { .transparent_stats(#transparent) });
    }

    calls
}

/// Builds execution config if `time_budget` or `pacing` is set.
fn build_execution_config(attrs: &PTestAttrs) -> TokenStream {
    let has_time_budget = attrs.time_budget.is_some();
    let has_pacing = attrs.pacing.is_some();

    if !has_time_budget && !has_pacing {
        return TokenStream::new();
    }

    // For confidence-first, samples are computed at runtime — we can't set
    // them at compile time. The execution_config override will use a
    // placeholder that gets overridden by the runner.
    // For other approaches, we know samples at compile time.
    // For confidence-first, samples are computed at runtime; use 1 as
    // placeholder (the runner overrides with the approach-computed value).
    let samples_expr = attrs
        .samples
        .map_or_else(|| quote! { 1 }, |n| quote! { #n });

    let mut config_chain = quote! {
        feotest::controls::ExecutionConfig::new(#samples_expr)
    };

    if let Some(tb) = &attrs.time_budget {
        // Already validated at parse time
        let duration_expr = match parse_duration_str(tb).unwrap() {
            ParsedDuration::Seconds(s) => quote! { std::time::Duration::from_secs(#s) },
            ParsedDuration::Millis(ms) => quote! { std::time::Duration::from_millis(#ms) },
        };
        config_chain.extend(quote! { .with_time_budget(#duration_expr) });
    }

    if let Some(p) = &attrs.pacing {
        // Already validated at parse time
        let pacing_expr = match parse_pacing_str(p).unwrap() {
            ParsedPacing::PerSecond(rps) => quote! {
                feotest::controls::PacingConfig::new()
                    .max_requests_per_second(#rps)
            },
            ParsedPacing::PerMinute(rpm) => quote! {
                feotest::controls::PacingConfig::new()
                    .max_requests_per_minute(#rpm)
            },
        };
        config_chain.extend(quote! { .pacing(#pacing_expr) });
    }

    quote! { .execution_config(#config_chain) }
}

/// Builds the `ThresholdOrigin` expression from a string value.
fn build_threshold_origin_expr(origin: Option<&str>) -> TokenStream {
    match origin {
        Some("sla") => quote! { feotest::model::ThresholdOrigin::Sla },
        Some("slo") => quote! { feotest::model::ThresholdOrigin::Slo },
        Some("policy") => quote! { feotest::model::ThresholdOrigin::Policy },
        Some("empirical") => quote! { feotest::model::ThresholdOrigin::Empirical },
        _ => quote! { feotest::model::ThresholdOrigin::Unspecified },
    }
}
