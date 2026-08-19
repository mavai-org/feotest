# REQ-R-zero-baseline-boundary — Consume the zero-baseline boundary cases and the published sizing refusals

**Source release:** `mavai-R` v0.10.13.
**Triggered by:** `DIR-R-SIZING-boundary-cases` in the orchestrator. Additive release: nine new cases across three suites, one new binding expected field, no existing expected value changed.

> **feotest was run against these fixtures before this document was written, and it is red.** Six deviating cases plus a coverage gap, listed under *feotest's measured status* below. This is the intended outcome of the loop, not a surprise to be triaged — the fixtures were published precisely because nothing held feotest's boundary behaviour in place. `risk_driven_sizing` is in feotest's declared `SCOPE.json`, so the new sizing fields are an obligation, not an optional extra.

## Background

Companion §4.3.2 has prescribed the effective-baseline substitution since it was written, and §5.4.1 defines its baseline symbol as the *effective* rate with an explicit pointer to that guard. What was missing was evidence. No fixture in the family had ever put a **zero** baseline through a threshold, a verdict, or a sizing calculation — so the boundary was covered one layer below where it bites, in `wilson_lower`, and nowhere above it.

Writing those cases exposed a defect in the oracle itself, and the same defect in both frameworks.

At `p̂ = 0` the Wilson lower bound's leading term and its radical are the same quantity, `z²/(2n)`, so they cancel identically and the bound is **exactly 0** — an algebraic identity at every `n` and every confidence, not a small number. Binary floating point does not reliably cancel them: over `n` in 1:1000 a residue near `1e-18` survives at 265 sizes at one-sided 90%, 201 at 95%, and 121 at 99%. The round numbers one samples by hand — 10, 20, 100, 500 — mostly cancel cleanly, which is how this stayed invisible.

The residue is eighteen orders of magnitude below any tolerance a suite would set, and it is decisive on the artefact that binds. The decision artefact is `⌈n_test · p*⌉`, and `ceiling` turns any positive residue into a cutoff of **1**. So for roughly one test size in five at 95% confidence, a test whose baseline succeeded on no attempt — which the evidence says can demand nothing — was handed a cutoff demanding one success. Two implementations differing in the last ulp agree on the threshold to every tolerance anyone would set and return opposite verdicts.

This is the failure mode the conformance loop exists to prevent, in its sharpest form: the oracle and both frameworks carried the *same* fault, so no conformance run could see it. It was found only by writing a case at an input nobody had written a case for.

## What the release contains

`mavai-R` **v0.10.13**, additive. Nine new cases, one new expected field, **no existing expected value changed** in any of the seventeen suites.

```text
threshold_derivation  +3   family-mandatory
  ssf_zero_baseline_n10_test50_95pct       baseline 0/10,   test 50,  95%
  ssf_zero_baseline_n1000_test200_95pct    baseline 0/1000, test 200, 95%
  ssf_zero_baseline_n100_test85_99pct      baseline 0/100,  test 85,  99%
  expected: { threshold: 0, wilson_lower_real: 0, cutoff_integer: 0, achieved_size: 0 }

regression_decision   +2   family-mandatory
  zero_baseline_pass_on_nothing_observed   baseline 0/10,   test 50,  K = 0
  zero_baseline_pass_at_test_200           baseline 0/1000, test 200, K = 0
  expected: { threshold_real: 0, cutoff_integer: 0, displayed_rate: 0,
              achieved_size: 0, verdict: "PASS" }

risk_driven_sizing    +4   optional suite
  zero_baseline_required_n_refused         baseline_rate 0, required_n approach
  zero_baseline_power_at_refused           baseline_rate 0, power_at approach
  zero_baseline_detectable_rate_refused    baseline_rate 0, detectable_rate approach
  tolerance_at_baseline_refused            p_min = p0 = 0.90
  expected: { sizing_gate: "REFUSE", refusal_category: <cause>, <numerics>: null }
```

The test sizes are chosen, not arbitrary: 50, 200 and 85 are residue sites at their stated confidence levels, so each case discriminates an implementation carrying the cancellation residue rather than merely agreeing with everyone. All five value cases were verified to **bind** — computing the expectation the way an implementation without the guard would yields `cutoff_integer` 1 where the fixtures say 0.

**Companion §4.3.4 is new** and states the zero baseline's consequences explicitly, because two of them differ and both were previously left to be inferred:

- **Threshold derivation degenerates but stays defined.** `p* = 0`, cutoff 0, and every outcome clears it. A baseline that succeeded on no attempt supports no lower bound above zero, so it can demand nothing of its successor. This is the correct reading of the evidence, not a failure of the construction.
- **Sizing is undefined and must be declined.** §5.4.1 is defined for `p_min < p0`; with `p0 = 0` that domain is empty for every tolerance. The design must be declined and the criterion and its counts named — never repaired by clamping `p0` to something small or falling back to a fixed-threshold form, both of which price a design against evidence that does not support it, silently.

## Consumer-visible shape change (additive)

`risk_driven_sizing` cases now carry **`sizing_gate`** — `"ADMIT"` on every previously-published case, `"REFUSE"` on the four new ones — and refusals additionally carry **`refusal_category`** (`ZERO_BASELINE` or `EMPTY_TOLERANCE_INTERVAL`). Both are **binding** fields in the manifest.

The suite previously expressed the §5.4.1 domain restriction by *declining to emit cases outside it*: the generator's guard fired and the regime went unpublished, which left "correctly refuses" a convention each framework invented privately rather than an outcome any of them could be held to. It is now published, in the shape `criterion_verdict_inferential` already uses for `feasibility_gate`. Admissible cases carry the gate too, so its absence is never how a consumer infers admissibility.

Two refusal causes are distinguished deliberately. A framework that collapses them into one message loses the distinction the operator acts on: a zero baseline means *go and measure a baseline*, while a tolerance at or above the baseline means *re-measure the baseline rather than raising the tolerance*.

## The mandatory tier is unchanged

Recorded in the manifest as `familyMandatoryRationale`, because a tier decision that lives only in a directive is one consumers cannot see.

The substitution and its mirror are already obligations of the existing tier: `threshold_derivation` carries the perfect baseline at two baseline sizes and the zero baseline at three test sizes, and `regression_decision` carries both boundaries composed into a verdict. Promoting `baseline_object` or `criterion_verdict_inferential` was therefore unnecessary. One obligation deliberately sits outside the tier — the sizing refusal, in the optional `risk_driven_sizing`. A framework implementing no risk-driven sizing has nothing to refuse; one that does must consume the suite to be conformant with the feature it claims.

Also repaired: `manifest.json` had declared `fixtureVersion` **0.8.5** since that release. The cases had not changed since, so nobody regenerated it, and five releases' worth of consumers vendored a manifest that misdescribed its own version — including the hash-checking consumers most likely to trust it. It now reads `0.10.13`.

## feotest's measured status

Run on 2026-08-19 by temporarily vendoring the unreleased fixtures into `tests/conformance/` and running `cargo test --test conformance`. Four test functions failed: `conformance_threshold_derivation`, `conformance_regression_decision`, `conformance_risk_driven_sizing`, and `conformance_coverage_meets_manifest`.

**1 — The cancellation residue, identically to the oracle's (2 deviations).**

```text
threshold_derivation/ssf_zero_baseline_n10_test50_95pct/cutoff_integer     expected 0, got 1
threshold_derivation/ssf_zero_baseline_n1000_test200_95pct/cutoff_integer  expected 0, got 1
```

feotest reproduces the oracle's arithmetic faithfully — including the defect. `ssf_zero_baseline_n100_test85_99pct` passes, so this is site-specific in feotest exactly as it was in R: not a systematic difference in the construction, the same failure to cancel. Worth stating plainly, because it is the whole argument for this release: three independent implementations in three languages carried one fault, and no conformance run could see it, because the fixture that would have exposed it did not exist.

**2 — The zero-baseline verdict is FAIL where the companion says PASS (2 deviations).**

```text
regression_decision/zero_baseline_pass_on_nothing_observed/verdict  expected "PASS", got "FAIL"
regression_decision/zero_baseline_pass_at_test_200/verdict          expected "PASS", got "FAIL"
```

feotest is at least self-consistent here, which punit was not — punit returned INCONCLUSIVE for one and FAIL for the other. feotest's answer is wrong in one consistent way, most likely downstream of the cutoff being 1 rather than 0: with `c = 1` and `K = 0`, `K ≥ c` is false and FAIL follows correctly from a wrong cutoff. Fix the cancellation first and re-run before treating the verdict as a separate defect — it may not be one.

**3 — Sizing refuses by erroring, and once by panicking (4 deviations).**

```text
zero_baseline_required_n_refused        baseline_rate must be in (0, 1), got 0
zero_baseline_detectable_rate_refused   baseline_rate must be in (0, 1), got 0
tolerance_at_baseline_refused           minimum_acceptable_rate (0.9) must sit below baseline_rate (0.9): …
zero_baseline_power_at_refused          called `Option::unwrap()` on a `None` value
```

Three of these are validation errors with good messages — the `tolerance_at_baseline_refused` text already tells the operator to re-measure the baseline rather than raise the tolerance, which is exactly what the new `refusal_category` encodes. The fourth is not a refusal at all: `power_at` **panics** on an unwrap. A zero baseline reaches a `None` that the code does not expect, which means that path has no refusal, only an absence that happens to be fatal. Fix that one first regardless of the fixture work.

**4 — Coverage gap: 38 binding assertions the manifest demands and feotest never makes.**

Every `risk_driven_sizing` case now carries `sizing_gate`, and feotest asserts none of them; the two new `regression_decision` cases contribute their `cutoff_integer` / `displayed_rate` / `achieved_size` obligations as well. The coverage test naming these is the mechanism working as designed — selective assertion failing the build instead of passing silently.

## What feotest must do

1. **Re-vendor `tests/conformance/` at v0.10.13** and bump `VERSION`. `SCOPE.json` needs no change: `risk_driven_sizing` is already declared, which is why the sizing refusals are binding here.

2. **Fix the cancellation at a zero rate.** Return the algebraic value when `p_hat == 0.0` rather than the computed one, in the from-rate form of the Wilson lower bound. Assert equality **exactly**, and sweep `n` densely over 1..=1000 at 0.90 / 0.95 / 0.99 — sampling round `n` and comparing under an epsilon are the two habits that could not see this. The test should fail against the pre-fix body.

3. **Re-run the verdict cases before treating them as a second defect.** The FAIL verdicts are consistent with a correct rule applied to the wrong cutoff. If they go green with the cutoff fixed, there is nothing further to do; if they do not, §4.3.4 now states the answer — cutoff 0, `K >= 0` for every outcome, so PASS.

4. **Fix the `power_at` panic, then move all four refusals onto `Result`.** An inadmissible sizing design is an expected outcome carried as data — idiomatic `Result<T, E>` in Rust, the analogue of the family's `Outcome` convention — not a panic and not a bare validation error the harness cannot distinguish from a crash. Keep the existing message text and carry the two causes separately so `ZERO_BASELINE` and `EMPTY_TOLERANCE_INTERVAL` stay distinguishable.

5. **Assert `sizing_gate` and `refusal_category`** so the 38 outstanding binding assertions clear.

## Acceptance

- `cargo test --test conformance` green at v0.10.13, with `conformance_coverage_meets_manifest` reporting no unmet binding assertion.
- A regression test pins `(n as f64 * wilson_lower_from_rate(0.0, n, c)).ceil() == 0.0` densely over `n` and at three confidence levels, and fails against the pre-fix body.
- No panic on any sizing input; `power_at` at a zero baseline returns a refusal.
- Inadmissible sizing designs return `Err`, with the two causes distinguishable.
