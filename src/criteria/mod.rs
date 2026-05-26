//! Criterion decomposition: a contract's functional dimension partitioned
//! into named criteria, each judged independently on every sample.
//!
//! A criterion is the unit of judgement. Each carries its own target and its
//! own postconditions, and every criterion is evaluated on every sample — no
//! short-circuit across criteria — so the run yields a pass rate per criterion
//! rather than one aggregate figure. A criterion's per-sample outcome is
//! two-valued ([`CriterionOutcome`]): a clean pass, or a fail carrying the
//! reason. A failed transform is a fail, not a third state.
//!
//! ```
//! use feotest::criteria::{Criteria, Criterion};
//! use feotest::model::ContractViolation;
//!
//! let criteria = Criteria::<String>::of([
//!     Criterion::meeting().pass_rate(0.99)
//!         .name("non-empty")
//!         .satisfies("response not empty", |r: &String| {
//!             if r.is_empty() {
//!                 Err(ContractViolation::new("empty", "no content"))
//!             } else {
//!                 Ok(())
//!             }
//!         })
//!         .build(),
//! ]);
//!
//! let results = criteria.evaluate(&"hello".to_string());
//! assert!(results[0].passed());
//! ```

mod builder;
mod counts;
mod criterion;
mod result;

pub use builder::{CriterionBuild, EmpiricalCriterion, NormativeCriterion, TransformingBuild};
pub use counts::{CriteriaCounts, CriterionCounts};
pub use criterion::{Criterion, CriterionTarget};
pub use result::{CriterionOutcome, CriterionSampleResult};

use std::collections::HashSet;

/// The criteria of a contract: a non-empty, name-unique set of criteria
/// evaluated independently on every sample.
pub struct Criteria<O> {
    criteria: Vec<Criterion<O>>,
}

impl<O: 'static> Criteria<O> {
    /// Assembles the criteria of a contract.
    ///
    /// # Panics
    ///
    /// Panics if there are no criteria, or if two share a name.
    #[must_use]
    pub fn of<const K: usize>(criteria: [Criterion<O>; K]) -> Self {
        assert!(K > 0, "a contract requires at least one criterion");
        let mut seen = HashSet::with_capacity(K);
        for criterion in &criteria {
            assert!(
                seen.insert(criterion.name()),
                "duplicate criterion name '{}': criteria must be uniquely named",
                criterion.name()
            );
        }
        Self {
            criteria: criteria.into(),
        }
    }

    /// Evaluates every criterion against one sample's output, independently,
    /// yielding one result per criterion in declaration order.
    #[must_use]
    pub fn evaluate(&self, output: &O) -> Vec<CriterionSampleResult> {
        self.criteria
            .iter()
            .map(|criterion| criterion.evaluate(output))
            .collect()
    }

    /// The number of criteria.
    #[must_use]
    pub fn len(&self) -> usize {
        self.criteria.len()
    }

    /// Always `false` — a `Criteria` is non-empty by construction.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.criteria.is_empty()
    }

    /// The criteria names, in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.criteria.iter().map(Criterion::name).collect()
    }

    /// Each criterion's name paired with the target it is judged against, in
    /// declaration order. The verdict layer uses these to resolve a per-
    /// criterion threshold.
    #[must_use]
    pub fn targets(&self) -> Vec<(&str, &CriterionTarget)> {
        self.criteria
            .iter()
            .map(|c| (c.name(), c.target()))
            .collect()
    }
}

impl<O> std::fmt::Debug for Criteria<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Criteria")
            .field("criteria", &self.criteria)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ContractViolation;

    #[allow(
        clippy::unnecessary_wraps,
        reason = "must match the satisfies postcondition signature Fn(&O) -> Outcome"
    )]
    fn passes(_: &String) -> crate::model::Outcome {
        Ok(())
    }

    fn fails(check: &'static str) -> impl Fn(&String) -> crate::model::Outcome {
        move |_: &String| Err(ContractViolation::new(check, "forced failure"))
    }

    #[test]
    fn evaluates_every_criterion_without_short_circuit() {
        // The first criterion fails; the second must still be evaluated.
        let criteria = Criteria::<String>::of([
            Criterion::meeting()
                .pass_rate(0.9)
                .name("first")
                .satisfies("a", fails("a"))
                .build(),
            Criterion::meeting()
                .pass_rate(0.9)
                .name("second")
                .satisfies("b", passes)
                .build(),
        ]);

        let results = criteria.evaluate(&"x".to_string());

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].criterion(), "first");
        assert!(!results[0].passed());
        assert_eq!(results[1].criterion(), "second");
        assert!(results[1].passed());
    }

    #[test]
    fn failing_postcondition_yields_fail_with_reason() {
        let criteria = Criteria::<String>::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("c")
            .satisfies("not-empty", fails("not-empty"))
            .build()]);

        let results = criteria.evaluate(&"x".to_string());

        assert_eq!(results[0].outcome(), CriterionOutcome::Fail);
        assert_eq!(results[0].reason().unwrap().check(), "not-empty");
    }

    #[test]
    fn multiple_satisfies_all_must_pass() {
        // A criterion with several postconditions passes only if every clause
        // passes; a later-clause failure surfaces as that clause's reason.
        let criteria = Criteria::<String>::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("c")
            .satisfies("first", passes)
            .satisfies("second", passes)
            .satisfies("third", passes)
            .build()]);
        assert!(criteria.evaluate(&"x".to_string())[0].passed());

        let with_late_failure = Criteria::<String>::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("c")
            .satisfies("first", passes)
            .satisfies("second", passes)
            .satisfies("third", fails("third"))
            .build()]);
        let results = with_late_failure.evaluate(&"x".to_string());
        assert!(!results[0].passed());
        assert_eq!(results[0].reason().unwrap().check(), "third");
    }

    #[test]
    fn clean_pass_carries_no_reason() {
        let criteria = Criteria::<String>::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("c")
            .satisfies("ok", passes)
            .build()]);

        let results = criteria.evaluate(&"x".to_string());

        assert_eq!(results[0].outcome(), CriterionOutcome::Pass);
        assert!(results[0].reason().is_none());
    }

    #[test]
    fn first_failing_postcondition_within_a_criterion_wins() {
        let criteria = Criteria::<String>::of([Criterion::meeting()
            .pass_rate(0.9)
            .name("c")
            .satisfies("first", fails("first"))
            .satisfies("second", fails("second"))
            .build()]);

        let results = criteria.evaluate(&"x".to_string());

        assert_eq!(results[0].reason().unwrap().check(), "first");
    }

    #[test]
    fn failed_transform_is_a_fail_not_a_third_state() {
        // The transform cannot parse the output, so the criterion fails for
        // that sample (counted), carrying the transform's reason — never a
        // panic or an inconclusive per-sample state.
        let criteria = Criteria::<String>::of([Criterion::empirical()
            .pass_rate()
            .transforming(|s: &String| {
                s.parse::<u32>()
                    .map_err(|_| ContractViolation::new("transform", "not an integer"))
            })
            .name("parses")
            .satisfies("positive", |n: &u32| {
                if *n > 0 {
                    Ok(())
                } else {
                    Err(ContractViolation::new("non-positive", "zero"))
                }
            })
            .build()]);

        let failed = criteria.evaluate(&"not-a-number".to_string());
        assert_eq!(failed[0].outcome(), CriterionOutcome::Fail);
        assert_eq!(failed[0].reason().unwrap().check(), "transform");

        let passed = criteria.evaluate(&"42".to_string());
        assert!(passed[0].passed());
    }

    #[test]
    fn transformed_postcondition_judges_the_transformed_value() {
        let criteria = Criteria::<String>::of([Criterion::empirical()
            .pass_rate()
            .transforming(|s: &String| {
                s.parse::<u32>()
                    .map_err(|_| ContractViolation::new("transform", "not an integer"))
            })
            .name("parses")
            .satisfies("positive", |n: &u32| {
                if *n > 0 {
                    Ok(())
                } else {
                    Err(ContractViolation::new("non-positive", "zero"))
                }
            })
            .build()]);

        let results = criteria.evaluate(&"0".to_string());
        assert_eq!(results[0].reason().unwrap().check(), "non-positive");
    }

    #[test]
    fn targets_and_postcondition_names_are_recorded() {
        let normative = Criterion::meeting()
            .pass_rate(0.99)
            .name("n")
            .satisfies("p", passes)
            .build();
        assert_eq!(normative.target(), &CriterionTarget::NormativeRate(0.99));
        assert_eq!(normative.postconditions(), ["p"]);

        let empirical = Criterion::<String>::empirical()
            .pass_rate()
            .name("e")
            .satisfies("p", passes)
            .build();
        assert_eq!(empirical.target(), &CriterionTarget::EmpiricalRate);

        let zero = Criterion::<String>::meeting()
            .zero_failures()
            .name("z")
            .satisfies("p", passes)
            .build();
        assert_eq!(zero.target(), &CriterionTarget::ZeroFailures);
    }

    #[test]
    #[should_panic(expected = "duplicate criterion name 'dup'")]
    fn rejects_duplicate_criterion_names() {
        let _ = Criteria::<String>::of([
            Criterion::meeting()
                .pass_rate(0.9)
                .name("dup")
                .satisfies("a", passes)
                .build(),
            Criterion::meeting()
                .pass_rate(0.9)
                .name("dup")
                .satisfies("b", passes)
                .build(),
        ]);
    }

    #[test]
    #[should_panic(expected = "at least one criterion")]
    fn rejects_empty_criteria() {
        let _ = Criteria::<String>::of([]);
    }

    #[test]
    #[should_panic(expected = "requires a name")]
    fn build_requires_a_name() {
        let _ = Criterion::<String>::meeting()
            .pass_rate(0.9)
            .satisfies("a", passes)
            .build();
    }

    #[test]
    #[should_panic(expected = "pass rate must be in (0, 1)")]
    fn rejects_out_of_range_pass_rate() {
        let _ = Criterion::<String>::meeting().pass_rate(1.5);
    }
}
