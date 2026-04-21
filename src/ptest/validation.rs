//! Validation of macro-supplied test configurations.
//!
//! Enforces the six mutual-exclusivity and completeness rules that govern
//! how probabilistic test parameters may be combined. Called at test
//! execution time (after baseline resolution, before sample execution).

use crate::model::ThresholdOrigin;

/// Configuration descriptor mirroring the attributes captured by the
/// `#[probabilistic_test]` macro.
#[derive(Debug)]
pub struct MacroConfig {
    /// The test function name.
    pub test_name: String,
    /// Number of samples (`samples` attribute).
    pub samples: Option<u32>,
    /// Explicit minimum pass rate (`threshold` attribute).
    pub threshold: Option<f64>,
    /// Confidence level (`confidence` attribute).
    pub confidence: Option<f64>,
    /// Minimum detectable effect (`min_detectable_effect` attribute).
    pub min_detectable_effect: Option<f64>,
    /// Statistical power (`power` attribute).
    pub power: Option<f64>,
    /// Provenance of the threshold value.
    pub threshold_origin: ThresholdOrigin,
    /// Whether a baseline spec was successfully resolved.
    pub has_baseline: bool,
    /// The observed success rate from the baseline spec, if available.
    pub baseline_rate: Option<f64>,
}

/// Validates a macro configuration, panicking with an actionable message
/// if any of the six rules are violated.
///
/// Rules are checked in order. Only the first violation is reported.
///
/// # Panics
///
/// Panics if the configuration violates any validation rule.
pub fn validate(config: &MacroConfig) {
    rule1_baseline_threshold_conflict(config);
    rule2_no_threshold_defined(config);
    rule3_confidence_requires_baseline(config);
    rule4_confidence_first_requires_baseline_rate(config);
    let over_specified = rule5_over_specified(config);
    if !over_specified {
        rule6_incomplete_confidence_first(config);
    }
}

/// Rule 1: Baseline + explicit threshold = CONFLICT (unless normative origin).
fn rule1_baseline_threshold_conflict(config: &MacroConfig) {
    if !config.has_baseline || config.threshold.is_none() {
        return;
    }
    let threshold = config.threshold.unwrap();

    if config.threshold_origin.is_normative() {
        return; // normative origin justifies the override
    }

    let origin_label = match config.threshold_origin {
        ThresholdOrigin::Empirical => "Empirical",
        _ => "Unspecified",
    };

    panic!(
        "\n\nCONFLICT in probabilistic test '{test}':\n\n\
         A baseline spec exists (observed rate: {rate}) AND an explicit threshold \
         ({threshold:.4}) was specified with origin '{origin}'.\n\n\
         Why this is invalid:\n  \
         An empirically derived threshold should come FROM the baseline, not override it.\n  \
         Specifying both creates ambiguity about which value governs the test.\n\n\
         How to fix:\n  \
         - Remove `threshold = {threshold}` to derive the threshold from the baseline, OR\n  \
         - Set `threshold_origin = \"sla\"` (or \"slo\", \"policy\") if the explicit threshold\n    \
           is a normative requirement that intentionally overrides the baseline.\n\n",
        test = config.test_name,
        rate = config
            .baseline_rate
            .map_or_else(|| "unknown".to_string(), |r| format!("{r:.4}")),
        threshold = threshold,
        origin = origin_label,
    );
}

/// Rule 2: No baseline + no threshold = UNDEFINED.
fn rule2_no_threshold_defined(config: &MacroConfig) {
    if config.has_baseline || config.threshold.is_some() {
        return;
    }

    // Confidence-first and sample-size-first derive thresholds from baselines,
    // so if neither baseline nor explicit threshold exists, there is no criterion.
    // However, if confidence-first params are present, Rule 4 will catch the
    // missing baseline more specifically. Only fire Rule 2 when there are no
    // confidence-first params at all.
    let has_confidence_first_params = config.confidence.is_some()
        || config.min_detectable_effect.is_some()
        || config.power.is_some();

    if has_confidence_first_params {
        return; // Rule 4 will handle this case
    }

    panic!(
        "\n\nUNDEFINED in probabilistic test '{test}':\n\n\
         No baseline spec was found and no explicit threshold was specified.\n\
         The test has no pass/fail criterion.\n\n\
         How to fix:\n  \
         - Add `threshold = 0.90` (or your target pass rate), OR\n  \
         - Add `spec = \"path/to/baseline.yaml\"` to derive a threshold from\n    \
           an empirical baseline.\n\n",
        test = config.test_name,
    );
}

/// Rule 3: Sample-size-first `confidence` requires a baseline.
fn rule3_confidence_requires_baseline(config: &MacroConfig) {
    // Sample-size-first: samples + confidence, no threshold
    let is_sample_size_first =
        config.samples.is_some() && config.confidence.is_some() && config.threshold.is_none();

    if !is_sample_size_first || config.has_baseline {
        return;
    }

    panic!(
        "\n\nREQUIRES_BASELINE in probabilistic test '{test}':\n\n\
         Sample-size-first approach (samples = {samples}, confidence = {confidence:.4}) \
         requires a baseline spec to derive the threshold from.\n\n\
         Why this is invalid:\n  \
         The sample-size-first approach uses the Wilson score interval on baseline\n  \
         data to compute a statistically sound threshold. Without baseline data,\n  \
         there is nothing to derive from.\n\n\
         How to fix:\n  \
         - Add `spec = \"path/to/baseline.yaml\"` to provide baseline data, OR\n  \
         - Switch to threshold-first: `samples = {samples}, threshold = 0.90`\n    \
           (replace 0.90 with your target pass rate).\n\n",
        test = config.test_name,
        samples = config.samples.unwrap(),
        confidence = config.confidence.unwrap(),
    );
}

/// Rule 4: Confidence-first parameters require a baseline rate.
fn rule4_confidence_first_requires_baseline_rate(config: &MacroConfig) {
    let has_all_three = config.confidence.is_some()
        && config.min_detectable_effect.is_some()
        && config.power.is_some();

    if !has_all_three {
        return;
    }

    // A baseline rate can come from the baseline spec OR an explicit threshold
    if config.has_baseline || config.threshold.is_some() {
        return;
    }

    panic!(
        "\n\nREQUIRES_BASELINE_RATE in probabilistic test '{test}':\n\n\
         Confidence-first approach (confidence = {confidence:.4}, \
         min_detectable_effect = {mde:.4}, power = {power:.4}) requires a baseline\n\
         success rate to compute the required sample size via power analysis.\n\n\
         Why this is invalid:\n  \
         Power analysis needs a baseline rate (p₀) to determine how many samples\n  \
         are needed to detect a degradation of the specified effect size.\n\n\
         How to fix:\n  \
         - Add `spec = \"path/to/baseline.yaml\"` to provide an empirical baseline, OR\n  \
         - Add `threshold = 0.90` as a normative baseline rate (with appropriate\n    \
           `threshold_origin`).\n\n",
        test = config.test_name,
        confidence = config.confidence.unwrap(),
        mde = config.min_detectable_effect.unwrap(),
        power = config.power.unwrap(),
    );
}

/// Rule 5: Over-specification (all three key variables pinned).
///
/// Returns `true` if the rule fired, so Rule 6 can be skipped.
fn rule5_over_specified(config: &MacroConfig) -> bool {
    // Over-specified when any confidence parameter is set AND an explicit
    // threshold is also set.
    let has_confidence_param = config.confidence.is_some();
    let has_threshold = config.threshold.is_some();

    if !has_confidence_param || !has_threshold {
        return false;
    }

    let mut params = Vec::new();
    if let Some(s) = config.samples {
        params.push(format!("samples = {s}"));
    }
    if let Some(c) = config.confidence {
        params.push(format!("confidence = {c:.4}"));
    }
    if let Some(t) = config.threshold {
        params.push(format!("threshold = {t:.4}"));
    }
    if let Some(mde) = config.min_detectable_effect {
        params.push(format!("min_detectable_effect = {mde:.4}"));
    }
    if let Some(p) = config.power {
        params.push(format!("power = {p:.4}"));
    }

    panic!(
        "\n\nOVER_SPECIFIED in probabilistic test '{test}':\n\n\
         Parameters: {params}\n\n\
         Why this is invalid:\n  \
         Sample size, confidence, and threshold are mathematically linked.\n  \
         You choose two; statistics derives the third. Specifying all three\n  \
         creates an over-determined system with no consistent solution.\n\n\
         How to fix — pick one approach:\n  \
         - Threshold-first:  samples + threshold            → remove confidence params\n  \
         - Sample-size-first: samples + confidence + spec   → remove threshold\n  \
         - Confidence-first:  confidence + min_detectable_effect + power + spec → remove threshold\n\n",
        test = config.test_name,
        params = params.join(", "),
    );
}

/// Rule 6: Partial confidence-first = INCOMPLETE.
fn rule6_incomplete_confidence_first(config: &MacroConfig) {
    let cf_params = [
        config.confidence.is_some(),
        config.min_detectable_effect.is_some(),
        config.power.is_some(),
    ];

    let count = cf_params.iter().filter(|&&p| p).count();

    // 0 = not attempting confidence-first, 3 = complete
    if count == 0 || count == 3 {
        return;
    }

    // Also skip if samples is present — this may be sample-size-first
    // with just confidence, which is valid.
    if config.samples.is_some() && count == 1 && config.confidence.is_some() {
        return;
    }

    let mut present = Vec::new();
    let mut missing = Vec::new();

    if let Some(c) = config.confidence {
        present.push(format!("confidence = {c:.4}"));
    } else {
        missing.push("confidence");
    }
    if let Some(mde) = config.min_detectable_effect {
        present.push(format!("min_detectable_effect = {mde:.4}"));
    } else {
        missing.push("min_detectable_effect");
    }
    if let Some(p) = config.power {
        present.push(format!("power = {p:.4}"));
    } else {
        missing.push("power");
    }

    panic!(
        "\n\nINCOMPLETE in probabilistic test '{test}':\n\n\
         The confidence-first approach requires all three parameters:\n  \
         confidence, min_detectable_effect, and power.\n\n\
         Present: {present}\n\
         Missing: {missing}\n\n\
         How to fix:\n  \
         - Add the missing parameter(s): {missing_attrs}, OR\n  \
         - Switch to a different approach:\n    \
           Threshold-first:   samples + threshold\n    \
           Sample-size-first: samples + confidence + spec\n\n",
        test = config.test_name,
        present = present.join(", "),
        missing = missing.join(", "),
        missing_attrs = missing
            .iter()
            .map(|m| format!("{m} = ..."))
            .collect::<Vec<_>>()
            .join(", "),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config(test_name: &str) -> MacroConfig {
        MacroConfig {
            test_name: test_name.to_string(),
            samples: None,
            threshold: None,
            confidence: None,
            min_detectable_effect: None,
            power: None,
            threshold_origin: ThresholdOrigin::Unspecified,
            has_baseline: false,
            baseline_rate: None,
        }
    }

    // --- Valid configurations ---

    #[test]
    fn threshold_first_valid() {
        let config = MacroConfig {
            samples: Some(100),
            threshold: Some(0.90),
            ..base_config("valid_threshold_first")
        };
        validate(&config); // should not panic
    }

    #[test]
    fn sample_size_first_valid() {
        let config = MacroConfig {
            samples: Some(200),
            confidence: Some(0.95),
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("valid_sample_size_first")
        };
        validate(&config);
    }

    #[test]
    fn confidence_first_valid() {
        let config = MacroConfig {
            confidence: Some(0.95),
            min_detectable_effect: Some(0.05),
            power: Some(0.80),
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("valid_confidence_first")
        };
        validate(&config);
    }

    #[test]
    fn baseline_with_sla_threshold_allowed() {
        let config = MacroConfig {
            samples: Some(100),
            threshold: Some(0.95),
            threshold_origin: ThresholdOrigin::Sla,
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("normative_override_sla")
        };
        validate(&config);
    }

    #[test]
    fn baseline_with_slo_threshold_allowed() {
        let config = MacroConfig {
            samples: Some(100),
            threshold: Some(0.95),
            threshold_origin: ThresholdOrigin::Slo,
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("normative_override_slo")
        };
        validate(&config);
    }

    #[test]
    fn baseline_with_policy_threshold_allowed() {
        let config = MacroConfig {
            samples: Some(100),
            threshold: Some(0.95),
            threshold_origin: ThresholdOrigin::Policy,
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("normative_override_policy")
        };
        validate(&config);
    }

    // --- Rule 1: CONFLICT ---

    #[test]
    #[should_panic(expected = "CONFLICT")]
    fn rule1_baseline_and_threshold_without_normative_origin() {
        let config = MacroConfig {
            samples: Some(100),
            threshold: Some(0.90),
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("rule1_conflict")
        };
        validate(&config);
    }

    #[test]
    #[should_panic(expected = "CONFLICT")]
    fn rule1_baseline_and_threshold_with_empirical_origin() {
        let config = MacroConfig {
            samples: Some(100),
            threshold: Some(0.90),
            threshold_origin: ThresholdOrigin::Empirical,
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("rule1_empirical")
        };
        validate(&config);
    }

    // --- Rule 2: UNDEFINED ---

    #[test]
    #[should_panic(expected = "UNDEFINED")]
    fn rule2_no_baseline_no_threshold() {
        let config = MacroConfig {
            samples: Some(100),
            ..base_config("rule2_undefined")
        };
        validate(&config);
    }

    // --- Rule 3: REQUIRES_BASELINE ---

    #[test]
    #[should_panic(expected = "REQUIRES_BASELINE")]
    fn rule3_sample_size_first_without_baseline() {
        let config = MacroConfig {
            samples: Some(200),
            confidence: Some(0.95),
            ..base_config("rule3_no_baseline")
        };
        validate(&config);
    }

    // --- Rule 4: REQUIRES_BASELINE_RATE ---

    #[test]
    #[should_panic(expected = "REQUIRES_BASELINE_RATE")]
    fn rule4_confidence_first_without_baseline_or_threshold() {
        let config = MacroConfig {
            confidence: Some(0.95),
            min_detectable_effect: Some(0.05),
            power: Some(0.80),
            ..base_config("rule4_no_rate")
        };
        validate(&config);
    }

    // --- Rule 5: OVER_SPECIFIED ---

    #[test]
    #[should_panic(expected = "OVER_SPECIFIED")]
    fn rule5_confidence_and_threshold_both_set() {
        // No baseline, so Rule 1 does not fire. But confidence + threshold
        // is over-specified regardless.
        let config = MacroConfig {
            samples: Some(100),
            confidence: Some(0.95),
            threshold: Some(0.90),
            ..base_config("rule5_over")
        };
        validate(&config);
    }

    // --- Rule 6: INCOMPLETE ---

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn rule6_partial_confidence_first() {
        let config = MacroConfig {
            confidence: Some(0.95),
            min_detectable_effect: Some(0.05),
            // power is missing
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("rule6_incomplete")
        };
        validate(&config);
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn rule6_only_power_and_mde() {
        let config = MacroConfig {
            min_detectable_effect: Some(0.05),
            power: Some(0.80),
            // confidence is missing
            has_baseline: true,
            baseline_rate: Some(0.92),
            ..base_config("rule6_no_confidence")
        };
        validate(&config);
    }
}
