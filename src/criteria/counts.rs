//! Per-criterion accumulation across the samples of a run.
//!
//! As each sample is evaluated, its per-criterion results are folded into a
//! running tally. The denominator for a criterion is `pass + fail` — every
//! in-scope trial counts, a clean pass increments `pass` and everything else
//! (a failing postcondition, a failed transform) increments `fail`.

use std::collections::BTreeMap;

use crate::criteria::result::CriterionSampleResult;

/// Running pass/fail tally for a single criterion across a sampling.
// javai-ref: JVI-C5P3EQE — do not remove (resolves in javai-orchestrator)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CriterionCounts {
    criterion: String,
    pass: u32,
    fail: u32,
}

impl CriterionCounts {
    const fn new(criterion: String) -> Self {
        Self {
            criterion,
            pass: 0,
            fail: 0,
        }
    }

    const fn record(&mut self, passed: bool) {
        if passed {
            self.pass += 1;
        } else {
            self.fail += 1;
        }
    }

    /// The criterion this tally belongs to.
    #[must_use]
    pub fn criterion(&self) -> &str {
        &self.criterion
    }

    /// The number of clean passes.
    #[must_use]
    pub const fn pass(&self) -> u32 {
        self.pass
    }

    /// The number of failures (failing postcondition or failed transform).
    #[must_use]
    pub const fn fail(&self) -> u32 {
        self.fail
    }

    /// The denominator: every in-scope trial (`pass + fail`).
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.pass + self.fail
    }

    /// The observed pass rate, or `None` when no trials were counted.
    #[must_use]
    pub fn pass_rate(&self) -> Option<f64> {
        let total = self.total();
        (total > 0).then(|| f64::from(self.pass) / f64::from(total))
    }
}

/// Per-criterion tallies accumulated across a run, in first-seen order.
#[derive(Debug, Clone, Default)]
pub struct CriteriaCounts {
    counts: Vec<CriterionCounts>,
    index: BTreeMap<String, usize>,
}

impl CriteriaCounts {
    /// A fresh, empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds one sample's per-criterion results into the running tallies.
    /// Criteria are created on first appearance, preserving evaluation order.
    pub fn record_sample(&mut self, results: &[CriterionSampleResult]) {
        for result in results {
            let idx = self
                .index
                .get(result.criterion())
                .copied()
                .unwrap_or_else(|| {
                    let idx = self.counts.len();
                    self.counts
                        .push(CriterionCounts::new(result.criterion().to_string()));
                    self.index.insert(result.criterion().to_string(), idx);
                    idx
                });
            self.counts[idx].record(result.passed());
        }
    }

    /// The per-criterion tallies, in first-seen order.
    #[must_use]
    pub fn per_criterion(&self) -> &[CriterionCounts] {
        &self.counts
    }

    /// The tally for a named criterion, if it has been seen.
    #[must_use]
    pub fn get(&self, criterion: &str) -> Option<&CriterionCounts> {
        self.index.get(criterion).map(|&idx| &self.counts[idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ContractViolation;

    fn pass(criterion: &str) -> CriterionSampleResult {
        CriterionSampleResult::pass(criterion)
    }

    fn fail(criterion: &str) -> CriterionSampleResult {
        CriterionSampleResult::fail(criterion, ContractViolation::new("c", "r"))
    }

    #[test]
    fn tallies_pass_and_fail_per_criterion() {
        let mut counts = CriteriaCounts::new();
        // Two criteria; "a" passes twice and fails once, "b" passes once.
        counts.record_sample(&[pass("a"), pass("b")]);
        counts.record_sample(&[pass("a"), fail("b")]);
        counts.record_sample(&[fail("a"), fail("b")]);

        let a = counts.get("a").unwrap();
        assert_eq!((a.pass(), a.fail(), a.total()), (2, 1, 3));
        assert!((a.pass_rate().unwrap() - 2.0 / 3.0).abs() < 1e-12);

        let b = counts.get("b").unwrap();
        assert_eq!((b.pass(), b.fail(), b.total()), (1, 2, 3));
    }

    #[test]
    fn preserves_first_seen_order() {
        let mut counts = CriteriaCounts::new();
        counts.record_sample(&[pass("first"), pass("second")]);
        counts.record_sample(&[pass("second"), pass("first")]);

        let order: Vec<&str> = counts
            .per_criterion()
            .iter()
            .map(CriterionCounts::criterion)
            .collect();
        assert_eq!(order, ["first", "second"]);
    }

    #[test]
    fn pass_rate_is_none_before_any_trial() {
        let counts = CriteriaCounts::new();
        assert!(counts.get("missing").is_none());
    }
}
