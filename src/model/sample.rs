//! Sample aggregate — summary statistics from a collection of trial outcomes.

use std::time::Duration;

use crate::model::outcome::ContractViolation;

/// Aggregate statistics from a collection of trial outcomes.
///
/// This is the bridge between raw trial results and statistical inference.
/// The execution engine builds a `SampleAggregate` from individual trials;
/// the statistics module consumes it.
#[derive(Debug, Clone)]
pub struct SampleAggregate {
    successes: u32,
    failures: u32,
    failure_distribution: Vec<(String, u32)>,
    total_elapsed: Duration,
    example_failures: Vec<ContractViolation>,
    conformance_mismatches: u32,
    example_mismatches: Vec<String>,
}

impl SampleAggregate {
    /// Creates a new empty aggregate.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            successes: 0,
            failures: 0,
            failure_distribution: Vec::new(),
            total_elapsed: Duration::ZERO,
            example_failures: Vec::new(),
            conformance_mismatches: 0,
            example_mismatches: Vec::new(),
        }
    }

    /// Records a successful trial.
    pub fn record_success(&mut self, elapsed: Duration) {
        self.successes += 1;
        self.total_elapsed += elapsed;
    }

    /// Records a failed trial.
    ///
    /// Captures the violation for failure distribution tracking and optionally
    /// stores it as an example failure (up to `max_examples`).
    pub fn record_failure(
        &mut self,
        violation: &ContractViolation,
        elapsed: Duration,
        max_examples: u32,
    ) {
        self.failures += 1;
        self.total_elapsed += elapsed;

        // Update failure distribution
        let check = violation.check().to_string();
        if let Some(entry) = self
            .failure_distribution
            .iter_mut()
            .find(|(k, _)| k == &check)
        {
            entry.1 += 1;
        } else {
            self.failure_distribution.push((check, 1));
        }

        // Store example failure if under limit
        #[allow(clippy::cast_possible_truncation)]
        if (self.example_failures.len() as u32) < max_examples {
            self.example_failures.push(violation.clone());
        }
    }

    /// Total number of trials recorded.
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.successes + self.failures
    }

    /// Number of successful trials.
    #[must_use]
    pub const fn successes(&self) -> u32 {
        self.successes
    }

    /// Number of failed trials.
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// Observed success rate, or 0.0 if no trials recorded.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            f64::from(self.successes) / f64::from(self.total())
        }
    }

    /// Distribution of failures by postcondition check name.
    #[must_use]
    pub fn failure_distribution(&self) -> &[(String, u32)] {
        &self.failure_distribution
    }

    /// Example failures captured for diagnostic purposes.
    #[must_use]
    pub fn example_failures(&self) -> &[ContractViolation] {
        &self.example_failures
    }

    /// Total elapsed time across all trials.
    #[must_use]
    pub const fn total_elapsed(&self) -> Duration {
        self.total_elapsed
    }

    /// Average elapsed time per trial, or zero if no trials recorded.
    #[must_use]
    pub fn avg_elapsed(&self) -> Duration {
        if self.total() == 0 {
            Duration::ZERO
        } else {
            self.total_elapsed / self.total()
        }
    }

    /// Records a conformance mismatch.
    ///
    /// Stores the diff string as an example up to `max_examples`.
    pub fn record_conformance_mismatch(&mut self, diff: &str, max_examples: u32) {
        self.conformance_mismatches += 1;
        #[allow(clippy::cast_possible_truncation)]
        if (self.example_mismatches.len() as u32) < max_examples {
            self.example_mismatches.push(diff.to_owned());
        }
    }

    /// Number of conformance mismatches across all trials.
    #[must_use]
    pub const fn conformance_mismatches(&self) -> u32 {
        self.conformance_mismatches
    }

    /// Example mismatch diff strings captured for diagnostic purposes.
    #[must_use]
    pub fn example_mismatches(&self) -> &[String] {
        &self.example_mismatches
    }
}

impl Default for SampleAggregate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_aggregate_has_zero_totals() {
        let agg = SampleAggregate::new();
        assert_eq!(agg.total(), 0);
        assert_eq!(agg.successes(), 0);
        assert_eq!(agg.failures(), 0);
        assert_eq!(agg.success_rate(), 0.0);
    }

    #[test]
    fn records_successes_and_failures() {
        let mut agg = SampleAggregate::new();
        agg.record_success(Duration::from_millis(10));
        agg.record_success(Duration::from_millis(20));
        let v = ContractViolation::new("parse", "bad json");
        agg.record_failure(&v, Duration::from_millis(5), 5);

        assert_eq!(agg.total(), 3);
        assert_eq!(agg.successes(), 2);
        assert_eq!(agg.failures(), 1);
        assert!((agg.success_rate() - 2.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn tracks_failure_distribution() {
        let mut agg = SampleAggregate::new();
        let v1 = ContractViolation::new("parse", "bad json");
        let v2 = ContractViolation::new("content", "empty");
        agg.record_failure(&v1, Duration::ZERO, 5);
        agg.record_failure(&v1, Duration::ZERO, 5);
        agg.record_failure(&v2, Duration::ZERO, 5);

        let dist = agg.failure_distribution();
        assert_eq!(dist.len(), 2);
        assert_eq!(dist[0], ("parse".to_string(), 2));
        assert_eq!(dist[1], ("content".to_string(), 1));
    }

    #[test]
    fn respects_max_example_failures() {
        let mut agg = SampleAggregate::new();
        let v = ContractViolation::new("check", "reason");
        for _ in 0..10 {
            agg.record_failure(&v, Duration::ZERO, 3);
        }
        assert_eq!(agg.example_failures().len(), 3);
        assert_eq!(agg.failures(), 10);
    }

    #[test]
    fn computes_average_elapsed() {
        let mut agg = SampleAggregate::new();
        agg.record_success(Duration::from_millis(10));
        agg.record_success(Duration::from_millis(30));
        assert_eq!(agg.avg_elapsed(), Duration::from_millis(20));
    }

    #[test]
    fn records_conformance_mismatches() {
        let mut agg = SampleAggregate::new();
        agg.record_conformance_mismatch("expected A, got B", 5);
        agg.record_conformance_mismatch("expected X, got Y", 5);
        assert_eq!(agg.conformance_mismatches(), 2);
        assert_eq!(agg.example_mismatches().len(), 2);
        assert_eq!(agg.example_mismatches()[0], "expected A, got B");
    }

    #[test]
    fn respects_max_example_mismatches() {
        let mut agg = SampleAggregate::new();
        for i in 0..10 {
            agg.record_conformance_mismatch(&format!("diff {i}"), 3);
        }
        assert_eq!(agg.conformance_mismatches(), 10);
        assert_eq!(agg.example_mismatches().len(), 3);
    }

    #[test]
    fn empty_aggregate_has_no_conformance_mismatches() {
        let agg = SampleAggregate::new();
        assert_eq!(agg.conformance_mismatches(), 0);
        assert!(agg.example_mismatches().is_empty());
    }
}
