//! Conformance tests against mavai-R reference data.
//!
//! Validates feotest's statistics engine against canonical reference values
//! published by [mavai-R](https://github.com/mavai-org/mavai-R).
//!
//! Pinned mavai-R version: see `tests/conformance/VERSION`.
//!
//! # Coverage accounting
//!
//! The oracle publishes `manifest.json` alongside its fixture suites:
//! per-suite case rosters, a binding-vs-informational classification of every
//! expected field, per-suite content hashes, and a family-mandatory suite
//! tier. The obligation on a consumer is the set of
//! `(suite, case, binding-field)` triples across the family-mandatory tier
//! plus this repository's committed `tests/conformance/SCOPE.json` — and the
//! obligation is *self-verified*: every conformance assertion records the
//! triple it asserts into a [`Ledger`], and the umbrella test
//! [`conformance_coverage_meets_manifest`] diffs the recorded set against the
//! manifest. A binding field that is loaded but never asserted is a gap, not
//! a pass.
//!
//! # The scenario suite
//!
//! The `regression_decision` suite is evaluated through the **production
//! verdict path** (`ProbabilisticTest::for_contract(..).run()`), not a
//! test-side reimplementation: a measure experiment establishes the baseline
//! from a scripted contract, a scripted contract delivers the case's observed
//! successes, and the verdict asserted is the one the runner rendered.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use md5::Digest;
use serde::Deserialize;

use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::MeasureExperiment;
use feotest::model::{ContractViolation, ThresholdOrigin};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::service_contract::ServiceContract;
use feotest::spec::SpecResolver;
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
// The coverage ledger
// ---------------------------------------------------------------------------

/// One asserted or obliged `(suite, case, binding-field)` triple.
type Triple = (String, String, String);

/// A manifest field that the oracle's serialiser may unbox to a scalar when
/// it holds a single element.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn to_set(&self) -> BTreeSet<String> {
        match self {
            Self::One(value) => BTreeSet::from([value.clone()]),
            Self::Many(values) => values.iter().cloned().collect(),
        }
    }
}

/// One suite's manifest entry: its file, roster, binding classification, and
/// content hash.
#[derive(Deserialize)]
struct SuiteEntry {
    file: String,
    #[serde(rename = "caseCount")]
    case_count: u32,
    #[serde(rename = "bindingFields")]
    binding_fields: OneOrMany,
    md5: String,
}

/// The oracle's published conformance manifest.
#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "manifestVersion")]
    #[allow(
        clippy::struct_field_names,
        reason = "field name mirrors the oracle's published manifest key"
    )]
    manifest_version: u32,
    #[serde(rename = "fixtureVersion")]
    fixture_version: String,
    #[serde(rename = "familyMandatory")]
    family_mandatory: Vec<String>,
    suites: BTreeMap<String, SuiteEntry>,
}

/// This repository's committed extend-only scope beyond the family-mandatory
/// tier.
#[derive(Deserialize)]
struct Scope {
    suites: Vec<String>,
}

/// The directory holding the vendored fixture snapshot.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("conformance")
}

/// Accumulates asserted `(suite, case, binding-field)` triples and diffs them
/// against the manifest's obligations.
struct Ledger {
    manifest: Manifest,
    scope_suites: Vec<String>,
    asserted: BTreeSet<Triple>,
}

impl Ledger {
    /// Loads the manifest and the committed scope file from the vendored
    /// fixture directory.
    fn load() -> Self {
        let dir = fixtures_dir();
        let manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        let scope: Scope =
            serde_json::from_str(&std::fs::read_to_string(dir.join("SCOPE.json")).unwrap())
                .unwrap();
        Self {
            manifest,
            scope_suites: scope.suites,
            asserted: BTreeSet::new(),
        }
    }

    /// Records that one binding field of one case was asserted.
    fn record(&mut self, suite: &str, case_name: &str, field: &str) {
        self.asserted
            .insert((suite.to_owned(), case_name.to_owned(), field.to_owned()));
    }

    /// Family-mandatory plus committed scope, deduplicated, manifest order.
    fn in_scope_suites(&self) -> Vec<String> {
        let wanted: BTreeSet<&str> = self
            .manifest
            .family_mandatory
            .iter()
            .chain(self.scope_suites.iter())
            .map(String::as_str)
            .collect();
        self.manifest
            .suites
            .keys()
            .filter(|name| wanted.contains(name.as_str()))
            .cloned()
            .collect()
    }

    /// Every `(suite, case, binding-field)` triple the given suites demand.
    ///
    /// A case owes exactly the binding fields present in its own `expected`
    /// block — suites whose case groups carry different expected shapes
    /// (e.g. `threshold_derivation`'s two approaches) owe per-case, not the
    /// suite-wide union.
    fn obligations(&self, suites: &[String]) -> BTreeSet<Triple> {
        let mut out = BTreeSet::new();
        for suite in suites {
            let binding = self.manifest.suites[suite].binding_fields.to_set();
            for (case_name, expected_fields) in self.suite_expected_fields(suite) {
                out.extend(
                    expected_fields
                        .into_iter()
                        .filter(|field| binding.contains(field))
                        .map(|field| (suite.clone(), case_name.clone(), field)),
                );
            }
        }
        out
    }

    /// The obligations not (yet) discharged by a recorded assertion.
    fn gaps(&self) -> BTreeSet<Triple> {
        self.obligations(&self.in_scope_suites())
            .difference(&self.asserted)
            .cloned()
            .collect()
    }

    /// Manifest suites outside scope, with their case counts — reported,
    /// never silently skipped.
    fn unaddressed_suites(&self) -> Vec<(String, u32)> {
        let in_scope: BTreeSet<String> = self.in_scope_suites().into_iter().collect();
        self.manifest
            .suites
            .iter()
            .filter(|(name, _)| !in_scope.contains(*name))
            .map(|(name, entry)| (name.clone(), entry.case_count))
            .collect()
    }

    /// The MD5 hex digest of the vendored suite file.
    fn vendored_md5(&self, suite: &str) -> String {
        use std::fmt::Write as _;
        let bytes = std::fs::read(fixtures_dir().join(&self.manifest.suites[suite].file)).unwrap();
        md5::Md5::digest(&bytes)
            .iter()
            .fold(String::new(), |mut hex, byte| {
                write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
                hex
            })
    }

    /// The MD5 hex digest the manifest publishes for the suite.
    fn manifest_md5(&self, suite: &str) -> &str {
        &self.manifest.suites[suite].md5
    }

    /// The one-line summary every coverage run prints.
    fn standing(&self) -> String {
        let mandatory = self.obligations(&self.manifest.family_mandatory);
        let scoped = self.obligations(&self.scope_suites);
        let mut parts = vec![
            format!("fixtures v{}", self.manifest.fixture_version),
            format!(
                "mandatory {}/{} binding assertions over {} suites",
                mandatory.intersection(&self.asserted).count(),
                mandatory.len(),
                self.manifest.family_mandatory.len()
            ),
            format!(
                "scope {}/{} over {} suites",
                scoped.intersection(&self.asserted).count(),
                scoped.len(),
                self.scope_suites.len()
            ),
        ];
        let unaddressed = self.unaddressed_suites();
        if !unaddressed.is_empty() {
            let named: Vec<String> = unaddressed
                .iter()
                .map(|(name, count)| format!("{name} ({count})"))
                .collect();
            parts.push(format!("unaddressed: {}", named.join(", ")));
        }
        format!("conformance standing: {}", parts.join("; "))
    }

    /// The machine-readable per-run report, for CI surfacing.
    fn report(&self) -> serde_json::Value {
        let gaps: Vec<serde_json::Value> = self
            .gaps()
            .iter()
            .map(|(suite, case, field)| {
                serde_json::json!({ "suite": suite, "case": case, "field": field })
            })
            .collect();
        let unaddressed: Vec<serde_json::Value> = self
            .unaddressed_suites()
            .iter()
            .map(|(name, count)| serde_json::json!({ "suite": name, "caseCount": count }))
            .collect();
        serde_json::json!({
            "fixtureVersion": self.manifest.fixture_version,
            "manifestVersion": self.manifest.manifest_version,
            "mandatorySuites": self.manifest.family_mandatory,
            "scopeSuites": self.scope_suites,
            "assertedTriples": self.asserted.len(),
            "obligedTriples": self.obligations(&self.in_scope_suites()).len(),
            "gaps": gaps,
            "unaddressedSuites": unaddressed,
            "standing": self.standing(),
        })
    }

    /// Writes the JSON coverage report to `target/conformance-report.json`.
    fn write_report(&self) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("conformance-report.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{:#}\n", self.report())).unwrap();
    }

    /// Loads a suite file and returns each case's name and the keys of its
    /// own `expected` block.
    fn suite_expected_fields(&self, suite: &str) -> Vec<(String, Vec<String>)> {
        let raw = std::fs::read_to_string(fixtures_dir().join(&self.manifest.suites[suite].file))
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| {
                let name = case["name"].as_str().unwrap().to_owned();
                let fields = case["expected"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect();
                (name, fields)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Recording assertion helpers
// ---------------------------------------------------------------------------

/// Runs `check` for every case, letting every case run even when an earlier
/// one fails, then panics with the collected failures.
///
/// Fail-late keeps the demonstrated red set complete — one deviating case
/// cannot mask another — and every attempted assertion still records its
/// coverage triple before the failure propagates.
fn assert_all_cases<C>(cases: &[C], name: impl Fn(&C) -> &str, mut check: impl FnMut(&C)) {
    let mut failures: Vec<String> = Vec::new();
    for case in cases {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| check(case))) {
            failures.push(format!("{}: {}", name(case), panic_text(payload.as_ref())));
        }
    }
    assert!(
        failures.is_empty(),
        "\n{} case(s) deviate from the oracle:\n{}\n",
        failures.len(),
        failures.join("\n")
    );
}

/// Asserts one binding field against the oracle within `tolerance` and
/// records its triple. Recording happens before the assertion: an
/// attempted-and-failed assertion is a failure, not a coverage gap.
fn assert_oracle_close(
    ledger: &mut Ledger,
    suite: &str,
    case_name: &str,
    field: &str,
    actual: f64,
    expected: f64,
    tolerance: f64,
) {
    ledger.record(suite, case_name, field);
    let diff = (actual - expected).abs();
    assert!(
        diff <= tolerance,
        "{suite}/{case_name}/{field}: \
         expected {expected}, got {actual} (diff: {diff}, tolerance: {tolerance})"
    );
}

/// Asserts one binding field against the oracle by exact equality and records
/// its triple. For booleans, integers, and strings.
fn assert_oracle_eq<T: PartialEq + std::fmt::Debug + Copy>(
    ledger: &mut Ledger,
    suite: &str,
    case_name: &str,
    field: &str,
    actual: T,
    expected: T,
) {
    ledger.record(suite, case_name, field);
    assert_eq!(
        actual, expected,
        "{suite}/{case_name}/{field}: expected {expected:?}, got {actual:?}"
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

fn check_wilson_ci(ledger: &mut Ledger) {
    let suite: Suite<WilsonCiCase> =
        serde_json::from_str(include_str!("conformance/wilson_ci.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
            let cl = ConfidenceLevel::new(case.inputs.confidence);
            let est = proportion::estimate(case.inputs.successes, case.inputs.trials, cl);

            assert_oracle_close(
                ledger,
                "wilson_ci",
                &case.name,
                "point",
                est.point_estimate(),
                case.expected.point,
                suite.tolerance,
            );
            assert_oracle_close(
                ledger,
                "wilson_ci",
                &case.name,
                "lower",
                est.lower_bound(),
                case.expected.lower,
                suite.tolerance,
            );
            assert_oracle_close(
                ledger,
                "wilson_ci",
                &case.name,
                "upper",
                est.upper_bound(),
                case.expected.upper,
                suite.tolerance,
            );
        },
    );
}

#[test]
fn conformance_wilson_ci() {
    check_wilson_ci(&mut Ledger::load());
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

fn check_wilson_lower(ledger: &mut Ledger) {
    let suite: Suite<WilsonLowerCase> =
        serde_json::from_str(include_str!("conformance/wilson_lower.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
            let cl = ConfidenceLevel::new(case.inputs.confidence);
            let lb = proportion::lower_bound(case.inputs.successes, case.inputs.trials, cl);

            assert_oracle_close(
                ledger,
                "wilson_lower",
                &case.name,
                "lower_bound",
                lb,
                case.expected.lower_bound,
                suite.tolerance,
            );
        },
    );
}

#[test]
fn conformance_wilson_lower() {
    check_wilson_lower(&mut Ledger::load());
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
    wilson_lower_real: Option<f64>,
    #[serde(default)]
    cutoff_integer: Option<u32>,
    #[serde(default)]
    achieved_size: Option<f64>,
    #[serde(default)]
    implied_confidence: Option<f64>,
    #[serde(default)]
    is_sound: Option<bool>,
}

fn check_threshold_derivation(ledger: &mut Ledger) {
    let suite: Suite<ThresholdCase> =
        serde_json::from_str(include_str!("conformance/threshold_derivation.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
            match case.approach.as_str() {
                "sample_size_first" => {
                    let cl = ConfidenceLevel::new(case.inputs.confidence.unwrap());
                    let dt = threshold::derive_sample_size_first(
                        case.inputs.baseline_successes,
                        case.inputs.baseline_trials,
                        case.inputs.test_samples.unwrap(),
                        cl,
                    );

                    assert_oracle_close(
                        ledger,
                        "threshold_derivation",
                        &case.name,
                        "threshold",
                        dt.value(),
                        case.expected.threshold.unwrap(),
                        suite.tolerance,
                    );
                    assert_oracle_close(
                        ledger,
                        "threshold_derivation",
                        &case.name,
                        "wilson_lower_real",
                        dt.value(),
                        case.expected.wilson_lower_real.unwrap(),
                        suite.tolerance,
                    );
                    // The binding decision artefacts of the regression
                    // procedure, produced by the deriver alongside the
                    // real-valued threshold.
                    let artefacts = dt
                        .decision_cutoff()
                        .expect("a sample-size-first derivation carries its decision cutoff");
                    assert_oracle_eq(
                        ledger,
                        "threshold_derivation",
                        &case.name,
                        "cutoff_integer",
                        artefacts.cutoff(),
                        case.expected.cutoff_integer.unwrap(),
                    );
                    assert_oracle_close(
                        ledger,
                        "threshold_derivation",
                        &case.name,
                        "achieved_size",
                        artefacts.achieved_size(),
                        case.expected.achieved_size.unwrap(),
                        suite.tolerance,
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

                    assert_oracle_close(
                        ledger,
                        "threshold_derivation",
                        &case.name,
                        "implied_confidence",
                        dt.context().confidence().value(),
                        case.expected.implied_confidence.unwrap(),
                        suite.tolerance,
                    );
                    assert_oracle_eq(
                        ledger,
                        "threshold_derivation",
                        &case.name,
                        "is_sound",
                        dt.is_statistically_sound(),
                        case.expected.is_sound.unwrap(),
                    );
                }
                other => panic!("Unknown approach '{other}' in case '{}'", case.name),
            }
        },
    );
}

#[test]
fn conformance_threshold_derivation() {
    check_threshold_derivation(&mut Ledger::load());
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

fn check_power_analysis(ledger: &mut Ledger) {
    let suite: Suite<PowerCase> =
        serde_json::from_str(include_str!("conformance/power_analysis.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
            let cl = ConfidenceLevel::new(case.inputs.confidence);
            let req = sample_size::calculate_for_power(
                case.inputs.baseline_rate,
                case.inputs.min_detectable_effect,
                cl,
                case.inputs.power,
            );

            assert_oracle_eq(
                ledger,
                "power_analysis",
                &case.name,
                "required_samples",
                req.required_samples(),
                case.expected.required_samples,
            );

            let achieved = sample_size::calculate_achieved_power(
                req.required_samples(),
                case.inputs.baseline_rate,
                case.inputs.min_detectable_effect,
                cl,
            );

            assert_oracle_close(
                ledger,
                "power_analysis",
                &case.name,
                "achieved_power",
                achieved,
                case.expected.achieved_power,
                suite.tolerance,
            );
        },
    );
}

#[test]
fn conformance_power_analysis() {
    check_power_analysis(&mut Ledger::load());
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
    criterion: String,
}

fn check_feasibility(ledger: &mut Ledger) {
    let suite: Suite<FeasibilityCase> =
        serde_json::from_str(include_str!("conformance/feasibility.json")).unwrap();
    assert!(
        suite.tolerance.abs() < f64::EPSILON,
        "exact-match fixture; no float slop expected"
    );

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
            let cl = ConfidenceLevel::new(case.inputs.confidence);
            let result = feasibility::feasibility_check(
                case.inputs.sample_size,
                case.inputs.target_proportion,
                cl,
            );

            assert_oracle_eq(
                ledger,
                "feasibility",
                &case.name,
                "feasible",
                result.feasible(),
                case.expected.feasible,
            );
            assert_oracle_eq(
                ledger,
                "feasibility",
                &case.name,
                "minimum_samples",
                result.minimum_samples(),
                case.expected.minimum_samples,
            );
            assert_oracle_eq(
                ledger,
                "feasibility",
                &case.name,
                "criterion",
                result.criterion(),
                case.expected.criterion.as_str(),
            );
        },
    );
}

#[test]
fn conformance_feasibility() {
    check_feasibility(&mut Ledger::load());
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

fn check_verdict(ledger: &mut Ledger) {
    let suite: Suite<VerdictCase> =
        serde_json::from_str(include_str!("conformance/verdict.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
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

            assert_oracle_eq(
                ledger,
                "verdict",
                &case.name,
                "passed",
                verdict.passed(),
                case.expected.passed,
            );
            assert_oracle_close(
                ledger,
                "verdict",
                &case.name,
                "observed_rate",
                verdict.observed_rate(),
                case.expected.observed_rate,
                suite.tolerance,
            );
            assert_oracle_close(
                ledger,
                "verdict",
                &case.name,
                "false_positive_probability",
                verdict.false_positive_probability(),
                case.expected.false_positive_probability,
                suite.tolerance,
            );

            // z-test statistic and p-value are computed via proportion functions,
            // not via the evaluator. The evaluator uses a simple comparison; the
            // reference data validates the full z-test pipeline.
            let z = proportion::z_test_statistic(
                verdict.observed_rate(),
                case.inputs.threshold,
                case.inputs.trials,
            );
            assert_oracle_close(
                ledger,
                "verdict",
                &case.name,
                "test_statistic",
                z,
                case.expected.test_statistic,
                suite.tolerance,
            );

            let p = proportion::one_sided_p_value(z);
            assert_oracle_close(
                ledger,
                "verdict",
                &case.name,
                "p_value",
                p,
                case.expected.p_value,
                suite.tolerance,
            );
        },
    );
}

#[test]
fn conformance_verdict() {
    check_verdict(&mut Ledger::load());
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
    enum OneOrManyLatency {
        One(f64),
        Many(Vec<f64>),
    }

    match OneOrManyLatency::deserialize(deserializer)? {
        OneOrManyLatency::One(v) => Ok(vec![v]),
        OneOrManyLatency::Many(v) => Ok(v),
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
    max: Option<f64>,
}

fn check_latency_percentile(ledger: &mut Ledger) {
    let suite: Suite<LatencyPercentileCase> =
        serde_json::from_str(include_str!("conformance/latency_percentile.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
            if let Some(p) = case.inputs.percentile {
                let result = latency::nearest_rank_percentile(&case.inputs.latencies, p);
                assert_oracle_close(
                    ledger,
                    "latency_percentile",
                    &case.name,
                    "value",
                    result,
                    case.expected.value.unwrap(),
                    suite.tolerance,
                );
            } else {
                let summary = latency::LatencySummary::from_latencies(&case.inputs.latencies);

                assert_oracle_close(
                    ledger,
                    "latency_percentile",
                    &case.name,
                    "mean",
                    summary.mean(),
                    case.expected.mean.unwrap(),
                    suite.tolerance,
                );
                assert_oracle_close(
                    ledger,
                    "latency_percentile",
                    &case.name,
                    "max",
                    summary.max(),
                    case.expected.max.unwrap(),
                    suite.tolerance,
                );
            }
        },
    );
}

#[test]
fn conformance_latency_percentile() {
    check_latency_percentile(&mut Ledger::load());
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
    baseline_latencies: Vec<f64>,
    p: f64,
    confidence: f64,
}

#[derive(Deserialize)]
struct LatencyThresholdExpected {
    rank: u32,
    threshold: f64,
    baseline_percentile: f64,
    n: u32,
}

fn check_latency_threshold(ledger: &mut Ledger) {
    let suite: Suite<LatencyThresholdCase> =
        serde_json::from_str(include_str!("conformance/latency_threshold.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| {
            let result = latency::derive_latency_threshold(
                &case.inputs.baseline_latencies,
                case.inputs.p,
                case.inputs.confidence,
            );

            assert_oracle_eq(
                ledger,
                "latency_threshold",
                &case.name,
                "rank",
                result.rank(),
                case.expected.rank,
            );
            assert_oracle_eq(
                ledger,
                "latency_threshold",
                &case.name,
                "n",
                result.n(),
                case.expected.n,
            );
            assert_oracle_close(
                ledger,
                "latency_threshold",
                &case.name,
                "threshold",
                result.threshold(),
                case.expected.threshold,
                suite.tolerance,
            );
            assert_oracle_close(
                ledger,
                "latency_threshold",
                &case.name,
                "baseline_percentile",
                result.baseline_percentile(),
                case.expected.baseline_percentile,
                suite.tolerance,
            );
        },
    );
}

#[test]
fn conformance_latency_threshold() {
    check_latency_threshold(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// latency_percentile_minimums
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PercentileMinimumsCase {
    name: String,
    approach: String,
    inputs: PercentileMinimumsInputs,
    expected: PercentileMinimumsExpected,
}

#[derive(Deserialize)]
struct PercentileMinimumsInputs {
    percentile: f64,
}

#[derive(Deserialize)]
struct PercentileMinimumsExpected {
    #[serde(default)]
    minimum_contributing_samples: Option<u32>,
}

/// The per-percentile emission gate must equal the published family
/// standard exactly (the companion's non-degeneracy minimums; suite
/// tolerance 0 — every value is an integer).
///
/// The suite's `bound_existence` cases (the judgement-time minimums for a
/// non-saturated order-statistic upper bound) are not asserted here: the
/// threshold deriver does not yet expose its saturation boundary. The suite
/// is therefore outside the committed scope (`tests/conformance/SCOPE.json`),
/// and the emission assertions below are extra assertions beyond it.
fn check_latency_percentile_minimums(ledger: &mut Ledger) {
    let suite: Suite<PercentileMinimumsCase> =
        serde_json::from_str(include_str!("conformance/latency_percentile_minimums.json")).unwrap();

    let emission: Vec<&PercentileMinimumsCase> = suite
        .cases
        .iter()
        .filter(|c| c.approach == "emission_non_degeneracy")
        .collect();
    assert_eq!(
        emission.len(),
        4,
        "expected one emission case per percentile level"
    );

    assert_all_cases(
        &emission,
        |case| case.name.as_str(),
        |case| {
            let expected = case
                .expected
                .minimum_contributing_samples
                .expect("emission case carries minimum_contributing_samples");
            assert_oracle_eq(
                ledger,
                "latency_percentile_minimums",
                &case.name,
                "minimum_contributing_samples",
                latency::min_samples_for(case.inputs.percentile),
                expected,
            );
        },
    );
}

#[test]
fn conformance_latency_percentile_minimums() {
    check_latency_percentile_minimums(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// regression_decision — the composed decision rules, evaluated through the
// production verdict path.
// ---------------------------------------------------------------------------

/// Which target the scripted contract's single criterion carries.
enum ScriptedTarget {
    /// A baseline-derived pass rate (`Criterion::empirical().pass_rate()`).
    Empirical,
    /// A declared pass rate (`Criterion::meeting().pass_rate(rate)`).
    Normative(f64),
}

/// A contract whose single criterion passes on exactly the first `passing`
/// judged samples and fails on every one after — a deterministic script for
/// reproducing an exact success tally through the production sampling loop.
struct ScriptedContract {
    id: String,
    passing: u32,
    target: ScriptedTarget,
}

impl ServiceContract for ScriptedContract {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        &self.id
    }

    fn invoke(
        &self,
        input: &String,
        _cost: &mut feotest::controls::Cost,
    ) -> Result<String, feotest::model::Defect> {
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        // A fresh counter per criteria set: each run judges its samples
        // against its own script, regardless of prior runs on the same
        // contract value.
        let passing = self.passing;
        let judged = AtomicU32::new(0);
        let scripted = move |_: &String| {
            if judged.fetch_add(1, Ordering::SeqCst) < passing {
                Ok(())
            } else {
                Err(ContractViolation::new(
                    "scripted acceptance",
                    "scripted failure",
                ))
            }
        };
        let criterion = match self.target {
            ScriptedTarget::Empirical => Criterion::empirical().pass_rate(),
            ScriptedTarget::Normative(rate) => Criterion::meeting().pass_rate(rate),
        }
        .name("scripted acceptance")
        .satisfies("scripted acceptance", scripted)
        .build();
        Criteria::of([criterion])
    }
}

/// Establishes a baseline with exactly `successes` of `trials` passing
/// samples, written into a fresh temp directory (returned to keep it alive).
fn establish_scripted_baseline(
    service_contract_id: &str,
    successes: u32,
    trials: u32,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let id = service_contract_id.to_owned();

    MeasureExperiment::builder()
        .service_contract_id(service_contract_id)
        .service_contract(move || ScriptedContract {
            id: id.clone(),
            passing: successes,
            target: ScriptedTarget::Empirical,
        })
        .samples(trials)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    dir
}

#[derive(Deserialize)]
struct DecisionCase {
    name: String,
    procedure: String,
    inputs: DecisionInputs,
    expected: DecisionExpected,
}

#[derive(Deserialize)]
struct DecisionInputs {
    #[serde(default)]
    baseline_successes: Option<u32>,
    #[serde(default)]
    baseline_trials: Option<u32>,
    test_samples: u32,
    confidence: f64,
    observed_successes: u32,
    #[serde(default)]
    threshold: Option<f64>,
}

#[derive(Deserialize)]
struct DecisionExpected {
    #[serde(default)]
    threshold_real: Option<f64>,
    #[serde(default)]
    cutoff_integer: Option<u32>,
    #[serde(default)]
    displayed_rate: Option<f64>,
    #[serde(default)]
    achieved_size: Option<f64>,
    #[serde(default)]
    wilson_lower: Option<f64>,
    verdict: String,
}

/// One regression-procedure case through the production verdict path: the
/// baseline is measured, the threshold is derived by the runner itself, a
/// scripted contract delivers the observed tally, and the rendered verdict is
/// asserted against the oracle's.
fn check_regression_case(ledger: &mut Ledger, case: &DecisionCase, tolerance: f64) {
    let id = format!("regression-decision-{}", case.name);
    let baseline_dir = establish_scripted_baseline(
        &id,
        case.inputs.baseline_successes.unwrap(),
        case.inputs.baseline_trials.unwrap(),
    );

    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(ScriptedContract {
        id,
        passing: case.inputs.observed_successes,
        target: ScriptedTarget::Empirical,
    })
    .inputs(&inputs)
    .approach(ThresholdApproach::SampleSizeFirst {
        samples: case.inputs.test_samples,
        confidence: case.inputs.confidence,
    })
    .threshold_origin(ThresholdOrigin::Empirical)
    .spec_resolver(SpecResolver::with_dir(baseline_dir.path()))
    .disable_early_termination()
    .run();

    let record = result.verdict_record();
    let row = &record.functional_assessment().criteria()[0];
    assert_eq!(
        row.verdict(),
        record.verdict(),
        "single-criterion composite must equal its row's verdict"
    );
    let analysis = row
        .statistical_analysis()
        .expect("an inferential criterion row carries a statistical analysis");

    assert_oracle_close(
        ledger,
        "regression_decision",
        &case.name,
        "threshold_real",
        analysis.threshold(),
        case.expected.threshold_real.unwrap(),
        tolerance,
    );
    assert_oracle_eq(
        ledger,
        "regression_decision",
        &case.name,
        "verdict",
        record.verdict().to_string().as_str(),
        case.expected.verdict.as_str(),
    );
    // The binding decision artefact (the integer cutoff the verdict above
    // was decided on) and the §3.4 report obligations, from the production
    // deriver — the same construction the runner just performed.
    let derived = threshold::derive_sample_size_first(
        case.inputs.baseline_successes.unwrap(),
        case.inputs.baseline_trials.unwrap(),
        case.inputs.test_samples,
        ConfidenceLevel::new(case.inputs.confidence),
    );
    let artefacts = derived
        .decision_cutoff()
        .expect("a sample-size-first derivation carries its decision cutoff");
    assert_oracle_eq(
        ledger,
        "regression_decision",
        &case.name,
        "cutoff_integer",
        artefacts.cutoff(),
        case.expected.cutoff_integer.unwrap(),
    );
    assert_oracle_close(
        ledger,
        "regression_decision",
        &case.name,
        "displayed_rate",
        artefacts.displayed_rate(),
        case.expected.displayed_rate.unwrap(),
        tolerance,
    );
    assert_oracle_close(
        ledger,
        "regression_decision",
        &case.name,
        "achieved_size",
        artefacts.achieved_size(),
        case.expected.achieved_size.unwrap(),
        tolerance,
    );
}

/// One compliance-procedure case through the production verdict path: the
/// threshold is given (a declared, normative rate), a scripted contract
/// delivers the observed tally, and the rendered verdict and the test
/// sample's own Wilson lower bound are asserted against the oracle's.
fn check_compliance_case(ledger: &mut Ledger, case: &DecisionCase, tolerance: f64) {
    // The production threshold-first path judges at the framework default
    // confidence; every published compliance case uses exactly that level.
    assert!(
        (case.inputs.confidence - 0.95).abs() < f64::EPSILON,
        "compliance case '{}' declares confidence {}, but the production \
         threshold-first path judges at the framework default 0.95",
        case.name,
        case.inputs.confidence
    );
    let declared = case.inputs.threshold.unwrap();

    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(ScriptedContract {
        id: format!("compliance-decision-{}", case.name),
        passing: case.inputs.observed_successes,
        target: ScriptedTarget::Normative(declared),
    })
    .inputs(&inputs)
    .approach(ThresholdApproach::ThresholdFirst {
        samples: case.inputs.test_samples,
        min_pass_rate: declared,
    })
    .threshold_origin(ThresholdOrigin::Sla)
    .disable_early_termination()
    .run();

    let record = result.verdict_record();
    let row = &record.functional_assessment().criteria()[0];
    let analysis = row
        .statistical_analysis()
        .expect("an inferential criterion row carries a statistical analysis");

    assert_oracle_close(
        ledger,
        "regression_decision",
        &case.name,
        "wilson_lower",
        analysis.wilson_lower(),
        case.expected.wilson_lower.unwrap(),
        tolerance,
    );
    assert_oracle_eq(
        ledger,
        "regression_decision",
        &case.name,
        "verdict",
        record.verdict().to_string().as_str(),
        case.expected.verdict.as_str(),
    );
}

fn check_regression_decision(ledger: &mut Ledger) {
    let suite: Suite<DecisionCase> =
        serde_json::from_str(include_str!("conformance/regression_decision.json")).unwrap();

    assert_all_cases(
        &suite.cases,
        |case| case.name.as_str(),
        |case| match case.procedure.as_str() {
            "REGRESSION" => check_regression_case(ledger, case, suite.tolerance),
            "COMPLIANCE" => check_compliance_case(ledger, case, suite.tolerance),
            other => panic!("Unknown procedure '{other}' in case '{}'", case.name),
        },
    );
}

#[test]
fn conformance_regression_decision() {
    check_regression_decision(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// Vendoring drift and the coverage obligation
// ---------------------------------------------------------------------------

/// The vendored snapshot must be byte-identical to the file the manifest
/// describes — silent vendoring drift is a conformance failure.
#[test]
fn vendored_fixtures_match_manifest_hashes() {
    let ledger = Ledger::load();
    for suite in ledger.in_scope_suites() {
        assert_eq!(
            ledger.vendored_md5(&suite),
            ledger.manifest_md5(&suite),
            "{suite}: vendored fixture differs from the manifest's content hash; \
             re-vendor from the pinned mavai-R release"
        );
    }
}

/// Renders a caught panic payload as text.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map_or_else(
        || {
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "non-string panic payload".to_owned())
        },
        |s| (*s).to_owned(),
    )
}

/// Runs every suite check into one ledger, diffs the asserted triples against
/// the manifest's obligation (family-mandatory ∪ committed scope), prints the
/// one-line standing, writes the machine-readable coverage report, and fails
/// on any suite failure or coverage gap.
#[test]
fn conformance_coverage_meets_manifest() {
    /// One suite's recording check function.
    type SuiteCheck = fn(&mut Ledger);

    let mut ledger = Ledger::load();
    let checks: [(&str, SuiteCheck); 10] = [
        ("wilson_ci", check_wilson_ci),
        ("wilson_lower", check_wilson_lower),
        ("threshold_derivation", check_threshold_derivation),
        ("power_analysis", check_power_analysis),
        ("feasibility", check_feasibility),
        ("verdict", check_verdict),
        ("latency_percentile", check_latency_percentile),
        ("latency_threshold", check_latency_threshold),
        (
            "latency_percentile_minimums",
            check_latency_percentile_minimums,
        ),
        ("regression_decision", check_regression_decision),
    ];

    // Each check runs to completion under catch_unwind so a failing suite
    // cannot mask the coverage diff (or vice versa); triples recorded before
    // a failure still count as attempted.
    let mut suite_failures: Vec<String> = Vec::new();
    for (name, check) in checks {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| check(&mut ledger))) {
            suite_failures.push(format!("{name}: {}", panic_text(payload.as_ref())));
        }
    }

    ledger.write_report();
    println!("{}", ledger.standing());

    let gaps = ledger.gaps();
    let gap_lines: Vec<String> = gaps
        .iter()
        .take(10)
        .map(|(suite, case, field)| format!("{suite}/{case}/{field}"))
        .collect();
    assert!(
        suite_failures.is_empty() && gaps.is_empty(),
        "\nconformance coverage failed.\n\
         suite failures ({}):\n{}\n\
         binding assertions required by the manifest but never made ({}):\n{}{}\n",
        suite_failures.len(),
        suite_failures.join("\n"),
        gaps.len(),
        gap_lines.join("\n"),
        if gaps.len() > 10 { "\n…" } else { "" }
    );
}
