//! Attribute parsing for `#[probabilistic_test(...)]`.

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitBool, LitFloat, LitInt, LitStr, Token};

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
    pub transparent_stats: Option<bool>,
    pub time_budget: Option<String>,
    pub pacing: Option<String>,
}

/// The detected operational approach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::enum_variant_names,
    reason = "the -First suffix carries the semantic distinction"
)]
pub enum Approach {
    ThresholdFirst,
    SampleSizeFirst,
    ConfidenceFirst,
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

        // Confidence-first: confidence + mde + power + spec, no samples/threshold
        if has_confidence && has_mde && has_power && has_spec && !has_samples && !has_threshold {
            return Ok(Approach::ConfidenceFirst);
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
                 Sample-size-first: samples + confidence + spec\n  \
                 Confidence-first:  confidence + min_detectable_effect + power + spec\n\
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
                "transparent_stats" => {
                    let lit: LitBool = input.parse()?;
                    attrs.transparent_stats = Some(lit.value());
                }
                "time_budget" => {
                    let lit: LitStr = input.parse()?;
                    // Validate format at compile time
                    let val = lit.value();
                    parse_duration_str(&val).map_err(|msg| syn::Error::new(lit.span(), msg))?;
                    attrs.time_budget = Some(val);
                }
                "pacing" => {
                    let lit: LitStr = input.parse()?;
                    let val = lit.value();
                    parse_pacing_str(&val).map_err(|msg| syn::Error::new(lit.span(), msg))?;
                    attrs.pacing = Some(val);
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

/// Parsed duration (seconds, milliseconds).
#[derive(Debug, Clone, Copy)]
pub enum ParsedDuration {
    Seconds(u64),
    Millis(u64),
}

/// Parses a duration string like "30s", "5m", "500ms".
pub fn parse_duration_str(s: &str) -> Result<ParsedDuration, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix("ms") {
        let val: u64 = rest
            .parse()
            .map_err(|_| format!("invalid duration: \"{s}\" — expected a number before \"ms\""))?;
        Ok(ParsedDuration::Millis(val))
    } else if let Some(rest) = s.strip_suffix('s') {
        let val: u64 = rest
            .parse()
            .map_err(|_| format!("invalid duration: \"{s}\" — expected a number before \"s\""))?;
        Ok(ParsedDuration::Seconds(val))
    } else if let Some(rest) = s.strip_suffix('m') {
        let val: u64 = rest
            .parse()
            .map_err(|_| format!("invalid duration: \"{s}\" — expected a number before \"m\""))?;
        Ok(ParsedDuration::Seconds(val * 60))
    } else {
        Err(format!(
            "invalid duration: \"{s}\" — expected format like \"30s\", \"5m\", or \"500ms\""
        ))
    }
}

/// Parsed pacing (requests per second or per minute).
#[derive(Debug, Clone, Copy)]
pub enum ParsedPacing {
    PerSecond(f64),
    PerMinute(f64),
}

/// Parses a pacing string like "10/s" or "100/m".
pub fn parse_pacing_str(s: &str) -> Result<ParsedPacing, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix("/s") {
        let val: f64 = rest
            .parse()
            .map_err(|_| format!("invalid pacing: \"{s}\" — expected a number before \"/s\""))?;
        if val <= 0.0 {
            return Err(format!("invalid pacing: rate must be positive, got {val}"));
        }
        Ok(ParsedPacing::PerSecond(val))
    } else if let Some(rest) = s.strip_suffix("/m") {
        let val: f64 = rest
            .parse()
            .map_err(|_| format!("invalid pacing: \"{s}\" — expected a number before \"/m\""))?;
        if val <= 0.0 {
            return Err(format!("invalid pacing: rate must be positive, got {val}"));
        }
        Ok(ParsedPacing::PerMinute(val))
    } else {
        Err(format!(
            "invalid pacing: \"{s}\" — expected format like \"10/s\" or \"100/m\""
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_seconds() {
        assert!(matches!(
            parse_duration_str("30s"),
            Ok(ParsedDuration::Seconds(30))
        ));
    }

    #[test]
    fn parse_duration_minutes() {
        assert!(matches!(
            parse_duration_str("5m"),
            Ok(ParsedDuration::Seconds(300))
        ));
    }

    #[test]
    fn parse_duration_millis() {
        assert!(matches!(
            parse_duration_str("500ms"),
            Ok(ParsedDuration::Millis(500))
        ));
    }

    #[test]
    fn parse_duration_invalid() {
        assert!(parse_duration_str("abc").is_err());
        assert!(parse_duration_str("30x").is_err());
    }

    #[test]
    fn parse_pacing_per_second() {
        assert!(
            matches!(parse_pacing_str("10/s"), Ok(ParsedPacing::PerSecond(v)) if (v - 10.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn parse_pacing_per_minute() {
        assert!(
            matches!(parse_pacing_str("100/m"), Ok(ParsedPacing::PerMinute(v)) if (v - 100.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn parse_pacing_invalid() {
        assert!(parse_pacing_str("abc").is_err());
        assert!(parse_pacing_str("0/s").is_err());
        assert!(parse_pacing_str("-5/s").is_err());
    }
}
