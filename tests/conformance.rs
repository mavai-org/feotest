//! Conformance tests against mavai-R reference data.
//!
//! Validates feotest's decision rules against the canonical reference values
//! published by [mavai-R](https://github.com/mavai-org/mavai-R), the
//! statistical oracle of the Statistical Companion (methodology 1.5.0).
//!
//! Pinned mavai-R version: see `tests/conformance/VERSION`.
//!
//! # Coverage accounting
//!
//! The oracle publishes `manifest.json` alongside its fixture suites:
//! per-suite case rosters, a binding-vs-informational classification of every
//! expected field, per-suite content hashes, a family-mandatory suite tier,
//! the methodology version and the versioned decision rules. The obligation
//! on a consumer is the set of `(suite, case, binding-field)` triples across
//! the family-mandatory tier plus this repository's committed
//! `tests/conformance/SCOPE.json` — and the obligation is *self-verified*:
//! every conformance assertion records the triple it asserts into a
//! [`Ledger`], and the umbrella test [`conformance_coverage_meets_manifest`]
//! diffs the recorded set against the manifest. A binding field that is
//! loaded but never asserted is a gap, not a pass. Informational fields are
//! asserted too, where this crate computes them, but owe nothing.
//!
//! # The production path
//!
//! The pass-rate decision suites (`regression_decision`,
//! `compliance_decision`, and the criterion cases of `verdict`) are
//! evaluated through the **production verdict path**
//! (`ProbabilisticTest::for_contract(..).run()`), not a test-side
//! reimplementation: a measure experiment establishes the baseline from a
//! scripted contract, a scripted contract delivers the case's observed
//! successes, and the verdict — or the refusal, with its ordered
//! configuration errors — asserted is the one the runner rendered. The
//! latency suites supply latencies as data, which a run cannot reproduce;
//! they are evaluated through the statistics functions the latency
//! dimension delegates to, and the test-size design rule through the same
//! `check_test_size` the runner's preflight applies.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use md5::Digest;
use serde::Deserialize;
use serde_json::{Value, json};

use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::MeasureExperiment;
use feotest::model::{ContractViolation, TestIntent, ThresholdOrigin};
use feotest::ptest::ProbabilisticTest;
use feotest::service_contract::ServiceContract;
use feotest::spec::SpecResolver;
use feotest::statistics::compliance::{self, AlternativeKind, DEFAULT_SIZING_HORIZON};
use feotest::statistics::decision::{Trigger, Verdict, compose_overall_verdict};
use feotest::statistics::feasibility::feasibility_check;
use feotest::statistics::latency::{
    self, ConstraintThreshold, LatencyConstraint, LatencyMode, LatencyOutcome, ThresholdSource,
};
use feotest::statistics::proportion;
use feotest::statistics::regression;
use feotest::statistics::risk_driven_sizing::{self, SizingRefusal};
use feotest::statistics::rules::{
    ConfigurationError, DecisionRule, METHODOLOGY_VERSION, check_test_size,
};
use feotest::statistics::types::ConfidenceLevel;
use feotest::verdict::{RuleEvidence, VerdictRecord};

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
    #[serde(rename = "decisionRules", default)]
    decision_rules: Vec<RuleEntry>,
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
    #[serde(rename = "methodologyVersion")]
    methodology_version: String,
    #[serde(rename = "decisionRules")]
    decision_rules: Vec<RuleEntry>,
    #[serde(rename = "familyMandatory")]
    family_mandatory: Vec<String>,
    suites: BTreeMap<String, SuiteEntry>,
}

/// A versioned decision rule as the manifest and each suite name it.
#[derive(Deserialize)]
struct RuleEntry {
    id: String,
    version: u32,
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

// ---------------------------------------------------------------------------
// Field-level recording assertions over the fixture JSON
// ---------------------------------------------------------------------------

/// One suite's cases and its published tolerance.
struct SuiteData {
    tolerance: f64,
    cases: Vec<Value>,
}

/// Loads a vendored suite file.
fn load_suite(suite: &str) -> SuiteData {
    let raw = std::fs::read_to_string(fixtures_dir().join(format!("{suite}.json"))).unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    SuiteData {
        tolerance: value["tolerance"].as_f64().unwrap_or(0.0),
        cases: value["cases"].as_array().unwrap().clone(),
    }
}

/// A case's name.
fn case_name(case: &Value) -> &str {
    case["name"].as_str().unwrap()
}

/// A required integer input.
fn input_u32(case: &Value, key: &str) -> u32 {
    let value = case["inputs"][key]
        .as_u64()
        .unwrap_or_else(|| panic!("{}: input {key} is not an integer", case_name(case)));
    u32::try_from(value).unwrap()
}

/// A required numeric input.
fn input_f64(case: &Value, key: &str) -> f64 {
    case["inputs"][key]
        .as_f64()
        .unwrap_or_else(|| panic!("{}: input {key} is not a number", case_name(case)))
}

/// A required list of numbers; the oracle's serialiser unboxes a
/// one-element list to a scalar.
fn input_list(case: &Value, key: &str) -> Vec<f64> {
    numbers(&case["inputs"][key])
        .unwrap_or_else(|| panic!("{}: input {key} is not a list", case_name(case)))
}

/// A list of numbers, or one number as a one-element list.
fn numbers(value: &Value) -> Option<Vec<f64>> {
    match value {
        Value::Array(values) => values.iter().map(Value::as_f64).collect(),
        Value::Number(number) => number.as_f64().map(|v| vec![v]),
        _ => None,
    }
}

/// The case's declared intent (VERIFICATION when unstated).
fn input_intent(case: &Value) -> TestIntent {
    match case["inputs"]["intent"].as_str() {
        Some("SMOKE") => TestIntent::Smoke,
        Some("VERIFICATION") | None => TestIntent::Verification,
        Some(other) => panic!("{}: unknown intent {other}", case_name(case)),
    }
}

/// The confidence whose one-sided level is `alpha`.
fn confidence_of(alpha: f64) -> f64 {
    1.0 - alpha
}

/// The expected value of a field, when the case states it.
fn expected<'a>(case: &'a Value, field: &str) -> Option<&'a Value> {
    case["expected"].as_object().and_then(|e| e.get(field))
}

/// Asserts a field by JSON equality (integers, booleans, strings, lists,
/// `null`), recording the triple when the case states the field.
fn check_json(ledger: &mut Ledger, suite: &str, case: &Value, field: &str, actual: &Value) {
    let Some(expected) = expected(case, field) else {
        return;
    };
    ledger.record(suite, case_name(case), field);
    assert_eq!(
        actual,
        expected,
        "{suite}/{}/{field}: expected {expected}, got {actual}",
        case_name(case)
    );
}

/// Asserts a numeric field within `tolerance` (`None` matches `null`),
/// recording the triple when the case states the field.
fn check_close(
    ledger: &mut Ledger,
    suite: &str,
    case: &Value,
    field: &str,
    actual: Option<f64>,
    tolerance: f64,
) {
    let Some(expected) = expected(case, field) else {
        return;
    };
    ledger.record(suite, case_name(case), field);
    match (actual, expected.as_f64()) {
        (None, None) => {}
        (Some(actual), Some(expected)) => {
            let diff = (actual - expected).abs();
            assert!(
                diff <= tolerance,
                "{suite}/{}/{field}: expected {expected}, got {actual} \
                 (diff {diff}, tolerance {tolerance})",
                case_name(case)
            );
        }
        (actual, _) => panic!(
            "{suite}/{}/{field}: expected {expected}, got {actual:?}",
            case_name(case)
        ),
    }
}

/// A verdict's wire name.
fn verdict_name(verdict: Verdict) -> Value {
    json!(verdict.to_string())
}

/// An optional integer as JSON (`null` when absent).
fn opt_u32(value: Option<u32>) -> Value {
    value.map_or(Value::Null, |v| json!(v))
}

/// The configuration errors as the fixtures list them.
fn error_list(errors: &[ConfigurationError]) -> Value {
    json!(errors.iter().map(|e| e.code()).collect::<Vec<_>>())
}

/// Runs a suite's per-case check under the fail-late harness.
fn for_each_case(suite: &str, mut check: impl FnMut(&Value, f64)) {
    let data = load_suite(suite);
    let tolerance = data.tolerance;
    assert_all_cases(
        &data.cases,
        |case| case_name(case),
        |case| {
            check(case, tolerance);
        },
    );
}

// ---------------------------------------------------------------------------
// The methodology and its rules
// ---------------------------------------------------------------------------

/// The versioned rules this crate implements, as `(id, version)`.
fn implemented_rules() -> BTreeSet<(String, u32)> {
    DecisionRule::ALL
        .iter()
        .map(|rule| (rule.id().to_owned(), rule.version()))
        .collect()
}

#[test]
fn manifest_names_the_methodology_this_crate_implements() {
    let ledger = Ledger::load();
    assert_eq!(ledger.manifest.methodology_version, METHODOLOGY_VERSION);
    let published: BTreeSet<(String, u32)> = ledger
        .manifest
        .decision_rules
        .iter()
        .map(|rule| (rule.id.clone(), rule.version))
        .collect();
    assert_eq!(published, implemented_rules());
}

#[test]
fn every_suite_in_scope_declares_this_methodology_and_known_rules() {
    let ledger = Ledger::load();
    let implemented = implemented_rules();
    for suite in ledger.in_scope_suites() {
        let entry = &ledger.manifest.suites[&suite];
        let raw = std::fs::read_to_string(fixtures_dir().join(&entry.file)).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            value["methodologyVersion"].as_str(),
            Some(METHODOLOGY_VERSION),
            "{suite}: methodology version"
        );
        for rule in &entry.decision_rules {
            assert!(
                implemented.contains(&(rule.id.clone(), rule.version)),
                "{suite}: rule {} v{} is not implemented",
                rule.id,
                rule.version
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The production path: scripted contracts and baselines
// ---------------------------------------------------------------------------

/// How a scripted criterion is judged.
#[derive(Clone, Copy)]
enum ScriptedBar {
    /// Against its baseline (`Criterion::empirical().pass_rate()`).
    Regression,
    /// Against a declared requirement (`Criterion::meeting().pass_rate(..)`).
    Compliance(f64),
}

/// One scripted criterion: its name, its bar and its own confidence.
#[derive(Clone)]
struct ScriptedCriterion {
    name: String,
    bar: ScriptedBar,
    confidence: f64,
}

/// A contract whose criteria each pass on exactly the first `passing`
/// judged samples and fail on every one after — a deterministic script for
/// reproducing an exact success tally through the production sampling loop.
struct ScriptedContract {
    id: String,
    passing: u32,
    criteria: Vec<ScriptedCriterion>,
}

impl ScriptedContract {
    /// One scripted criterion as a built criterion, with its own counter.
    fn criterion(&self, scripted: &ScriptedCriterion) -> Criterion<String> {
        let passing = self.passing;
        let judged = AtomicU32::new(0);
        let check = move |_: &String| {
            if judged.fetch_add(1, Ordering::SeqCst) < passing {
                Ok(())
            } else {
                Err(ContractViolation::new(
                    "scripted acceptance",
                    "scripted failure",
                ))
            }
        };
        match scripted.bar {
            ScriptedBar::Regression => Criterion::empirical().pass_rate(),
            ScriptedBar::Compliance(rate) => Criterion::meeting().pass_rate(rate),
        }
        .name(scripted.name.clone())
        .confidence(scripted.confidence)
        .satisfies("scripted acceptance", check)
        .build()
    }
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
        match self.criteria.as_slice() {
            [only] => Criteria::of([self.criterion(only)]),
            [first, second] => Criteria::of([self.criterion(first), self.criterion(second)]),
            other => panic!("unsupported criterion count: {}", other.len()),
        }
    }
}

/// A distinct contract identity per run, so baselines never collide.
fn unique_id(case: &Value) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "conformance-{}-{}",
        case_name(case),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

/// Establishes a baseline in which the named criterion passed exactly
/// `successes` of `trials`, in a fresh directory (returned to keep it
/// alive).
fn establish_baseline(id: &str, criterion: &str, successes: u32, trials: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let (id_owned, name) = (id.to_owned(), criterion.to_owned());
    MeasureExperiment::builder()
        .service_contract_id(id)
        .service_contract(move || ScriptedContract {
            id: id_owned.clone(),
            passing: successes,
            criteria: vec![ScriptedCriterion {
                name: name.clone(),
                bar: ScriptedBar::Regression,
                confidence: 0.95,
            }],
        })
        .samples(trials)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();
    dir
}

/// Runs a scripted contract through the production path: every planned
/// sample (no early termination), against the baseline when one is given.
fn run_scripted(
    contract: ScriptedContract,
    baseline: Option<&tempfile::TempDir>,
    samples: u32,
    intent: TestIntent,
) -> VerdictRecord {
    let inputs = vec!["input".to_string()];
    let mut test = ProbabilisticTest::for_contract(contract)
        .inputs(&inputs)
        .samples(samples)
        .threshold_origin(ThresholdOrigin::Sla)
        .disable_early_termination();
    if let Some(dir) = baseline {
        test = test.spec_resolver(SpecResolver::with_dir(dir.path()));
    }
    if intent == TestIntent::Smoke {
        test = test.smoke();
    }
    test.run().verdict_record().clone()
}

/// The rule evidence of a record's named criterion row.
fn evidence_of<'a>(record: &'a VerdictRecord, criterion: &str) -> &'a RuleEvidence {
    record
        .functional_assessment()
        .criteria()
        .iter()
        .find(|row| row.name() == criterion)
        .and_then(|row| row.statistical_analysis())
        .unwrap_or_else(|| panic!("criterion {criterion} was not decided by a rule"))
        .evidence()
}

/// The record's verdict as JSON (`null` for a refusal).
fn record_verdict(record: &VerdictRecord) -> Value {
    record.verdict().map_or(Value::Null, verdict_name)
}

// ---------------------------------------------------------------------------
// wilson_ci, wilson_lower — descriptive primitives
// ---------------------------------------------------------------------------

fn check_wilson_ci(ledger: &mut Ledger) {
    let suite = "wilson_ci";
    for_each_case(suite, |case, tolerance| {
        let estimate = proportion::estimate(
            input_u32(case, "successes"),
            input_u32(case, "trials"),
            ConfidenceLevel::new(input_f64(case, "confidence")),
        );
        check_close(
            ledger,
            suite,
            case,
            "lower",
            Some(estimate.lower_bound()),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "point",
            Some(estimate.point_estimate()),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "upper",
            Some(estimate.upper_bound()),
            tolerance,
        );
    });
}

#[test]
fn conformance_wilson_ci() {
    check_wilson_ci(&mut Ledger::load());
}

fn check_wilson_lower(ledger: &mut Ledger) {
    let suite = "wilson_lower";
    for_each_case(suite, |case, tolerance| {
        let lower = proportion::lower_bound(
            input_u32(case, "successes"),
            input_u32(case, "trials"),
            ConfidenceLevel::new(input_f64(case, "confidence")),
        );
        check_close(ledger, suite, case, "lower_bound", Some(lower), tolerance);
    });
}

#[test]
fn conformance_wilson_lower() {
    check_wilson_lower(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// regression_decision — regression/fisher through the production path
// ---------------------------------------------------------------------------

fn check_regression_decision(ledger: &mut Ledger) {
    let suite = "regression_decision";
    for_each_case(suite, |case, tolerance| {
        let id = unique_id(case);
        let criterion = "scripted acceptance";
        let baseline = establish_baseline(
            &id,
            criterion,
            input_u32(case, "baseline_successes"),
            input_u32(case, "baseline_trials"),
        );
        let record = run_scripted(
            ScriptedContract {
                id,
                passing: input_u32(case, "observed_successes"),
                criteria: vec![ScriptedCriterion {
                    name: criterion.to_owned(),
                    bar: ScriptedBar::Regression,
                    confidence: confidence_of(input_f64(case, "alpha")),
                }],
            },
            Some(&baseline),
            input_u32(case, "test_samples"),
            TestIntent::Verification,
        );
        let errors = error_list(record.configuration_errors());
        check_json(ledger, suite, case, "configuration_error", &errors);
        check_json(ledger, suite, case, "verdict", &record_verdict(&record));
        if record.is_refused() {
            check_json(ledger, suite, case, "cutoff_integer", &Value::Null);
            return;
        }
        let RuleEvidence::Regression(evidence) = evidence_of(&record, criterion) else {
            panic!("a baseline-derived criterion is decided by regression/fisher");
        };
        check_json(
            ledger,
            suite,
            case,
            "cutoff_integer",
            &json!(evidence.cutoff),
        );
        let n_t = f64::from(input_u32(case, "test_samples"));
        let threshold = f64::from(evidence.cutoff) / n_t;
        check_close(
            ledger,
            suite,
            case,
            "threshold_real",
            Some(threshold),
            tolerance,
        );
        let displayed = (threshold * 1e6).round() / 1e6;
        check_close(
            ledger,
            suite,
            case,
            "displayed_rate",
            Some(displayed),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "size_at_assumed_common_rate",
            evidence.size_at_assumed_common_rate,
            tolerance,
        );
    });
}

#[test]
fn conformance_regression_decision() {
    check_regression_decision(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// compliance_decision — compliance/exact-binomial through the production path
// ---------------------------------------------------------------------------

fn check_compliance_decision(ledger: &mut Ledger) {
    let suite = "compliance_decision";
    for_each_case(suite, |case, tolerance| {
        let criterion = "scripted acceptance";
        let record = run_scripted(
            ScriptedContract {
                id: unique_id(case),
                passing: input_u32(case, "observed_successes"),
                criteria: vec![ScriptedCriterion {
                    name: criterion.to_owned(),
                    bar: ScriptedBar::Compliance(input_f64(case, "threshold")),
                    confidence: confidence_of(input_f64(case, "alpha")),
                }],
            },
            None,
            input_u32(case, "test_samples"),
            input_intent(case),
        );
        let errors = error_list(record.configuration_errors());
        check_json(ledger, suite, case, "configuration_error", &errors);
        check_json(ledger, suite, case, "verdict", &record_verdict(&record));
        if record.is_refused() {
            for field in ["k_min", "pass_possible"] {
                check_json(ledger, suite, case, field, &Value::Null);
            }
            return;
        }
        let RuleEvidence::Compliance(evidence) = evidence_of(&record, criterion) else {
            panic!("a declared requirement is decided by compliance/exact-binomial");
        };
        check_json(
            ledger,
            suite,
            case,
            "k_min",
            &opt_u32(evidence.minimum_passing_count),
        );
        check_json(
            ledger,
            suite,
            case,
            "pass_possible",
            &json!(evidence.minimum_passing_count.is_some()),
        );
        // Zero when no count can pass: nothing can falsely comply.
        check_close(
            ledger,
            suite,
            case,
            "false_compliance",
            Some(evidence.false_compliance),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "clopper_pearson_lower",
            Some(evidence.clopper_pearson_lower),
            tolerance,
        );
    });
}

#[test]
fn conformance_compliance_decision() {
    check_compliance_decision(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// feasibility
// ---------------------------------------------------------------------------

fn check_feasibility(ledger: &mut Ledger) {
    let suite = "feasibility";
    for_each_case(suite, |case, _| {
        let result = feasibility_check(
            input_u32(case, "sample_size"),
            input_f64(case, "target_proportion"),
            input_f64(case, "alpha"),
        );
        check_json(ledger, suite, case, "feasible", &json!(result.feasible()));
        check_json(
            ledger,
            suite,
            case,
            "minimum_samples",
            &json!(result.minimum_samples()),
        );
        check_json(ledger, suite, case, "criterion", &json!(result.criterion()));
    });
}

#[test]
fn conformance_feasibility() {
    check_feasibility(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// threshold_derivation — the cutoff and the threshold-first inversion
// ---------------------------------------------------------------------------

fn check_threshold_derivation(ledger: &mut Ledger) {
    let suite = "threshold_derivation";
    for_each_case(suite, |case, tolerance| {
        let k_b = input_u32(case, "baseline_successes");
        let n_b = input_u32(case, "baseline_trials");
        let n_t = input_u32(case, "test_samples");
        if case["approach"] == "threshold_first" {
            let implied =
                regression::implied_alpha(k_b, n_b, n_t, input_u32(case, "declared_cutoff"));
            check_close(
                ledger,
                suite,
                case,
                "implied_alpha",
                implied.alpha(),
                tolerance,
            );
            let sound = implied.is_sound().map_or(Value::Null, |s| json!(s));
            check_json(ledger, suite, case, "is_sound", &sound);
            return;
        }
        let errors: Vec<ConfigurationError> = check_test_size(n_b, n_t).into_iter().collect();
        check_json(
            ledger,
            suite,
            case,
            "configuration_error",
            &error_list(&errors),
        );
        if !errors.is_empty() {
            check_json(ledger, suite, case, "cutoff_integer", &Value::Null);
            return;
        }
        let derivation =
            regression::derive_regression_cutoff(k_b, n_b, n_t, input_f64(case, "alpha"));
        check_json(
            ledger,
            suite,
            case,
            "cutoff_integer",
            &json!(derivation.cutoff()),
        );
        check_close(
            ledger,
            suite,
            case,
            "threshold_real",
            Some(derivation.threshold_real()),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "displayed_rate",
            Some(derivation.displayed_rate()),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "size_at_assumed_common_rate",
            derivation.size_at_assumed_common_rate(),
            tolerance,
        );
    });
}

#[test]
fn conformance_threshold_derivation() {
    check_threshold_derivation(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// power_analysis — compliance sizing and the regression powers
// ---------------------------------------------------------------------------

fn check_power_analysis(ledger: &mut Ledger) {
    let suite = "power_analysis";
    for_each_case(suite, |case, tolerance| {
        let alpha = input_f64(case, "alpha");
        match case["approach"].as_str().unwrap() {
            "compliance_sizing" => check_compliance_sizing(ledger, case, tolerance),
            "regression_power" => {
                let rate = input_f64(case, "baseline_rate");
                let power = regression::design_power(
                    input_u32(case, "baseline_trials"),
                    input_u32(case, "test_samples"),
                    alpha,
                    rate,
                    rate - input_f64(case, "min_detectable_effect"),
                );
                check_close(ledger, suite, case, "design_power", Some(power), tolerance);
            }
            "regression_resolved_power" => {
                let (k_b, n_b, n_t) = (
                    input_u32(case, "baseline_successes"),
                    input_u32(case, "baseline_trials"),
                    input_u32(case, "test_samples"),
                );
                let cutoff = regression::fisher_cutoff(k_b, n_b, n_t, alpha);
                check_json(ledger, suite, case, "cutoff_integer", &json!(cutoff));
                let power = regression::resolved_power(
                    k_b,
                    n_b,
                    n_t,
                    alpha,
                    input_f64(case, "design_alternative_rate"),
                );
                check_close(
                    ledger,
                    suite,
                    case,
                    "resolved_test_power",
                    Some(power),
                    tolerance,
                );
            }
            "regression_mdd" => {
                let drop = regression::minimum_detectable_degradation(
                    input_u32(case, "baseline_trials"),
                    input_u32(case, "test_samples"),
                    alpha,
                    input_f64(case, "baseline_rate"),
                    input_f64(case, "power"),
                );
                check_close(
                    ledger,
                    suite,
                    case,
                    "minimum_detectable_degradation",
                    drop,
                    tolerance,
                );
            }
            other => panic!("unknown power_analysis approach {other}"),
        }
    });
}

/// One `compliance_sizing` case of `power_analysis`.
fn check_compliance_sizing(ledger: &mut Ledger, case: &Value, tolerance: f64) {
    let suite = "power_analysis";
    let sizing = compliance::size_compliance(
        input_f64(case, "threshold"),
        input_f64(case, "min_detectable_effect"),
        input_f64(case, "alpha"),
        input_f64(case, "power"),
        case["inputs"]["alternative_rate"].as_f64(),
        DEFAULT_SIZING_HORIZON,
    );
    check_json(
        ledger,
        suite,
        case,
        "required_samples",
        &opt_u32(sizing.required_samples()),
    );
    check_close(
        ledger,
        suite,
        case,
        "achieved_power",
        sizing.achieved_power(),
        tolerance,
    );
    let alternative = sizing.alternative();
    check_close(
        ledger,
        suite,
        case,
        "alternative_rate",
        Some(alternative.rate()),
        tolerance,
    );
    let kind = match alternative.kind() {
        AlternativeKind::Margin | AlternativeKind::Midway | AlternativeKind::Declared => {
            alternative.kind().name()
        }
    };
    check_json(ledger, suite, case, "alternative_kind", &json!(kind));
    check_json(
        ledger,
        suite,
        case,
        "first_crossing",
        &opt_u32(sizing.first_crossing()),
    );
}

#[test]
fn conformance_power_analysis() {
    check_power_analysis(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// risk_driven_sizing — design and resolved sizing of the regression rule
// ---------------------------------------------------------------------------

/// Records a refused sizing case: the gate, the category and every sized
/// figure `null`.
fn check_sizing_refusal(ledger: &mut Ledger, case: &Value, refusal: SizingRefusal) {
    let suite = "risk_driven_sizing";
    check_json(ledger, suite, case, "sizing_gate", &json!("REFUSE"));
    check_json(
        ledger,
        suite,
        case,
        "refusal_category",
        &json!(refusal.category()),
    );
    for field in [
        "required_n",
        "achieved_power",
        "power",
        "detectable_rate",
        "resolved_power",
        "first_crossing",
        "cutoff_integer",
    ] {
        check_json(ledger, suite, case, field, &Value::Null);
    }
}

/// The domain refusal a sizing case meets, if any.
fn sizing_domain(case: &Value, baseline_rate: f64) -> Option<SizingRefusal> {
    risk_driven_sizing::check_sizing_domain(
        baseline_rate,
        input_u32(case, "baseline_trials"),
        case["inputs"]["design_alternative_rate"].as_f64(),
        case["inputs"]["test_samples"]
            .as_u64()
            .map(|n| u32::try_from(n).unwrap()),
    )
}

/// The baseline rate a sizing case states, directly or as a count.
fn sizing_baseline_rate(case: &Value) -> f64 {
    case["inputs"]["baseline_rate"].as_f64().unwrap_or_else(|| {
        f64::from(input_u32(case, "baseline_successes"))
            / f64::from(input_u32(case, "baseline_trials"))
    })
}

fn check_risk_driven_sizing(ledger: &mut Ledger) {
    let suite = "risk_driven_sizing";
    for_each_case(suite, |case, tolerance| {
        let rate = sizing_baseline_rate(case);
        if let Some(refusal) = sizing_domain(case, rate) {
            check_sizing_refusal(ledger, case, refusal);
            return;
        }
        let n_b = input_u32(case, "baseline_trials");
        let alpha = input_f64(case, "alpha");
        match case["approach"].as_str().unwrap() {
            "required_n" => {
                let sizing = risk_driven_sizing::design_required_samples(
                    rate,
                    n_b,
                    input_f64(case, "design_alternative_rate"),
                    alpha,
                    input_f64(case, "target_power"),
                );
                let Some(sizing) = sizing else {
                    check_sizing_refusal(ledger, case, SizingRefusal::BaselineTooSmall);
                    return;
                };
                check_json(ledger, suite, case, "sizing_gate", &json!("ADMIT"));
                check_json(
                    ledger,
                    suite,
                    case,
                    "required_n",
                    &json!(sizing.required_samples()),
                );
                check_close(
                    ledger,
                    suite,
                    case,
                    "achieved_power",
                    Some(sizing.power()),
                    tolerance,
                );
            }
            "power_at" => {
                let power = risk_driven_sizing::design_power_at(
                    input_u32(case, "test_samples"),
                    rate,
                    n_b,
                    input_f64(case, "design_alternative_rate"),
                    alpha,
                );
                check_json(ledger, suite, case, "sizing_gate", &json!("ADMIT"));
                check_close(ledger, suite, case, "power", Some(power), tolerance);
            }
            "detectable_rate" => {
                let detectable = risk_driven_sizing::design_detectable_rate(
                    input_u32(case, "test_samples"),
                    rate,
                    n_b,
                    alpha,
                    input_f64(case, "target_power"),
                );
                check_json(ledger, suite, case, "sizing_gate", &json!("ADMIT"));
                check_close(
                    ledger,
                    suite,
                    case,
                    "detectable_rate",
                    detectable,
                    tolerance,
                );
            }
            "resolved_required_n" => check_resolved_sizing(ledger, case, tolerance),
            "resolved_power_at" => {
                let (k_b, n_t) = (
                    input_u32(case, "baseline_successes"),
                    input_u32(case, "test_samples"),
                );
                let alternative = input_f64(case, "design_alternative_rate");
                check_json(ledger, suite, case, "sizing_gate", &json!("ADMIT"));
                let cutoff = regression::fisher_cutoff(k_b, n_b, n_t, alpha);
                check_json(ledger, suite, case, "cutoff_integer", &json!(cutoff));
                let power = regression::resolved_power(k_b, n_b, n_t, alpha, alternative);
                check_close(
                    ledger,
                    suite,
                    case,
                    "resolved_power",
                    Some(power),
                    tolerance,
                );
            }
            other => panic!("unknown risk_driven_sizing approach {other}"),
        }
    });
}

/// One `resolved_required_n` case of `risk_driven_sizing`.
fn check_resolved_sizing(ledger: &mut Ledger, case: &Value, tolerance: f64) {
    let suite = "risk_driven_sizing";
    let sizing = risk_driven_sizing::resolved_sizing(
        input_u32(case, "baseline_successes"),
        input_u32(case, "baseline_trials"),
        input_f64(case, "design_alternative_rate"),
        input_f64(case, "alpha"),
        input_f64(case, "target_power"),
    );
    let Some(sizing) = sizing else {
        check_sizing_refusal(ledger, case, SizingRefusal::BaselineTooSmall);
        return;
    };
    check_json(ledger, suite, case, "sizing_gate", &json!("ADMIT"));
    check_json(
        ledger,
        suite,
        case,
        "required_n",
        &json!(sizing.required_samples()),
    );
    check_close(
        ledger,
        suite,
        case,
        "resolved_power",
        Some(sizing.power()),
        tolerance,
    );
    check_json(
        ledger,
        suite,
        case,
        "first_crossing",
        &json!(sizing.first_crossing()),
    );
}

#[test]
fn conformance_risk_driven_sizing() {
    check_risk_driven_sizing(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// latency_percentile
// ---------------------------------------------------------------------------

fn check_latency_percentile(ledger: &mut Ledger) {
    let suite = "latency_percentile";
    for_each_case(suite, |case, tolerance| {
        let latencies = input_list(case, "latencies");
        if let Some(percentile) = case["inputs"]["percentile"].as_f64() {
            let value = latency::nearest_rank_percentile(&latencies, percentile);
            check_close(ledger, suite, case, "value", Some(value), tolerance);
        }
        let summary = latency::LatencySummary::from_latencies(&latencies);
        check_close(ledger, suite, case, "mean", Some(summary.mean()), tolerance);
        check_close(ledger, suite, case, "max", Some(summary.max()), tolerance);
    });
}

#[test]
fn conformance_latency_percentile() {
    check_latency_percentile(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// latency_percentile_minimums — emission minimums and the latency gates
// ---------------------------------------------------------------------------

/// The fixture's intent, source and mode inputs.
fn gate_inputs(case: &Value) -> (TestIntent, LatencyMode, ThresholdSource) {
    let mode = if case["inputs"]["enforced"].as_bool().unwrap() {
        LatencyMode::Enforced
    } else {
        LatencyMode::Advisory
    };
    let source = match case["inputs"]["threshold_source"].as_str().unwrap() {
        "explicit" => ThresholdSource::Explicit,
        "baseline-derived" => ThresholdSource::BaselineDerived,
        other => panic!("unknown threshold source {other}"),
    };
    (input_intent(case), mode, source)
}

fn check_latency_percentile_minimums(ledger: &mut Ledger) {
    let suite = "latency_percentile_minimums";
    for_each_case(suite, |case, tolerance| {
        assert!(tolerance.abs() < f64::EPSILON, "exact equality throughout");
        let percentile = input_f64(case, "percentile");
        match case["approach"].as_str().unwrap() {
            // The emission gate the artefact writers apply.
            "emission_non_degeneracy" => check_json(
                ledger,
                suite,
                case,
                "minimum_contributing_samples",
                &json!(latency::min_samples_for(percentile)),
            ),
            "nondegeneracy_planning" => {
                let planning = latency::plan_nondegeneracy(
                    percentile,
                    input_u32(case, "planned_samples"),
                    input_f64(case, "baseline_success_rate"),
                );
                let expected = json!(planning.expected_test_samples());
                check_json(ledger, suite, case, "expected_test_samples", &expected);
                let minimum = json!(planning.minimum_contributing_samples());
                check_json(
                    ledger,
                    suite,
                    case,
                    "minimum_contributing_samples",
                    &minimum,
                );
                check_json(ledger, suite, case, "warning", &json!(planning.warning()));
                let needed = json!(planning.planned_samples_needed());
                check_json(ledger, suite, case, "planned_samples_needed", &needed);
            }
            "nondegeneracy_decision" => {
                let (intent, mode, source) = gate_inputs(case);
                let decision = latency::decide_nondegeneracy(
                    percentile,
                    input_u32(case, "test_samples"),
                    intent,
                    mode,
                    source,
                );
                check_json(ledger, suite, case, "applies", &json!(decision.applies()));
                check_json(
                    ledger,
                    suite,
                    case,
                    "degenerate",
                    &json!(decision.degenerate()),
                );
                check_json(
                    ledger,
                    suite,
                    case,
                    "outcome",
                    &json!(decision.outcome().name()),
                );
            }
            "precedence_existence" => {
                let rank = latency::precedence_rank(
                    input_u32(case, "baseline_trials"),
                    input_u32(case, "test_samples"),
                    percentile,
                    input_f64(case, "alpha"),
                );
                check_json(ledger, suite, case, "saturated", &json!(rank.is_none()));
                check_json(ledger, suite, case, "rank", &opt_u32(rank));
            }
            "precedence_planning" => {
                let planning = latency::plan_precedence(
                    input_u32(case, "baseline_trials"),
                    input_u32(case, "planned_samples"),
                    input_f64(case, "baseline_success_rate"),
                    percentile,
                    input_f64(case, "alpha"),
                );
                let expected = json!(planning.expected_test_samples());
                check_json(ledger, suite, case, "expected_test_samples", &expected);
                check_json(ledger, suite, case, "warning", &json!(planning.warning()));
                let rank = opt_u32(planning.planning_rank());
                check_json(ledger, suite, case, "planning_rank", &rank);
                let minimum = opt_u32(planning.minimum_baseline_trials());
                check_json(ledger, suite, case, "minimum_baseline_trials", &minimum);
            }
            other => panic!("unknown latency_percentile_minimums approach {other}"),
        }
    });
}

#[test]
fn conformance_latency_percentile_minimums() {
    check_latency_percentile_minimums(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// latency_threshold — latency/precedence
// ---------------------------------------------------------------------------

fn check_latency_threshold(ledger: &mut Ledger) {
    let suite = "latency_threshold";
    for_each_case(suite, |case, tolerance| {
        // The design rule the preflight applies to an enforced
        // baseline-derived constraint: the planned test against the
        // baseline run it consumes.
        let errors: Vec<ConfigurationError> = check_test_size(
            input_u32(case, "baseline_samples"),
            input_u32(case, "planned_samples"),
        )
        .into_iter()
        .collect();
        check_json(
            ledger,
            suite,
            case,
            "configuration_error",
            &error_list(&errors),
        );
        if !errors.is_empty() {
            for field in ["rank", "threshold", "saturated"] {
                check_json(ledger, suite, case, field, &Value::Null);
            }
            return;
        }
        let derived = latency::derive_precedence_threshold(
            &input_list(case, "baseline_latencies"),
            input_u32(case, "test_samples"),
            input_f64(case, "p"),
            input_f64(case, "alpha"),
        );
        check_json(ledger, suite, case, "rank", &opt_u32(derived.rank()));
        check_close(
            ledger,
            suite,
            case,
            "threshold",
            derived.threshold(),
            tolerance,
        );
        check_json(
            ledger,
            suite,
            case,
            "saturated",
            &json!(derived.saturated()),
        );
        check_close(
            ledger,
            suite,
            case,
            "breach_probability",
            derived.breach_probability(),
            tolerance,
        );
        check_json(
            ledger,
            suite,
            case,
            "test_rank",
            &json!(derived.test_rank()),
        );
        check_json(ledger, suite, case, "n", &json!(derived.n()));
        check_close(
            ledger,
            suite,
            case,
            "baseline_percentile",
            Some(derived.baseline_percentile()),
            tolerance,
        );
    });
}

#[test]
fn conformance_latency_threshold() {
    check_latency_threshold(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// latency_compliance_decision — latency/compliance-exact-binomial
// ---------------------------------------------------------------------------

fn check_latency_compliance_decision(ledger: &mut Ledger) {
    let suite = "latency_compliance_decision";
    for_each_case(suite, |case, tolerance| {
        let percentile = input_f64(case, "percentile");
        let alpha = input_f64(case, "alpha");
        let intent = input_intent(case);
        // The preflight's rule for an explicit requirement: under
        // verification, a planned size from which no count can demonstrate
        // it is refused.
        let planned = input_u32(case, "planned_samples");
        let infeasible = intent == TestIntent::Verification
            && !feasibility_check(planned, percentile, alpha).feasible();
        let errors: Vec<ConfigurationError> = infeasible
            .then_some(ConfigurationError::ComplianceInfeasible)
            .into_iter()
            .collect();
        check_json(
            ledger,
            suite,
            case,
            "configuration_error",
            &error_list(&errors),
        );
        if infeasible {
            for field in [
                "test_samples",
                "within_threshold",
                "y_min",
                "pass_possible",
                "verdict",
            ] {
                check_json(ledger, suite, case, field, &Value::Null);
            }
            return;
        }
        let latencies = input_list(case, "latencies");
        let threshold_ms = input_f64(case, "threshold_ms");
        let judgement = latency::judge_latency_constraint(
            &latencies,
            &LatencyConstraint {
                percentile,
                alpha,
                mode: LatencyMode::Enforced,
                threshold: ConstraintThreshold::Explicit(threshold_ms),
            },
            intent,
        );
        assert_eq!(
            judgement.rule(),
            Some(DecisionRule::LatencyComplianceExactBinomial)
        );
        let compliance = judgement
            .compliance()
            .expect("an enforced explicit requirement carries its decision");
        check_json(
            ledger,
            suite,
            case,
            "test_samples",
            &json!(compliance.test_samples()),
        );
        let within = json!(compliance.within_threshold());
        check_json(ledger, suite, case, "within_threshold", &within);
        check_json(
            ledger,
            suite,
            case,
            "y_min",
            &opt_u32(compliance.minimum_within()),
        );
        let possible = json!(compliance.pass_possible());
        check_json(ledger, suite, case, "pass_possible", &possible);
        let verdict = verdict_name(judgement.verdict().expect("an enforced constraint"));
        check_json(ledger, suite, case, "verdict", &verdict);
        check_close(
            ledger,
            suite,
            case,
            "false_compliance",
            compliance.false_compliance(),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "clopper_pearson_lower",
            compliance.clopper_pearson_lower(),
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "observed_percentile_ms",
            compliance.observed_percentile_ms(),
            tolerance,
        );
        let advisory = compliance
            .advisory_percentile_pass(threshold_ms)
            .map_or(Value::Null, |pass| json!(pass));
        check_json(ledger, suite, case, "advisory_percentile_pass", &advisory);
    });
}

#[test]
fn conformance_latency_compliance_decision() {
    check_latency_compliance_decision(&mut Ledger::load());
}

// ---------------------------------------------------------------------------
// verdict — criteria through the production path, and the test verdict
// ---------------------------------------------------------------------------

/// The scripted criteria of a verdict case: one criterion, or a requirement
/// and a baseline over the same postcondition as two.
fn verdict_criteria(case: &Value) -> Vec<ScriptedCriterion> {
    let inputs = &case["inputs"];
    if case["approach"] == "two_criteria" {
        return vec![
            ScriptedCriterion {
                name: inputs["compliance_criterion"].as_str().unwrap().to_owned(),
                bar: ScriptedBar::Compliance(input_f64(case, "threshold")),
                confidence: confidence_of(input_f64(case, "compliance_alpha")),
            },
            ScriptedCriterion {
                name: inputs["regression_criterion"].as_str().unwrap().to_owned(),
                bar: ScriptedBar::Regression,
                confidence: confidence_of(input_f64(case, "regression_alpha")),
            },
        ];
    }
    let bar = if inputs.get("baseline_trials").is_some() {
        ScriptedBar::Regression
    } else {
        ScriptedBar::Compliance(input_f64(case, "threshold"))
    };
    vec![ScriptedCriterion {
        name: "c".to_owned(),
        bar,
        confidence: confidence_of(input_f64(case, "alpha")),
    }]
}

/// Runs the criteria of a verdict case (or of a test-verdict case's
/// functional part) through the production path.
fn run_verdict_criteria(
    case: &Value,
    inputs: &Value,
    criteria: Vec<ScriptedCriterion>,
) -> VerdictRecord {
    let id = unique_id(case);
    let regression = criteria
        .iter()
        .find(|c| matches!(c.bar, ScriptedBar::Regression))
        .map(|c| c.name.clone());
    let baseline = regression.map(|name| {
        establish_baseline(
            &id,
            &name,
            u32::try_from(inputs["baseline_successes"].as_u64().unwrap()).unwrap(),
            u32::try_from(inputs["baseline_trials"].as_u64().unwrap()).unwrap(),
        )
    });
    let intent = match inputs["intent"].as_str() {
        Some("SMOKE") => TestIntent::Smoke,
        _ => TestIntent::Verification,
    };
    run_scripted(
        ScriptedContract {
            id,
            passing: u32::try_from(inputs["successes"].as_u64().unwrap()).unwrap(),
            criteria,
        },
        baseline.as_ref(),
        u32::try_from(inputs["trials"].as_u64().unwrap()).unwrap(),
        intent,
    )
}

/// The per-criterion rows a two-criteria case lists.
fn criterion_rows(record: &VerdictRecord) -> Value {
    let rows: Vec<Value> = record
        .functional_assessment()
        .criteria()
        .iter()
        .map(|row| {
            let analysis = row
                .statistical_analysis()
                .expect("a judged criterion is decided by a rule");
            let procedure = match analysis.evidence() {
                RuleEvidence::Compliance(_) => "COMPLIANCE",
                RuleEvidence::Regression(_) => "REGRESSION",
            };
            json!({
                "criterion_id": row.name(),
                "procedure": procedure,
                "decisionRule": analysis.decision_rule().id(),
                "alpha": analysis.alpha(),
                "verdict": row.verdict().to_string(),
            })
        })
        .collect();
    json!(rows)
}

/// One criterion case of `verdict`: single, or two criteria.
fn check_verdict_criteria_case(ledger: &mut Ledger, case: &Value, tolerance: f64) {
    let suite = "verdict";
    let inputs = &case["inputs"];
    let record = run_verdict_criteria(case, inputs, verdict_criteria(case));
    let errors = error_list(record.configuration_errors());
    check_json(ledger, suite, case, "configuration_error", &errors);
    check_json(ledger, suite, case, "verdict", &record_verdict(&record));
    let observed = f64::from(input_u32(case, "successes")) / f64::from(input_u32(case, "trials"));
    check_close(
        ledger,
        suite,
        case,
        "observed_rate",
        Some(observed),
        tolerance,
    );
    let triggering: Vec<&str> = record
        .triggering()
        .iter()
        .filter(|t| t.kind().name() == "criterion")
        .map(Trigger::id)
        .collect();
    check_json(
        ledger,
        suite,
        case,
        "triggering_criteria",
        &json!(triggering),
    );
    if record.is_refused() {
        check_json(ledger, suite, case, "criteria", &json!([]));
        check_close(
            ledger,
            suite,
            case,
            "false_compliance_envelope",
            None,
            tolerance,
        );
        check_close(
            ledger,
            suite,
            case,
            "false_degradation_signal_envelope",
            None,
            tolerance,
        );
        return;
    }
    let rules: Vec<&str> = record
        .functional_assessment()
        .criteria()
        .iter()
        .filter_map(|row| row.statistical_analysis())
        .map(|analysis| analysis.decision_rule().id())
        .collect();
    let declared: Vec<Value> = match &case["decisionRule"] {
        Value::Array(rules) => rules.clone(),
        single => vec![single.clone()],
    };
    assert_eq!(
        json!(rules),
        json!(declared),
        "{}: decision rules",
        case_name(case)
    );
    check_json(ledger, suite, case, "criteria", &criterion_rows(&record));
    let envelopes = record.envelopes();
    check_close(
        ledger,
        suite,
        case,
        "false_compliance_envelope",
        envelopes.false_compliance(),
        tolerance,
    );
    check_close(
        ledger,
        suite,
        case,
        "false_degradation_signal_envelope",
        envelopes.false_degradation_signal(),
        tolerance,
    );
}

/// One latency constraint of a test-verdict case, judged by the rule for
/// its threshold source: its fixture row, and its verdict when enforced.
fn latency_constraint_row(constraint: &Value) -> (Value, Option<Verdict>) {
    let latencies = numbers(&constraint["latencies"]).unwrap();
    let baseline = numbers(&constraint["baseline_latencies"]).unwrap_or_default();
    let threshold = match constraint["source"].as_str().unwrap() {
        "explicit" => ConstraintThreshold::Explicit(constraint["threshold_ms"].as_f64().unwrap()),
        _ => ConstraintThreshold::BaselineDerived(&baseline),
    };
    let mode = match constraint["mode"].as_str().unwrap() {
        "enforced" => LatencyMode::Enforced,
        _ => LatencyMode::Advisory,
    };
    let judgement = latency::judge_latency_constraint(
        &latencies,
        &LatencyConstraint {
            percentile: constraint["percentile"].as_f64().unwrap(),
            alpha: constraint["alpha"].as_f64().unwrap(),
            mode,
            threshold,
        },
        TestIntent::Verification,
    );
    let outcome = match judgement.outcome() {
        LatencyOutcome::Decided(verdict) => verdict.to_string(),
        LatencyOutcome::Advisory(advisory) => advisory.name().to_owned(),
    };
    let row = json!({
        "constraint_id": constraint["constraint_id"],
        "source": judgement.source().name(),
        "mode": if mode == LatencyMode::Enforced { "enforced" } else { "advisory" },
        "participates": mode == LatencyMode::Enforced,
        "decisionRule": judgement.rule().map_or(Value::Null, |rule| json!(rule.id())),
        "verdict": outcome,
    });
    (row, judgement.verdict())
}

/// One test-verdict case of `verdict`: the functional criterion through the
/// production path, the latency constraints by their rules, composed by the
/// structural rule the runner uses.
fn check_test_verdict_case(ledger: &mut Ledger, case: &Value) {
    let suite = "verdict";
    let inputs = &case["inputs"];
    let mut criteria: Vec<(String, Verdict)> = Vec::new();
    if let Some(functional) = inputs.get("functional") {
        let name = functional["criterion_id"].as_str().unwrap().to_owned();
        let bar = if functional.get("baseline_trials").is_some() {
            ScriptedBar::Regression
        } else {
            ScriptedBar::Compliance(functional["threshold"].as_f64().unwrap())
        };
        let scripted = ScriptedCriterion {
            name: name.clone(),
            bar,
            confidence: confidence_of(functional["alpha"].as_f64().unwrap()),
        };
        let record = run_verdict_criteria(case, functional, vec![scripted]);
        let row = &record.functional_assessment().criteria()[0];
        criteria.push((name, row.verdict()));
    }
    let mut rows = Vec::new();
    let mut enforced = Vec::new();
    for constraint in inputs["latency_constraints"].as_array().unwrap() {
        let (row, verdict) = latency_constraint_row(constraint);
        if let Some(verdict) = verdict {
            enforced.push((row["constraint_id"].as_str().unwrap().to_owned(), verdict));
        }
        rows.push(row);
    }
    let overall = compose_overall_verdict(&criteria, &enforced);
    let criteria_rows: Vec<Value> = criteria
        .iter()
        .map(|(id, verdict)| json!({"criterion_id": id, "verdict": verdict.to_string()}))
        .collect();
    check_json(ledger, suite, case, "criteria", &json!(criteria_rows));
    check_json(ledger, suite, case, "latency_constraints", &json!(rows));
    let rate = overall.rate_verdict().map_or(Value::Null, verdict_name);
    check_json(ledger, suite, case, "rate_verdict", &rate);
    let latency_verdict = overall.latency_verdict().map_or(Value::Null, verdict_name);
    check_json(ledger, suite, case, "latency_verdict", &latency_verdict);
    check_json(
        ledger,
        suite,
        case,
        "test_verdict",
        &verdict_name(overall.verdict()),
    );
    let triggering: Vec<Value> = overall
        .triggering()
        .iter()
        .map(|t| json!({"kind": t.kind().name(), "id": t.id()}))
        .collect();
    check_json(ledger, suite, case, "triggering", &json!(triggering));
}

fn check_verdict(ledger: &mut Ledger) {
    for_each_case("verdict", |case, tolerance| {
        if case["approach"] == "test_verdict" {
            check_test_verdict_case(ledger, case);
        } else {
            check_verdict_criteria_case(ledger, case, tolerance);
        }
    });
}

#[test]
fn conformance_verdict() {
    check_verdict(&mut Ledger::load());
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
    let checks: [(&str, SuiteCheck); 13] = [
        ("wilson_ci", check_wilson_ci),
        ("wilson_lower", check_wilson_lower),
        ("regression_decision", check_regression_decision),
        ("compliance_decision", check_compliance_decision),
        ("feasibility", check_feasibility),
        ("threshold_derivation", check_threshold_derivation),
        ("power_analysis", check_power_analysis),
        ("risk_driven_sizing", check_risk_driven_sizing),
        ("verdict", check_verdict),
        ("latency_percentile", check_latency_percentile),
        (
            "latency_percentile_minimums",
            check_latency_percentile_minimums,
        ),
        ("latency_threshold", check_latency_threshold),
        (
            "latency_compliance_decision",
            check_latency_compliance_decision,
        ),
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
