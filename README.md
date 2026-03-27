# feotest

A probabilistic testing framework for stochastic services, written in Rust.

## Why feotest exists

Many modern services are inherently stochastic. An LLM-backed endpoint may
produce different outputs on each invocation. A ranking service may reorder
results depending on real-time signals. A classifier may disagree with itself
across repeated calls. For systems like these, the correctness of a single
execution is not a meaningful concept — correctness is a property of behaviour
observed over many executions under controlled conditions.

Rust has excellent tools for deterministic testing, property-based testing,
benchmarking, and test execution orchestration. What it does not yet have is a
framework for **statistically sound verdicting of stochastic service behaviour**
— one that models repeated trials explicitly, applies established inference
methods, and produces verdicts grounded in confidence bounds rather than ad hoc
thresholds or naïve averaging.

`feotest` is intended to fill that gap.

## What feotest is not

`feotest` is **not** about:

- random test data generation
- fuzzing
- property-based testing
- flaky-test retries
- benchmarking
- replacing Rust's normal testing infrastructure

It is not trying to replace or compete with:

- `cargo test`
- `cargo-nextest`
- `rstest`
- `proptest`
- `quickcheck`
- `criterion`
- mocking or snapshot tools

These tools are good at what they do. `feotest` is designed to **complement**
them by addressing a category of testing problem they were not built for.

## Core idea

`feotest` treats each invocation of a stochastic service as a **trial** with a
binary outcome: success or failure, as defined by a contract. A sequence of such
trials, conducted under controlled conditions, can — under suitable assumptions
— be modelled as **Bernoulli trials** with a common success probability.

Given this model, the framework applies established statistical methods to
determine whether the observed pass rate meets a specified threshold. Verdicts
are based on **confidence-bound-based threshold checks**, not on point estimates
or arbitrary retry counts.

The workflow follows a measure-then-test discipline:

1. **Measure**: run a large number of trials under controlled conditions to
   establish an empirical baseline for the service.
2. **Derive**: compute a statistically grounded threshold from the baseline,
   accounting for sampling variability.
3. **Test**: run a smaller number of trials and apply statistical inference to
   determine whether the service still meets the derived threshold.

This separation ensures that thresholds are grounded in evidence, not guesswork.

## Statistical basis

The statistical model rests on several explicit assumptions:

- **Approximate independence**: the outcome of one trial does not materially
  influence subsequent trials.
- **Approximate stationarity**: the service's success probability does not
  change materially over the sampling window.
- **Clear success/failure criteria**: each trial produces an unambiguous binary
  outcome.
- **Controlled operational conditions**: exogenous factors (network state,
  model version, input distribution) are sufficiently stable during a test run.

When these assumptions are reasonably satisfied, the sequence of trial outcomes
can be treated as draws from a Bernoulli distribution. Confidence intervals for
the true success probability are then computed using established methods (Wilson
score intervals, normal approximation where appropriate), and verdicts are
derived from one-sided confidence bounds.

`feotest` does not claim that these assumptions are always perfectly met. It
does insist that they be made explicit and that departures from them be
acknowledged and, where possible, mitigated through operational controls.

## Scope of the first version

### In scope

- A Rust-first framework for probabilistic testing of stochastic services.
- Bernoulli-trial modelling of binary outcome quality.
- Confidence-bound-based threshold verdicts.
- Support for a "measure experiment" workflow to establish empirical baselines.
- Support for operational safeguards:
  - warm-up runs to mitigate cold-start non-stationarity
  - catastrophic outcome detection and halt
  - covariate capture for reproducibility
  - cost and time budgets
- A clean internal API suitable for later higher-level test ergonomics.

The initial implementation focus is a **statistics and inference engine** that is
independent of any test runner integration. Getting the statistical core right
is the first priority.

### Not in scope (initially)

- A replacement test runner or `cargo test` alternative.
- Property-based test generation.
- Fuzzing.
- Snapshot testing.
- Mocking.
- Full benchmarking infrastructure.
- Exhaustive coverage of statistical techniques.
- Full latency inference from the outset.

While latency is an important dimension of stochastic-service testing, the
initial emphasis is on the more mature and defensible Bernoulli/pass-rate model.
Latency support will follow once the pass-rate foundation is solid.

## Likely architecture

The framework is organised around a small number of core modules:

| Module | Responsibility |
|--------|---------------|
| `statistics` | Confidence intervals, threshold derivation, hypothesis testing |
| `model` | Domain types: trials, outcomes, sample aggregates |
| `verdict` | Mapping statistical results to pass/fail decisions |
| `spec` | Baseline specifications from empirical measurement |
| `contract` | Success/failure criteria for individual invocations |
| `controls` | Operational safeguards: warm-up, budgets, catastrophic halt |
| `experiment` | Experiment workflows for baseline establishment |
| `reporting` | Structured output of verdicts and diagnostics |
| `usecase` | The named unit of work under test |

The initial codebase is deliberately modest. The emphasis is on correct module
boundaries and explicit domain types, not on premature abstraction or feature
breadth. Runner integration and ergonomic macros (proc-macro attribute macros,
for example) will be layered on once the core is stable.

## Intended audience

`feotest` is intended for Rust developers who build or integrate with stochastic
services and who need to make rigorous, evidence-based claims about service
quality. Typical target systems include:

- LLM-backed services and agents
- ranking and recommendation systems
- classifiers and scoring models
- externally influenced APIs
- any system whose behaviour varies meaningfully across repeated executions

Users are expected to have a basic understanding of statistical concepts
(confidence intervals, hypothesis testing, sample size) or a willingness to
learn. The framework aims to make the statistical model transparent, not to
hide it behind opaque abstractions.

## Project status

**Early stage.** The project structure and module boundaries are established.
The first objective is to build a correct, well-tested, and idiomatic statistics
and inference core before layering on ergonomics or runner integration.

Contributions, feedback, and discussion are welcome, but the API should be
considered unstable.

## Possible future directions

These are areas the project may grow into once the core is solid:

- `#[feotest]` proc-macro attribute for ergonomic test declaration
- integration with `cargo test` and `cargo-nextest`
- async trial execution and concurrency controls
- latency-dimension inference (beyond pass-rate)
- multi-dimensional verdicts (pass-rate × latency)
- specification file formats and management tooling
- reporting integrations (CI, observability platforms)
- covariate-aware baseline selection
- early termination (impossibility and guaranteed-pass detection)

## Relationship to PUnit

`feotest` is inspired by [PUnit](https://github.com/javai-org/punit), a JUnit 5
extension framework for probabilistic testing of non-deterministic systems in
Java. The core statistical philosophy — Bernoulli modelling, confidence-bound
verdicts, measure-then-test discipline, operational safeguards — is shared.

`feotest` is not a port of PUnit. It is designed from the ground up for Rust
developers and the Rust ecosystem, following Rust idioms, conventions, and best
practices. The two projects may inform each other, but they are independent.

## Licence

Apache-2.0
