//! Code expansion for `#[measure_experiment]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemFn;

use crate::measure_parse::MeasureAttrs;
use crate::parse::{ParsedDuration, ParsedPacing, parse_duration_str, parse_pacing_str};

/// Expands the `#[measure_experiment(...)]` attribute into a `#[test]` function.
pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let attrs: MeasureAttrs = syn::parse2(attr)?;
    attrs.validate()?;

    let func: ItemFn = syn::parse2(item)?;
    validate_function_signature(&func)?;

    let fn_name = &func.sig.ident;
    let fn_body = &func.block;
    let fn_vis = &func.vis;

    let has_input_param = !func.sig.inputs.is_empty();

    let use_case = attrs.use_case.as_ref().unwrap();
    let samples = attrs.samples.unwrap();

    // Build inputs expression
    let inputs_expr = attrs.inputs.as_ref().map_or_else(
        || quote! { vec!["default".to_string()] },
        |inputs| {
            let items = inputs.iter().map(|s| quote! { #s.to_string() });
            quote! { vec![#(#items),*] }
        },
    );

    // Build the trial function (adapting signature)
    let trial_fn = if has_input_param {
        quote! {
            fn __feotest_trial_fn(input: &str) -> feotest::model::TrialOutcome #fn_body
        }
    } else {
        quote! {
            fn __feotest_trial_fn(_input: &str) -> feotest::model::TrialOutcome {
                let __feotest_inner = move || -> feotest::model::TrialOutcome #fn_body;
                __feotest_inner()
            }
        }
    };

    // Build execution config
    let config_expr = build_execution_config(&attrs);

    // Build optional builder calls
    let experiment_id_call = attrs
        .experiment_id
        .as_ref()
        .map_or_else(TokenStream::new, |id| {
            quote! { .with_experiment_id(#id) }
        });

    let dir = attrs.spec_dir.as_deref().unwrap_or("tests/baselines");
    let spec_resolver_call = quote! {
        .with_spec_resolver(feotest::spec::SpecResolver::with_dir(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/", #dir))
        ))
    };

    Ok(quote! {
        #[test]
        #fn_vis fn #fn_name() {
            #trial_fn

            struct __FeoTestMacroUseCase;
            impl feotest::usecase::UseCase for __FeoTestMacroUseCase {
                fn id(&self) -> &str { #use_case }
            }
            let __feotest_uc = __FeoTestMacroUseCase;

            let __feotest_inputs = #inputs_expr;

            #config_expr

            let __feotest_result = feotest::experiment::MeasureExperiment::new(
                &__feotest_uc,
                #samples,
                &__feotest_inputs,
                __feotest_trial_fn,
            )
            .with_config(__feotest_config)
            #experiment_id_call
            #spec_resolver_call
            .run();
        }
    })
}

/// Validates the function signature for measure experiments.
fn validate_function_signature(func: &ItemFn) -> syn::Result<()> {
    // Must return TrialOutcome (we check for a path ending in "TrialOutcome")
    match &func.sig.output {
        syn::ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                &func.sig,
                "measure experiment function must return TrialOutcome",
            ));
        }
        syn::ReturnType::Type(_, ty) => {
            if let syn::Type::Path(tp) = ty.as_ref() {
                let last = tp.path.segments.last();
                if last.is_none() || last.unwrap().ident != "TrialOutcome" {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "measure experiment function must return TrialOutcome",
                    ));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    ty,
                    "measure experiment function must return TrialOutcome",
                ));
            }
        }
    }

    if func.sig.inputs.len() > 1 {
        return Err(syn::Error::new_spanned(
            &func.sig.inputs,
            "measure experiment function must take 0 or 1 parameter (input: &str)",
        ));
    }

    if func.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "measure experiment functions cannot be async",
        ));
    }

    Ok(())
}

/// Builds the `ExecutionConfig` expression from attributes.
fn build_execution_config(attrs: &MeasureAttrs) -> TokenStream {
    let samples = attrs.samples.unwrap();

    let mut chain = quote! {
        let __feotest_config = feotest::controls::ExecutionConfig::new(#samples)
    };

    if let Some(warmup) = attrs.warmup {
        chain.extend(quote! { .with_warmup(#warmup) });
    }

    if let Some(tb) = &attrs.time_budget {
        let duration_expr = match parse_duration_str(tb).unwrap() {
            ParsedDuration::Seconds(s) => quote! { std::time::Duration::from_secs(#s) },
            ParsedDuration::Millis(ms) => quote! { std::time::Duration::from_millis(#ms) },
        };
        chain.extend(quote! { .with_time_budget(#duration_expr) });
    }

    if let Some(budget) = attrs.token_budget {
        chain.extend(quote! { .with_token_budget(#budget) });
    }

    if let Some(p) = &attrs.pacing {
        let pacing_expr = match parse_pacing_str(p).unwrap() {
            ParsedPacing::PerSecond(rps) => quote! {
                feotest::controls::PacingConfig::new()
                    .with_max_requests_per_second(#rps)
            },
            ParsedPacing::PerMinute(rpm) => quote! {
                feotest::controls::PacingConfig::new()
                    .with_max_requests_per_minute(#rpm)
            },
        };
        chain.extend(quote! { .with_pacing(#pacing_expr) });
    }

    chain.extend(quote! { ; });
    chain
}
