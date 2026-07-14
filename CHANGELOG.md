# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

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
