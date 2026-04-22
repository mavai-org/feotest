//! Integration test for transparent statistics end-to-end wiring.
//!
//! Verifies that the `transparent_stats` flag on both builder APIs
//! produces output containing the expected section markers, without
//! asserting the full formatted layout (unit tests cover that).

use std::time::Duration;

use feotest::model::TrialOutcome;
use feotest::ptest::ProbabilisticTestBuilder;
use feotest::ptest::builder::ThresholdApproach;
use feotest::reporting::render_transparent_stats;
use feotest::verdict::Verdict;

fn always_succeeds(_input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

#[test]
fn builder_result_carries_approach() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("wiring-test", &inputs, always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 30,
            min_pass_rate: 0.80,
        })
        .run();

    // The approach should be accessible on the result
    assert!(matches!(
        result.approach(),
        ThresholdApproach::ThresholdFirst { .. }
    ));
    assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
}

#[test]
fn render_produces_all_sections() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("section-test", &inputs, always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 50,
            min_pass_rate: 0.80,
        })
        .run();

    let mut buf = String::new();
    render_transparent_stats(result.verdict_record(), result.approach(), &mut buf)
        .expect("rendering should not fail");

    // Verify all mandatory section markers are present
    assert!(buf.contains("TRANSPARENT STATISTICS"), "missing header");
    assert!(buf.contains("HYPOTHESES"), "missing hypotheses");
    assert!(buf.contains("H0:"), "missing H0");
    assert!(buf.contains("H1:"), "missing H1");
    assert!(
        buf.contains("OBSERVED DATA AND INFERENCE"),
        "missing observed data"
    );
    assert!(buf.contains("VERDICT"), "missing verdict");
    assert!(buf.contains("PASS"), "missing pass verdict");
}

#[test]
fn render_with_sample_size_first_approach() {
    let dir = tempfile::tempdir().unwrap();

    struct Uc;
    impl feotest::usecase::UseCase for Uc {
        fn id(&self) -> &str {
            "transparent-ssf"
        }
    }
    let uc = Uc;
    let inputs = vec!["input".to_string()];

    // Establish a baseline
    feotest::experiment::MeasureExperiment::builder()
        .use_case_id("transparent-ssf")
        .use_case(|| ())
        .samples(200)
        .inputs(&inputs)
        .trial(|(): &(), input| always_succeeds(input))
        .baseline_dir(dir.path())
        .build()
        .run();

    // Run a probabilistic test using the baseline
    let resolver = feotest::spec::SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("transparent-ssf", &inputs, always_succeeds)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .run();

    let mut buf = String::new();
    render_transparent_stats(result.verdict_record(), result.approach(), &mut buf).unwrap();

    assert!(buf.contains("Sample-size-first"));
    assert!(buf.contains("derived from baseline at 0.950 confidence"));
}

#[test]
fn render_fail_verdict_includes_rejection() {
    // Force failures: 8 out of 10 inputs are "fail"
    let inputs: Vec<String> = (0..10)
        .map(|i| {
            if i < 8 {
                "fail".to_string()
            } else {
                "ok".to_string()
            }
        })
        .collect();

    let result = ProbabilisticTestBuilder::new("fail-test", &inputs, |input| {
        if input == "fail" {
            TrialOutcome::failure(
                feotest::model::ContractViolation::new("check", "forced"),
                Duration::from_millis(1),
            )
        } else {
            TrialOutcome::success(Duration::from_millis(1))
        }
    })
    .approach(ThresholdApproach::ThresholdFirst {
        samples: 100,
        min_pass_rate: 0.90,
    })
    .run();

    let mut buf = String::new();
    render_transparent_stats(result.verdict_record(), result.approach(), &mut buf).unwrap();

    assert!(buf.contains("FAIL"));
    assert!(buf.contains("null hypothesis is rejected"));
}

#[test]
fn render_output_has_consistent_box_width() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("box-test", &inputs, always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 30,
            min_pass_rate: 0.80,
        })
        .run();

    let mut buf = String::new();
    render_transparent_stats(result.verdict_record(), result.approach(), &mut buf).unwrap();

    for line in buf.lines() {
        let char_count = line.chars().count();
        assert!(
            char_count <= 63,
            "Line exceeds 63 chars ({char_count}): {line:?}"
        );
    }
}
