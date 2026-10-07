//! Latency: empirical percentiles and the two latency decision rules
//! (Statistical Companion §12).
//!
//! Latency is treated non-parametrically throughout: percentiles are read
//! directly off the order statistics of the *successful* latencies — those
//! of the samples that passed every functional criterion (§12.2.1). For a
//! percentile `p` over `n` sorted observations the estimate is the order
//! statistic at rank `⌈p n⌉` (nearest rank), so integer-millisecond samples
//! give integer-millisecond estimates.
//!
//! A latency constraint is decided by the rule for its threshold source:
//!
//! - an **explicit** threshold `τ` by `latency/compliance-exact-binomial`
//!   (§12.3.4): the count of latencies at or below `τ` judged by the exact
//!   one-sided binomial test of compliance with `p_req = p`;
//! - a **baseline-derived** threshold by `latency/precedence` (§12.4): the
//!   smallest baseline rank whose exact no-degradation breach probability
//!   for the test's nearest-rank percentile is at most alpha, the threshold
//!   being the observed baseline latency at that rank.
//!
//! Both decisions are made after the run on the actual number of successful
//! latencies. Before the run the same searches on the *expected* number
//! give warnings and planning figures (§12.5.3), never a verdict. The raw
//! comparison of the observed percentile with a threshold is a labelled
//! figure that decides nothing. A constraint is judged the same way whether
//! the latency dimension is enforced or advisory (§12.6).

use crate::statistics::compliance::{clopper_pearson_lower, minimum_passing_count};
use crate::statistics::decision::Verdict;
use crate::statistics::distributions::binomial_upper_tail;
use crate::statistics::exact::{at_most_alpha, breach_probability_exact};
use crate::statistics::rules::{DecisionRule, TestIntent};

/// Tolerance on the expected-count products of §12.5.3 (a planned size
/// times a passing rate), so a product that is an integer in exact
/// arithmetic is not floored to the integer below it.
const EXPECTED_COUNT_SLACK: f64 = 1e-9;

/// The supported percentile as an integer percentage.
///
/// # Panics
///
/// Panics unless `percentile` is one of 0.50, 0.90, 0.95 and 0.99 — the
/// only levels the precedence rank and the non-degeneracy gate are defined
/// for.
fn percent(percentile: f64) -> u32 {
    let scaled = 100.0 * percentile;
    let rounded = scaled.round();
    let supported = [50.0, 90.0, 95.0, 99.0].contains(&rounded);
    assert!(
        supported && (scaled - rounded).abs() <= 1e-9,
        "latency percentile must be one of p50, p90, p95, p99, got {percentile}"
    );
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the rounded percentage is one of 50, 90, 95, 99"
    )]
    let percentage = rounded as u32;
    percentage
}

/// Computes the nearest-rank empirical percentile.
///
/// Uses the ceiling method: `index = ⌈p n⌉ − 1`, clamped to `[0, n − 1]`.
/// Matches R's `quantile(type = 1)` behaviour.
///
/// # Panics
///
/// Panics if `latencies` is empty or `percentile` is not in `(0, 1]`.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "rank is bounded above by n (usize); percentile in (0, 1]"
)]
// mavai-ref: JVI-CHSD1WP — do not remove (resolves in mavai-orchestrator)
pub fn nearest_rank_percentile(latencies: &[f64], percentile: f64) -> f64 {
    assert!(!latencies.is_empty(), "latencies must not be empty");
    assert!(
        percentile > 0.0 && percentile <= 1.0,
        "percentile must be in (0, 1], got {percentile}"
    );
    let sorted = sorted(latencies);
    let n = sorted.len();
    let raw_index = (percentile * n as f64).ceil() as usize;
    let index = raw_index.saturating_sub(1).min(n - 1);
    sorted[index]
}

/// The latencies in ascending order.
fn sorted(latencies: &[f64]) -> Vec<f64> {
    let mut sorted = latencies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("latencies must not contain NaN"));
    sorted
}

/// Summary statistics for a latency sample.
///
/// Reports sample mean and maximum only. Standard deviation is deliberately
/// omitted: the decision rules are non-parametric and do not use it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencySummary {
    mean: f64,
    max: f64,
}

impl LatencySummary {
    /// Computes summary statistics from a latency sample.
    ///
    /// # Panics
    ///
    /// Panics if `latencies` is empty.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "sample count realistically fits in f64 mantissa"
    )]
    pub fn from_latencies(latencies: &[f64]) -> Self {
        assert!(!latencies.is_empty(), "latencies must not be empty");
        let n = latencies.len() as f64;
        let mean = latencies.iter().sum::<f64>() / n;
        let max = latencies.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Self { mean, max }
    }

    /// Sample arithmetic mean.
    #[must_use]
    pub const fn mean(&self) -> f64 {
        self.mean
    }

    /// Maximum observed latency.
    #[must_use]
    pub const fn max(&self) -> f64 {
        self.max
    }
}

/// The non-degeneracy minimum of a percentile (§12.5.2).
///
/// Below it the empirical percentile is the sample maximum (or minimum):
/// artefacts omit it, and a baseline-derived assertion cannot be decided on
/// it under verification.
///
/// p ≤ 0.50 → 5 (an engineering minimum; the mathematical minimum is 3);
/// p ≤ 0.90 → 10; p ≤ 0.95 → 20; otherwise 100.
#[must_use]
pub const fn min_samples_for(percentile: f64) -> u32 {
    if percentile <= 0.50 {
        5
    } else if percentile <= 0.90 {
        10
    } else if percentile <= 0.95 {
        20
    } else {
        100
    }
}

/// The test's nearest rank `r = ⌈P n_t / 100⌉`, in integer arithmetic.
///
/// # Panics
///
/// Panics if `test_samples` is zero or the percentile is unsupported.
#[must_use]
pub fn nearest_rank(test_samples: u32, percentile: f64) -> u32 {
    assert!(test_samples > 0, "test_samples must be positive");
    (percent(percentile) * test_samples).div_ceil(100)
}

/// The no-degradation breach probability of baseline rank `k` (§12.4.2).
///
/// The probability, for continuous i.i.d. latencies, that fewer than `r` of
/// the `n_t` test latencies fall at or below the baseline's `k`-th order
/// statistic — that the test's nearest-rank percentile exceeds it:
/// `Σ_{j < r} C(n_t, j) B(k + j, m + n_t − j) / B(k, m)` with
/// `m = n_b − k + 1`, summed by the ratio of consecutive terms.
///
/// # Panics
///
/// Panics if `rank` is not in `1..=baseline_trials`, `test_samples` is
/// zero, or the percentile is unsupported.
#[must_use]
pub fn breach_probability(
    baseline_trials: u32,
    rank: u32,
    test_samples: u32,
    percentile: f64,
) -> f64 {
    assert!(
        (1..=baseline_trials).contains(&rank),
        "rank must be in 1..=baseline_trials"
    );
    let test_rank = nearest_rank(test_samples, percentile);
    let k = f64::from(rank);
    let m = f64::from(baseline_trials - rank + 1);
    let n_t = f64::from(test_samples);
    // ln T_0 = ln B(k, m + n_t) − ln B(k, m) = Σ_{i < n_t} ln((m + i) / (k + m + i)).
    let mut log_term: f64 = (0..test_samples)
        .map(|i| (-k / (k + m + f64::from(i))).ln_1p())
        .sum();
    // The terms span hundreds of orders of magnitude, so they are summed in
    // logarithms against the largest.
    let mut log_terms = Vec::with_capacity(test_rank as usize);
    log_terms.push(log_term);
    for j in 0..test_rank.saturating_sub(1) {
        let j = f64::from(j);
        log_term += ((n_t - j) / (j + 1.0) * (k + j) / (m + n_t - j - 1.0)).ln();
        log_terms.push(log_term);
    }
    let largest = log_terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let scaled: f64 = log_terms.iter().map(|t| (t - largest).exp()).sum();
    largest.exp() * scaled
}

/// Whether baseline rank `k` achieves alpha (exact-boundary convention).
fn rank_admits(
    baseline_trials: u32,
    rank: u32,
    test_samples: u32,
    percentile: f64,
    alpha: f64,
) -> bool {
    at_most_alpha(
        breach_probability(baseline_trials, rank, test_samples, percentile),
        alpha,
        || {
            breach_probability_exact(
                baseline_trials,
                rank,
                test_samples,
                nearest_rank(test_samples, percentile),
            )
        },
    )
}

/// The rank of `latency/precedence`: the smallest `k ≤ n_b` with
/// `breach(k) ≤ alpha`; `None` when none achieves it (saturated).
///
/// The breach probability decreases in `k`, so a rank exists exactly when
/// the top rank achieves alpha, and the smallest is found by bisection.
///
/// # Panics
///
/// Panics if a size is zero, `alpha` is not in `(0, 1)`, or the percentile
/// is unsupported.
#[must_use]
pub fn precedence_rank(
    baseline_trials: u32,
    test_samples: u32,
    percentile: f64,
    alpha: f64,
) -> Option<u32> {
    assert!(baseline_trials > 0, "baseline_trials must be positive");
    assert!(
        alpha > 0.0 && alpha < 1.0,
        "alpha must be strictly between 0 and 1, got {alpha}"
    );
    if !rank_admits(
        baseline_trials,
        baseline_trials,
        test_samples,
        percentile,
        alpha,
    ) {
        return None;
    }
    // Invariant: `low` does not admit (or is 0), `high` admits.
    let (mut low, mut high) = (0_u32, baseline_trials);
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if rank_admits(baseline_trials, mid, test_samples, percentile, alpha) {
            high = mid;
        } else {
            low = mid;
        }
    }
    Some(high)
}

/// A baseline-derived latency threshold under `latency/precedence`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrecedenceThreshold {
    rank: Option<u32>,
    threshold: Option<f64>,
    breach_probability: Option<f64>,
    test_rank: u32,
    baseline_latencies: u32,
    baseline_percentile: f64,
}

impl PrecedenceThreshold {
    /// The precedence rank; `None` when saturated.
    #[must_use]
    pub const fn rank(&self) -> Option<u32> {
        self.rank
    }

    /// The baseline latency at the rank — an observed value by
    /// construction; `None` when saturated.
    #[must_use]
    pub const fn threshold(&self) -> Option<f64> {
        self.threshold
    }

    /// No rank achieves alpha for a test of this size: the assertion is
    /// INCONCLUSIVE, and no rank is clamped to manufacture a threshold.
    #[must_use]
    pub const fn saturated(&self) -> bool {
        self.rank.is_none()
    }

    /// The breach probability at the rank; `None` when saturated.
    #[must_use]
    pub const fn breach_probability(&self) -> Option<f64> {
        self.breach_probability
    }

    /// The test's nearest rank `r`.
    #[must_use]
    pub const fn test_rank(&self) -> u32 {
        self.test_rank
    }

    /// The number of baseline latencies.
    #[must_use]
    pub const fn n(&self) -> u32 {
        self.baseline_latencies
    }

    /// The baseline's own nearest-rank percentile, for reporting; never the
    /// threshold.
    #[must_use]
    pub const fn baseline_percentile(&self) -> f64 {
        self.baseline_percentile
    }
}

/// The `latency/precedence` threshold for a test of `test_samples`
/// successful latencies against a baseline's successful latencies.
///
/// # Panics
///
/// Panics if the baseline is empty, `test_samples` is zero, the percentile
/// is unsupported, or `alpha` is not in `(0, 1)`.
#[must_use]
// mavai-ref: JVI-QVNG2SX — do not remove (resolves in mavai-orchestrator)
pub fn derive_precedence_threshold(
    baseline_latencies: &[f64],
    test_samples: u32,
    percentile: f64,
    alpha: f64,
) -> PrecedenceThreshold {
    assert!(
        !baseline_latencies.is_empty(),
        "cannot derive a latency threshold from an empty baseline"
    );
    let ordered = sorted(baseline_latencies);
    let n_b = u32::try_from(ordered.len()).expect("baseline latency count fits in u32");
    let rank = precedence_rank(n_b, test_samples, percentile, alpha);
    PrecedenceThreshold {
        rank,
        threshold: rank.map(|k| ordered[k as usize - 1]),
        breach_probability: rank.map(|k| breach_probability(n_b, k, test_samples, percentile)),
        test_rank: nearest_rank(test_samples, percentile),
        baseline_latencies: n_b,
        baseline_percentile: nearest_rank_percentile(&ordered, percentile),
    }
}

/// `⌊n_planned · p_baseline⌋`: an expectation, not a lower bound (§12.5.3).
#[must_use]
pub fn expected_successful_count(planned_samples: u32, baseline_success_rate: f64) -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a rate in [0, 1] times a u32 size is a non-negative value within u32"
    )]
    let expected = f64::from(planned_samples)
        .mul_add(baseline_success_rate, EXPECTED_COUNT_SLACK)
        .floor() as u32;
    expected
}

/// The pre-run existence check of a baseline-derived assertion (§12.5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrecedencePlanning {
    expected_test_samples: u32,
    planning_rank: Option<u32>,
    minimum_baseline_trials: Option<u32>,
}

impl PrecedencePlanning {
    /// The expected number of successful latencies.
    #[must_use]
    pub const fn expected_test_samples(&self) -> u32 {
        self.expected_test_samples
    }

    /// No rank exists at the expected count: a warning, never a verdict.
    #[must_use]
    pub const fn warning(&self) -> bool {
        self.planning_rank.is_none()
    }

    /// The rank at the expected count; `None` under a warning.
    #[must_use]
    pub const fn planning_rank(&self) -> Option<u32> {
        self.planning_rank
    }

    /// The smallest baseline, no smaller than the expected count, that
    /// supports a rank for it; `None` when no successful latency is
    /// expected at all.
    #[must_use]
    pub const fn minimum_baseline_trials(&self) -> Option<u32> {
        self.minimum_baseline_trials
    }
}

/// The rank search on the expected successful count: a warning and planning
/// figures, never a verdict — saturation is decided after the run.
///
/// # Panics
///
/// Panics if `baseline_trials` is zero, the percentile is unsupported, or
/// `alpha` is not in `(0, 1)`.
#[must_use]
pub fn plan_precedence(
    baseline_trials: u32,
    planned_samples: u32,
    baseline_success_rate: f64,
    percentile: f64,
    alpha: f64,
) -> PrecedencePlanning {
    let expected = expected_successful_count(planned_samples, baseline_success_rate);
    if expected == 0 {
        return PrecedencePlanning {
            expected_test_samples: 0,
            planning_rank: None,
            minimum_baseline_trials: None,
        };
    }
    let rank = precedence_rank(baseline_trials, expected, percentile, alpha);
    let mut minimum = expected;
    while !rank_admits(minimum, minimum, expected, percentile, alpha) {
        minimum += 1;
    }
    PrecedencePlanning {
        expected_test_samples: expected,
        planning_rank: rank,
        minimum_baseline_trials: Some(minimum),
    }
}

/// The pre-run non-degeneracy check (§12.5.3): a warning and a planning
/// figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NondegeneracyPlanning {
    expected_test_samples: u32,
    minimum_contributing_samples: u32,
    planned_samples_needed: u32,
}

impl NondegeneracyPlanning {
    /// The expected number of successful latencies.
    #[must_use]
    pub const fn expected_test_samples(&self) -> u32 {
        self.expected_test_samples
    }

    /// The percentile's minimum (§12.5.2).
    #[must_use]
    pub const fn minimum_contributing_samples(&self) -> u32 {
        self.minimum_contributing_samples
    }

    /// The expected count falls short of the minimum.
    #[must_use]
    pub const fn warning(&self) -> bool {
        self.expected_test_samples < self.minimum_contributing_samples
    }

    /// The smallest planned size whose expected count reaches the minimum.
    #[must_use]
    pub const fn planned_samples_needed(&self) -> u32 {
        self.planned_samples_needed
    }
}

/// The expected successful count against the non-degeneracy minimum.
///
/// # Panics
///
/// Panics if `baseline_success_rate` is outside `(0, 1]` — with no passing
/// baseline sample there is no rate to plan from — or the percentile is
/// unsupported.
#[must_use]
pub fn plan_nondegeneracy(
    percentile: f64,
    planned_samples: u32,
    baseline_success_rate: f64,
) -> NondegeneracyPlanning {
    assert!(
        baseline_success_rate > 0.0 && baseline_success_rate <= 1.0,
        "baseline_success_rate must be in (0, 1], got {baseline_success_rate}"
    );
    percent(percentile);
    let minimum = min_samples_for(percentile);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a minimum of at most 100 over a positive rate fits in u32"
    )]
    let mut needed =
        (f64::from(minimum) / baseline_success_rate - EXPECTED_COUNT_SLACK).ceil() as u32;
    while expected_successful_count(needed, baseline_success_rate) < minimum {
        needed += 1;
    }
    NondegeneracyPlanning {
        expected_test_samples: expected_successful_count(planned_samples, baseline_success_rate),
        minimum_contributing_samples: minimum,
        planned_samples_needed: needed,
    }
}

/// Where a latency threshold comes from (§12.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdSource {
    /// A threshold the contract declares.
    Explicit,
    /// A threshold derived from a baseline's latencies.
    BaselineDerived,
}

impl ThresholdSource {
    /// The source's name, as reports state it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::BaselineDerived => "baseline-derived",
        }
    }
}

/// The post-run non-degeneracy decision (§12.5.2, §12.5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NondegeneracyOutcome {
    /// The gate does not apply, or the percentile is not degenerate: the
    /// assertion is decided by its rule.
    Decided,
    /// A baseline-derived assertion under verification with too few
    /// successful latencies.
    Inconclusive,
    /// Too few successful latencies under smoke intent: evaluated, and
    /// marked as a directional signal only.
    Indicative,
}

impl NondegeneracyOutcome {
    /// The outcome's name, as reports state it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Decided => "DECIDED",
            Self::Inconclusive => "INCONCLUSIVE",
            Self::Indicative => "INDICATIVE",
        }
    }
}

/// Whether the non-degeneracy gate applies, whether the percentile is
/// degenerate, and the outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NondegeneracyDecision {
    applies: bool,
    degenerate: bool,
    outcome: NondegeneracyOutcome,
}

impl NondegeneracyDecision {
    /// Whether the gate applies to this assertion.
    #[must_use]
    pub const fn applies(&self) -> bool {
        self.applies
    }

    /// Whether the successful count is below the percentile's minimum.
    #[must_use]
    pub const fn degenerate(&self) -> bool {
        self.degenerate
    }

    /// The decision.
    #[must_use]
    pub const fn outcome(&self) -> NondegeneracyOutcome {
        self.outcome
    }
}

/// The non-degeneracy decision on the actual count of successful latencies.
///
/// The gate applies where the decision statistic is the empirical
/// percentile — a baseline-derived assertion — and not to an explicit
/// requirement, which decides on the within-threshold count and has its own
/// feasibility condition. Whether the latency dimension is enforced or
/// advisory does not enter (§12.6): an advisory assertion is gated exactly
/// as an enforced one.
///
/// # Panics
///
/// Panics if the percentile is unsupported.
#[must_use]
pub fn decide_nondegeneracy(
    percentile: f64,
    test_samples: u32,
    intent: TestIntent,
    source: ThresholdSource,
) -> NondegeneracyDecision {
    percent(percentile);
    let applies = source == ThresholdSource::BaselineDerived;
    let degenerate = test_samples < min_samples_for(percentile);
    let outcome = if !applies || !degenerate {
        NondegeneracyOutcome::Decided
    } else if intent == TestIntent::Verification {
        NondegeneracyOutcome::Inconclusive
    } else {
        NondegeneracyOutcome::Indicative
    };
    NondegeneracyDecision {
        applies,
        degenerate,
        outcome,
    }
}

/// An explicit latency requirement decided by
/// `latency/compliance-exact-binomial`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyCompliance {
    test_samples: u32,
    within_threshold: u32,
    minimum_within: Option<u32>,
    verdict: Verdict,
    false_compliance: Option<f64>,
    clopper_pearson_lower: Option<f64>,
    observed_percentile_ms: Option<f64>,
}

impl LatencyCompliance {
    /// The number of successful latencies `n_s`.
    #[must_use]
    pub const fn test_samples(&self) -> u32 {
        self.test_samples
    }

    /// `Y`, the latencies at or below the threshold (a latency equal to it
    /// counts as within).
    #[must_use]
    pub const fn within_threshold(&self) -> u32 {
        self.within_threshold
    }

    /// `y_min`, the smallest count demonstrating compliance; `None` when no
    /// count can pass at `n_s`.
    #[must_use]
    pub const fn minimum_within(&self) -> Option<u32> {
        self.minimum_within
    }

    /// Whether any count of the successful latencies could pass.
    #[must_use]
    pub const fn pass_possible(&self) -> bool {
        self.minimum_within.is_some()
    }

    /// PASS iff `Y ≥ y_min`; INCONCLUSIVE when no count can pass — too few
    /// successful latencies to decide.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// `P_p(Y ≥ y_min)`, the false-compliance probability at the boundary;
    /// `None` when no count can pass.
    #[must_use]
    pub const fn false_compliance(&self) -> Option<f64> {
        self.false_compliance
    }

    /// The one-sided lower bound on `F(τ)`, reported beside the verdict;
    /// `None` with no latencies.
    #[must_use]
    pub const fn clopper_pearson_lower(&self) -> Option<f64> {
        self.clopper_pearson_lower
    }

    /// The raw nearest-rank percentile — a labelled figure that decides
    /// nothing; `None` with no latencies.
    #[must_use]
    pub const fn observed_percentile_ms(&self) -> Option<f64> {
        self.observed_percentile_ms
    }

    /// The raw percentile comparison `Q(p) ≤ τ`; it decides nothing,
    /// whether the requirement is enforced or advisory.
    #[must_use]
    pub fn raw_percentile_pass(&self, threshold_ms: f64) -> Option<bool> {
        self.observed_percentile_ms
            .map(|observed| observed <= threshold_ms)
    }
}

/// Decides an explicit latency requirement on the successful latencies.
///
/// # Panics
///
/// Panics if the percentile is unsupported or `alpha` is not in `(0, 1)`.
#[must_use]
pub fn evaluate_latency_compliance(
    latencies: &[f64],
    threshold_ms: f64,
    percentile: f64,
    alpha: f64,
) -> LatencyCompliance {
    percent(percentile);
    assert!(
        alpha > 0.0 && alpha < 1.0,
        "alpha must be strictly between 0 and 1, got {alpha}"
    );
    let n_s = u32::try_from(latencies.len()).expect("latency count fits in u32");
    let within = u32::try_from(latencies.iter().filter(|&&l| l <= threshold_ms).count())
        .expect("within count fits in u32");
    let minimum_within = (n_s > 0)
        .then(|| minimum_passing_count(percentile, n_s, alpha))
        .flatten();
    let verdict = match minimum_within {
        None => Verdict::Inconclusive,
        Some(y_min) if within >= y_min => Verdict::Pass,
        Some(_) => Verdict::Fail,
    };
    LatencyCompliance {
        test_samples: n_s,
        within_threshold: within,
        minimum_within,
        verdict,
        false_compliance: minimum_within
            .map(|y_min| binomial_upper_tail(i64::from(y_min), n_s, percentile)),
        clopper_pearson_lower: (n_s > 0).then(|| clopper_pearson_lower(within, n_s, alpha)),
        observed_percentile_ms: (n_s > 0).then(|| nearest_rank_percentile(latencies, percentile)),
    }
}

/// One latency constraint judged on a run's successful latencies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyJudgement {
    source: ThresholdSource,
    percentile: f64,
    alpha: f64,
    successful_latencies: u32,
    observed_ms: Option<f64>,
    threshold_ms: Option<f64>,
    verdict: Verdict,
    compliance: Option<LatencyCompliance>,
    precedence: Option<PrecedenceThreshold>,
    nondegeneracy: Option<NondegeneracyDecision>,
}

impl LatencyJudgement {
    /// The threshold's source.
    #[must_use]
    pub const fn source(&self) -> ThresholdSource {
        self.source
    }

    /// The percentile level.
    #[must_use]
    pub const fn percentile(&self) -> f64 {
        self.percentile
    }

    /// The constraint's one-sided level.
    #[must_use]
    pub const fn alpha(&self) -> f64 {
        self.alpha
    }

    /// `n_s`, the latencies judged.
    #[must_use]
    pub const fn successful_latencies(&self) -> u32 {
        self.successful_latencies
    }

    /// The test's raw nearest-rank percentile; `None` with no latencies.
    #[must_use]
    pub const fn observed_ms(&self) -> Option<f64> {
        self.observed_ms
    }

    /// The explicit threshold, or the derived one (`None` when saturated or
    /// underived).
    #[must_use]
    pub const fn threshold_ms(&self) -> Option<f64> {
        self.threshold_ms
    }

    /// The constraint's verdict under its rule.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// The exact-binomial decision of an explicit requirement.
    #[must_use]
    pub const fn compliance(&self) -> Option<&LatencyCompliance> {
        self.compliance.as_ref()
    }

    /// The precedence derivation of a baseline-derived threshold, when one
    /// was attempted.
    #[must_use]
    pub const fn precedence(&self) -> Option<&PrecedenceThreshold> {
        self.precedence.as_ref()
    }

    /// The non-degeneracy decision, where the gate applies.
    #[must_use]
    pub const fn nondegeneracy(&self) -> Option<&NondegeneracyDecision> {
        self.nondegeneracy.as_ref()
    }

    /// The rule that decided the constraint, by its threshold source.
    #[must_use]
    pub const fn rule(&self) -> DecisionRule {
        match self.source {
            ThresholdSource::Explicit => DecisionRule::LatencyComplianceExactBinomial,
            ThresholdSource::BaselineDerived => DecisionRule::LatencyPrecedence,
        }
    }

    /// Evaluated below the non-degeneracy minimum: a directional signal
    /// only.
    #[must_use]
    pub fn indicative(&self) -> bool {
        self.nondegeneracy
            .is_some_and(|d| d.outcome() == NondegeneracyOutcome::Indicative)
    }
}

/// The threshold a latency constraint is judged against.
#[derive(Debug, Clone, Copy)]
pub enum ConstraintThreshold<'a> {
    /// An explicit threshold in milliseconds.
    Explicit(f64),
    /// A threshold derived from these baseline successful latencies.
    BaselineDerived(&'a [f64]),
}

/// One latency constraint as declared: its percentile, level and
/// threshold.
#[derive(Debug, Clone, Copy)]
pub struct LatencyConstraint<'a> {
    /// The percentile level (0.50, 0.90, 0.95 or 0.99).
    pub percentile: f64,
    /// The one-sided level.
    pub alpha: f64,
    /// The threshold and where it comes from.
    pub threshold: ConstraintThreshold<'a>,
}

/// Judges one latency constraint on the run's successful latencies by the
/// rule for its threshold source.
///
/// An explicit requirement is decided by
/// `latency/compliance-exact-binomial`; a baseline-derived constraint by the
/// non-degeneracy gate and `latency/precedence` (a test percentile equal to
/// the threshold is not a breach). The judgement is the same whether the
/// latency dimension is enforced or advisory (§12.6); the mode decides only
/// whether it binds.
///
/// A baseline-derived constraint with no baseline latencies has no
/// threshold and is INCONCLUSIVE.
///
/// # Panics
///
/// Panics if the percentile is unsupported or `alpha` is not in `(0, 1)`.
#[must_use]
pub fn judge_latency_constraint(
    latencies: &[f64],
    constraint: &LatencyConstraint<'_>,
    intent: TestIntent,
) -> LatencyJudgement {
    let LatencyConstraint {
        percentile,
        alpha,
        threshold,
    } = *constraint;
    percent(percentile);
    let n_s = u32::try_from(latencies.len()).expect("latency count fits in u32");
    let observed = (n_s > 0).then(|| nearest_rank_percentile(latencies, percentile));
    match threshold {
        ConstraintThreshold::Explicit(tau) => {
            let compliance = evaluate_latency_compliance(latencies, tau, percentile, alpha);
            LatencyJudgement {
                source: ThresholdSource::Explicit,
                percentile,
                alpha,
                successful_latencies: n_s,
                observed_ms: observed,
                threshold_ms: Some(tau),
                verdict: compliance.verdict(),
                compliance: Some(compliance),
                precedence: None,
                nondegeneracy: None,
            }
        }
        ConstraintThreshold::BaselineDerived(baseline) => {
            judge_baseline_derived(latencies, baseline, percentile, alpha, intent)
        }
    }
}

/// A baseline-derived threshold: the non-degeneracy gate, the precedence
/// rank, and the comparison of the test percentile with the derived
/// threshold.
fn judge_baseline_derived(
    latencies: &[f64],
    baseline: &[f64],
    percentile: f64,
    alpha: f64,
    intent: TestIntent,
) -> LatencyJudgement {
    let n_s = u32::try_from(latencies.len()).expect("latency count fits in u32");
    let observed = (n_s > 0).then(|| nearest_rank_percentile(latencies, percentile));
    let nondegeneracy =
        decide_nondegeneracy(percentile, n_s, intent, ThresholdSource::BaselineDerived);
    let precedence = (n_s > 0 && !baseline.is_empty())
        .then(|| derive_precedence_threshold(baseline, n_s, percentile, alpha));
    let threshold = precedence.and_then(|p| p.threshold());
    let verdict = match (observed, threshold) {
        _ if nondegeneracy.outcome() == NondegeneracyOutcome::Inconclusive => Verdict::Inconclusive,
        (Some(observed), Some(threshold)) if observed <= threshold => Verdict::Pass,
        (Some(_), Some(_)) => Verdict::Fail,
        _ => Verdict::Inconclusive,
    };
    LatencyJudgement {
        source: ThresholdSource::BaselineDerived,
        percentile,
        alpha,
        successful_latencies: n_s,
        observed_ms: observed,
        threshold_ms: threshold,
        verdict,
        compliance: None,
        precedence,
        nondegeneracy: Some(nondegeneracy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn percentile_of_single_value() {
        assert_relative_eq!(nearest_rank_percentile(&[42.0], 0.5), 42.0);
        assert_relative_eq!(nearest_rank_percentile(&[42.0], 0.99), 42.0);
    }

    #[test]
    fn percentile_sorts_unsorted_input() {
        assert_relative_eq!(nearest_rank_percentile(&[300.0, 100.0, 200.0], 0.5), 200.0);
    }

    #[test]
    fn summary_mean_and_max() {
        let s = LatencySummary::from_latencies(&[100.0, 200.0, 300.0, 400.0, 500.0]);
        assert_relative_eq!(s.mean(), 300.0);
        assert_relative_eq!(s.max(), 500.0);
    }

    #[test]
    fn min_samples_matches_the_non_degeneracy_minimums() {
        assert_eq!(min_samples_for(0.50), 5);
        assert_eq!(min_samples_for(0.90), 10);
        assert_eq!(min_samples_for(0.95), 20);
        assert_eq!(min_samples_for(0.99), 100);
    }

    #[test]
    fn nearest_rank_is_integer_ceiling() {
        assert_eq!(nearest_rank(192, 0.95), 183);
        assert_eq!(nearest_rank(100, 0.99), 99);
        assert_eq!(nearest_rank(1, 0.5), 1);
    }

    #[test]
    #[should_panic(expected = "latency percentile must be one of")]
    fn rejects_an_unsupported_percentile() {
        let _ = nearest_rank(100, 0.75);
    }

    #[test]
    fn breach_with_one_test_latency_is_the_share_above_the_rank() {
        // n_t = 1: breach(k) = (n_b - k + 1) / (n_b + 1).
        assert_relative_eq!(
            breach_probability(19, 19, 1, 0.5),
            1.0 / 20.0,
            max_relative = 1e-13
        );
        assert_relative_eq!(
            breach_probability(99, 50, 1, 0.99),
            50.0 / 100.0,
            max_relative = 1e-13
        );
    }

    #[test]
    fn breach_decreases_in_the_rank() {
        let lower = breach_probability(935, 900, 192, 0.95);
        let higher = breach_probability(935, 920, 192, 0.95);
        assert!(higher < lower);
    }

    #[test]
    fn precedence_rank_is_the_smallest_admitted_rank() {
        let rank = precedence_rank(935, 192, 0.95, 0.05).unwrap();
        assert!(breach_probability(935, rank, 192, 0.95) <= 0.05);
        assert!(breach_probability(935, rank - 1, 192, 0.95) > 0.05);
    }

    #[test]
    fn a_small_baseline_saturates() {
        // The slowest of 100 baseline latencies against a test of 15 is
        // exceeded with probability 15 / 115 > 0.05.
        assert_eq!(precedence_rank(100, 15, 0.95, 0.05), None);
        let derived = derive_precedence_threshold(&[1.0; 100], 15, 0.95, 0.05);
        assert!(derived.saturated());
        assert_eq!(derived.threshold(), None);
    }

    #[test]
    fn expected_count_is_floored_with_slack() {
        assert_eq!(expected_successful_count(100, 0.8), 80);
        assert_eq!(expected_successful_count(110, 0.8), 88);
        assert_eq!(expected_successful_count(3, 0.5), 1);
    }

    #[test]
    fn planning_warns_when_no_rank_exists_at_the_expected_count() {
        let planning = plan_precedence(100, 20, 0.75, 0.95, 0.05);
        assert_eq!(planning.expected_test_samples(), 15);
        assert!(planning.warning());
        assert!(planning.minimum_baseline_trials().unwrap() > 100);
    }

    #[test]
    fn planning_with_nothing_expected_has_no_minimum() {
        let planning = plan_precedence(100, 1, 0.5, 0.95, 0.05);
        assert_eq!(planning.expected_test_samples(), 0);
        assert!(planning.warning());
        assert_eq!(planning.minimum_baseline_trials(), None);
    }

    #[test]
    fn non_degeneracy_planning_finds_the_size_needed() {
        let planning = plan_nondegeneracy(0.99, 110, 0.8);
        assert_eq!(planning.expected_test_samples(), 88);
        assert!(planning.warning());
        assert_eq!(planning.planned_samples_needed(), 125);
    }

    #[test]
    fn the_gate_does_not_apply_to_an_explicit_requirement() {
        let decision =
            decide_nondegeneracy(0.5, 4, TestIntent::Verification, ThresholdSource::Explicit);
        assert!(!decision.applies());
        assert_eq!(decision.outcome(), NondegeneracyOutcome::Decided);
    }

    #[test]
    fn a_degenerate_baseline_assertion_is_inconclusive_under_verification() {
        let decision = decide_nondegeneracy(
            0.99,
            99,
            TestIntent::Verification,
            ThresholdSource::BaselineDerived,
        );
        assert!(decision.degenerate());
        assert_eq!(decision.outcome(), NondegeneracyOutcome::Inconclusive);
        let smoke = decide_nondegeneracy(
            0.99,
            99,
            TestIntent::Smoke,
            ThresholdSource::BaselineDerived,
        );
        assert_eq!(smoke.outcome(), NondegeneracyOutcome::Indicative);
    }

    #[test]
    fn latency_compliance_counts_a_tie_as_within() {
        let latencies = vec![100.0; 59];
        let compliance = evaluate_latency_compliance(&latencies, 100.0, 0.95, 0.05);
        assert_eq!(compliance.within_threshold(), 59);
        assert_eq!(compliance.verdict(), Verdict::Pass);
    }

    #[test]
    fn latency_compliance_is_inconclusive_when_no_count_can_pass() {
        let latencies = vec![100.0; 58];
        let compliance = evaluate_latency_compliance(&latencies, 200.0, 0.95, 0.05);
        assert!(!compliance.pass_possible());
        assert_eq!(compliance.verdict(), Verdict::Inconclusive);
        assert_eq!(compliance.false_compliance(), None);
    }

    #[test]
    fn an_explicit_constraint_is_decided_by_the_exact_binomial_rule() {
        let latencies: Vec<f64> = (1..=100).map(f64::from).collect();
        let constraint = LatencyConstraint {
            percentile: 0.95,
            alpha: 0.05,
            threshold: ConstraintThreshold::Explicit(99.0),
        };
        let judged = judge_latency_constraint(&latencies, &constraint, TestIntent::Verification);
        assert_eq!(judged.rule(), DecisionRule::LatencyComplianceExactBinomial);
        assert_eq!(judged.verdict(), Verdict::Pass);
    }

    #[test]
    fn a_breached_explicit_constraint_fails_its_rule() {
        let latencies: Vec<f64> = (1..=100).map(f64::from).collect();
        let constraint = LatencyConstraint {
            percentile: 0.95,
            alpha: 0.05,
            threshold: ConstraintThreshold::Explicit(50.0),
        };
        let judged = judge_latency_constraint(&latencies, &constraint, TestIntent::Verification);
        assert_eq!(judged.rule(), DecisionRule::LatencyComplianceExactBinomial);
        assert_eq!(judged.verdict(), Verdict::Fail);
        assert_eq!(
            judged.compliance().unwrap().raw_percentile_pass(50.0),
            Some(false)
        );
    }

    #[test]
    fn a_saturated_constraint_is_inconclusive() {
        let baseline: Vec<f64> = (1..=100).map(f64::from).collect();
        let test: Vec<f64> = (1..=91).map(f64::from).collect();
        let constraint = LatencyConstraint {
            percentile: 0.99,
            alpha: 0.05,
            threshold: ConstraintThreshold::BaselineDerived(&baseline),
        };
        let judged = judge_latency_constraint(&test, &constraint, TestIntent::Smoke);
        assert!(judged.precedence().unwrap().saturated());
        assert_eq!(judged.threshold_ms(), None);
        assert_eq!(judged.verdict(), Verdict::Inconclusive);
    }

    #[test]
    fn a_baseline_constraint_passes_at_the_threshold() {
        let baseline: Vec<f64> = (1..=1000).map(f64::from).collect();
        let test: Vec<f64> = (1..=100).map(f64::from).collect();
        let constraint = LatencyConstraint {
            percentile: 0.95,
            alpha: 0.05,
            threshold: ConstraintThreshold::BaselineDerived(&baseline),
        };
        let judged = judge_latency_constraint(&test, &constraint, TestIntent::Verification);
        assert_eq!(judged.rule(), DecisionRule::LatencyPrecedence);
        assert_eq!(judged.verdict(), Verdict::Pass);
    }

    #[test]
    fn a_constraint_without_baseline_latencies_is_inconclusive() {
        let test: Vec<f64> = (1..=100).map(f64::from).collect();
        let constraint = LatencyConstraint {
            percentile: 0.95,
            alpha: 0.05,
            threshold: ConstraintThreshold::BaselineDerived(&[]),
        };
        let judged = judge_latency_constraint(&test, &constraint, TestIntent::Verification);
        assert_eq!(judged.precedence(), None);
        assert_eq!(judged.threshold_ms(), None);
        assert_eq!(judged.verdict(), Verdict::Inconclusive);
    }
}
