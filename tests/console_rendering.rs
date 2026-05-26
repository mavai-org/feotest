//! Integration tests for the console renderer fed with runner-produced records.
//!
//! Unlike the unit tests in `reporting/console.rs` which use hand-built
//! VerdictRecords, these tests run the full pipeline and then render the
//! result. This catches field-population bugs that unit tests cannot.

mod common;

use std::time::Duration;

use feotest::latency::LatencyCriterion;
use feotest::model::ThresholdOrigin;
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::reporting::ConsoleRenderer;
use feotest::spec::SpecResolver;

/// A contract that sleeps a fixed (small) duration on every invocation and
/// commits to a p95 ceiling, so the latency section renders from real
/// invoke-elapsed measurements.
struct LatencyContract {
    latency: Duration,
    p95_ceiling: Duration,
}

impl feotest::service_contract::ServiceContract for LatencyContract {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        "cr-latency"
    }

    fn invoke(
        &self,
        input: &String,
        _cost: &mut feotest::controls::Cost,
    ) -> Result<String, feotest::model::Defect> {
        std::thread::sleep(self.latency);
        Ok(input.clone())
    }

    fn criteria(&self) -> feotest::criteria::Criteria<String> {
        feotest::criteria::Criteria::of([feotest::criteria::Criteria::meeting()
            .pass_rate(0.5)
            .name("response received")
            .satisfies("response received", |_: &String| Ok(()))
            .build()])
    }

    fn latency(&self) -> Option<LatencyCriterion> {
        Some(
            LatencyCriterion::meeting()
                .at_most(feotest::latency::Percentile::P95, self.p95_ceiling),
        )
    }
}

// ---------------------------------------------------------------------------
// Pass verdict — full detail
// ---------------------------------------------------------------------------

#[test]
fn pass_with_baseline_renders_all_sections() {
    let dir = common::establish_baseline("cr-pass-full", 200);

    let result = common::run_against_baseline("cr-pass-full", dir.path(), 50, 0.80);

    let renderer = ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    // Header
    assert!(output.contains("VERDICT: PASS"));
    assert!(output.contains(">="));

    // Body sections
    assert!(output.contains("Test:"));
    assert!(output.contains("Pass rate:"));
    assert!(output.contains("Threshold:"));
    assert!(output.contains("Confidence:"));
    assert!(output.contains("Wilson lower:"));
    assert!(output.contains("Baseline:"));
    assert!(output.contains("cr-pass-full.yaml"));
    assert!(output.contains("minPassRate="));
    assert!(output.contains("Spec:"));
    assert!(output.contains("Origin:"));
    assert!(output.contains("Elapsed:"));
}

// ---------------------------------------------------------------------------
// Fail verdict
// ---------------------------------------------------------------------------

#[test]
fn fail_verdict_renders_correctly() {
    let inputs: Vec<String> = (0..5)
        .map(|i| if i < 4 { "fail".into() } else { "ok".into() })
        .collect();

    let result = ProbabilisticTest::for_contract(common::InputJudgedContract::new("cr-fail"))
        .inputs(&inputs)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        })
        .run();

    let renderer = ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    assert!(output.contains("VERDICT: FAIL"));
    assert!(output.contains('<'));
}

// ---------------------------------------------------------------------------
// Warnings render
// ---------------------------------------------------------------------------

#[test]
fn smoke_normative_warnings_render() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(common::SimpleServiceContract::new("cr-smoke"))
        .inputs(&inputs)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 50,
            min_pass_rate: 0.80,
        })
        .smoke()
        .threshold_origin(ThresholdOrigin::Sla)
        .run();

    let renderer = ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    assert!(output.contains("Warning:"));
    assert!(output.contains("SMOKE_NORMATIVE"));
}

// ---------------------------------------------------------------------------
// Latency section renders from real execution
// ---------------------------------------------------------------------------

#[test]
fn latency_section_renders_when_thresholds_set() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(LatencyContract {
        latency: Duration::from_millis(10),
        p95_ceiling: Duration::from_millis(50),
    })
    .inputs(&inputs)
    .approach(ThresholdApproach::ThresholdFirst {
        samples: 30,
        min_pass_rate: 0.80,
    })
    .run();

    let renderer = ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    assert!(output.contains("Latency"));
    assert!(output.contains("p95:"));
    assert!(output.contains("PASS"));
}

// ---------------------------------------------------------------------------
// Suite summary from multiple runner-produced records
// ---------------------------------------------------------------------------

#[test]
fn suite_summary_from_real_results() {
    let inputs = vec!["input".to_string()];

    let pass_result =
        ProbabilisticTest::for_contract(common::SimpleServiceContract::new("cr-suite-pass"))
            .inputs(&inputs)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 30,
                min_pass_rate: 0.80,
            })
            .run();

    let fail_inputs: Vec<String> = (0..5)
        .map(|i| if i < 4 { "fail".into() } else { "ok".into() })
        .collect();

    let fail_result =
        ProbabilisticTest::for_contract(common::InputJudgedContract::new("cr-suite-fail"))
            .inputs(&fail_inputs)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.90,
            })
            .run();

    let records = vec![
        pass_result.verdict_record().clone(),
        fail_result.verdict_record().clone(),
    ];

    let renderer = ConsoleRenderer::without_colour();
    let mut buf = String::new();
    renderer
        .render_summary(&records, Duration::from_secs_f64(1.5), &mut buf)
        .unwrap();

    assert!(buf.contains("2 tests"));
    assert!(buf.contains("1 passed"));
    assert!(buf.contains("1 failed"));
    assert!(buf.contains("0 inconclusive"));
}

// ---------------------------------------------------------------------------
// Sample-size-first renders with baseline provenance
// ---------------------------------------------------------------------------

#[test]
fn sample_size_first_renders_baseline_provenance() {
    let dir = common::establish_baseline("cr-ssf", 200);

    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(common::SimpleServiceContract::new("cr-ssf"))
        .inputs(&inputs)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(SpecResolver::with_dir(dir.path()))
        .threshold_origin(ThresholdOrigin::Empirical)
        .run();

    let renderer = ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    assert!(output.contains("Baseline:"));
    assert!(output.contains("cr-ssf.yaml"));
    assert!(output.contains("samples"));
    assert!(output.contains("minPassRate="));
}
