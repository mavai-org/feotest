# feotest

A probabilistic testing framework for stochastic services, written in Rust.

## The problem

Many modern services are inherently non-deterministic. An LLM-backed endpoint
may produce different outputs on each invocation. A ranking service may reorder
results depending on real-time signals. A classifier may disagree with itself
across repeated calls. For systems like these, the correctness of a single
execution is not a meaningful concept — correctness is a property of behaviour
observed over many executions under controlled conditions.

Rust has excellent tools for deterministic testing (`cargo test`), property-based
testing (`proptest`, `quickcheck`), and benchmarking (`criterion`). What it does
not yet have is a framework for **statistically sound verdicting of stochastic
service behaviour** — one that models repeated trials explicitly, applies
established inference methods, and produces verdicts grounded in confidence
bounds rather than ad hoc thresholds or naive averaging.

`feotest` fills that gap.

## What feotest does

`feotest` treats each invocation of a stochastic service as a **Bernoulli
trial** with a binary outcome: success or failure, as defined by a contract.
Given a sequence of such trials, the framework applies established statistical
methods to determine whether the observed pass rate meets a specified threshold.

The workflow follows a measure-then-test discipline:

1. **Measure** — run a large number of trials under controlled conditions to
   establish an empirical baseline for the service.
2. **Derive** — compute a statistically grounded threshold from the baseline,
   accounting for sampling variability via Wilson score confidence intervals.
3. **Test** — run a smaller number of trials and apply statistical inference to
   determine whether the service still meets the derived threshold.

This separation ensures that thresholds are grounded in evidence, not guesswork.

## What feotest is not

`feotest` is not about random test data generation, fuzzing, property-based
testing, flaky-test retries, or benchmarking. It does not replace or compete
with `cargo test`, `cargo-nextest`, `rstest`, `proptest`, `quickcheck`,
`criterion`, or any mocking or snapshot tool. These tools are good at what they
do. `feotest` is designed to **complement** them by addressing a category of
testing problem they were not built for.

## Quick start

Add `feotest` as a dependency:

```toml
[dependencies]
feotest = { path = "../feotest" }  # or from crates.io once published
```

The lowest-ceremony entry point is the `#[probabilistic_test]` attribute macro: the function body judges one service response, and the attribute carries the sampling plan. The framework runs the body as repeated trials and produces a statistical verdict.

```rust
use feotest::model::ContractViolation;
use feotest::probabilistic_test;

#[probabilistic_test(samples = 100, threshold = 0.95, threshold_origin = "slo")]
fn service_returns_valid_json() -> Result<(), ContractViolation> {
    // Call your service here and judge its response.
    let response = my_service_call("request");
    if response.starts_with('{') {
        Ok(())
    } else {
        Err(ContractViolation::new("format", "response was not JSON"))
    }
}
```

`Err(ContractViolation)` is a counted trial failure, not a crash — the verdict is statistical, over all 100 samples.

For richer contracts — multiple criteria, typed inputs and outputs, latency commitments, covariates — implement the `ServiceContract` trait and run it through the contract-driven builder:

```rust
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;

let result = ProbabilisticTest::for_contract(MyServiceContract::new())
    .inputs(&inputs)
    .approach(ThresholdApproach::ThresholdFirst {
        samples: 100,
        min_pass_rate: 0.95,
    })
    .run();

assert!(result.passed());
```

For a complete worked example, see
[feotest-examples](https://github.com/mavai-org/feotest-examples).

## Three operational approaches

Every probabilistic test configures a **threshold** — the minimum pass rate the
service must achieve. The framework offers three approaches for determining this
threshold, each fixing two variables and deriving the third:

| Approach | You specify | Framework computes |
|---|---|---|
| **Threshold-first** | samples + threshold | implied confidence |
| **Sample-size-first** | samples + confidence | threshold (from baseline) |
| **Confidence-first** | confidence + effect size + power | required samples |

**Threshold-first** is the simplest: "I know the pass rate must be at least 95%.
Run 100 samples and tell me if it passes." This is natural for SLA-driven
services.

**Sample-size-first** is the empirical approach: "I have budget for 100 samples.
Derive the best threshold the baseline supports at 95% confidence." This is
natural for services where the threshold is not known upfront.

**Confidence-first** is the quality-driven approach: "I need to detect a 5%
degradation with 95% confidence and 80% power. Tell me how many samples I need."

## Architecture

The framework is organised around a small number of core modules:

| Module | Responsibility |
|---|---|
| `statistics` | Confidence intervals, threshold derivation, hypothesis testing |
| `model` | Domain types: outcomes, violations, sample aggregates, warnings |
| `criteria` | Acceptance criteria: normative, empirical, and reference-matching |
| `service_contract` | The `ServiceContract` trait: the named unit of work under test |
| `latency` | Latency percentile criteria and enforcement |
| `verdict` | Mapping statistical results to pass/fail decisions |
| `spec` | Baseline specs: generation, resolution, covariate matching, expiration |
| `controls` | Operational safeguards: warm-up, budgets, pacing, token tracking |
| `experiment` | Experiment workflows: measure, explore, optimize |
| `ptest` | Probabilistic test execution and verdict production |
| `reporting` | Structured output: console, JUnit XML, verdict XML, HTML |
| `sentinel` | Deployable reliability sentinel runtime |

Dependencies point inward: statistics and model are at the core, reporting is at
the periphery. Nothing depends on reporting; everything depends on model.

## Statistical basis

The statistical model rests on explicit assumptions:

- **Approximate independence**: the outcome of one trial does not materially
  influence subsequent trials.
- **Approximate stationarity**: the service's success probability does not
  change materially over the sampling window.
- **Clear success/failure criteria**: each trial produces an unambiguous binary
  outcome defined by a service contract.
- **Controlled operational conditions**: exogenous factors (network state,
  model version, input distribution) are sufficiently stable during a test run.

Confidence intervals are computed using **Wilson score intervals**. Thresholds
are derived from one-sided confidence bounds. Verdicts use one-sided z-tests.

`feotest` does not claim that these assumptions are always perfectly met. It
insists that they be made explicit and that departures from them be acknowledged
and, where possible, mitigated through operational controls.

## Intended audience

`feotest` is for Rust developers who build or integrate with stochastic services
and who need to make rigorous, evidence-based claims about service quality.
Typical target systems include:

- LLM-backed services and agents
- ranking and recommendation systems
- classifiers and scoring models
- externally influenced APIs
- any system whose behaviour varies meaningfully across repeated executions

Users are expected to have a basic understanding of statistical concepts
(confidence intervals, hypothesis testing, sample size) or a willingness to
learn. The framework aims to make the statistical model transparent, not to hide
it behind opaque abstractions.

## Documentation

- **[User Guide](docs/USER-GUIDE.md)** — comprehensive guide to the framework's
  concepts, workflows, and API
- **[feotest-examples](https://github.com/mavai-org/feotest-examples)** — worked
  examples with two complete service contracts

## Project status

**Early stage.** The statistics core is complete and well-tested. The execution
engine, experiment types, and probabilistic test builder are implemented. The
API should be considered unstable.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

## Contributing

Contributions are welcome. All contributions are accepted under Apache 2.0 and
require a [Developer Certificate of Origin](dco.txt) sign-off (`git commit -s`).
See [CONTRIBUTING.md](CONTRIBUTING.md) for details.
