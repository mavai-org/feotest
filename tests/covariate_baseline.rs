//! Integration tests for the covariate-aware baseline lifecycle.
//!
//! Exercises the full path: measure with covariates → write spec →
//! resolve with matching/mismatched covariates → verify warnings and
//! verdict record fields.

use std::time::Duration;

use feotest::experiment::MeasureExperiment;
use feotest::model::{ThresholdOrigin, TrialOutcome};
use feotest::ptest::ProbabilisticTestBuilder;
use feotest::ptest::builder::ThresholdApproach;
use feotest::spec::SpecResolver;
use feotest::spec::namer::CovariateProfile;
use feotest::usecase::{CovariateCategory, CovariateDeclaration, UseCase};
use feotest::verdict::Verdict;

fn always_succeeds(_input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

// ---------------------------------------------------------------------------
// Use case with covariates
// ---------------------------------------------------------------------------

struct CovariateUseCase {
    id: &'static str,
    model: &'static str,
}

impl UseCase for CovariateUseCase {
    fn id(&self) -> &str {
        self.id
    }

    fn covariates(&self) -> Vec<CovariateDeclaration> {
        vec![CovariateDeclaration::new(
            "model",
            CovariateCategory::ExternalDependency,
        )]
    }

    fn resolve_covariates(&self) -> CovariateProfile {
        CovariateProfile::builder().put("model", self.model).build()
    }
}

// ---------------------------------------------------------------------------
// Measure + test with matching covariates
// ---------------------------------------------------------------------------

#[test]
fn matching_covariates_resolves_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let uc = CovariateUseCase {
        id: "cov-match",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    // Establish baseline with model=gpt-4o
    MeasureExperiment::builder()
        .use_case(&uc)
        .samples(200)
        .inputs(&inputs)
        .trial(always_succeeds)
        .baseline_dir(dir.path())
        .build()
        .run();

    // Run test with same covariate profile
    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("cov-match", &inputs, always_succeeds)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
        .use_case(&uc)
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    // No covariate mismatch warnings
    assert!(
        !record
            .warnings()
            .iter()
            .any(|w| w.code() == "COVARIATE_MISMATCH"),
        "matching covariates should not produce mismatch warnings"
    );
}

// ---------------------------------------------------------------------------
// Measure + test with mismatched covariates
// ---------------------------------------------------------------------------

#[test]
fn mismatched_covariates_produce_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let baseline_uc = CovariateUseCase {
        id: "cov-mismatch",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    // Establish baseline with model=gpt-4o
    MeasureExperiment::builder()
        .use_case(&baseline_uc)
        .samples(200)
        .inputs(&inputs)
        .trial(always_succeeds)
        .baseline_dir(dir.path())
        .build()
        .run();

    // Run test with model=gpt-4o-mini (different covariate value)
    let test_uc = CovariateUseCase {
        id: "cov-mismatch",
        model: "gpt-4o-mini",
    };
    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("cov-mismatch", &inputs, always_succeeds)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
        .use_case(&test_uc)
        .run();

    let record = result.verdict_record();
    // The test still passes (baseline still resolves, just with a mismatch warning)
    assert_eq!(record.verdict(), Verdict::Pass);
    assert!(
        record
            .warnings()
            .iter()
            .any(|w| w.code() == "COVARIATE_MISMATCH"),
        "mismatched covariates should produce a COVARIATE_MISMATCH warning"
    );
}

// ---------------------------------------------------------------------------
// Baseline provenance populated when covariates are used
// ---------------------------------------------------------------------------

#[test]
fn baseline_provenance_present_with_covariates() {
    let dir = tempfile::tempdir().unwrap();
    let uc = CovariateUseCase {
        id: "cov-prov",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .use_case(&uc)
        .samples(200)
        .inputs(&inputs)
        .trial(always_succeeds)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("cov-prov", &inputs, always_succeeds)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
        .use_case(&uc)
        .run();

    let bp = result
        .verdict_record()
        .baseline_provenance()
        .expect("baseline provenance should be populated");
    assert!(bp.baseline_samples() > 0);
    assert!(bp.baseline_rate() > 0.0);
}

// ---------------------------------------------------------------------------
// Threshold-first with covariates still loads baseline for integrity
// ---------------------------------------------------------------------------

#[test]
fn threshold_first_with_covariates_loads_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let uc = CovariateUseCase {
        id: "cov-tf",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .use_case(&uc)
        .samples(200)
        .inputs(&inputs)
        .trial(always_succeeds)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("cov-tf", &inputs, always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 50,
            min_pass_rate: 0.80,
        })
        .threshold_origin(ThresholdOrigin::Sla)
        .spec_resolver(resolver)
        .use_case(&uc)
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    // Baseline was loaded (for integrity check), so provenance is populated
    assert!(record.baseline_provenance().is_some());
}

// ---------------------------------------------------------------------------
// Console rendering with covariate warnings
// ---------------------------------------------------------------------------

#[test]
fn console_renders_covariate_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let baseline_uc = CovariateUseCase {
        id: "cov-render",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .use_case(&baseline_uc)
        .samples(200)
        .inputs(&inputs)
        .trial(always_succeeds)
        .baseline_dir(dir.path())
        .build()
        .run();

    let test_uc = CovariateUseCase {
        id: "cov-render",
        model: "claude-3-opus",
    };
    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("cov-render", &inputs, always_succeeds)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
        .use_case(&test_uc)
        .run();

    let renderer = feotest::reporting::ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    assert!(output.contains("Warning:"));
    assert!(output.contains("COVARIATE_MISMATCH"));
}
