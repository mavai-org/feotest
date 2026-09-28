# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.3.0] - 2026-09-28

Every verdict is now decided by the decision rules of Statistical Companion
1.5.0 (methodology 1.5.0), conformance-tested against the mavai-R v0.11.1
fixtures: 445 of 445 family-mandatory binding assertions and 359 of 359 in
this crate's scope. Cutoffs, sample sizes and verdicts change: a test that
passed under 0.2 may fail, pass at a different size, come back INCONCLUSIVE,
or be refused before it runs. There is no compatibility switch; re-run your
tests. Historical verdicts are reproducible from the 0.2.0 release.

### Changed (breaking)

- **A baseline-derived criterion is decided by `regression/fisher`.** The
  one-sided Fisher exact test of the test's successes against the
  baseline's, as an integer cutoff derived from the baseline's counts at the
  test's own size, replaces the Wilson-bound cutoff (whose false-alarm rate
  was not controlled when the baseline was itself a sample) and its
  perfect-baseline special case. The record states what the design can
  detect: the size at the assumed common rate, the minimum detectable
  degradation (the inversion of the design power at 80%), and — for a sized
  run — the design power and the resolved power at the design alternative
  rate, named apart.

- **A declared requirement is decided by `compliance/exact-binomial`.** A
  `Criterion::meeting().pass_rate(p)` criterion passes when at least the
  smallest count the exact one-sided binomial test accepts succeeds, where it
  used to pass when the run's Wilson lower bound cleared `p`. The feasibility
  minimum becomes `⌈ln alpha / ln p⌉` (a 0.95 requirement needs 59 samples at
  95% confidence); `feasibility_check(samples, target, alpha)` reports it
  under the criterion `exact_binomial_pass_possible`. Experiment-time
  normative judgement uses the same rule.

- **An invalid configuration is refused whole, before any sample runs.** A
  test planned larger than a baseline it consumes (`TEST_LARGER_THAN_BASELINE`,
  judged for every baseline-derived criterion and enforced baseline-derived
  latency constraint, whatever the intent), and — under verification — a
  requirement or explicit latency ceiling no outcome of the planned size can
  demonstrate (`COMPLIANCE_INFEASIBLE`), refuse the whole configuration, with
  every applicable error reported in that fixed order. A refused test still
  returns a `VerdictRecord`: the configuration errors, no verdict, no sample
  executed, and the termination reason `CONFIGURATION_REFUSED`. This replaces
  the panic on an infeasible verification run. A requirement alone has no
  upper test size; a smoke run of an undersized requirement runs and cannot
  pass.

- **The record's verdict is the test verdict `V_test`.** The functional
  criteria compose by one structural rule — PASS if all pass, FAIL if any
  fails, INCONCLUSIVE otherwise (a FAIL now dominates an INCONCLUSIVE) — and
  the same rule composes them with the enforced latency constraints; advisory
  latency never enters it. `VerdictRecord::verdict()` returns
  `Option<Verdict>` (`None` for a refusal) and the record gains
  `methodology_version()`, `configuration_errors()`, `triggering()` (what
  decided a FAIL or an INCONCLUSIVE), `envelopes()` (the Type-I envelopes by
  direction), `single_decision_rule()` and `confidence_level()`.
  `FunctionalAssessment::new` takes only the rows and derives the composite.

- **Latency is decided after the run, by the rule for its threshold source.**
  An explicit ceiling is a requirement decided by
  `latency/compliance-exact-binomial` — the count of successful latencies at
  or below it — at the latency criterion's confidence
  (`LatencyCriterion::confidence`, default 0.95). A baseline-derived threshold
  is decided by `latency/precedence`: the baseline latency at the smallest
  rank an undegraded service would exceed with probability at most alpha, for
  the test's own count of successful latencies; when no rank achieves that,
  the evaluation is `SATURATED`, has no threshold, and is INCONCLUSIVE. It
  replaces the order-statistic confidence bound derived before the run. The
  pre-run existence and non-degeneracy checks of enforced constraints are now
  warnings with a planning figure (`LATENCY_SATURATION_EXPECTED`,
  `LATENCY_DEGENERATE_EXPECTED`), never refusals. Evaluations carry their
  rule, their confidence and the within-threshold counts; the dimension
  carries `V_latency`. The latency population is unchanged: the samples that
  passed every functional criterion.

- **Risk-driven sizing is resolved sizing against the observed baseline.**
  `ThresholdApproach::RiskDriven { minimum_acceptable_rate, .. }` becomes
  `RiskDriven { design_alternative_rate, .. }`: the true rate at which the
  test is to reach its target power, not a tolerance — the test still flags
  any degradation it can see. The run size is the smallest from which the
  resolved power stays at the target for every larger test up to the
  baseline's size; a baseline too small for that is refused
  (`BASELINE_TOO_SMALL`). `ConfidenceFirst` is the same sizing with the
  design alternative rate stated as `baseline rate − min_detectable_effect`,
  replacing the fixed-threshold closed form. `ThresholdFirst` against a
  baseline discloses the implied alpha of its declared cutoff (§6.3) instead
  of an implied confidence. The disclosure key `sizing-tolerated-rate`
  becomes `sizing-design-alternative-rate`, and the detectable rate is the
  resolved one.

- **Verdict XML moves to schema 1.7.** Every record states
  `methodology-version`; criterion rows, strict latency evaluations and — when
  one rule decided the whole test — the `<verdict>` name their versioned
  decision rule; a refused record carries `configuration-error` and no
  `value`; a saturated latency evaluation carries status `SATURATED` and no
  `threshold-ms` or `baseline-rank`; `<statistics>` gains the regression
  disclosures and loses the z-test statistic and p-value. The JSON wire shape
  of `VerdictRecord` changes accordingly (`statisticalAnalysis` carries the
  rule and its evidence). The HTML report shows the rules and refusals.

- **The statistics API is replaced, not extended.** Withdrawn:
  `statistics::threshold` (`derive_sample_size_first`,
  `derive_threshold_first`), `statistics::sample_size`,
  `statistics::evaluator`, `latency::derive_latency_threshold`,
  `risk_driven_sizing::{self_consistent_power, required_sample_size,
  detectable_rate}`, and the types `DerivedThreshold`, `DerivationContext`,
  `DecisionCutoff`, `OperationalApproach`, `SampleSizeRequirement`,
  `VerdictWithConfidence` and `ResolvedLatencyThreshold`;
  `StatisticalAnalysis::new` takes the rule's evidence and `with_test_results`
  is gone. `TestIntent` and `Verdict` now live in the statistics core
  (`statistics::rules`, `statistics::decision`) and are re-exported from
  `model` and `verdict` as before. `ConfidenceLevel::alpha` returns the level
  as the decimal it was written as (`0.95` gives exactly `0.05`).

### Added

- **The four decision rules and their exact computations**:
  `statistics::regression` (the Fisher cutoff, its size, design and resolved
  power, minimum detectable degradation, resolved detectable rate, implied
  alpha), `statistics::compliance` (the smallest passing count, the
  feasibility minimum, the Clopper–Pearson lower bound, compliance sizing with
  a declared, margin or midway design alternative), `statistics::latency`
  (the precedence rank and threshold, the latency compliance decision, the
  pre-run planning and post-run non-degeneracy gates), `statistics::decision`
  (one criterion under its rule, the structural composite, the Type-I
  envelopes, the test verdict) and `statistics::rules` (the rule identifiers,
  the methodology version, the configuration errors).
- **The exact-boundary convention** (`statistics::exact`, Companion §10.6): a
  probability within a relative guard band of 1e-9 of alpha is recomputed in
  exact rational arithmetic from the declared inputs, and the inclusive rule
  is applied to the exact value. Adds the `num-bigint` dependency.
- **Per-criterion confidence**: `.confidence(level)` on a criterion builder
  decides that criterion at its own level — as when a requirement and a
  baseline over the same postconditions are judged at different levels, as
  two criteria each with its own rule and verdict.
- **Design and resolved sizing** of the regression rule
  (`risk_driven_sizing::{design_required_samples, design_power_at,
  design_detectable_rate, resolved_sizing, resolved_cutoffs,
  check_sizing_domain}`) with the refusal categories `ZERO_BASELINE`,
  `ALTERNATIVE_NOT_BELOW_BASELINE`, `TEST_LARGER_THAN_BASELINE` and
  `BASELINE_TOO_SMALL`.
- **User guide**: advice not to sort input samples by difficulty or any other
  characteristic — inputs are cycled in list order, so a run that covers only
  part of a sorted list measures a different mix from its baseline — and the
  approaches, intent, refusals and latency rules as they now stand.

### Conformance

- The vendored fixtures in `tests/conformance/` are re-pinned at mavai-R
  **v0.11.1** (fixture schema 2, methodology 1.5.0), byte-identical to the
  `cases-v0.11.1.zip` release asset; the manifest's methodology version and
  decision rules are checked against what this crate implements rather than
  stated. The scope adds `threshold_derivation`, `latency_percentile`,
  `latency_percentile_minimums` and `latency_compliance_decision`
  (`feasibility` and `power_analysis` are now family-mandatory);
  `latency_threshold_bootstrap` is withdrawn with the 1.4.1 rules. The
  pass-rate decision suites run through the production verdict path,
  refusals included. The vendored interchange schemas are re-pinned at the
  `interchange-v0.11.1.zip` asset: `verdict-1.7.xsd` replaces
  `verdict-1.2.xsd`, and the emitted records — decided, two-criteria,
  saturated and refused — validate against it. Report snapshots take their
  numbers from named oracle fixture cases.

## [0.2.0] - 2026-07-17

### Changed

- **Canonical interchange schema for optimization output (breaking).**
  Optimize runs now emit the mavai family's canonical `mavai-optimize-1`
  format in place of the `feotest-spec-1` shape: `serviceContractId`
  replaces the legacy `useCaseId` wire name, each iteration carries its
  full descriptive observation (per-criterion tallies, failure
  distributions, cost, and the gated value-or-absent latency
  percentiles — most are absent at optimization's small per-iteration
  counts, which is the minimum-sample gate working), factor values land
  in a `factors` mapping (a struct factor as its own mapping, a scalar
  factor under the key `factor`), and the convergence block restates
  the selected optimum's score and factors, cross-checked by the
  interchange conformance test against the pinned published schema.
  The `mavai optimize` report renders these documents directly.

### Added

- **Risk-driven sizing.** Declare a risk appetite and let the framework
  derive the sample size: the `RiskDriven` threshold approach expresses
  the confidence-first design — the stipulated pass rate, the
  detectable margin, and the tolerated error risks in, the required `n`
  out, computed by bisection over the exact binomial design and
  conformance-locked to the oracle's risk-driven sizing suite. The HTML
  report discloses the run's sizing design; latency strips render five
  named landmarks in place of bars.

- **Named scorers, stated in the optimize artefact.** `Scorer` gains an
  optional identity (`Scorer::name`, default `None`) and a built-in
  named implementation, `ObservedPassRate`, which scores each iteration
  by the observed pass rate the artefact's statistics block states.
  A named scorer is stated in the artefact's additive `scorer` field
  (e.g. `scorer: observed-pass-rate`) so downstream consumers can label
  what the score measures; a bespoke unnamed scorer leaves the field
  absent. `OptimizeResult` additionally exposes the per-iteration
  descriptive observations (`observations()`) and the scorer's name
  (`scorer_name()`).

### Removed

- **The exploration comparison HTML renderer.** Rendering exploration
  artefacts is now the job of the family's shared `mavai` tool
  (`mavai explore <dir>`), whose public binaries for macOS, Linux, and
  Windows are downloadable from
  <https://github.com/mavai-org/mavai/releases>. `ExploreHtmlReportWriter`
  is deleted without a deprecation cycle because it never shipped in any
  release — it existed only on unreleased main. This crate keeps the emit
  side: the canonical exploration artefacts and their conformance tests
  are unchanged, and the test-suite verdict HTML report
  (`HtmlReportWriter`) is untouched.

### Changed (breaking artefact format)

- **Artefact key discipline, and the redesigned failure distribution.**
  Emitted exploration and optimization artefacts follow the mavai
  family's interchange key discipline (vendored conformance snapshot
  re-pinned at mavai-R v0.9.0): `failureDistribution` is now a
  *sequence* of `{condition, inputIndex?, inputExcerpt?, count}`
  entries (previously a mapping keyed by check name), each failed
  trial attributed to its first failing condition so counts sum to the
  stated failures; every free-text identity and mapping key is bounded
  at 256 characters, over-long identities truncated to a distinct
  prefix-plus-hash form; and result-projection values are escaped with
  genuine YAML escapes (the previous Debug-style escaping could emit
  `\u{..}` forms that are not valid YAML). A conformance test drives a
  >1,024-character input end to end: emitted documents must parse in a
  spec-strict YAML parser and validate against the pinned schemas.

- **Exploration output is now the family's canonical `mavai-explore-1`
  interchange format.** The per-configuration YAML sheds the crate-local
  schema: `schemaVersion` is `mavai-explore-1`; `useCaseId` becomes
  `serviceContractId`; `executionContext` becomes `factors`; per-criterion
  tallies are keyed `observedPassRate`/`pass`/`fail`; and the `latency`
  block carries its basis (`passing-samples`), the contributing/total
  sample counts, and the **stated** percentiles — `p50Ms`/`p95Ms`/`p99Ms`
  emitted value-or-absent under this crate's minimum-sample gates —
  alongside the sorted passing durations.
  `ExploreSpecWriter::write_one` and the previous `feotest-spec-1`
  exploration shape are superseded; verdict XML, baseline specs, and
  optimize output are untouched.

### Added

- **Interchange conformance tests.** Emitted exploration artefacts are
  validated against the vendored copy of the published `mavai-explore-1`
  JSON Schema (`tests/conformance/interchange/`, pinned per family schema
  release), plus the semantic obligations the schema cannot express
  (latency-vector sortedness, percentile gating). Emitted verdict XML is
  now validated against the vendored published `verdict-1.2.xsd` via
  `xmllint` (skipped gracefully where not installed) — previously it was
  checked only against this crate's own snapshots.

- **Exploration comparison HTML report.** `ExploreHtmlReportWriter` renders a
  single self-contained page over a directory of exploration YAMLs
  (`<root>/<service>/*.yaml`): an overview of services with their best
  configuration, and per service a ranked leaderboard (observed rate, then
  median passing latency, then average cost — with a presentational
  "too close to call" marker between equally-reliable configurations whose
  medians are within 5%), a per-criterion comparison matrix over the union of
  criteria, and per-configuration latency-distribution strips with the median
  marked. No JavaScript, no external assets; every number is read from the
  spec or is a nearest-rank percentile over the recorded passing latencies.
- **Richer exploration YAML.** Each per-configuration exploration spec now
  additionally carries its `configuration` display name, per-criterion
  tallies (`statistics.criteria.<name>`: observed / successes / failures /
  failure distribution), and the sorted passing-trial durations
  (`latency.sortedPassingLatenciesMs`). All three are additive and optional —
  existing `feotest-spec-1` files parse unchanged. `ExploreSpecWriter::write_one`
  gains a `projections` parameter to source the latency detail.

- **Normative judgement at experiment time.** A measure experiment over a
  contract that declares normative criteria
  (`Criterion::meeting().pass_rate(..)`) now judges each one against its
  stipulated threshold using the run's own samples — the one-sided Wilson
  lower bound at the run's sample count, at the framework's default 95%
  confidence. The judgement (met / failed / unsupportable-with-feasible-
  minimum) is rendered in the experiment's output, recorded per criterion in
  the baseline spec's additive, optional `normativeJudgement` block, and
  exposed on `MeasureResult::judgements()`. Empirical criteria remain
  unjudged at experiment time. `run()`'s completion semantics are unchanged —
  it never fails on a failed judgement; the new `assert_meets()` terminal
  (mutually exclusive with `run()`) performs the same run and persistence,
  then fails the test case on a failed judgement (`normative judgement
  failed`) and on an unsupportable one under distinct wording
  (`unsupportable judgement at this sample size`, stating the feasible
  minimum), with the baseline spec on disk before any failure
  propagates. Existing `feotest-spec-1` files parse unchanged; threshold
  derivation and spec resolution ignore the new block.

## [0.1.2] - 2026-06-10

### Added

- **Reference-matching criteria.** A criterion can now
  judge each sample's output against a per-sample *expected* value supplied by
  the contract, rather than only intrinsic postconditions. `ServiceContract`
  gains an `expected(&input) -> Option<Output>` method (defaulted to `None`)
  that surfaces the per-sample reference; `Criterion`'s builder gains
  `matching(matcher)` and `matching_equality()`, which route the actual output
  and the expected value through a matcher and fail the sample with a named
  violation on mismatch. Purely additive — existing postcondition-based criteria
  and contracts are unaffected, and a contract that does not override `expected`
  behaves exactly as before.

## [0.1.1] - 2026-06-08

### Changed

- **Verdict-XML namespace `http://javai.org/verdict/1.0` →
  `http://mavai.org/verdict/1.0` (breaking interchange change).** The
  verdict-XML wire namespace and the HTML report stylesheet move off
  `javai.org` to complete the family rename, in lockstep with punit. Only the
  namespace host changes; the schema shape is unchanged. Consumers parsing the
  emitted verdict XML by namespace must update. (Released as a patch version by
  project decision despite the breaking nature; note that `feotest = "0.1"`
  dependents will pick this up automatically.)

## [0.1.0] - 2026-05-31

First public release on crates.io.

### Added

- **Statistics and inference core.** Wilson score confidence intervals,
  threshold derivation, feasibility/power analysis, and verdict evaluation
  for proportions, validated against the mavai-R statistical oracle by a
  conformance suite.
- **Contract-driven probabilistic testing.** Define success/failure
  criteria for a stochastic service and assess whether it meets a
  pass-rate threshold over repeated trials.
- **Latency dimension.** Percentile-based latency thresholds alongside
  the proportion criteria.
- **Sentinels and experiments.** Authoring surface for tests plus
  measure/explore/optimize experiment workflows for establishing
  empirical baselines.
- **Reporting.** Console, HTML, JUnit, and XML verdict output.

### Changed (license)

- **Relicensed from Attribution Required License (ARL-1.0) to
  Apache License, Version 2.0.** All source, `Cargo.toml`
  metadata, and documentation now reference Apache 2.0. The
  `LICENSE` and `NOTICE` files at the crate root carry the
  canonical text. Versions of feotest published prior to this
  change remain available under their original ARL-1.0 terms;
  the relicense applies from this release forward.
- **Contributions now governed by the Developer Certificate of
  Origin (DCO).** The DCO 1.1 text is committed verbatim as
  `dco.txt`; `CONTRIBUTING.md` documents the `git commit -s`
  sign-off requirement. A GitHub Actions workflow
  (`.github/workflows/dco.yml`) blocks unsigned commits on pull
  requests. No separate contributor agreement is required —
  Apache 2.0 §5 (inbound = outbound) combined with the per-commit
  DCO sign-off carries the legal weight.
