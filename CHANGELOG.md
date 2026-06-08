# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
