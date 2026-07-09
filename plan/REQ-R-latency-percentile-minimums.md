# REQ-R-latency-percentile-minimums — Consume the new `latency_percentile_minimums.json` fixture

**Source release:** `mavai-R` v0.8.3.
**Triggered by:** publication of the family standard for empirical-latency-percentile minimum sample sizes as a conformance fixture (new suite; additive release).

## Background

The per-percentile minimum-contributing-samples rule was stated in the Statistical Companion (§12.5.2) but never published as a fixture, and implementations drifted family-wide on the p50 row: the companion says **5**, punit's emission gate says 1, and **feotest already says 5** (`statistics::latency::min_samples_for`) — feotest is conformant, but nothing locks it. The new suite makes the values a conformance contract so the gating table cannot drift in either direction.

## Fixture shape

Suite `latency_percentile_minimums`, `tolerance: 0` (all values integers; exact equality). Two case groups, distinguished by the `approach` field:

```text
approach: emission_non_degeneracy          # companion §12.5.2 — emission gate
  inputs:   { percentile }
  expected: { minimum_contributing_samples }   # 5 / 10 / 20 / 100

approach: bound_existence                  # companion §12.5.2.1 — judgement-time gate
  inputs:   { percentile, confidence }         # confidence ∈ {0.95, 0.99}
  expected: { minimum_baseline_samples }       # ceiling(log(alpha) / log(p)), Wilks
```

The `bound_existence` group is **not** an emission rule — it is the minimum baseline size for the binomial order-statistic construction to admit a non-saturated upper bound, consumed by latency-criterion evaluation.

## What feotest must do

1. Bump the vendored `tests/conformance/` snapshot to the v0.8.3 release (the new suite is additive; no existing suite changed).
2. Add a conformance test asserting `min_samples_for(p)` equals each `emission_non_degeneracy` case exactly. Expected to pass immediately — the test exists so feotest *stays* conformant.
3. Verify every emission path (baseline spec, exploration output, verdict) routes through `min_samples_for` — any path with its own copy of the rule is drift and must be rerouted.
4. Optionally assert the `bound_existence` cases against the resolver's saturation/existence check.

Tracked by `DIR-PERCENTILE-MINIMUMS-family` in the orchestrator.
