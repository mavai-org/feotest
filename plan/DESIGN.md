# Execution Engine Design

This document describes the design of feotest's execution engine: the machinery that drives experiments, probabilistic tests, and sentinel applications. It covers all major concepts and their idiomatic Rust realisations.

The functional requirements come from punit. The implementation follows Rust norms: traits over inheritance, closures over reflection, builders over annotations, explicit lifetimes over garbage collection.

---

## 1. Use Case

A **use case** is the central abstraction. It represents a named, repeatable service invocation together with its success criteria, configuration surface, and operational metadata.

### Trait definition

```rust
pub trait UseCase: Send + Sync {
    /// Unique identifier for this use case (used in spec filenames, reports).
    fn id(&self) -> &str;

    /// Human-readable description.
    fn description(&self) -> &str { "" }

    /// Number of warm-up invocations to discard before counting.
    fn warmup(&self) -> u32 { 0 }

    /// Covariate declarations for this use case.
    fn covariates(&self) -> Vec<CovariateDeclaration> { vec![] }
}
```

### Service contract

In Java, postconditions are declared via `ServiceContract.define().ensure(...).derive(...).build()`. In Rust we use a builder that collects typed closure-based checks:

```rust
let contract = ServiceContract::<Input, Response>::builder()
    .ensure("Response has content", |_input, response| {
        if response.content().is_empty() {
            Err(ContractViolation::new("content", "empty response"))
        } else {
            Ok(())
        }
    })
    .derive("Parsed action", |_input, response| parse_action(response))
    .ensure("Action is valid", |_input, action| {
        if action.is_valid() { Ok(()) }
        else { Err(ContractViolation::new("action", "invalid action structure")) }
    })
    .build();
```

Each `ensure` closure returns `Result<(), ContractViolation>` — Rust's native `Result` replaces punit's `Outcome` type, which existed only because Java lacks an equivalent. `ContractViolation` carries the check name and a reason string, serving the same diagnostic role as `Outcome.fail(check, reason)`.

The contract is evaluated eagerly in declaration order (fail-fast). Each `ensure` receives the original input and the current value (either the original response or a derived value). `derive` transforms the value flowing through subsequent checks.

`UseCaseOutcome` bundles the result with contract evaluation:

```rust
let outcome = UseCaseOutcome::evaluate(&contract, &input, || service.call(&input));
outcome.assert_contract();   // functional postconditions only
outcome.assert_latency();    // duration constraint only
outcome.assert_all();        // both dimensions
```

### Factors and covariate sources

punit uses `@FactorGetter`/`@FactorSetter` annotations with reflection. In Rust, factors are exposed through an optional trait:

```rust
pub trait Configurable: UseCase {
    /// Returns the current value of a named factor.
    fn get_factor(&self, name: &str) -> Option<FactorValue>;

    /// Sets a named factor. Returns Err if the factor name is unknown.
    fn set_factor(&mut self, name: &str, value: FactorValue) -> Result<(), FactorError>;

    /// Lists the names of all configurable factors.
    fn factor_names(&self) -> Vec<&str>;
}
```

A use case that does not expose configurable factors simply does not implement `Configurable`. Experiments that need factor manipulation (explore, optimize) require `T: UseCase + Configurable`.

### Input sources

punit uses `@InputSource("methodName")` with reflection. In Rust, input sources are iterators provided explicitly:

```rust
let inputs: Vec<String> = vec![
    "Add 2 apples".into(),
    "Remove the milk".into(),
    "Clear the basket".into(),
];
```

These are passed to experiment/test builders. When the sample count exceeds the input count, inputs cycle round-robin (as in punit).

### Use case construction

punit uses `UseCaseFactory` with `register(Class, Supplier)`. In Rust, a factory closure serves the same purpose:

```rust
let factory: Box<dyn Fn() -> MyUseCase> = Box::new(|| MyUseCase::new(config));
```

For the execution engine, use case factories are registered in an `Engine` or `Sentinel` builder (see sections below).

---

## 2. Common Execution Engine

All experiment types and probabilistic tests share a single execution engine. The engine is responsible for:

- Constructing use case instances from registered factories
- Executing warm-up invocations (discarded)
- Cycling through inputs round-robin
- Invoking the trial closure for each sample
- Recording outcomes (success/failure, latency, metadata)
- Enforcing budget constraints (time, tokens)
- Applying pacing/rate-limiting
- Handling exceptions (count as failure vs abort)
- Early termination when failure is inevitable or success is guaranteed
- Producing a structured `ExecutionResult` consumed by downstream logic

### Core execution loop (simplified)

```rust
pub struct ExecutionEngine;

impl ExecutionEngine {
    pub fn run<U, F>(
        config: &ExecutionConfig,
        factory: &dyn Fn() -> U,
        inputs: &[Input],
        trial: F,
    ) -> ExecutionResult
    where
        U: UseCase,
        F: Fn(&mut U, &Input) -> UseCaseOutcome,
    {
        // 1. Construct use case instance
        // 2. Run warmup invocations (discard results)
        // 3. For each sample (up to config.samples):
        //    a. Select input (round-robin)
        //    b. Check budget constraints
        //    c. Apply pacing delay
        //    d. Execute trial closure
        //    e. Record outcome
        //    f. Check early termination conditions
        // 4. Build and return ExecutionResult
    }
}
```

`ExecutionConfig` captures all parameters that govern execution:

```rust
pub struct ExecutionConfig {
    pub samples: u32,
    pub warmup: u32,
    pub time_budget: Option<Duration>,
    pub token_budget: Option<u64>,
    pub on_budget_exhausted: BudgetExhaustedBehavior,
    pub on_exception: ExceptionHandling,
    pub max_example_failures: u32,
    pub pacing: Option<PacingConfig>,
}
```

The engine is agnostic to what happens with the results. Experiments feed them into spec generation; tests feed them into verdict evaluation.

---

## 3. Experiment Types

All three experiment types use the execution engine. They differ in what they do before and after execution.

### 3.1 Measure Experiment

**Purpose**: Run many samples (1000+ recommended) to establish a precise statistical baseline.

**Builder API**:

```rust
let result = MeasureExperiment::builder()
    .use_case_factory(|| MyUseCase::new(config))
    .inputs(&inputs)
    .trial(|uc, input| uc.do_work(input))
    .samples(1000)
    .experiment_id("baseline-v1")
    .expires_in_days(30)
    .build()
    .run();
```

**Output**: A spec YAML file. The format matches punit's `punit-spec-1` schema:

```yaml
schemaVersion: feotest-spec-1
useCaseId: my-use-case
generatedAt: 2026-03-27T10:00:00Z
experimentId: baseline-v1

execution:
  samplesPlanned: 1000
  samplesExecuted: 1000
  terminationReason: COMPLETED

requirements:
  minPassRate: 0.7512

statistics:
  successRate:
    observed: 0.7770
    standardError: 0.0132
    confidenceInterval95: [0.7512, 0.8028]
  successes: 777
  failures: 223
  failureDistribution:
    postcondition_failure: 223

cost:
  totalTimeMs: 920
  avgTimePerSampleMs: 0
  totalTokens: 197342
  avgTokensPerSample: 197

contentFingerprint: <sha256>
```

We adopt YAML (via `serde_yaml`) for baseline specs. This is a deliberate cross-ecosystem choice: the same format works in both punit and feotest, and YAML is human-readable and diff-friendly. There is no Rust convention that argues against this.

**Spec location**: `specs/{use_case_id}.yaml` relative to a configurable spec root directory.

### 3.2 Explore Experiment

**Purpose**: Rapidly compare multiple configurations with small sample sizes to identify promising candidates before committing to a full measurement.

**Builder API**:

```rust
let result = ExploreExperiment::builder()
    .use_case_factory(|| MyUseCase::new(config))
    .inputs(&inputs)
    .trial(|uc, input| uc.do_work(input))
    .samples_per_config(10)
    .config("gpt-4o", |uc| {
        uc.set_factor("model", "gpt-4o".into())?;
        Ok(())
    })
    .config("claude-sonnet", |uc| {
        uc.set_factor("model", "claude-sonnet-4-6".into())?;
        Ok(())
    })
    .experiment_id("model-comparison")
    .build()
    .run();
```

**Output**: Per-configuration YAML files at `explorations/{use_case_id}/{config_name}.yaml`.

Each file includes full result projections: per-sample input, postcondition outcomes, raw content. This is intentionally verbose for diff-friendly comparison between configurations.

### 3.3 Optimize Experiment

**Purpose**: Iteratively refine a single control factor to maximise or minimise a scoring function.

**Core abstractions**:

```rust
pub trait Scorer: Send + Sync {
    fn score(&self, results: &IterationResult) -> f64;
}

pub trait FactorMutator: Send + Sync {
    fn mutate(&self, current: &FactorValue, history: &[IterationRecord]) -> FactorValue;
}
```

**Builder API**:

```rust
let result = OptimizeExperiment::builder()
    .use_case_factory(|| MyUseCase::new(config))
    .inputs(&inputs)
    .trial(|uc, input| uc.do_work(input))
    .control_factor("temperature")
    .initial_value(FactorValue::Float(0.5))
    .scorer(SuccessRateScorer)
    .mutator(MyMutator)
    .objective(Objective::Maximize)
    .samples_per_iteration(20)
    .max_iterations(20)
    .no_improvement_window(5)
    .experiment_id("temperature-tuning")
    .build()
    .run();
```

**Output**: Timestamped YAML at `optimizations/{use_case_id}/{experiment_id}_{timestamp}.yaml` containing optimisation policy, best iteration, and full iteration history.

---

## 4. Probabilistic Test

A probabilistic test runs a use case repeatedly and applies statistical inference to determine whether the service meets a threshold.

### Configuration

The three operational approaches from punit's parameter triangle are supported:

| Approach              | User specifies                                   | Framework computes                   |
|-----------------------|--------------------------------------------------|--------------------------------------|
| **Sample-size-first** | `samples` + `threshold_confidence`               | `min_pass_rate` (from baseline spec) |
| **Confidence-first**  | `confidence` + `min_detectable_effect` + `power` | `samples`                            |
| **Threshold-first**   | `samples` + `min_pass_rate`                      | implied confidence                   |

### Builder API

```rust
let verdict = ProbabilisticTest::builder()
    .use_case_factory(|| MyUseCase::new(config))
    .inputs(&inputs)
    .trial(|uc, input| uc.do_work(input))
    .samples(100)
    // Threshold derived from spec by default when use_case is set.
    // Override with explicit threshold:
    // .min_pass_rate(0.90)
    // Or use confidence-first:
    // .confidence(0.95).min_detectable_effect(0.05).power(0.80)
    .intent(TestIntent::Verification)
    .threshold_origin(ThresholdOrigin::Empirical)
    .build()
    .run();
```

### Intent

- **Verification** (default): Evidential claim. The framework rejects the configuration before execution if the sample size cannot support verification at 95% confidence (when threshold origin is normative).
- **Smoke**: Lightweight early-warning. Accepts undersized configurations but labels the verdict accordingly.

### Threshold origin

Unchanged from punit: `Sla`, `Slo`, `Policy`, `Empirical`, `Unspecified`. Normative origins (`Sla`, `Slo`, `Policy`) trigger feasibility enforcement under `Verification` intent.

### Spec resolution

Baseline specs are resolved by:
1. Environment-local directory (`FEOTEST_SPEC_DIR` env var)
2. A configured spec root path

When a use case declares covariates, the engine captures current covariate values at test time and selects the best-matching baseline. Misalignment produces an `Inconclusive` verdict (as in punit).

### Early termination

The engine stops early when:
- Failure is **inevitable**: remaining samples cannot change the outcome even if all succeed.
- Success is **guaranteed**: enough successes already observed that remaining failures cannot change the outcome.

This is a statistical optimisation that reduces cost without affecting correctness.

---

## 5. Verdict

There is **one verdict type** used throughout the codebase. All experiment verdicts, test verdicts, and sentinel verdicts flow through the same structure.

### Verdict enum

```rust
pub enum Verdict {
    Pass,
    Fail,
    Inconclusive,
}
```

- **Pass**: Insufficient evidence to reject H0; no statistically significant divergence from baseline.
- **Fail**: H0 rejected; sufficient evidence of divergence.
- **Inconclusive**: Covariate misalignment or other condition that prevents reliable statistical analysis.

### Verdict record

The full verdict record is the single source of truth consumed by all rendering paths:

```rust
pub struct VerdictRecord {
    pub identity: TestIdentity,
    pub verdict: Verdict,
    pub intent: TestIntent,
    pub execution: ExecutionSummary,
    pub functional: FunctionalDimension,
    pub latency: Option<LatencyDimension>,
    pub statistical_analysis: StatisticalAnalysis,
    pub spec_provenance: Option<SpecProvenance>,
    pub cost: CostSummary,
    pub termination: TerminationInfo,
    pub warnings: Vec<Warning>,
}
```

All downstream consumers (machine-readable output, human-readable reports, sentinel verdict sinks) consume `VerdictRecord`. No rendering logic touches raw execution data directly.

---

## 6. Output and Reporting

### 6.1 Machine-readable output (feotest core)

The framework writes structured output in a standardised format. In the Rust ecosystem, the closest equivalent to JUnit/Surefire XML is **JUnit XML**, which is:
- Understood by `cargo-nextest`
- Consumed by CI systems (GitHub Actions, GitLab CI, Jenkins)
- The de facto standard for test result interchange

feotest writes JUnit-compatible XML verdict files to a configurable output directory. Each `VerdictRecord` is serialised to XML. This is the **only structured output format** the core framework produces.

The `verdict` module owns the `VerdictRecord` type. The `reporting` module owns serialisation to XML.

### 6.2 Human-readable reports (separate module)

A separate `feotest-report` crate (or module, initially) transforms XML verdict files into a standalone HTML report. This mirrors punit-report's architecture:

- Reads XML verdict files from a directory
- Produces a single `index.html` with embedded CSS (no external dependencies)
- Results grouped by use case ID
- Per-test detail: verdict summary, statistical analysis, latency percentiles
- Expandable sections for detailed statistical reasoning

This separation ensures:
- The core library has no HTML/templating dependencies
- Reports can be generated after the fact, from stored XML
- Alternative report formats can be added without touching the core

### 6.3 Console output

The core codebase may write to stdout for interactive feedback (progress, transparent statistics mode). This is informal and not part of the structured output contract. The `reporting` module provides a `ConsoleRenderer` for formatted terminal output, but this is opt-in and separate from the machine-readable path.

### 6.4 Transparent statistics

When enabled (via configuration), the framework emits detailed statistical reasoning alongside verdicts: hypothesis formulation, observed data, confidence intervals, p-values, z-scores, and plain-language interpretation. This uses the same `VerdictRecord`; the renderer simply includes more fields.

---

## 7. Sentinel

A **sentinel** is a lightweight, standalone command-line tool that runs both probabilistic tests and measure experiments in deployed environments (staging, production). Its mandate covers two activities:

1. **Verification**: run probabilistic tests against the live environment to confirm service reliability where it matters most.
2. **Baseline establishment**: run measure experiments in-situ to generate baselines that reflect production conditions — model versions, infrastructure, load patterns — rather than development-time approximations.

### Design goals

- **Minimal dependencies**: the sentinel binary should be small and fast to start. It depends on feotest-core but not on test harness infrastructure.
- **No test framework dependency**: sentinels do not require `cargo test`, `libtest`, or any test runner. They are plain Rust binaries.
- **Unix-native CLI**: the sentinel is a well-behaved command-line tool. Structured output (XML, JSON) goes to stdout by default, progress and diagnostics go to stderr. This makes the sentinel composable with standard Unix tools — pipes, redirection, `jq`, `tee`, `cron`, shell scripts. Operators integrate the sentinel into their existing operational workflows rather than adopting a framework-specific harness.
- **Shared specifications**: sentinels and test-time probabilistic tests use the same `UseCase` trait, the same `ServiceContract`, and the same `VerdictRecord`. A reliability specification written for sentinel use can also be exercised in `cargo test` via a thin adapter.

### Reliability specification

A sentinel reliability specification is a plain Rust struct that registers use case factories and declares measurements and tests:

```rust
pub struct ShoppingBasketReliability;

impl Reliability for ShoppingBasketReliability {
    fn register(&self, registry: &mut UseCaseRegistry) {
        registry.add::<ShoppingBasketUseCase>(|| ShoppingBasketUseCase::new(config));
    }

    fn specs(&self) -> Vec<Spec> {
        vec![
            Spec::measure("baseline-v1")
                .use_case::<ShoppingBasketUseCase>()
                .samples(1000)
                .trial(|uc: &mut ShoppingBasketUseCase, input| {
                    uc.translate_instruction(input)
                }),

            Spec::test("verification")
                .use_case::<ShoppingBasketUseCase>()
                .samples(100)
                .trial(|uc: &mut ShoppingBasketUseCase, input| {
                    uc.translate_instruction(input).assert_contract()
                }),
        ]
    }
}
```

### Sentinel binary

The sentinel is built as a standalone binary:

```rust
fn main() {
    let sentinel = Sentinel::builder()
        .reliability(ShoppingBasketReliability)
        .reliability(PaymentGatewayReliability)
        .spec_dir("./specs")
        .build();

    let exit_code = sentinel.run_from_args();
    std::process::exit(exit_code);
}
```

### CLI interface

The sentinel is a Unix-native command-line tool. Structured output goes to stdout; diagnostics go to stderr. This enables standard shell composition:

```
# Run tests, write XML verdicts to a file
feo-sentinel test > verdicts.xml

# Run a specific use case, pipe JSON to jq for extraction
feo-sentinel test --use-case shopping-basket --format json | jq '.verdict'

# Measure a new baseline in production, save the spec
feo-sentinel measure --use-case shopping-basket > specs/shopping-basket.yaml

# Run tests and post results to a webhook
feo-sentinel test --format json | curl -X POST -d @- https://hooks.example.com/verdicts

# List available use cases
feo-sentinel test --list

# Verbose mode (diagnostics on stderr, structured output still on stdout)
feo-sentinel test --verbose 2>diag.log > verdicts.xml

# Quiet mode — exit code only, no output
feo-sentinel test --quiet; echo $?
```

The CLI follows standard Unix conventions throughout. Built with `clap` (the de facto Rust CLI library), which provides `--help` and `--version` automatically.

**Top-level**:
```
$ feo-sentinel --help
A probabilistic testing sentinel for stochastic services

Usage: feo-sentinel <COMMAND> [OPTIONS]

Commands:
  test       Run probabilistic tests against baseline specs
  measure    Run measure experiments to establish or refresh baselines

Options:
  -h, --help     Print help
  -V, --version  Print version

$ feo-sentinel --version
feo-sentinel 0.1.0
```

**Subcommand help**:
```
$ feo-sentinel test --help
Run probabilistic tests against baseline specs

Usage: feo-sentinel test [OPTIONS]

Options:
  -u, --use-case <ID>       Run only the named use case
  -f, --format <FORMAT>     Output format [default: xml] [possible values: xml, json, text]
  -s, --spec-dir <PATH>     Override spec directory
  -l, --list                List available use cases and exit
  -v, --verbose             Detailed diagnostics on stderr
  -q, --quiet               Suppress stdout output; exit code only
  -h, --help                Print help
```

**Commands**:
- `test` — run probabilistic tests against baseline specs
- `measure` — run measure experiments to establish or refresh baselines

**Common flags** (available on both subcommands):

| Short | Long         | Argument          | Description                            |
|-------|--------------|-------------------|----------------------------------------|
| `-u`  | `--use-case` | `<ID>`            | Run only the named use case            |
| `-f`  | `--format`   | `xml\|json\|text` | Output format (default: `xml`)         |
| `-s`  | `--spec-dir` | `<PATH>`          | Override spec directory                |
| `-l`  | `--list`     |                   | List available use cases and exit      |
| `-v`  | `--verbose`  |                   | Detailed diagnostics on stderr         |
| `-q`  | `--quiet`    |                   | Suppress stdout output; exit code only |
| `-h`  | `--help`     |                   | Print help                             |

**Global flags**:

| Short | Long        | Description   |
|-------|-------------|---------------|
| `-h`  | `--help`    | Print help    |
| `-V`  | `--version` | Print version |

**Exit codes**:

| Code | Meaning                                                                                                  |
|------|----------------------------------------------------------------------------------------------------------|
| 0    | All verdicts passed                                                                                      |
| 1    | One or more verdicts: **Fail** — statistically significant degradation detected                          |
| 2    | One or more verdicts: **Inconclusive** — covariate mismatch, insufficient data — but none failed         |
| 3    | Execution error — test aborted, budget exhausted, exception, infrastructure failure — no verdict reached |
| 4    | Usage error — bad arguments, missing spec, unknown use case                                              |

The ordering is deliberate: higher codes mean "further from a meaningful statistical result." Code 0 is a clean bill of health. Code 1 is actionable evidence. Codes 2-3 mean the sentinel could not produce a reliable answer. Code 4 means the invocation itself was wrong.

A script that just checks `$? -eq 0` gets a safe default (anything non-zero is "not clean"). A script that cares about the distinction can branch:

```bash
feo-sentinel test --quiet --use-case shopping-basket
case $? in
    0) echo "Passed" ;;
    1) echo "Degradation detected — rollback"
       kubectl rollout undo deployment/shopping ;;
    2) echo "Inconclusive — investigate covariates" ;;
    3) echo "Execution error — check infrastructure" ;;
    4) echo "Bad invocation — check arguments" ;;
esac
```

Because the sentinel is a standard CLI tool, operators compose it into their existing infrastructure using whatever they already have — cron, systemd timers, CI pipelines, shell scripts, monitoring hooks. No framework-specific orchestration layer is needed.

### Verdict sinks

For programmatic use (embedding the sentinel in a larger application rather than invoking it as a CLI), verdict sinks receive verdict events for dispatch to external systems:

```rust
pub trait VerdictSink: Send + Sync {
    fn receive(&self, verdict: &VerdictRecord);
}
```

Built-in sinks: `LogVerdictSink` (structured logging), `WebhookVerdictSink` (HTTP POST). Users can implement custom sinks. In the CLI path, stdout output replaces the need for sinks in most operational scenarios.

### Dual consumption

A reliability specification can be consumed two ways:
1. **Sentinel binary**: runs directly as a CLI tool, no test framework.
2. **cargo test adapter**: a thin wrapper that bridges the reliability spec into Rust's test harness:

```rust
#[cfg(test)]
mod tests {
    use super::ShoppingBasketReliability;

    #[test]
    fn shopping_basket_reliability() {
        feotest::run_reliability(ShoppingBasketReliability);
    }
}
```

This is the Rust equivalent of punit's one-line JUnit adapter subclass.

---

## 8. Test Harness Integration

### Phase 1: Library API (current target)

All experiment and test types are usable as a library via builder APIs. Users call them from `#[test]` functions or standalone binaries. No proc macros required.

### Phase 2: Proc-macro ergonomics (future)

Once the library API stabilises, a `feotest-macros` crate can provide attribute macros:

```rust
#[feotest::probabilistic_test(
    use_case = ShoppingBasketUseCase,
    samples = 100,
    intent = "verification",
)]
fn test_instruction_translation(uc: &mut ShoppingBasketUseCase, input: &str) {
    uc.translate_instruction(input).assert_contract();
}
```

This is additive. The macro expands to the same builder API. The library API remains the source of truth.

### Integration with cargo-nextest

feotest's JUnit XML output is natively compatible with `cargo-nextest`. No special integration is needed beyond writing XML to the expected directory.

---

## 9. Module Boundaries and Dependencies

```
                    ┌──────────────┐
                    │  statistics   │  ← no intra-crate deps (enforced by arch test)
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────┴─────┐ ┌───┴───┐ ┌─────┴─────┐
        │   model    │ │verdict│ │   spec     │
        └─────┬─────┘ └───┬───┘ └─────┬─────┘
              │            │            │
        ┌─────┴────────────┴────────────┴─────┐
        │              contract                │
        └─────────────────┬───────────────────┘
                          │
              ┌───────────┼───────────┐
              │           │           │
        ┌─────┴─────┐ ┌──┴──┐ ┌─────┴─────┐
        │  controls  │ │uc   │ │ experiment │
        └───────────┘ └─────┘ └───────────┘
                          │
                    ┌─────┴─────┐
                    │ reporting  │  ← depends on verdict, model; outputs XML
                    └───────────┘
```

Dependencies point inward toward statistics and model. Reporting depends on everything but nothing depends on reporting. The execution engine lives in `experiment` and depends on `contract`, `controls`, `usecase`, and `model`.

`feotest-report` (HTML generation) is a separate crate that depends only on the XML schema — not on feotest-core at all.

---

## 10. Crate Structure

Initially, everything lives in a single `feotest` crate. When the following become necessary, we split:

| Crate            | Contents                                                                                                 | When to split                      |
|------------------|----------------------------------------------------------------------------------------------------------|------------------------------------|
| `feotest`        | Core library: statistics, model, verdict, spec, contract, controls, usecase, experiment, reporting (XML) | Already exists                     |
| `feotest-macros` | Proc-macro attribute macros                                                                              | When macro ergonomics are designed |
| `feotest-report` | HTML report generation from XML                                                                          | When reporting is implemented      |
| `feo-sentinel`   | Sentinel binary scaffold + CLI                                                                           | When sentinel is implemented       |

A Cargo workspace is introduced only when the second crate materialises.

---

## 11. Key Differences from punit

| Concern                | punit (Java)                                   | feotest (Rust)                                                                |
|------------------------|------------------------------------------------|-------------------------------------------------------------------------------|
| Use case definition    | `@UseCase` annotation on class                 | `UseCase` trait implementation                                                |
| Service contract       | Builder with lambdas + reflection              | Builder with closures, no reflection                                          |
| Factor access          | `@FactorGetter`/`@FactorSetter` via reflection | `Configurable` trait with explicit methods                                    |
| Input sources          | `@InputSource("methodName")` via reflection    | Explicit `Vec<Input>` or iterator                                             |
| Test declaration       | `@ProbabilisticTest` annotation                | Builder API (phase 1), proc-macro (phase 2)                                   |
| Experiment declaration | `@MeasureExperiment` etc.                      | Builder API                                                                   |
| Dependency injection   | JUnit `ParameterResolver`                      | Factory closures passed to builders                                           |
| Sentinel specification | `@Sentinel` class extending base               | `Reliability` trait implementation                                            |
| Test adapter           | Subclass inheritance                           | `#[test]` function calling library API                                        |
| Report format          | Custom XML schema + HTML                       | JUnit XML + separate HTML crate                                               |
| Concurrency            | JUnit parallel execution (incomplete in punit) | Omitted from first iteration; awaiting punit's design outcome before tackling |

---

## 12. Design Decisions

Resolved during design:

1. **Async execution**: Deferred. Concurrent/async execution is not yet complete in punit, so feotest will omit it from the first iteration. The execution engine will be synchronous. Once punit's concurrency design stabilises, we can evaluate the right approach for Rust (likely `async` + `tokio`). In the meantime, avoid design choices that would make a future async migration unnecessarily painful.

2. **Serialisation format for verdict XML**: JUnit XML. This maximises CI compatibility (GitHub Actions, GitLab CI, Jenkins, cargo-nextest) without requiring a custom schema.

3. **Spec file location convention**: Default to `specs/` relative to the crate root, configurable via `FEOTEST_SPEC_DIR` or builder API. This location is neutral — equally accessible from `cargo test` and `feo-sentinel` in production — and avoids overloading the `tests/` directory, which has a specific meaning in Rust (integration test crates).

4. **Error handling in trial closures**: Panics are not caught. A panic is a defect, not a contract violation. The `UseCaseOutcome` captures contract failures via `Result`; a panic means the code itself is broken and should abort the run with a clear diagnostic. This aligns with the project's existing stance that violated preconditions are programming errors, not runtime conditions.

5. **Token budget tracking**: Provide a `TokenRecorder` backed by an `AtomicU64` that the trial closure can call to report token consumption. Also support a static `token_charge` per sample for simple cases where consumption is predictable.