# REQ-R-risk-driven-sizing — New `risk_driven_sizing.json` suite (adoption-gated)

**Source release:** `mavai-R` v0.8.5.
**Triggered by:** publication of companion §5.4.1 (self-consistent power for baseline-derived thresholds, companion 1.4.0/1.4.1) as a conformance suite. Tracked by `DIR-BAS-SIZING-risk-driven` in the orchestrator; baseltest is the first adopter.

## What this is

The §5.3/§5.4 power forms hold the threshold constant; a regression-procedure test derives its acceptance floor at its own size, so the floor falls as n shrinks and the fixed-threshold forms overstate the power of small designs. The new suite materialises the self-consistent form: `power_at` (floor and power at a candidate n), `required_n` (smallest n meeting a target power for a declared minimal acceptable rate), and `detectable_rate` (the inversion at a fixed n; bisection to 1e-10). Defined for `minimum_acceptable_rate < baseline_rate` only. Suite tolerance 1e-6; the `approach` field discriminates the three groups; all expected fields are manifest-binding.

## What feotest must do — nothing yet

The suite is **not family-mandatory** and is not in feotest's conformance scope. It becomes binding when feotest adopts risk-driven sizing (a future directive; the existing `sample_size::calculate_for_power` closed form corresponds to the §5.4 seed, not the self-consistent form). Until then feotest's conformance standing will list `risk_driven_sizing` among the unaddressed suites — that is the designed behaviour, not a gap. When adoption comes: the new statistics belong in `feotest::statistics`, conformance-locked to this suite through the coverage ledger, with the suite added to feotest's SCOPE.json and the fixtures re-vendored at v0.8.5.
