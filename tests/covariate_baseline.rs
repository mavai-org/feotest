//! Integration tests for the covariate-aware baseline lifecycle.
//!
//! Exercises the full path: measure with covariates → write spec →
//! resolve with matching/mismatched covariates → verify warnings and
//! verdict record fields.

mod common;

use feotest::experiment::MeasureExperiment;
use feotest::model::ThresholdOrigin;
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::service_contract::{CovariateCategory, CovariateDeclaration, ServiceContract};
use feotest::spec::SpecResolver;
use feotest::spec::namer::CovariateProfile;
use feotest::verdict::Verdict;

// ---------------------------------------------------------------------------
// Service contract with covariates
// ---------------------------------------------------------------------------

struct CovariateServiceContract {
    id: &'static str,
    model: &'static str,
}

impl ServiceContract for CovariateServiceContract {
    type Input = String;
    type Output = String;

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

    fn invoke(
        &self,
        input: &String,
        _cost: &mut feotest::controls::Cost,
    ) -> Result<String, feotest::model::Defect> {
        Ok(input.clone())
    }

    fn criteria(&self) -> feotest::criteria::Criteria<String> {
        feotest::criteria::Criteria::of([feotest::criteria::Criteria::meeting()
            .pass_rate(0.5)
            .name("response received")
            .satisfies("response received", |_: &String| Ok(()))
            .build()])
    }
}

// ---------------------------------------------------------------------------
// Measure + test with matching covariates
// ---------------------------------------------------------------------------

#[test]
fn matching_covariates_resolves_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let uc = CovariateServiceContract {
        id: "cov-match",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    // Establish baseline with model=gpt-4o
    MeasureExperiment::builder()
        .service_contract_id(uc.id().to_owned())
        .service_contract(|| common::SimpleServiceContract::new("baseline"))
        .samples(200)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .covariates(vec!["model".to_owned()], uc.resolve_covariates())
        .build()
        .run();

    // Run test with same covariate profile
    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTest::for_contract(uc)
        .inputs(&inputs)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
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
    let baseline_uc = CovariateServiceContract {
        id: "cov-mismatch",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    // Establish baseline with model=gpt-4o
    MeasureExperiment::builder()
        .service_contract_id(baseline_uc.id().to_owned())
        .service_contract(|| common::SimpleServiceContract::new("baseline"))
        .samples(200)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .covariates(vec!["model".to_owned()], baseline_uc.resolve_covariates())
        .build()
        .run();

    // Run test with model=gpt-4o-mini (different covariate value)
    let test_uc = CovariateServiceContract {
        id: "cov-mismatch",
        model: "gpt-4o-mini",
    };
    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTest::for_contract(test_uc)
        .inputs(&inputs)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
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
    let uc = CovariateServiceContract {
        id: "cov-prov",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .service_contract_id(uc.id().to_owned())
        .service_contract(|| common::SimpleServiceContract::new("baseline"))
        .samples(200)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .covariates(vec!["model".to_owned()], uc.resolve_covariates())
        .build()
        .run();

    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTest::for_contract(uc)
        .inputs(&inputs)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
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
    let uc = CovariateServiceContract {
        id: "cov-tf",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .service_contract_id(uc.id().to_owned())
        .service_contract(|| common::SimpleServiceContract::new("baseline"))
        .samples(200)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .covariates(vec!["model".to_owned()], uc.resolve_covariates())
        .build()
        .run();

    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTest::for_contract(uc)
        .inputs(&inputs)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 50,
            min_pass_rate: 0.80,
        })
        .threshold_origin(ThresholdOrigin::Sla)
        .spec_resolver(resolver)
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
    let baseline_uc = CovariateServiceContract {
        id: "cov-render",
        model: "gpt-4o",
    };
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .service_contract_id(baseline_uc.id().to_owned())
        .service_contract(|| common::SimpleServiceContract::new("baseline"))
        .samples(200)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .covariates(vec!["model".to_owned()], baseline_uc.resolve_covariates())
        .build()
        .run();

    let test_uc = CovariateServiceContract {
        id: "cov-render",
        model: "claude-3-opus",
    };
    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTest::for_contract(test_uc)
        .inputs(&inputs)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
        .run();

    let renderer = feotest::reporting::ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    assert!(output.contains("Warning:"));
    assert!(output.contains("COVARIATE_MISMATCH"));
}
