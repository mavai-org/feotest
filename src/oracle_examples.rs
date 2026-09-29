//! Oracle-sourced examples for the report tests.
//!
//! Report snapshots show numbers; those numbers come from the vendored
//! conformance fixtures of the reference oracle, not from values typed into
//! the test. Each example names its fixture case, recomputes the decision
//! with this crate's statistics, and asserts the rule's binding figures
//! against the fixture before any renderer sees them.

use serde_json::Value;

use crate::model::ThresholdOrigin;
use crate::statistics::decision::{
    Trigger, compose_overall_verdict, evaluate_compliance, evaluate_regression,
};
use crate::statistics::regression::{MDD_POWER, minimum_detectable_degradation};
use crate::verdict::{CriterionRow, DesignDisclosure, StatisticalAnalysis, Verdict};

/// The vendored `regression_decision` suite.
const REGRESSION: &str = include_str!("../tests/conformance/regression_decision.json");

/// The vendored `verdict` suite.
const VERDICT: &str = include_str!("../tests/conformance/verdict.json");

/// The vendored `compliance_decision` suite.
const COMPLIANCE: &str = include_str!("../tests/conformance/compliance_decision.json");

/// One case of a vendored suite, by name.
fn case(suite: &str, name: &str) -> Value {
    let parsed: Value = serde_json::from_str(suite).expect("a vendored suite is valid JSON");
    parsed["cases"]
        .as_array()
        .expect("a suite carries cases")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no fixture case named {name}"))
        .clone()
}

/// A non-negative integer input.
fn count(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("a count")).expect("a count fits in u32")
}

/// The fixture's verdict as a `Verdict`.
fn verdict(value: &Value) -> Verdict {
    match value.as_str().expect("a verdict") {
        "PASS" => Verdict::Pass,
        "FAIL" => Verdict::Fail,
        other => panic!("unexpected fixture verdict {other}"),
    }
}

/// The criterion row of a `regression_decision` case, decided by this
/// crate's `regression/fisher` and checked against the fixture.
pub fn regression_row(name: &str, criterion: &str) -> CriterionRow {
    let case = case(REGRESSION, name);
    let (inputs, expected) = (&case["inputs"], &case["expected"]);
    let successes = count(&inputs["observed_successes"]);
    let trials = count(&inputs["test_samples"]);
    let alpha = inputs["alpha"].as_f64().expect("alpha");
    let decision = evaluate_regression(
        successes,
        trials,
        count(&inputs["baseline_successes"]),
        count(&inputs["baseline_trials"]),
        alpha,
    );
    assert_eq!(decision.cutoff(), count(&expected["cutoff_integer"]));
    assert_eq!(decision.verdict(), verdict(&expected["verdict"]));
    let baseline_trials = count(&inputs["baseline_trials"]);
    let baseline_rate =
        f64::from(count(&inputs["baseline_successes"])) / f64::from(baseline_trials);
    let disclosure = DesignDisclosure {
        minimum_detectable_degradation: minimum_detectable_degradation(
            baseline_trials,
            trials,
            alpha,
            baseline_rate,
            MDD_POWER,
        ),
        ..DesignDisclosure::default()
    };
    let analysis = StatisticalAnalysis::regression(&decision, 1.0 - alpha, disclosure);
    CriterionRow::new(
        criterion,
        successes,
        trials - successes,
        failure_distribution(trials - successes),
        Some(analysis),
        decision.verdict(),
    )
}

/// The criterion row of a `compliance_decision` case, decided by this
/// crate's `compliance/exact-binomial` and checked against the fixture.
pub fn compliance_row(name: &str, criterion: &str, origin: ThresholdOrigin) -> CriterionRow {
    let case = case(COMPLIANCE, name);
    let (inputs, expected) = (&case["inputs"], &case["expected"]);
    let successes = count(&inputs["observed_successes"]);
    let trials = count(&inputs["test_samples"]);
    let alpha = inputs["alpha"].as_f64().expect("alpha");
    let decision = evaluate_compliance(
        successes,
        trials,
        inputs["threshold"].as_f64().expect("threshold"),
        alpha,
    );
    let k_min = expected["k_min"]
        .as_u64()
        .map(|k| u32::try_from(k).unwrap());
    assert_eq!(decision.minimum_passing(), k_min);
    assert_eq!(decision.verdict(), verdict(&expected["verdict"]));
    let analysis = StatisticalAnalysis::compliance(&decision, 1.0 - alpha, origin);
    CriterionRow::new(
        criterion,
        successes,
        trials - successes,
        failure_distribution(trials - successes),
        Some(analysis),
        decision.verdict(),
    )
}

/// The two criteria of a `verdict` two-criteria case — a requirement and a
/// baseline over the same postcondition and the same samples — decided by
/// this crate's rules and checked against the fixture, with the test's
/// triggering criteria.
pub fn two_criteria(name: &str) -> (Vec<CriterionRow>, Vec<Trigger>) {
    let case = case(VERDICT, name);
    let inputs = &case["inputs"];
    let successes = count(&inputs["successes"]);
    let trials = count(&inputs["trials"]);
    let compliance_alpha = inputs["compliance_alpha"].as_f64().expect("alpha");
    let regression_alpha = inputs["regression_alpha"].as_f64().expect("alpha");
    let compliance = evaluate_compliance(
        successes,
        trials,
        inputs["threshold"].as_f64().expect("threshold"),
        compliance_alpha,
    );
    let regression = evaluate_regression(
        successes,
        trials,
        count(&inputs["baseline_successes"]),
        count(&inputs["baseline_trials"]),
        regression_alpha,
    );
    let failures = failure_distribution(trials - successes);
    let rows = vec![
        CriterionRow::new(
            inputs["compliance_criterion"].as_str().expect("name"),
            successes,
            trials - successes,
            failures.clone(),
            Some(StatisticalAnalysis::compliance(
                &compliance,
                1.0 - compliance_alpha,
                ThresholdOrigin::Sla,
            )),
            compliance.verdict(),
        ),
        CriterionRow::new(
            inputs["regression_criterion"].as_str().expect("name"),
            successes,
            trials - successes,
            failures,
            Some(StatisticalAnalysis::regression(
                &regression,
                1.0 - regression_alpha,
                DesignDisclosure::default(),
            )),
            regression.verdict(),
        ),
    ];
    for (row, expected) in rows
        .iter()
        .zip(case["expected"]["criteria"].as_array().unwrap())
    {
        assert_eq!(row.verdict(), verdict(&expected["verdict"]));
    }
    let pairs: Vec<(String, Verdict)> = rows
        .iter()
        .map(|row| (row.name().to_owned(), row.verdict()))
        .collect();
    let overall = compose_overall_verdict(&pairs, &[]);
    assert_eq!(overall.verdict(), verdict(&case["expected"]["verdict"]));
    (rows, overall.triggering().to_vec())
}

/// A failure distribution attributing the failures to two checks.
fn failure_distribution(failures: u32) -> Vec<(String, u32)> {
    if failures == 0 {
        return Vec::new();
    }
    let parse = failures.div_ceil(2);
    let mut distribution = vec![("parse".to_owned(), parse)];
    if failures > parse {
        distribution.push(("content".to_owned(), failures - parse));
    }
    distribution
}

/// The record-level analysis of a row, as the runner attaches it.
pub fn analysis_of(row: &CriterionRow) -> StatisticalAnalysis {
    row.statistical_analysis()
        .cloned()
        .expect("an oracle example row is decided by a rule")
}
