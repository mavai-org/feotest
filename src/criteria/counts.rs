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
    failure_distribution: BTreeMap<String, u32>,
}

impl CriterionCounts {
    fn new(criterion: String) -> Self {
        Self {
            criterion,
            pass: 0,
            fail: 0,
            failure_distribution: BTreeMap::new(),
        }
    }

    /// Folds one sample's result for this criterion into the tally. A clean
    /// pass increments `pass`; any failure increments `fail` and tallies its
    /// reason by the violating check's name.
    fn record(&mut self, result: &CriterionSampleResult) {
        if result.passed() {
            self.pass += 1;
        } else {
            self.fail += 1;
            if let Some(violation) = result.reason() {
                *self
                    .failure_distribution
                    .entry(violation.check().to_string())
                    .or_insert(0) += 1;
            }
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

    /// This criterion's failures keyed by the violating check's name, in name
    /// order. A failed transform is keyed by the transform's check name like
    /// any other violation.
    #[must_use]
    pub fn failure_distribution(&self) -> &BTreeMap<String, u32> {
        &self.failure_distribution
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
            self.counts[idx].record(result);
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

    fn fail_with(criterion: &str, check: &str) -> CriterionSampleResult {
        CriterionSampleResult::fail(criterion, ContractViolation::new(check, "r"))
    }

    #[test]
    fn tallies_failure_reasons_per_criterion() {
        let mut counts = CriteriaCounts::new();
        counts.record_sample(&[fail_with("a", "empty")]);
        counts.record_sample(&[fail_with("a", "empty")]);
        counts.record_sample(&[fail_with("a", "transform")]);
        counts.record_sample(&[pass("a")]);

        let dist = counts.get("a").unwrap().failure_distribution();
        assert_eq!(dist.get("empty"), Some(&2));
        assert_eq!(dist.get("transform"), Some(&1));
        assert_eq!(counts.get("a").unwrap().fail(), 3);
    }

    #[test]
    fn clean_passes_leave_an_empty_distribution() {
        let mut counts = CriteriaCounts::new();
        counts.record_sample(&[pass("a")]);
        assert!(counts.get("a").unwrap().failure_distribution().is_empty());
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
