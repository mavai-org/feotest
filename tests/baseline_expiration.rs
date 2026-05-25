//! Integration tests for EX08 baseline expiration.

use std::time::{Duration, SystemTime};

use feotest::experiment::MeasureExperiment;
use feotest::model::{ExpirationStatus, TrialOutcome};
use feotest::ptest::ProbabilisticTestBuilder;
use feotest::ptest::builder::ThresholdApproach;
use feotest::spec::common::{iso8601_plus_days, parse_iso8601};
use feotest::spec::expiration;
use feotest::spec::{BaselineSpec, SpecResolver};
use feotest::service_contract::ServiceContract;
use feotest::verdict::Verdict;

struct TestUc(&'static str);
impl ServiceContract for TestUc {
    type Input = String;
    type Output = String;
    fn id(&self) -> &str {
        self.0
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

const fn always_succeed(_input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

#[test]
fn measure_writes_expiration_block_that_round_trips_through_resolver() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let measure_result = MeasureExperiment::builder()
        .service_contract_id("expiry-uc")
        .service_contract(|| ())
        .samples(30)
        .inputs(&inputs)
        .trial(|(): &(), input| always_succeed(input))
        .baseline_dir(dir.path())
        .expires_in_days(1)
        .build()
        .run();

    let spec_path = measure_result.spec_path().expect("spec was written");
    let raw = std::fs::read_to_string(spec_path).unwrap();
    assert!(raw.contains("expiration:"));
    assert!(raw.contains("expiresInDays: 1"));

    let spec = BaselineSpec::from_yaml(&raw).unwrap();
    let exp = spec.expiration.as_ref().unwrap();
    assert_eq!(exp.expires_in_days, 1);

    // Fresh baseline → Valid when evaluated immediately.
    let info = expiration::evaluate(&spec);
    assert_eq!(info.status(), &ExpirationStatus::Valid);
}

#[test]
fn evaluate_at_future_time_crosses_expiry_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .service_contract_id("boundary-uc")
        .service_contract(|| ())
        .samples(30)
        .inputs(&inputs)
        .trial(|(): &(), input| always_succeed(input))
        .baseline_dir(dir.path())
        .expires_in_days(1)
        .build()
        .run();

    let raw = std::fs::read_to_string(
        std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
    )
    .unwrap();
    let spec = BaselineSpec::from_yaml(&raw).unwrap();
    let expiration_date = spec.expiration.as_ref().unwrap().expiration_date.clone();

    // One second before expiry: either Valid or ExpiringImminently depending
    // on where "now" sits relative to a 1-day window. Both mean "not yet
    // expired".
    let just_before = parse_iso8601(&expiration_date).unwrap() - Duration::from_secs(1);
    let before = expiration::evaluate_at(&spec, just_before);
    assert_ne!(before.status(), &ExpirationStatus::Expired);

    // One second after expiry: Expired.
    let just_after = parse_iso8601(&expiration_date).unwrap() + Duration::from_secs(1);
    let after = expiration::evaluate_at(&spec, just_after);
    assert_eq!(after.status(), &ExpirationStatus::Expired);
}

#[test]
fn ptest_with_expired_baseline_warns_by_default_and_still_passes() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    // Measure with a 1-day window, then hand-edit the baseline to claim it
    // ended in the distant past so it is definitively expired. Because the
    // fingerprint covers the expiration block, we must recompute the spec
    // end-to-end rather than patching the YAML directly.
    MeasureExperiment::builder()
        .service_contract_id("warn-only")
        .service_contract(|| ())
        .samples(30)
        .inputs(&inputs)
        .trial(|(): &(), input| always_succeed(input))
        .baseline_dir(dir.path())
        .expires_in_days(1)
        .build()
        .run();

    let entry = std::fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let raw = std::fs::read_to_string(entry.path()).unwrap();
    let mut spec = BaselineSpec::from_yaml(&raw).unwrap();
    // Rewrite the expiration block to one that ended 10 days ago.
    let end = "2020-01-01T00:00:00Z".to_string();
    spec.expiration = Some(feotest::spec::baseline::ExpirationBlock {
        expires_in_days: 1,
        baseline_end_time: end.clone(),
        expiration_date: iso8601_plus_days(&end, 1).unwrap(),
    });

    let result = ProbabilisticTestBuilder::new("warn-only", &inputs, always_succeed)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 50,
            confidence: 0.95,
        })
        .threshold_origin(feotest::model::ThresholdOrigin::Empirical)
        .baseline_spec(spec)
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    assert!(
        record
            .warnings()
            .iter()
            .any(|w| w.code() == "BASELINE_EXPIRED"),
        "expected BASELINE_EXPIRED warning, got: {:?}",
        record.warnings()
    );
    // Provenance carries the ExpirationInfo.
    let prov = record.spec_provenance().unwrap();
    let info = prov.expiration().expect("expiration info attached");
    assert_eq!(info.status(), &ExpirationStatus::Expired);
}

#[test]
fn ptest_with_fail_on_expired_produces_fail_verdict() {
    let inputs = vec!["input".to_string()];

    let end = "2020-01-01T00:00:00Z".to_string();
    let mut spec = MeasureExperiment::builder()
        .service_contract_id("strict")
        .service_contract(|| ())
        .samples(30)
        .inputs(&inputs)
        .trial(|(): &(), input| always_succeed(input))
        .build()
        .run();
    let mut spec_with_expiry = spec.spec().clone();
    spec_with_expiry.expiration = Some(feotest::spec::baseline::ExpirationBlock {
        expires_in_days: 1,
        baseline_end_time: end.clone(),
        expiration_date: iso8601_plus_days(&end, 1).unwrap(),
    });
    // Suppress unused-variable lint.
    let _ = &mut spec;

    let result = ProbabilisticTestBuilder::new("strict", &inputs, always_succeed)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 50,
            confidence: 0.95,
        })
        .threshold_origin(feotest::model::ThresholdOrigin::Empirical)
        .baseline_spec(spec_with_expiry)
        .fail_on_expired_baseline(true)
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Fail);
    assert!(
        record
            .warnings()
            .iter()
            .any(|w| w.code() == "BASELINE_EXPIRED")
    );
}

#[test]
fn ptest_with_no_expiration_block_attaches_no_info() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    // No expires_in_days: the block is omitted.
    MeasureExperiment::builder()
        .service_contract_id("no-block")
        .service_contract(|| ())
        .samples(30)
        .inputs(&inputs)
        .trial(|(): &(), input| always_succeed(input))
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("no-block", &inputs, always_succeed)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 50,
            confidence: 0.95,
        })
        .threshold_origin(feotest::model::ThresholdOrigin::Empirical)
        .spec_resolver(resolver)
        .run();

    let prov = result.verdict_record().spec_provenance().unwrap();
    assert!(
        prov.expiration().is_none(),
        "no expiration block → no info on provenance"
    );
}

#[test]
fn evaluate_at_now_matches_evaluate() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    MeasureExperiment::builder()
        .service_contract_id("matches-now")
        .service_contract(|| ())
        .samples(30)
        .inputs(&inputs)
        .trial(|(): &(), input| always_succeed(input))
        .baseline_dir(dir.path())
        .expires_in_days(30)
        .build()
        .run();

    let raw = std::fs::read_to_string(
        std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
    )
    .unwrap();
    let spec = BaselineSpec::from_yaml(&raw).unwrap();

    let a = expiration::evaluate(&spec);
    let b = expiration::evaluate_at(&spec, SystemTime::now());
    assert_eq!(a.status(), b.status());
}
