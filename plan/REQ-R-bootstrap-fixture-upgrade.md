# REQ-R-bootstrap-fixture-upgrade — Consume the upgraded `latency_threshold_bootstrap.json` fixture

**Source release:** `javai-R` v0.7.0.
**Triggered by:** the role+shape upgrade of
`inst/cases/latency_threshold_bootstrap.json` from informational
R-internal comparison report to cross-framework conformance
contract.

## Background

Prior to javai-R v0.7.0, `latency_threshold_bootstrap.json`
carried suite-level `tolerance: 0` and a per-case shape that
made the suite unconsumable by an external framework:

```text
inputs:   { n, p, confidence }
expected: { point_estimate, binomial_bound, binomial_rank,
            bootstrap_upper, diff }
```

The inputs did not include the lognormal baseline R drew
internally, so a consuming framework could not reproduce
`binomial_bound` and `binomial_rank` against the same data.
The suite was effectively a documentation artefact for §12.4.4
of the Statistical Companion (the bootstrap-vs-binomial
comparison), not a conformance target.

v0.7.0 upgrades the fixture to a conformance contract for the
exact binomial order-statistic upper bound — the very method
feotest's threshold-derivation implementation already produces.
The bootstrap-vs-binomial comparison content is preserved, on
the `expected` side as informational fields.

## Upgraded shape

```text
inputs:
  baseline_latencies:    [t_{(1)}, t_{(2)}, …, t_{(n)}]   ← ascending-sorted
  p:                     <percentile level>
  confidence:            <confidence level>

expected:
  rank:                  <integer k>              ← conformance (exact)
  threshold:             <t_{(k)}>                ← conformance (exact)
  baseline_percentile:   <t_{([p·n])}>            ← conformance (exact)
  n:                     <integer n>              ← conformance (exact)
  bootstrap_upper:       <real>                   ← informational
  point_estimate:        <real>                   ← informational
  diff:                  <real>                   ← informational
```

`baseline_latencies` is published in **ascending order**, matching
the order-statistic notation in `STATISTICAL-COMPANION.md` §12.4.2
and the existing `latency_threshold.json` convention. Cases are
named `lognormal_n{n}_p{percentile-times-100}` (same names as
prior versions; only the per-case shape changed).

Suite `tolerance` is `0` — every conformance field is an integer
or a specific element of `baseline_latencies`, so exact equality
is the natural conformance check. Floating-point tolerance does
not apply.

## What feotest must do

Extend feotest's existing latency-threshold conformance test
(presumed location: `tests/conformance/` or equivalent under
the current layout — verify at implementation time) to consume
the upgraded `latency_threshold_bootstrap.json` suite. The test
must, per case:

1. Read `inputs.baseline_latencies`, `inputs.p`,
   `inputs.confidence` from the fixture.
2. Call feotest's threshold-derivation function on those inputs.
3. Assert exact equality on each conformance field:
   `rank`, `threshold`, `baseline_percentile`, `n`.
4. **Optionally** (recommended): assert the binomial-conservatism
   property `expected.threshold >= expected.bootstrap_upper`
   across every case — if this ever flips on a future fixture
   release, the test fails loudly. This is a sanity guard on the
   published comparison rather than a conformance check against
   feotest's own implementation.

The bootstrap-comparison fields (`bootstrap_upper`,
`point_estimate`, `diff`) are not conformance targets for
feotest. Do not implement a bootstrap method in feotest just to
satisfy them. The binomial order-statistic method is and remains
the production threshold; bootstrap is informational only.

## Consuming the new release

feotest's conformance fixtures are vendored — `tests/conformance/`
(or equivalent) holds a pinned copy of the javai-R release. Bump
the pinned version to v0.7.0 (or whatever javai-R tag carries
the upgraded fixture at the moment feotest picks it up) and
re-extract the cases-v0.7.0.zip artefact.

## Reference fixture cross-check

The n=935 baseline published in the upgraded
`latency_threshold_bootstrap.json` is byte-identical to the
`worked_example_p95_935_samples` baseline in
`latency_threshold.json` (same seed, same DGP — `set.seed(42);
sort(round(rlnorm(935, meanlog = log(500), sdlog = 0.3)))`). The
n=200 baseline matches the `large_sample` baseline used by
several cases in `latency_percentile.json`. This consistency is
intentional — fixture authors can trust that the seeded
baselines refer to the same arrays across suites.

## Non-conformance handling

If feotest's threshold-derivation function disagrees with any
conformance field, **investigate before adjusting**. Three
possibilities:

1. feotest has a bug in its rank-selection or threshold-emission
   logic.
2. javai-R has a bug — open a `javai-R` issue with the failing
   case and feotest's computed values.
3. A non-trivial methodology change has landed in javai-R that
   feotest hasn't picked up yet. In which case the corresponding
   javai-R CHANGELOG entry will say so.

Default to (1) — feotest is the consumer, javai-R is the oracle.

## Test plan

- [ ] Pinned conformance fixtures bumped to javai-R v0.7.0+.
- [ ] Conformance test consumes
  `latency_threshold_bootstrap.json` with the same per-case
  loop pattern as `latency_threshold.json`.
- [ ] Each conformance field asserted exact (no tolerance).
- [ ] Informational fields explicitly *not* asserted (or
  asserted only via the binomial-conservatism sanity guard).
- [ ] feotest's full test suite green against the upgraded
  fixture.

## See also

- `javai-R` v0.7.0 CHANGELOG entry for the full upgrade
  rationale.
- `plan/directives/DIR-LATENCY-THRESHOLD-FIXTURE-UPGRADE-javai-R.md`
  in the orchestrator (the directive that drove the upgrade).
- `plan/directives/DIR-CONFORMANCE-COVERAGE-GAPS-punit.md`
  Item 1 / Path C — the parallel work on the punit side.
