//! Code expansion for `#[sentinel_impl]`.
//!
//! Applied to the `impl` block of a reliability specification struct, this
//! macro processes two kinds of marker attributes on contained methods and
//! emits the corresponding sentinel-registry submissions:
//!
//! - `#[probabilistic_test(origin = "...", threshold = ..., samples = ...,
//!   confidence = ..., baseline = "...")]` — registers a probabilistic
//!   test descriptor. Normative origins (`sla`, `slo`, `policy`) use the
//!   threshold-first approach; `empirical` origin uses sample-size-first
//!   against a baseline resolved through the sentinel chain.
//! - `#[measure_experiment(baseline_for = "...", samples = ...)]` —
//!   registers a measure experiment whose output is consumed by the test
//!   named in `baseline_for`.
//!
//! The inner attributes are parsed and stripped; the method bodies are
//! emitted unchanged. The macro does not collide with the free-function
//! `#[probabilistic_test]` because the free-function macro never sees
//! the attribute — the outer `#[sentinel_impl]` expansion rewrites the
//! impl block before any further attribute expansion occurs.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
    Error, ImplItem, ImplItemFn, ItemImpl, LitFloat, LitInt, LitStr, Result, Type, parse2,
    spanned::Spanned,
};

const TEST_ATTR: &str = "probabilistic_test";
const EXPERIMENT_ATTR: &str = "measure_experiment";

pub fn expand(_attr: &TokenStream, item: TokenStream) -> Result<TokenStream> {
    let mut item_impl: ItemImpl = parse2(item)?;
    let self_ty = &item_impl.self_ty;
    let self_ident = type_ident(self_ty).ok_or_else(|| {
        Error::new(
            self_ty.span(),
            "`#[sentinel_impl]` requires the impl target to be a plain struct type",
        )
    })?;

    let mut registrations = Vec::new();
    let mut declared_tests: Vec<String> = Vec::new();
    let mut paired_measures: Vec<(String, String, Span)> = Vec::new();

    for item in &mut item_impl.items {
        let ImplItem::Fn(method) = item else { continue };
        let found = extract_marker(method)?;
        match found {
            None => {}
            Some(Marker::Test(cfg)) => {
                declared_tests.push(method.sig.ident.to_string());
                registrations.push(emit_test(&self_ident, method, &cfg)?);
            }
            Some(Marker::Experiment(cfg)) => {
                if let Some(target) = cfg.baseline_for.as_ref() {
                    paired_measures.push((
                        target.value(),
                        method.sig.ident.to_string(),
                        target.span(),
                    ));
                }
                registrations.push(emit_experiment(&self_ident, method, &cfg)?);
            }
        }
    }

    for (target, measure_name, span) in &paired_measures {
        if !declared_tests.iter().any(|t| t == target) {
            return Err(Error::new(
                *span,
                format!(
                    "measure experiment `{measure_name}` references a \
                     probabilistic test `{target}` that is not declared in \
                     this `impl` block"
                ),
            ));
        }
    }

    Ok(quote! {
        #item_impl
        #(#registrations)*
    })
}

enum Marker {
    Test(TestCfg),
    Experiment(ExperimentCfg),
}

fn extract_marker(method: &mut ImplItemFn) -> Result<Option<Marker>> {
    let original_attrs = core::mem::take(&mut method.attrs);
    let mut found: Option<Marker> = None;
    let mut kept = Vec::with_capacity(original_attrs.len());
    for attr in original_attrs {
        if attr.path().is_ident(TEST_ATTR) {
            if found.is_some() {
                method.attrs = kept;
                return Err(Error::new(
                    attr.span(),
                    "method may carry at most one sentinel marker attribute",
                ));
            }
            found = Some(Marker::Test(TestCfg::parse(&attr)?));
        } else if attr.path().is_ident(EXPERIMENT_ATTR) {
            if found.is_some() {
                method.attrs = kept;
                return Err(Error::new(
                    attr.span(),
                    "method may carry at most one sentinel marker attribute",
                ));
            }
            found = Some(Marker::Experiment(ExperimentCfg::parse(&attr)?));
        } else {
            kept.push(attr);
        }
    }
    method.attrs = kept;
    if found.is_some() {
        validate_return_type_is_bool(method)?;
    }
    Ok(found)
}

fn validate_return_type_is_bool(method: &ImplItemFn) -> Result<()> {
    let syn::ReturnType::Type(_, ty) = &method.sig.output else {
        return Err(Error::new(
            method.sig.output.span(),
            "sentinel test and experiment methods must return `bool`",
        ));
    };
    if let Type::Path(path) = ty.as_ref() {
        if path.path.is_ident("bool") {
            return Ok(());
        }
    }
    Err(Error::new(
        ty.span(),
        "sentinel test and experiment methods must return `bool`",
    ))
}

fn type_ident(ty: &Type) -> Option<syn::Ident> {
    let Type::Path(path) = ty else { return None };
    path.path.segments.last().map(|s| s.ident.clone())
}

// === Probabilistic test configuration ===

#[derive(Default)]
struct TestCfg {
    origin: Option<LitStr>,
    threshold: Option<LitFloat>,
    samples: Option<LitInt>,
    confidence: Option<LitFloat>,
    baseline: Option<LitStr>,
}

impl TestCfg {
    fn parse(attr: &syn::Attribute) -> Result<Self> {
        let mut out = Self::default();
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("origin") {
                out.origin = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("threshold") {
                out.threshold = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("samples") {
                out.samples = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("confidence") {
                out.confidence = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("baseline") {
                out.baseline = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error(
                    "unknown `#[probabilistic_test]` argument — expected one of: \
                     origin, threshold, samples, confidence, baseline",
                ));
            }
            Ok(())
        })?;
        Ok(out)
    }
}

#[derive(Default)]
struct ExperimentCfg {
    baseline_for: Option<LitStr>,
    samples: Option<LitInt>,
}

impl ExperimentCfg {
    fn parse(attr: &syn::Attribute) -> Result<Self> {
        let mut out = Self::default();
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("baseline_for") {
                out.baseline_for = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("samples") {
                out.samples = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error(
                    "unknown `#[measure_experiment]` argument — expected one of: \
                     baseline_for, samples",
                ));
            }
            Ok(())
        })?;
        Ok(out)
    }
}

fn emit_test(spec_ident: &syn::Ident, method: &ImplItemFn, cfg: &TestCfg) -> Result<TokenStream> {
    let parsed = ParsedTestCfg::from(cfg, method.sig.span())?;
    let method_name = method.sig.ident.clone();
    let method_name_str = method_name.to_string();
    let invoker_ident = format_ident!("__sentinel_invoke_{}_{}", spec_ident, method_name);
    let submit_mod = format_ident!("__sentinel_submit_{}_{}", spec_ident, method_name);
    let approach_tokens = parsed.approach_tokens();
    let baseline_resolution = parsed.baseline_resolution_tokens(&method_name_str);
    let origin_tok = parsed.origin.to_tokens();
    let threshold_tok = option_quote(parsed.threshold_val);
    let samples_val = parsed.samples_val;
    let samples_tok = quote! { Some(#samples_val) };
    let baseline_method_tok = cfg.baseline.as_ref().map_or_else(
        || quote! { None },
        |lit| {
            let v = lit.value();
            quote! { Some(#v) }
        },
    );

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, reason = "generated invoker follows Spec_Method naming for uniqueness")]
        fn #invoker_ident(spec_any: &dyn ::core::any::Any) -> ::feotest::verdict::VerdictRecord {
            use ::feotest::sentinel::ReliabilitySpec as _;
            let spec = spec_any
                .downcast_ref::<#spec_ident>()
                .expect("sentinel invoker: spec type mismatch");
            let baseline: ::core::option::Option<::feotest::spec::BaselineSpec> =
                #baseline_resolution;
            let inputs: ::std::vec::Vec<::std::string::String> =
                ::std::vec!["default".to_string()];
            let trial = |_input: &str| -> ::feotest::model::TrialOutcome {
                let start = ::std::time::Instant::now();
                let success = spec.#method_name();
                if success {
                    ::feotest::model::TrialOutcome::success(start.elapsed())
                } else {
                    ::feotest::model::TrialOutcome::failure(
                        ::feotest::model::ContractViolation::new(
                            "sentinel_probabilistic_test",
                            "trial method returned false",
                        ),
                        start.elapsed(),
                    )
                }
            };
            let use_case_id = ::std::format!("{}.{}", spec.name(), #method_name_str);
            let mut builder = ::feotest::ptest::ProbabilisticTestBuilder::new(
                use_case_id,
                &inputs,
                trial,
            )
            .approach(#approach_tokens)
            .threshold_origin(#origin_tok);
            if let Some(b) = baseline {
                builder = builder.baseline_spec(b);
            }
            builder.run().verdict_record().clone()
        }

        #[doc(hidden)]
        #[allow(non_snake_case, reason = "generated submission module")]
        mod #submit_mod {
            use super::*;

            ::feotest::inventory::submit! {
                ::feotest::sentinel::ContentDescriptor {
                    spec_type_id: || ::core::any::TypeId::of::<#spec_ident>(),
                    method_name: #method_name_str,
                    kind: ::feotest::sentinel::ContentKind::ProbabilisticTest(
                        ::feotest::sentinel::ProbabilisticTestConfig {
                            origin: #origin_tok,
                            threshold: #threshold_tok,
                            samples: #samples_tok,
                            baseline_method: #baseline_method_tok,
                        }
                    ),
                    invoker: ::feotest::sentinel::ContentInvoker::Test(super::#invoker_ident),
                }
            }
        }
    })
}

/// Validated, parsed form of a probabilistic-test configuration.
///
/// Factors parsing, defaulting, and cross-field validation out of
/// [`emit_test`] so the emitter can focus on token generation. Holds
/// the already-typed values (not `LitFloat` / `LitInt`) so downstream
/// rendering is a direct interpolation.
struct ParsedTestCfg {
    origin: OriginToken,
    threshold_val: Option<f64>,
    samples_val: u32,
    confidence_val: Option<f64>,
    has_baseline: bool,
}

impl ParsedTestCfg {
    fn from(cfg: &TestCfg, span: Span) -> Result<Self> {
        let origin = match cfg.origin.as_ref() {
            Some(lit) => parse_origin(lit)?,
            None => OriginToken::Unspecified,
        };
        let threshold_val = parse_opt_f64(cfg.threshold.as_ref())?;
        let confidence_val = parse_opt_f64(cfg.confidence.as_ref())?;
        let samples_val = parse_opt_u32(cfg.samples.as_ref())?.unwrap_or(100);
        let has_baseline = cfg.baseline.is_some();
        let parsed = Self {
            origin,
            threshold_val,
            samples_val,
            confidence_val,
            has_baseline,
        };
        parsed.validate(span)?;
        Ok(parsed)
    }

    fn validate(&self, span: Span) -> Result<()> {
        let is_normative = matches!(
            self.origin,
            OriginToken::Sla | OriginToken::Slo | OriginToken::Policy | OriginToken::Unspecified
        );
        if is_normative && self.threshold_val.is_none() {
            return Err(Error::new(
                span,
                "normative / unspecified origin requires a `threshold = ...` argument",
            ));
        }
        if matches!(self.origin, OriginToken::Empirical) {
            if self.confidence_val.is_none() {
                return Err(Error::new(
                    span,
                    "empirical origin requires a `confidence = ...` argument for sample-size-first evaluation",
                ));
            }
            if !self.has_baseline {
                return Err(Error::new(
                    span,
                    "empirical origin requires a `baseline = \"<method>\"` argument naming the paired measure experiment",
                ));
            }
        }
        Ok(())
    }

    fn approach_tokens(&self) -> TokenStream {
        let samples = self.samples_val;
        if matches!(self.origin, OriginToken::Empirical) {
            let confidence = self.confidence_val.expect("validated");
            quote! {
                ::feotest::ptest::builder::ThresholdApproach::SampleSizeFirst {
                    samples: #samples,
                    confidence: #confidence,
                }
            }
        } else {
            let threshold = self.threshold_val.expect("validated");
            quote! {
                ::feotest::ptest::builder::ThresholdApproach::ThresholdFirst {
                    samples: #samples,
                    min_pass_rate: #threshold,
                }
            }
        }
    }

    fn baseline_resolution_tokens(&self, method_name: &str) -> TokenStream {
        if !matches!(self.origin, OriginToken::Empirical) {
            return quote! { None };
        }
        quote! {
            {
                let profile = ::feotest::spec::namer::CovariateProfile::empty();
                let use_case_id = format!("{}.{}", spec.name(), #method_name);
                let query = ::feotest::sentinel::BaselineQuery {
                    spec_name: spec.name(),
                    method_name: #method_name,
                    covariate_profile: &profile,
                    use_case_id: &use_case_id,
                };
                let embedded = ::feotest::sentinel::DefaultEmbeddedRegistry;
                let source = ::feotest::sentinel::baseline_source_from_env();
                match ::feotest::sentinel::resolve_baseline(&query, source.as_deref(), &embedded) {
                    Ok(spec) => Some(spec),
                    Err(err) => panic!("{err}"),
                }
            }
        }
    }
}

fn parse_opt_f64(lit: Option<&LitFloat>) -> Result<Option<f64>> {
    lit.map(LitFloat::base10_parse::<f64>).transpose()
}

fn parse_opt_u32(lit: Option<&LitInt>) -> Result<Option<u32>> {
    lit.map(LitInt::base10_parse::<u32>).transpose()
}

fn option_quote<T: quote::ToTokens>(v: Option<T>) -> TokenStream {
    v.map_or_else(|| quote! { None }, |x| quote! { Some(#x) })
}

fn emit_experiment(
    spec_ident: &syn::Ident,
    method: &ImplItemFn,
    cfg: &ExperimentCfg,
) -> Result<TokenStream> {
    let method_name = method.sig.ident.clone();
    let method_name_str = method_name.to_string();
    let samples: u32 = parse_opt_u32(cfg.samples.as_ref())?.unwrap_or(1000);
    let baseline_for_tok = cfg.baseline_for.as_ref().map_or_else(
        || quote! { None },
        |lit| {
            let v = lit.value();
            quote! { Some(#v) }
        },
    );

    let invoker_ident = format_ident!("__sentinel_invoke_{}_{}", spec_ident, method_name);
    let submit_mod = format_ident!("__sentinel_submit_{}_{}", spec_ident, method_name);
    let target_use_case = cfg
        .baseline_for
        .as_ref()
        .map_or_else(|| method_name_str.clone(), LitStr::value);

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, reason = "generated invoker follows Spec_Method naming for uniqueness")]
        fn #invoker_ident(spec_any: &dyn ::core::any::Any) -> ::feotest::spec::BaselineSpec {
            use ::feotest::sentinel::ReliabilitySpec as _;
            let spec = spec_any
                .downcast_ref::<#spec_ident>()
                .expect("sentinel invoker: spec type mismatch");
            let inputs: ::std::vec::Vec<::std::string::String> =
                ::std::vec!["default".to_string()];
            let trial = |_input: &str| -> ::feotest::model::TrialOutcome {
                let start = ::std::time::Instant::now();
                let success = spec.#method_name();
                if success {
                    ::feotest::model::TrialOutcome::success(start.elapsed())
                } else {
                    ::feotest::model::TrialOutcome::failure(
                        ::feotest::model::ContractViolation::new(
                            "sentinel_measure_experiment",
                            "trial method returned false",
                        ),
                        start.elapsed(),
                    )
                }
            };
            struct SentinelUseCase {
                id: ::std::string::String,
            }
            impl ::feotest::usecase::UseCase for SentinelUseCase {
                fn id(&self) -> &str {
                    &self.id
                }
            }
            let use_case = SentinelUseCase {
                id: ::std::format!("{}.{}", spec.name(), #target_use_case),
            };
            ::feotest::experiment::MeasureExperiment::new(
                &use_case,
                #samples,
                &inputs,
                trial,
            )
            .run()
            .spec()
            .clone()
        }

        #[doc(hidden)]
        #[allow(non_snake_case, reason = "generated submission module")]
        mod #submit_mod {
            use super::*;

            ::feotest::inventory::submit! {
                ::feotest::sentinel::ContentDescriptor {
                    spec_type_id: || ::core::any::TypeId::of::<#spec_ident>(),
                    method_name: #method_name_str,
                    kind: ::feotest::sentinel::ContentKind::MeasureExperiment(
                        ::feotest::sentinel::MeasureExperimentConfig {
                            samples: #samples,
                            baseline_for: #baseline_for_tok,
                        }
                    ),
                    invoker: ::feotest::sentinel::ContentInvoker::Experiment(super::#invoker_ident),
                }
            }
        }
    })
}

enum OriginToken {
    Sla,
    Slo,
    Policy,
    Empirical,
    Unspecified,
}

impl OriginToken {
    fn to_tokens(&self) -> TokenStream {
        match self {
            Self::Sla => quote! { ::feotest::model::ThresholdOrigin::Sla },
            Self::Slo => quote! { ::feotest::model::ThresholdOrigin::Slo },
            Self::Policy => quote! { ::feotest::model::ThresholdOrigin::Policy },
            Self::Empirical => quote! { ::feotest::model::ThresholdOrigin::Empirical },
            Self::Unspecified => quote! { ::feotest::model::ThresholdOrigin::Unspecified },
        }
    }
}

fn parse_origin(lit: &LitStr) -> Result<OriginToken> {
    match lit.value().as_str() {
        "sla" | "SLA" => Ok(OriginToken::Sla),
        "slo" | "SLO" => Ok(OriginToken::Slo),
        "policy" | "POLICY" => Ok(OriginToken::Policy),
        "empirical" | "EMPIRICAL" => Ok(OriginToken::Empirical),
        "unspecified" | "UNSPECIFIED" => Ok(OriginToken::Unspecified),
        other => Err(Error::new(
            lit.span(),
            format!(
                "unknown threshold origin `{other}` — expected one of: \
                 sla, slo, policy, empirical, unspecified"
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_origin() {
        let err = parse_origin(&LitStr::new("nonsense", Span::call_site())).unwrap_err();
        assert!(err.to_string().contains("unknown threshold origin"));
    }

    #[test]
    fn accepts_all_known_origins() {
        for o in &["sla", "slo", "policy", "empirical", "unspecified"] {
            assert!(parse_origin(&LitStr::new(o, Span::call_site())).is_ok());
        }
    }
}
