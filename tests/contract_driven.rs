//! Contract-driven probabilistic test: the engine invokes the contract and
//! judges every criterion on every sample, producing a per-criterion composite
//! verdict with latency reported as its own dimension.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use feotest::controls::Cost;
use feotest::criteria::Criteria;
use feotest::latency::{LatencyCriterion, Percentile};
use feotest::model::{ContractViolation, Defect};
use feotest::ptest::ProbabilisticTest;
use feotest::service_contract::ServiceContract;
use feotest::verdict::Verdict;

/// A contract whose response is the raw string the service returned. One
/// criterion checks the response is non-empty; another parses it as a positive
/// integer. A deterministic seed drives reproducible pass/fail mixes.
struct Counter {
    // Every nth sample (1-indexed) returns an empty response.
    empty_every: usize,
    calls: AtomicUsize,
}

impl ServiceContract for Counter {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        "counter"
    }

    fn invoke(&self, input: &String, cost: &mut Cost) -> Result<String, Defect> {
        let n = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        cost.record_tokens(10);
        if self.empty_every != 0 && n % self.empty_every == 0 {
            Ok(String::new())
        } else {
            Ok(input.clone())
        }
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([
            Criteria::meeting()
                .pass_rate(0.80)
                .name("non-empty")
                .satisfies("response not empty", |r: &String| {
                    if r.is_empty() {
                        Err(ContractViolation::new("empty", "no content"))
                    } else {
                        Ok(())
                    }
                })
                .build(),
            Criteria::meeting()
                .pass_rate(0.80)
                .name("parses")
                .transforming(|r: &String| {
                    r.parse::<u32>()
                        .map_err(|_| ContractViolation::new("parse", "not an integer"))
                })
                .satisfies("positive", |n: &u32| {
                    if *n > 0 {
                        Ok(())
                    } else {
                        Err(ContractViolation::new("non-positive", "zero"))
                    }
                })
                .build(),
        ])
    }

    fn latency(&self) -> Option<LatencyCriterion> {
        Some(LatencyCriterion::meeting().at_most(Percentile::P95, Duration::from_secs(5)))
    }
}

#[test]
fn composite_decomposes_per_criterion_with_independent_rates() {
    let inputs: Vec<String> = (1..=20).map(|i| i.to_string()).collect();
    let contract = Counter {
        empty_every: 5, // 20% of responses are empty
        calls: AtomicUsize::new(0),
    };

    let result = ProbabilisticTest::for_contract(contract)
        .inputs(&inputs)
        .samples(20)
        .confidence(0.95)
        .run();

    let record = result.verdict_record();
    let assessment = record.functional_assessment();

    // One row per criterion, in declaration order.
    let rows = assessment.criteria();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name(), "non-empty");
    assert_eq!(rows[1].name(), "parses");

    // "non-empty" fails on the 20% empty responses; "parses" fails on those too
    // (empty string is not a positive integer). Both see all 20 samples — no
    // short-circuit across criteria.
    assert_eq!(rows[0].total(), 20);
    assert_eq!(rows[1].total(), 20);
    assert_eq!(rows[0].fail(), 4);

    // The empty-response failures are attributed to their own check name.
    let non_empty_dist = rows[0].failure_distribution();
    assert_eq!(non_empty_dist, [("empty".to_string(), 4)]);
}

#[test]
fn malformed_response_is_a_counted_failure_not_a_defect() {
    // Every response is empty: invoke still returns Ok (a response came back),
    // and the parsing criterion FAILs it — never a defect/abort.
    let inputs = vec!["x".to_string()];
    let contract = Counter {
        empty_every: 1,
        calls: AtomicUsize::new(0),
    };

    let result = ProbabilisticTest::for_contract(contract)
        .inputs(&inputs)
        .samples(10)
        .run();

    let rows = result.verdict_record().functional_assessment().criteria();
    let parses = rows.iter().find(|r| r.name() == "parses").unwrap();
    // The run completed normally: every malformed response was counted as a
    // failure of the parsing criterion — not a defect that aborts the run.
    assert_eq!(parses.fail(), 10);
    assert_eq!(parses.total(), 10);
    // It certainly does not pass; the parse failures are attributed by reason.
    assert_ne!(result.verdict_record().verdict(), Verdict::Pass);
    assert_eq!(parses.failure_distribution(), [("parse".to_string(), 10)]);
}

#[test]
fn latency_is_reported_as_its_own_dimension() {
    let inputs = vec!["1".to_string()];
    let contract = Counter {
        empty_every: 0, // never empty: every sample passes both criteria
        calls: AtomicUsize::new(0),
    };

    let result = ProbabilisticTest::for_contract(contract)
        .inputs(&inputs)
        .samples(30)
        .run();

    let record = result.verdict_record();
    // Functional criteria all pass; the latency commitment surfaces separately.
    assert_eq!(record.verdict(), Verdict::Pass);
    let latency = record.latency().expect("latency dimension present");
    assert!(latency.passed());
    assert!(record.passed());
}

/// A contract that always panics: the engine catches it and aborts the run.
struct Panicker;

impl ServiceContract for Panicker {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        "panicker"
    }

    fn invoke(&self, _input: &String, _cost: &mut Cost) -> Result<String, Defect> {
        panic!("upstream exploded");
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criteria::meeting()
            .pass_rate(0.9)
            .name("ok")
            .satisfies("ok", |_: &String| Ok(()))
            .build()])
    }
}

#[test]
#[should_panic(expected = "upstream exploded")]
fn a_panicking_invocation_aborts_the_run() {
    let inputs = vec!["x".to_string()];
    let _ = ProbabilisticTest::for_contract(Panicker)
        .inputs(&inputs)
        .samples(5)
        .run();
}
