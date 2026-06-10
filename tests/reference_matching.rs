//! Reference-matching (instance conformance): a contract backed by a golden
//! dataset judges each sample's output against a per-sample known-correct value
//! surfaced through `ServiceContract::expected`, end-to-end through an engine.

use feotest::controls::Cost;
use feotest::criteria::{Criteria, Criterion};
use feotest::model::{ContractViolation, Defect};
use feotest::ptest::ProbabilisticTest;
use feotest::service_contract::ServiceContract;

/// One golden case: the request text paired with its known-correct answer.
struct TranslationCase {
    text: String,
    reference: String,
}

impl TranslationCase {
    fn new(text: &str, reference: &str) -> Self {
        Self {
            text: text.to_string(),
            reference: reference.to_string(),
        }
    }
}

/// A translator whose "translation" is simply the request text echoed back. A
/// case whose `reference` equals its `text` therefore matches; one whose
/// reference differs does not — a deterministic per-case pass/fail split.
struct EchoTranslator;

impl ServiceContract for EchoTranslator {
    type Input = TranslationCase;
    type Output = String;

    fn id(&self) -> &'static str {
        "translate.echo"
    }

    fn invoke(&self, input: &TranslationCase, cost: &mut Cost) -> Result<String, Defect> {
        cost.record_tokens(1);
        Ok(input.text.clone())
    }

    fn expected(&self, input: &TranslationCase) -> Option<String> {
        Some(input.reference.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("matches-reference")
            .matching_equality()
            .build()])
    }
}

/// Seven of ten cases have a reference equal to their text (the echo matches);
/// three differ. With one sample per input the matching criterion's pass rate
/// is exactly 7/10.
fn golden_dataset() -> Vec<TranslationCase> {
    vec![
        TranslationCase::new("a", "a"),
        TranslationCase::new("b", "b"),
        TranslationCase::new("c", "c"),
        TranslationCase::new("d", "d"),
        TranslationCase::new("e", "e"),
        TranslationCase::new("f", "f"),
        TranslationCase::new("g", "g"),
        TranslationCase::new("h", "WRONG"),
        TranslationCase::new("i", "WRONG"),
        TranslationCase::new("j", "WRONG"),
    ]
}

#[test]
fn golden_dataset_run_computes_the_matching_pass_rate() {
    let inputs = golden_dataset();

    let result = ProbabilisticTest::for_contract(EchoTranslator)
        .inputs(&inputs)
        .samples(10)
        .run();

    let rows = result.verdict_record().functional_assessment().criteria();
    assert_eq!(rows.len(), 1);
    let matching = &rows[0];
    assert_eq!(matching.name(), "matches-reference");

    // Per-sample expected values vary by input; seven echo their reference and
    // three do not, so the criterion sees exactly seven passes and three fails.
    assert_eq!(matching.total(), 10);
    assert_eq!(matching.pass(), 7);
    assert_eq!(matching.fail(), 3);

    // Every mismatch is attributed to the equality matcher's stable check name.
    assert_eq!(
        matching.failure_distribution(),
        [("not-equal".to_string(), 3)]
    );
}

/// A contract that declares a reference-matching criterion but supplies no
/// ground truth: `expected` falls through to the `None` default. Running it is
/// a defect — the contract promised reference-matching and failed to deliver a
/// reference — so the run aborts rather than counting a sample failure.
struct NoGroundTruth;

impl ServiceContract for NoGroundTruth {
    type Input = String;
    type Output = String;

    fn id(&self) -> &'static str {
        "no-ground-truth"
    }

    fn invoke(&self, input: &String, _cost: &mut Cost) -> Result<String, Defect> {
        Ok(input.clone())
    }

    // `expected` is left at its `None` default.

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("matches-reference")
            .matching_equality()
            .build()])
    }
}

#[test]
#[should_panic(expected = "matches-reference")]
fn matching_without_ground_truth_aborts_the_run() {
    let inputs = vec!["x".to_string()];
    let _ = ProbabilisticTest::for_contract(NoGroundTruth)
        .inputs(&inputs)
        .samples(5)
        .run();
}

/// A custom matcher carries its own equivalence notion and failure name all the
/// way into the per-criterion failure distribution.
struct CaseInsensitiveTranslator;

impl ServiceContract for CaseInsensitiveTranslator {
    type Input = TranslationCase;
    type Output = String;

    fn id(&self) -> &'static str {
        "translate.case-insensitive"
    }

    fn invoke(&self, input: &TranslationCase, _cost: &mut Cost) -> Result<String, Defect> {
        Ok(input.text.clone())
    }

    fn expected(&self, input: &TranslationCase) -> Option<String> {
        Some(input.reference.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("matches-reference")
            .matching(|expected: &String, actual: &String| {
                if expected.eq_ignore_ascii_case(actual) {
                    Ok(())
                } else {
                    Err(ContractViolation::new(
                        "case-insensitive-mismatch",
                        "differs beyond letter case",
                    ))
                }
            })
            .build()])
    }
}

#[test]
fn custom_matcher_failure_name_reaches_the_distribution() {
    // "A" echoes "A" and matches "a" case-insensitively; "B" echoes "B" and
    // does not match "WRONG". One pass, one fail under the custom matcher.
    let inputs = vec![
        TranslationCase::new("A", "a"),
        TranslationCase::new("B", "WRONG"),
    ];

    let result = ProbabilisticTest::for_contract(CaseInsensitiveTranslator)
        .inputs(&inputs)
        .samples(2)
        .run();

    let rows = result.verdict_record().functional_assessment().criteria();
    let matching = &rows[0];
    assert_eq!(matching.pass(), 1);
    assert_eq!(matching.fail(), 1);
    assert_eq!(
        matching.failure_distribution(),
        [("case-insensitive-mismatch".to_string(), 1)]
    );
}
