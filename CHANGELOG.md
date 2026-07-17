# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
