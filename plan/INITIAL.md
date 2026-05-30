# Statistics Module Implementation Plan

## Goal

Implement the `statistics` module as the foundational layer of feotest. This module must be provably correct, well-tested, and built on a recognised Rust statistics library — not hand-rolled mathematical primitives.

## Reference: punit-core's Statistics Package

The public API of `org.mavai.punit.statistics` (punit-core) defines the functional requirements. The Rust implementation should deliver equivalent statistical capability using idiomatic Rust patterns.

### punit-core Public API Summary

**Core types:**

| Java Type                          | Role                                                                         |
|------------------------------------|------------------------------------------------------------------------------|
| `OperationalApproach` (enum)       | Three strategies: `SAMPLE_SIZE_FIRST`, `CONFIDENCE_FIRST`, `THRESHOLD_FIRST` |
| `BinomialProportionEstimator`      | Wilson score CI, one-sided lower bound, z-scores, z-test, p-values           |
| `ProportionEstimate` (record)      | Point estimate + Wilson CI + confidence level + derived metrics              |
| `ThresholdDeriver`                 | Derives pass/fail thresholds from baseline data                              |
| `DerivedThreshold` (record)        | Threshold value + approach + derivation context + soundness flag             |
| `DerivationContext` (record)       | Baseline rate, sample sizes, confidence                                      |
| `SampleSizeCalculator`             | Power analysis for confidence-first approach                                 |
| `SampleSizeRequirement` (record)   | Required n + parameters that produced it                                     |
| `TestVerdictEvaluator`             | Evaluates test results against derived threshold                             |
| `VerdictWithConfidence` (record)   | Pass/fail + observed rate + threshold + false positive probability           |
| `StatisticalDefaults`              | Default confidence (0.95) and alpha (0.05)                                   |
| `ComplianceEvidenceEvaluator`      | Checks if sample size is sufficient for compliance-grade evidence            |
| `VerificationFeasibilityEvaluator` | Pre-flight feasibility check for configured sample sizes                     |

**Key methods on `BinomialProportionEstimator`:**
- `standardError(successes, trials)` → SE(p̂)
- `estimate(successes, trials, confidenceLevel)` → two-sided Wilson score CI
- `lowerBound(successes, trials, confidenceLevel)` → one-sided Wilson lower bound
- `zScoreOneSided(confidenceLevel)` → Φ⁻¹(1 − α)
- `zScoreTwoSided(confidenceLevel)` → Φ⁻¹(1 − α/2)
- `zTestStatistic(observedRate, hypothesizedRate, sampleSize)` → one-sided z-test
- `oneSidedPValue(z)` → lower-tail probability

**Key methods on `ThresholdDeriver`:**
- `deriveSampleSizeFirst(baselineSamples, baselineSuccesses, testSamples, confidence)` → threshold from Wilson lower bound
- `deriveThresholdFirst(baselineSamples, baselineSuccesses, testSamples, explicitThreshold)` → implied confidence via binary search

**Key methods on `SampleSizeCalculator`:**
- `calculateForPower(baselineRate, minDetectableEffect, confidence, power)` → required n
- `calculateAchievedPower(sampleSize, baselineRate, minDetectableEffect, confidence)` → achieved power

**Key methods on `TestVerdictEvaluator`:**
- `evaluate(testSuccesses, testSamples, threshold)` → verdict with confidence
- `summarizeMultipleRuns(verdicts...)` → multi-run summary with combined false positive probability

**Not in scope for initial implementation:**
- `LatencyDistribution` and `LatencyThresholdDeriver` (README explicitly defers latency)
- `transparent` sub-package (rendering/display concerns belong in `reporting`)

---

## Rust Foundation Library

### Recommendation: `statrs`

**Why `statrs`:**
- Most mature and widely-used Rust statistics crate (~4M downloads)
- Provides `Normal` distribution with CDF and inverse CDF (quantile function) — the two primitives punit uses from Apache Commons Statistics
- Pure Rust, no unsafe, no C bindings
- Covers: normal distribution, beta distribution, continuous/discrete distribution traits
- Well-tested against known reference values

**What we need from it:**
- `statrs::distribution::Normal` — standard normal distribution
- `Normal::inverse_cdf(p)` — quantile function (Φ⁻¹), used for z-scores
- `Normal::cdf(x)` — cumulative distribution function (Φ), used for p-values and power

**What we build ourselves (on top of `statrs`):**
- Wilson score interval computation
- Threshold derivation logic
- Power analysis / sample size calculation
- Verdict evaluation

This mirrors punit's relationship with Apache Commons Statistics: the library provides the distribution primitives, we build the domain-specific inference layer.

---

## Implementation Steps

### Step 1: Add dependency and establish module structure

- Add `statrs` to `Cargo.toml`
- Create sub-modules within `src/statistics/`:
  - `proportion.rs` — `BinomialProportionEstimator` equivalent
  - `threshold.rs` — `ThresholdDeriver` equivalent
  - `sample_size.rs` — `SampleSizeCalculator` equivalent
  - `evaluator.rs` — `TestVerdictEvaluator` equivalent
  - `types.rs` — shared types (`OperationalApproach`, `ProportionEstimate`, `DerivedThreshold`, etc.)
  - `defaults.rs` — `StatisticalDefaults` equivalent
  - `feasibility.rs` — `ComplianceEvidenceEvaluator` + `VerificationFeasibilityEvaluator` equivalents

### Step 2: Define types (`types.rs`, `defaults.rs`)

Types to implement (all as structs with named fields, not tuples):

```
OperationalApproach         — enum with three variants
ProportionEstimate          — point estimate + Wilson CI bounds + confidence + sample size
DerivedThreshold            — threshold value + approach + context + soundness flag
DerivationContext           — baseline rate, baseline samples, test samples, confidence
SampleSizeRequirement       — required n + all input parameters
VerdictWithConfidence        — pass/fail + observed rate + threshold + false positive prob
FeasibilityResult           — feasible flag + minimum samples + configured params
```

Design notes:
- Use newtypes where they prevent misuse (e.g., `ConfidenceLevel` wrapping f64 in (0,1))
- Derive `Debug`, `Clone`, `PartialEq` on all types
- Validate in constructors — return `Result` rather than panicking
- Consider a dedicated error enum for the statistics module

### Step 3: Implement `BinomialProportionEstimator` equivalent (`proportion.rs`)

Core functions (may be free functions or methods on a zero-sized struct):

1. `standard_error(successes, trials)` → f64
2. `estimate(successes, trials, confidence)` → `Result<ProportionEstimate>`
   - Two-sided Wilson score interval
3. `lower_bound(successes, trials, confidence)` → `Result<f64>`
   - One-sided Wilson lower bound (critical for threshold derivation)
4. `z_score_one_sided(confidence)` → f64
   - Uses `Normal::inverse_cdf`
5. `z_score_two_sided(confidence)` → f64
   - Uses `Normal::inverse_cdf`
6. `z_test_statistic(observed, hypothesized, sample_size)` → f64
7. `one_sided_p_value(z)` → f64
   - Uses `Normal::cdf`

Wilson score formula:
```
center = (p̂ + z²/(2n)) / (1 + z²/n)
margin = z × √(p̂(1−p̂)/n + z²/(4n²)) / (1 + z²/n)
lower  = center − margin
upper  = center + margin
```

### Step 4: Implement `ThresholdDeriver` equivalent (`threshold.rs`)

1. `derive_sample_size_first(baseline_successes, baseline_samples, test_samples, confidence)` → `Result<DerivedThreshold>`
   - Computes threshold as Wilson one-sided lower bound from baseline
2. `derive_threshold_first(baseline_successes, baseline_samples, test_samples, explicit_threshold)` → `Result<DerivedThreshold>`
   - Given explicit threshold, finds implied confidence via binary search
   - Flags as unsound if implied confidence < 0.80

### Step 5: Implement `SampleSizeCalculator` equivalent (`sample_size.rs`)

1. `calculate_for_power(baseline_rate, min_detectable_effect, confidence, power)` → `Result<SampleSizeRequirement>`
   - Formula: n = ((z_α × σ₀ + z_β × σ₁) / δ)², rounded up
2. `calculate_achieved_power(sample_size, baseline_rate, min_detectable_effect, confidence)` → `Result<f64>`
   - Inverse: given n, compute achieved power

### Step 6: Implement `TestVerdictEvaluator` equivalent (`evaluator.rs`)

1. `evaluate(test_successes, test_samples, threshold)` → `Result<VerdictWithConfidence>`
   - Pass if observed_rate >= threshold
   - Compute false positive probability
2. `summarize_multiple_runs(verdicts)` → summary with combined false positive probability

### Step 7: Implement feasibility checks (`feasibility.rs`)

1. `is_undersized(samples, target, alpha)` → bool
   - Wilson lower bound for perfect observation (k=n) < target
2. `feasibility_check(samples, target, confidence)` → `FeasibilityResult`
   - Pre-flight: can configured sample size produce verification-grade evidence?

### Step 8: Testing strategy

Each step above must include comprehensive unit tests:

- **Wilson score CI**: validate against known analytical results and published tables
- **Z-scores**: validate against standard normal tables (e.g., z₀.₉₅ = 1.6449, z₀.₉₇₅ = 1.9600)
- **Edge cases**: p̂ = 0, p̂ = 1, n = 1, very large n, confidence near 0 and 1
- **Threshold derivation**: validate with worked examples
- **Power analysis**: validate against published power tables or R's `power.prop.test`
- **Round-trip properties**: derive threshold then evaluate with known data; derive sample size then check achieved power meets target
- **Floating-point**: use `approx` crate (or manual epsilon comparison) for f64 assertions

Consider adding `approx` as a dev-dependency for floating-point test assertions.

---

## Decisions to Confirm Before Implementation

1. **Free functions vs struct methods**: punit uses stateless class instances (e.g., `new BinomialProportionEstimator()`). In Rust, free functions in a module are more idiomatic when there's no state. Recommendation: use free functions, grouped by module.

2. **Error handling**: define a `StatisticsError` enum with variants for invalid inputs (confidence not in (0,1), zero trials, etc.). Use `thiserror` once added as a dependency, or hand-implement `std::error::Error` initially.

3. **Newtype wrappers**: should we wrap `ConfidenceLevel`, `Proportion`, `SampleSize` as newtypes for compile-time safety? Recommendation: yes for `ConfidenceLevel` at minimum, as mixing up confidence level and proportion is a plausible bug.

4. **Visibility**: default to `pub(crate)` for everything. Only promote to `pub` once the API stabilises and we know what the `verdict` and `experiment` modules need.

---

## Architectural Enforcement Strategy

Preventing unwanted inter-module dependencies is a first-class concern for this project. The strategy is layered: compiler-enforced boundaries first, automated checks second, heavier tooling only if needed.

### Layer 1: Visibility discipline (compile-time, zero cost)

Rust's visibility system is the primary enforcement mechanism. The intended dependency direction is:

```
statistics  ← model, verdict, spec, contract, controls
model       ← verdict, spec, experiment
verdict     ← reporting, experiment
spec        ← experiment
contract    ← experiment
controls    ← experiment
experiment  ← (top-level / runner)
reporting   ← (top-level / runner)
usecase     ← experiment, spec, contract
```

Rules:
- All items default to `pub(crate)` or private. Only promote to `pub` when the public API requires it.
- Use `pub(in crate::statistics)` within the `statistics` module to keep sub-module internals hidden from the rest of the crate.
- The `statistics` module must have **zero dependencies** on any other feotest module. It depends only on `statrs` and `std`. This is the most critical constraint.

### Layer 2: Dependency-direction tests (CI, lightweight)

Write architecture tests that parse `use` statements and assert dependency rules. Two options:

**Option A — `cargo-modules` in CI:**
- `cargo modules dependencies --acyclic` detects circular dependencies.
- Free, maintained, works on stable Rust.
- Limitation: detects cycles but not arbitrary "A must not depend on B" rules.

**Option B — `arch_test_core` as a dev-dependency:**
- Write Rust tests that assert module dependency rules directly:
  ```rust
  #[test]
  fn statistics_has_no_internal_dependencies() {
      // Assert that src/statistics/ contains no `use crate::model`,
      // `use crate::verdict`, etc.
  }
  ```
- `arch_test_core` is unmaintained (last update 2021) but functional on stable Rust. Low risk as a dev-dependency.
- Alternatively, write these assertions as simple `grep`-based tests without the external dependency — scan source files for prohibited `use` paths.

**Recommendation:** Start with Option A (`cargo-modules --acyclic` in CI) plus hand-written `grep`-style tests for the critical constraint that `statistics` has no intra-crate dependencies. Adopt `arch_test_core` or `cargo-pup` later if richer rules are needed.

### Layer 3: Workspace crate splitting (if the project grows)

If feotest grows beyond a single crate, split along module boundaries:
- `feotest-stats` — the statistics engine (zero intra-workspace dependencies)
- `feotest-core` — model, verdict, spec, contract, controls
- `feotest` — experiment, reporting, usecase, runner integration

Cargo enforces that a crate cannot import from another crate unless it is declared as a dependency. This gives compiler-enforced layering that is impossible to circumvent. This is the nuclear option — only needed if the single-crate visibility approach proves insufficient.

### What to implement now

For the statistics module specifically:
1. All `statistics` sub-modules use `pub(in crate::statistics)` for internal items.
2. Add a CI step: `cargo modules dependencies --acyclic`.
3. Add a unit test that scans `src/statistics/**/*.rs` for any `use crate::` imports outside of `crate::statistics` and fails if found.

---

## Out of Scope for This Plan

- Latency distribution and latency threshold derivation (deferred per README)
- Transparent statistics rendering (belongs in `reporting` module)
- Integration with other modules (`model`, `verdict`, `experiment`)
- Runner integration or proc-macros
