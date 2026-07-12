# REQ-R-risk-driven-sizing — New `risk_driven_sizing.json` suite (adopted)

**Source release:** `mavai-R` v0.8.5.
**Triggered by:** publication of companion §5.4.1 (self-consistent power for baseline-derived thresholds, companion 1.4.0/1.4.1) as a conformance suite. Tracked by `DIR-BAS-SIZING-risk-driven` in the orchestrator; baseltest is the first adopter.

## What this is

The §5.3/§5.4 power forms hold the threshold constant; a regression-procedure test derives its acceptance floor at its own size, so the floor falls as n shrinks and the fixed-threshold forms overstate the power of small designs. The new suite materialises the self-consistent form: `power_at` (floor and power at a candidate n), `required_n` (smallest n meeting a target power for a declared minimal acceptable rate), and `detectable_rate` (the inversion at a fixed n; bisection to 1e-10). Defined for `minimum_acceptable_rate < baseline_rate` only. Suite tolerance 1e-6; the `approach` field discriminates the three groups; all expected fields are manifest-binding.

## Status — adopted

feotest has adopted risk-driven sizing, both the statistics and the authoring surface (the suite is not family-mandatory; it binds through feotest's committed scope):

- **Statistics**: `statistics::risk_driven_sizing` implements the self-consistent form — `self_consistent_power`, `required_sample_size` (doubling + bisection over the monotone power curve), and `detectable_rate` (bisection to 1e-10) — with the floor computed through the existing Wilson from-rate lower bound, so the sizing and the threshold derivation share one z convention. The existing `sample_size::calculate_for_power` closed form remains as the fixed-threshold seed.
- **Conformance**: `risk_driven_sizing` is in `tests/conformance/SCOPE.json`; all 13 cases' binding fields are asserted through the production functions via the coverage ledger, against the fixtures vendored at v0.8.5.
- **Authoring surface**: `ThresholdApproach::RiskDriven { minimum_acceptable_rate, confidence, target_power }` computes the governing sample count from the resolved baseline before sampling — each baseline-derived criterion sized against its own per-criterion baseline rate (contract-aggregate fallback exactly where per-criterion threshold derivation falls back), the largest requirement governing — and then proceeds as sample-size-first at that count. Over-reach (tolerance at or above the governing baseline rate) panics naming the governing criterion.
