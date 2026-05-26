//! Attribute parsing for `#[probabilistic_test(...)]`.

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitFloat, LitInt, LitStr, Token};

/// Parsed attributes from `#[probabilistic_test(...)]`.
#[derive(Debug, Default)]
pub struct PTestAttrs {
    pub samples: Option<u32>,
    pub confidence: Option<f64>,
    pub threshold: Option<f64>,
    pub min_detectable_effect: Option<f64>,
    pub power: Option<f64>,
    pub spec: Option<String>,
    pub intent: Option<String>,
    pub threshold_origin: Option<String>,
    pub contract_ref: Option<String>,
}

/// The operational approach a `#[probabilistic_test]` attribute selects.
///
/// The macro deliberately exposes only two of the methodology's three
/// approaches. Confidence-first (power-based sample sizing) is intentionally
/// omitted from the attribute surface — it remains available on the builder
/// API (`ProbabilisticTest::for_contract(..).approach(ThresholdApproach::ConfidenceFirst { .. })`),
/// which is the right home for its runtime-computed sample count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::enum_variant_names,
    reason = "the -First suffix carries the semantic distinction"
)]
pub enum Approach {
    ThresholdFirst,
    SampleSizeFirst,
}

impl PTestAttrs {
    /// Detects the operational approach from the attribute combination.
    ///
    /// # Errors
    ///
    /// Returns a compile error if the combination does not match any approach.
    pub fn detect_approach(&self) -> syn::Result<Approach> {
        let has_samples = self.samples.is_some();
        let has_confidence = self.confidence.is_some();
        let has_threshold = self.threshold.is_some();
        let has_mde = self.min_detectable_effect.is_some();
        let has_power = self.power.is_some();
        let has_spec = self.spec.is_some();

        // Threshold-first: samples + threshold, no confidence/mde/power
        if has_samples && has_threshold && !has_confidence && !has_mde && !has_power {
            return Ok(Approach::ThresholdFirst);
        }

        // Sample-size-first: samples + confidence + spec, no threshold
        if has_samples && has_confidence && has_spec && !has_threshold && !has_mde && !has_power {
            return Ok(Approach::SampleSizeFirst);
        }

        // If we get here, the combination doesn't match any approach.
        // Give a helpful error message.
        Err(syn::Error::new(
            Span::call_site(),
            format!(
                "cannot detect operational approach from the given attributes.\n\
                 \n\
                 Valid combinations:\n  \
                 Threshold-first:   samples + threshold\n  \
                 Sample-size-first: samples + confidence + spec\n\
                 \n\
                 Present: {}",
                self.present_attrs_summary(),
            ),
        ))
    }

    fn present_attrs_summary(&self) -> String {
        let mut parts = Vec::new();
        if self.samples.is_some() {
            parts.push("samples");
        }
        if self.threshold.is_some() {
            parts.push("threshold");
        }
        if self.confidence.is_some() {
            parts.push("confidence");
        }
        if self.min_detectable_effect.is_some() {
            parts.push("min_detectable_effect");
        }
        if self.power.is_some() {
            parts.push("power");
        }
        if self.spec.is_some() {
            parts.push("spec");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Parses a numeric literal as f64, accepting both integer and float literals.
fn parse_f64(input: ParseStream) -> syn::Result<f64> {
    let lookahead = input.lookahead1();
    if lookahead.peek(LitFloat) {
        let lit: LitFloat = input.parse()?;
        lit.base10_parse()
    } else if lookahead.peek(LitInt) {
        let lit: LitInt = input.parse()?;
        let val: u64 = lit.base10_parse()?;
        #[allow(
            clippy::cast_precision_loss,
            reason = "macro-input literal values fit in f64 mantissa"
        )]
        Ok(val as f64)
    } else {
        Err(lookahead.error())
    }
}

impl Parse for PTestAttrs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut attrs = Self::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;

            match key.to_string().as_str() {
                "samples" => {
                    let lit: LitInt = input.parse()?;
                    attrs.samples = Some(lit.base10_parse()?);
                }
                "confidence" => {
                    attrs.confidence = Some(parse_f64(input)?);
                }
                "threshold" => {
                    attrs.threshold = Some(parse_f64(input)?);
                }
                "min_detectable_effect" => {
                    attrs.min_detectable_effect = Some(parse_f64(input)?);
                }
                "power" => {
                    attrs.power = Some(parse_f64(input)?);
                }
                "spec" => {
                    let lit: LitStr = input.parse()?;
                    attrs.spec = Some(lit.value());
                }
                "intent" => {
                    let lit: LitStr = input.parse()?;
                    let val = lit.value();
                    if val != "verification" && val != "smoke" {
                        return Err(syn::Error::new(
                            lit.span(),
                            "intent must be \"verification\" or \"smoke\"",
                        ));
                    }
                    attrs.intent = Some(val);
                }
                "threshold_origin" => {
                    let lit: LitStr = input.parse()?;
                    let val = lit.value();
                    if !matches!(val.as_str(), "sla" | "slo" | "policy" | "empirical") {
                        return Err(syn::Error::new(
                            lit.span(),
                            "threshold_origin must be \"sla\", \"slo\", \"policy\", or \"empirical\"",
                        ));
                    }
                    attrs.threshold_origin = Some(val);
                }
                "contract_ref" => {
                    let lit: LitStr = input.parse()?;
                    attrs.contract_ref = Some(lit.value());
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown attribute: `{other}`"),
                    ));
                }
            }

            // Consume optional trailing comma
            if input.peek(Token![,]) {
                let _comma: Token![,] = input.parse()?;
            }
        }

        Ok(attrs)
    }
}
