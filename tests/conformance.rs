//! Conformance tests against javai-R reference data.
//!
//! Validates feotest's statistics engine against canonical reference values
//! published by [javai-R](https://github.com/javai-org/javai-R).
//!
//! Pinned javai-R version: see `tests/conformance/VERSION`.

use serde::Deserialize;

use feotest::statistics::evaluator;
use feotest::statistics::feasibility;
use feotest::statistics::latency;
use feotest::statistics::proportion;
use feotest::statistics::sample_size;
use feotest::statistics::threshold;
use feotest::statistics::types::{
    ConfidenceLevel, DerivationContext, DerivedThreshold, OperationalApproach,
};

// ---------------------------------------------------------------------------
// Assertion helper
// ---------------------------------------------------------------------------

/// Asserts that `actual` is within `tolerance` of `expected`, with a message
/// identifying the failing case and field.
fn assert_close(actual: f64, expected: f64, tolerance: f64, case_name: &str, field: &str) {
    let diff = (actual - expected).abs();
    assert!(
        diff <= tolerance,
        "Case '{case_name}', field '{field}': \
         expected {expected}, got {actual} (diff: {diff}, tolerance: {tolerance})"
    );
}

// ---------------------------------------------------------------------------
// Shared suite envelope
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Suite<C> {
    tolerance: f64,
    cases: Vec<C>,
}

// ---------------------------------------------------------------------------
// wilson_ci
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WilsonCiCase {
    name: String,
    inputs: WilsonCiInputs,
    expected: WilsonCiExpected,
}

#[derive(Deserialize)]
struct WilsonCiInputs {
    successes: u32,
    trials: u32,
    confidence: f64,
}

#[derive(Deserialize)]
struct WilsonCiExpected {
    lower: f64,
    upper: f64,
    point: f64,
}

#[test]
fn conformance_wilson_ci() {
    let suite: Suite<WilsonCiCase> =
        serde_json::from_str(include_str!("conformance/wilson_ci.json")).unwrap();

    for case in &suite.cases {
        let cl = ConfidenceLevel::new(case.inputs.confidence);
        let est = proportion::estimate(case.inputs.successes, case.inputs.trials, cl);

        assert_close(
            est.point_estimate(),
            case.expected.point,
            suite.tolerance,
            &case.name,
            "point",
        );
        assert_close(
            est.lower_bound(),
            case.expected.lower,
            suite.tolerance,
            &case.name,
            "lower",
        );
        assert_close(
            est.upper_bound(),
            case.expected.upper,
            suite.tolerance,
            &case.name,
            "upper",
        );
    }
}

// ---------------------------------------------------------------------------
// wilson_lower
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WilsonLowerCase {
    name: String,
    inputs: WilsonLowerInputs,
    expected: WilsonLowerExpected,
}

#[derive(Deserialize)]
struct WilsonLowerInputs {
    successes: u32,
    trials: u32,
    confidence: f64,
}

#[derive(Deserialize)]
struct WilsonLowerExpected {
    lower_bound: f64,
}

#[test]
fn conformance_wilson_lower() {
    let suite: Suite<WilsonLowerCase> =
        serde_json::from_str(include_str!("conformance/wilson_lower.json")).unwrap();

    for case in &suite.cases {
        let cl = ConfidenceLevel::new(case.inputs.confidence);
        let lb = proportion::lower_bound(case.inputs.successes, case.inputs.trials, cl);

        assert_close(
            lb,
            case.expected.lower_bound,
            suite.tolerance,
            &case.name,
            "lower_bound",
        );
    }
}

// ---------------------------------------------------------------------------
// threshold_derivation
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ThresholdCase {
    name: String,
    approach: String,
    inputs: ThresholdInputs,
    expected: ThresholdExpected,
}

#[derive(Deserialize)]
struct ThresholdInputs {
    baseline_successes: u32,
    baseline_trials: u32,
    #[serde(default)]
    test_samples: Option<u32>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    threshold: Option<f64>,
}

#[derive(Deserialize)]
struct ThresholdExpected {
    #[serde(default)]
    threshold: Option<f64>,
    #[serde(default)]
    implied_confidence: Option<f64>,
    #[serde(default)]
    is_sound: Option<bool>,
}

#[test]
fn conformance_threshold_derivation() {
    let suite: Suite<ThresholdCase> =
        serde_json::from_str(include_str!("conformance/threshold_derivation.json")).unwrap();

    for case in &suite.cases {
        match case.approach.as_str() {
            "sample_size_first" => {
                let cl = ConfidenceLevel::new(case.inputs.confidence.unwrap());
                let dt = threshold::derive_sample_size_first(
                    case.inputs.baseline_successes,
                    case.inputs.baseline_trials,
                    case.inputs.test_samples.unwrap(),
                    cl,
                );

                assert_close(
                    dt.value(),
                    case.expected.threshold.unwrap(),
                    suite.tolerance,
                    &case.name,
                    "threshold",
                );
            }
            "threshold_first" => {
                // test_samples is not provided for threshold-first cases;
                // the parameter does not affect implied confidence calculation.
                let test_samples = case.inputs.test_samples.unwrap_or(100);
                let dt = threshold::derive_threshold_first(
                    case.inputs.baseline_successes,
                    case.inputs.baseline_trials,
                    test_samples,
                    case.inputs.threshold.unwrap(),
                );

                assert_close(
                    dt.context().confidence().value(),
                    case.expected.implied_confidence.unwrap(),
                    suite.tolerance,
                    &case.name,
                    "implied_confidence",
                );
                assert_eq!(
                    dt.is_statistically_sound(),
                    case.expected.is_sound.unwrap(),
                    "Case '{}', field 'is_sound': expected {}, got {}",
                    case.name,
                    case.expected.is_sound.unwrap(),
                    dt.is_statistically_sound()
                );
            }
            other => panic!("Unknown approach '{other}' in case '{}'", case.name),
        }
    }
}

// ---------------------------------------------------------------------------
// power_analysis
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PowerCase {
    name: String,
    inputs: PowerInputs,
    expected: PowerExpected,
}

#[derive(Deserialize)]
struct PowerInputs {
    baseline_rate: f64,
    min_detectable_effect: f64,
    confidence: f64,
    power: f64,
}

#[derive(Deserialize)]
struct PowerExpected {
    required_samples: u32,
    achieved_power: f64,
}

#[test]
fn conformance_power_analysis() {
    let suite: Suite<PowerCase> =
        serde_json::from_str(include_str!("conformance/power_analysis.json")).unwrap();

    for case in &suite.cases {
        let cl = ConfidenceLevel::new(case.inputs.confidence);
        let req = sample_size::calculate_for_power(
            case.inputs.baseline_rate,
            case.inputs.min_detectable_effect,
            cl,
            case.inputs.power,
        );

        assert_eq!(
            req.required_samples(),
            case.expected.required_samples,
            "Case '{}', field 'required_samples': expected {}, got {}",
            case.name,
            case.expected.required_samples,
            req.required_samples()
        );

        let achieved = sample_size::calculate_achieved_power(
            req.required_samples(),
            case.inputs.baseline_rate,
            case.inputs.min_detectable_effect,
            cl,
        );

        assert_close(
            achieved,
            case.expected.achieved_power,
            suite.tolerance,
            &case.name,
            "achieved_power",
        );
    }
}

// ---------------------------------------------------------------------------
// feasibility
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FeasibilityCase {
    name: String,
    inputs: FeasibilityInputs,
    expected: FeasibilityExpected,
}

#[derive(Deserialize)]
struct FeasibilityInputs {
    target_proportion: f64,
    sample_size: u32,
    confidence: f64,
}

#[derive(Deserialize)]
struct FeasibilityExpected {
    feasible: bool,
    minimum_samples: u32,
}

#[test]
fn conformance_feasibility() {
    let suite: Suite<FeasibilityCase> =
        serde_json::from_str(include_str!("conformance/feasibility.json")).unwrap();

    for case in &suite.cases {
        let cl = ConfidenceLevel::new(case.inputs.confidence);
        let result = feasibility::feasibility_check(
            case.inputs.sample_size,
            case.inputs.target_proportion,
            cl,
        );

        assert_eq!(
            result.feasible(),
            case.expected.feasible,
            "Case '{}', field 'feasible': expected {}, got {}",
            case.name,
            case.expected.feasible,
            result.feasible()
        );
        assert_eq!(
            result.minimum_samples(),
            case.expected.minimum_samples,
            "Case '{}', field 'minimum_samples': expected {}, got {}",
            case.name,
            case.expected.minimum_samples,
            result.minimum_samples()
        );
    }
}

// ---------------------------------------------------------------------------
// verdict
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct VerdictCase {
    name: String,
    inputs: VerdictInputs,
    expected: VerdictExpected,
}

#[derive(Deserialize)]
struct VerdictInputs {
    successes: u32,
    trials: u32,
    threshold: f64,
    confidence: f64,
}

#[derive(Deserialize)]
struct VerdictExpected {
    passed: bool,
    observed_rate: f64,
    test_statistic: f64,
    p_value: f64,
    false_positive_probability: f64,
}

#[test]
fn conformance_verdict() {
    let suite: Suite<VerdictCase> =
        serde_json::from_str(include_str!("conformance/verdict.json")).unwrap();

    for case in &suite.cases {
        let cl = ConfidenceLevel::new(case.inputs.confidence);

        // Construct a DerivedThreshold for the evaluator. The baseline
        // parameters are synthetic — only threshold value and confidence
        // affect the evaluation.
        let ctx = DerivationContext::new(
            case.inputs.threshold,
            case.inputs.trials,
            case.inputs.trials,
            cl,
        );
        let dt = DerivedThreshold::new(
            case.inputs.threshold,
            OperationalApproach::SampleSizeFirst,
            ctx,
            true,
        );
        let verdict = evaluator::evaluate(case.inputs.successes, case.inputs.trials, &dt);

        assert_eq!(
            verdict.passed(),
            case.expected.passed,
            "Case '{}', field 'passed': expected {}, got {}",
            case.name,
            case.expected.passed,
            verdict.passed()
        );
        assert_close(
            verdict.observed_rate(),
            case.expected.observed_rate,
            suite.tolerance,
            &case.name,
            "observed_rate",
        );
        assert_close(
            verdict.false_positive_probability(),
            case.expected.false_positive_probability,
            suite.tolerance,
            &case.name,
            "false_positive_probability",
        );

        // z-test statistic and p-value are computed via proportion functions,
        // not via the evaluator. The evaluator uses a simple comparison; the
        // reference data validates the full z-test pipeline.
        let z = proportion::z_test_statistic(
            verdict.observed_rate(),
            case.inputs.threshold,
            case.inputs.trials,
        );
        assert_close(
            z,
            case.expected.test_statistic,
            suite.tolerance,
            &case.name,
            "test_statistic",
        );

        let p = proportion::one_sided_p_value(z);
        assert_close(
            p,
            case.expected.p_value,
            suite.tolerance,
            &case.name,
            "p_value",
        );
    }
}

// ---------------------------------------------------------------------------
// latency_percentile
// ---------------------------------------------------------------------------

/// Deserialises a latency input value that may be a single number or an array.
fn deserialize_latencies<'de, D>(deserializer: D) -> Result<Vec<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(f64),
        Many(Vec<f64>),
    }

    match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(v) => Ok(vec![v]),
        OneOrMany::Many(v) => Ok(v),
    }
}

#[derive(Deserialize)]
struct LatencyPercentileCase {
    name: String,
    inputs: LatencyPercentileInputs,
    expected: LatencyPercentileExpected,
}

#[derive(Deserialize)]
struct LatencyPercentileInputs {
    #[serde(deserialize_with = "deserialize_latencies")]
    latencies: Vec<f64>,
    #[serde(default)]
    percentile: Option<f64>,
}

#[derive(Deserialize)]
struct LatencyPercentileExpected {
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    mean: Option<f64>,
    #[serde(default)]
    sd: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
}

#[test]
fn conformance_latency_percentile() {
    let suite: Suite<LatencyPercentileCase> =
        serde_json::from_str(include_str!("conformance/latency_percentile.json")).unwrap();

    for case in &suite.cases {
        if let Some(p) = case.inputs.percentile {
            // Percentile case
            let result = latency::nearest_rank_percentile(&case.inputs.latencies, p);
            assert_close(
                result,
                case.expected.value.unwrap(),
                suite.tolerance,
                &case.name,
                "value",
            );
        } else {
            // Summary statistics case
            let summary = latency::LatencySummary::from_latencies(&case.inputs.latencies);

            assert_close(
                summary.mean(),
                case.expected.mean.unwrap(),
                suite.tolerance,
                &case.name,
                "mean",
            );
            assert_close(
                summary.max(),
                case.expected.max.unwrap(),
                suite.tolerance,
                &case.name,
                "max",
            );

            match case.expected.sd {
                Some(expected_sd) => {
                    assert_close(summary.sd(), expected_sd, suite.tolerance, &case.name, "sd");
                }
                None => {
                    assert!(
                        summary.sd().is_nan(),
                        "Case '{}', field 'sd': expected NaN, got {}",
                        case.name,
                        summary.sd()
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// latency_threshold
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LatencyThresholdCase {
    name: String,
    inputs: LatencyThresholdInputs,
    expected: LatencyThresholdExpected,
}

#[derive(Deserialize)]
struct LatencyThresholdInputs {
    baseline_percentile: f64,
    baseline_sd: f64,
    baseline_n: u32,
    confidence: f64,
}

#[derive(Deserialize)]
struct LatencyThresholdExpected {
    raw_upper: f64,
    threshold: f64,
}

#[test]
fn conformance_latency_threshold() {
    let suite: Suite<LatencyThresholdCase> =
        serde_json::from_str(include_str!("conformance/latency_threshold.json")).unwrap();

    for case in &suite.cases {
        let result = latency::derive_latency_threshold(
            case.inputs.baseline_percentile,
            case.inputs.baseline_sd,
            case.inputs.baseline_n,
            case.inputs.confidence,
        );

        assert_close(
            result.raw_upper(),
            case.expected.raw_upper,
            suite.tolerance,
            &case.name,
            "raw_upper",
        );
        assert_close(
            result.threshold(),
            case.expected.threshold,
            suite.tolerance,
            &case.name,
            "threshold",
        );
    }
}
