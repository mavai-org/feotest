# feotest User Guide

## For Rust developers who take quality seriously

Rust developers have earned a reputation for caring deeply about correctness.
The language itself demands it — the borrow checker, the type system, the
`Result`/`Option` discipline, the culture of `cargo clippy` and `#[deny(warnings)]`.
Where other ecosystems tolerate runtime surprises, Rust developers build systems
that are correct by construction.

This commitment to quality extends naturally to testing. Rust's testing
ecosystem is among the best available: `cargo test` for unit and integration
testing, `proptest` and `quickcheck` for property-based testing, `criterion` for
benchmarking, `rstest` for parameterised tests, and `cargo-nextest` for fast
parallel execution. These tools are excellent at what they do.

But there is a class of system that none of these tools were designed for.

## The gap

An LLM-backed service may return valid JSON on 94 of 100 calls and malformed
output on the remaining 6. A recommendation engine may produce relevant results
95% of the time and irrelevant ones 5% of the time. A classifier may agree with
human labels on 88% of inputs. These systems are not broken — they are
**stochastic**. Their quality is not a binary property of a single execution but
a statistical property of behaviour observed over many executions.

Today, Rust developers testing such systems face an uncomfortable choice:

- **Ignore the non-determinism** — write a single assertion and hope it passes.
  When it doesn't, mark the test `#[ignore]` or add a retry loop.
- **Use ad hoc thresholds** — hard-code "must pass 90% of the time" with no
  statistical justification for why 90% and not 85% or 95%.
- **Avoid testing altogether** — accept that the stochastic component is
  untestable and test only the deterministic scaffolding around it.

None of these approaches meet the standard of engineering discipline that Rust
developers apply everywhere else.

`feotest` closes this gap. It provides a principled framework for testing
stochastic services — one that models repeated trials explicitly, applies
established statistical inference methods, and produces verdicts grounded in
confidence bounds rather than guesswork. It complements, rather than competes
with, the existing Rust testing ecosystem by addressing the specific problem of
non-determinism with the rigour that Rust developers expect.

---

## Part 1: Core concepts

### Bernoulli trials

`feotest` models each service invocation as a **Bernoulli trial**: an experiment
with exactly two outcomes — success or failure — as defined by a **service
contract**. A sequence of independent trials with a common success probability
forms a Bernoulli process.

This is the foundational model for stochastic service quality. The question it
answers is: "Does this service meet a binary quality threshold with statistical
confidence?" A future version of the framework will extend this to a second
dimension — **latency** — where per-percentile thresholds (p50, p90, p95, p99)
are derived from baselines and enforced alongside pass-rate verdicts, producing
multi-dimensional assessments of service reliability.

### Service contracts

A service contract defines what "success" means for a single trial. Contracts
are built from an ordered chain of **postconditions** — closures that inspect the
service response and return `Ok(())` on success or `Err(ContractViolation)` on
failure.

```rust
use feotest::contract::ServiceContract;
use feotest::model::ContractViolation;

let contract = ServiceContract::<String, String>::builder()
    .ensure("Response has content", |_input, response| {
        if response.is_empty() {
            Err(ContractViolation::new("content", "empty response"))
        } else {
            Ok(())
        }
    })
    .ensure("Response is valid JSON", |_input, response| {
        if response.starts_with('{') {
            Ok(())
        } else {
            Err(ContractViolation::new("format", "not JSON"))
        }
    })
    .build();
```

Postconditions are evaluated in declaration order. The first failure
short-circuits — subsequent checks are not evaluated. This is a **fail-fast**
strategy that mirrors Rust's own `?` operator semantics.

A contract violation is not a software defect. It is a legitimate statistical
observation: the service was invoked correctly, but its output did not meet the
postconditions. This distinction is fundamental. A panic (a defect) must abort
the run. A contract violation (a trial outcome) must be counted and analysed.

### Verdicts

A **verdict** is the outcome of a probabilistic test:

| Verdict | Meaning |
|---|---|
| **Pass** | Insufficient evidence to reject H0. No statistically significant degradation detected. |
| **Fail** | H0 rejected. Sufficient statistical evidence of degradation. This is the call to action. |
| **Inconclusive** | Statistical analysis cannot be relied upon (e.g., covariate misalignment). |

A verdict is not based on whether individual trials succeeded or failed. It is
based on whether the **aggregate pass rate** meets the **threshold** at the
configured **confidence level**. A test can have failing trials and still produce
a Pass verdict — because the failure rate is within the expected range.

### Thresholds

Every probabilistic test requires a threshold: the minimum pass rate the service
must achieve. Thresholds can come from two sources:

- **Empirical** — derived from a measurement experiment. The threshold is the
  Wilson score lower bound of the observed pass rate: "the lowest plausible
  success rate given the data." This is the measure-then-test workflow.

- **Normative** — specified by an SLA, SLO, or policy. The threshold is a
  contractual requirement: "the service must succeed at least 99% of the time."
  No baseline measurement is needed.

The framework tracks the **origin** of each threshold (`Sla`, `Slo`, `Policy`,
`Empirical`, `Unspecified`) and uses it to calibrate feasibility enforcement and
verdict interpretation.

---

## Part 2: The measure-then-test workflow

This is the recommended workflow for services where no contractual threshold
exists. It produces a threshold that is grounded in empirical evidence rather
than guesswork.

### Step 1: Explore (optional)

Before committing to a configuration (model, temperature, prompt), run an
**explore experiment** to compare candidates. The experiment is defined
by a list of **factors** — one per configuration — and a **factory**
that constructs a service contract instance from each factor. The framework
walks the factors, builds the corresponding instance, and runs a fixed
number of trials against it.

```rust
use feotest::experiment::ExploreExperiment;
use feotest::model::TrialOutcome;
use std::fmt;

// The factor: what varies between configurations. Its `Display` impl
// supplies the configuration name used in reports and output filenames.
#[derive(Clone)]
struct ModelChoice { model: &'static str }
impl fmt::Display for ModelChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.model)
    }
}

// The service contract: what the factory produces from each factor.
struct MyService { model: &'static str }
impl MyService {
    fn new(model: &'static str) -> Self { Self { model } }
    fn call(&self, _instruction: &str) -> TrialOutcome {
        // invoke the service using `self.model` and return a TrialOutcome
        TrialOutcome::success(std::time::Duration::ZERO)
    }
}

let factors = vec![
    ModelChoice { model: "model-a" },
    ModelChoice { model: "model-b" },
];
let inputs = vec!["Add 2 apples".to_string(), "Remove the milk".to_string()];

let result = ExploreExperiment::builder()
    .service_contract_id("my-service")
    .factors(factors)
    .service_contract(|f: &ModelChoice| MyService::new(f.model))
    .samples_per_config(20)
    .inputs(&inputs)
    .trial(|svc: &MyService, input| svc.call(input))
    .build()
    .run();

for config in result.configs() {
    let rate = config.execution().summary().observed_pass_rate();
    println!("{}: {:.1}%", config.name(), rate * 100.0);
}
```

Because there is exactly one factory, every instance compared in the
experiment is by construction a variant of the same service contract —
the "one service contract, many configurations" principle is guaranteed
structurally, not by convention.

Explore experiments use small sample sizes (10–20 per configuration).
They are not statistically rigorous — they are for rapid filtering.

### Step 2: Measure

Once you have chosen a configuration, run a **measure experiment** to establish
a statistical baseline:

```rust
use feotest::experiment::MeasureExperiment;
use feotest::model::TrialOutcome;

// The service contract: what the factory produces.
struct MyService;
impl MyService {
    fn invoke(&self, _instruction: &str) -> TrialOutcome {
        // call the service and return a TrialOutcome
        TrialOutcome::success(std::time::Duration::from_millis(10))
    }
}

let inputs = standard_instructions();

let result = MeasureExperiment::builder()
    .service_contract_id("my-service")
    .service_contract(|| MyService)
    .samples(1000)
    .inputs(&inputs)
    .trial(|uc: &MyService, instruction| uc.invoke(instruction))
    .experiment_id("baseline-v1")
    .baseline_dir("specs")
    .build()
    .run();

let spec = result.spec();
println!("Observed rate:     {:.4}", spec.statistics.success_rate.observed);
println!("Derived threshold: {:.4}", spec.requirements.min_pass_rate);
```

The API mirrors `ExploreExperiment` and `OptimizeExperiment`:
`.service_contract_id(...)` names the thing being measured, `.service_contract(...)`
takes a factory that builds the instance, and `.trial(...)` receives
`&T` plus the input. Measure's factory takes no arguments — there's no
factor to vary, unlike explore and optimize.

`baseline_dir` sets the output directory for the spec YAML; the default
is `tests/baselines`. For more control (e.g., a pre-configured
`SpecResolver`), use `.spec_resolver(resolver)` instead.

The measure experiment runs a large number of samples (1000+ recommended),
computes the Wilson score confidence interval, and writes a **baseline spec** to
a YAML file. The derived threshold (`min_pass_rate`) is the lower bound of the
95% confidence interval — the most conservative estimate of the true pass rate
given the observed data.

### Step 3: Test

Run a **probabilistic test** that compares current behaviour against the
baseline:

```rust
use feotest::ptest::ProbabilisticTestBuilder;
use feotest::ptest::builder::ThresholdApproach;
use feotest::spec::SpecResolver;

let inputs = standard_instructions();
let resolver = SpecResolver::with_dir("specs");
let mut service_contract = MyServiceContract::new();

let result = ProbabilisticTestBuilder::new("my-service", &inputs, |instruction| {
    service_contract.invoke(instruction)
})
.approach(ThresholdApproach::SampleSizeFirst {
    samples: 100,
    confidence: 0.95,
})
.spec_resolver(resolver)
.run();

assert_eq!(result.verdict_record().verdict(), feotest::verdict::Verdict::Pass);
```

The test loads the baseline spec, derives the threshold, runs the configured
number of samples, and evaluates the result using one-sided hypothesis testing.

---

## Part 3: The three operational approaches

### Threshold-first

"I know the pass rate must be at least X. Run N samples and tell me if it
passes."

```rust
.approach(ThresholdApproach::ThresholdFirst {
    samples: 100,
    min_pass_rate: 0.95,
})
```

Use this when the threshold comes from an SLA, SLO, or policy document. The
framework evaluates whether the observed rate meets the threshold and computes
the implied confidence level. No baseline spec is needed.

### Sample-size-first

"I have budget for N samples. Derive the best threshold the baseline supports
at confidence C."

```rust
.approach(ThresholdApproach::SampleSizeFirst {
    samples: 100,
    confidence: 0.95,
})
.spec_resolver(resolver)  // baseline spec required
```

Use this for the empirical measure-then-test workflow. The framework loads the
baseline spec, computes the Wilson lower bound at the given confidence, and uses
that as the threshold. This is the most common approach for LLM-backed services
where no SLA exists.

### Confidence-first

"I need to detect a 5% degradation with 95% confidence and 80% power. Tell me
how many samples I need."

```rust
.approach(ThresholdApproach::ConfidenceFirst {
    confidence: 0.95,
    min_detectable_effect: 0.05,
    power: 0.80,
})
.spec_resolver(resolver)  // baseline spec required
```

Use this when detection sensitivity is the primary concern. The framework
computes the required sample size using power analysis and then runs that many
samples. This approach can be expensive — use it when you need to guarantee a
specific detection capability.

---

## Part 4: Test intent

Every probabilistic test declares an **intent**:

- **Verification** (default) — an evidential claim. The framework enforces
  statistical feasibility before execution: if the sample size cannot support
  verification at 95% confidence for a normative threshold, the test produces a
  warning. A Pass verdict under Verification intent is a genuine statistical
  claim.

- **Smoke** — a lightweight early-warning check. The framework accepts
  undersized configurations and labels the verdict as non-evidential. Smoke
  tests are intended for frequent monitoring — "is the service obviously
  broken?" — rather than rigorous verification.

```rust
use feotest::model::TestIntent;

ProbabilisticTestBuilder::new("my-service", &inputs, trial)
    .approach(approach)
    .intent(TestIntent::Smoke)  // or TestIntent::Verification (default)
    .run();
```

---

## Part 5: Service contracts in depth

### Contract evaluation

Contracts are evaluated by `ServiceContractOutcome::evaluate`, which times the service
call and runs all postconditions:

```rust
use feotest::contract::{ServiceContract, ServiceContractOutcome};
use feotest::model::ContractViolation;

let contract = ServiceContract::<String, String>::builder()
    .ensure("Has content", |_input, response| {
        if response.is_empty() {
            Err(ContractViolation::new("content", "empty"))
        } else {
            Ok(())
        }
    })
    .build();

let outcome = ServiceContractOutcome::evaluate(&contract, &"request".into(), || {
    my_service.call("request")
});

if outcome.is_success() {
    println!("Trial passed in {:?}", outcome.trial_outcome().elapsed());
} else {
    println!("Violation: {}", outcome.violation().unwrap());
}
```

### Contract vs defect

This distinction is critical:

| Situation | What it is | What happens |
|---|---|---|
| The service returns invalid JSON | **Contract violation** (trial failure) | Counted as a failed trial. Statistical analysis continues. |
| The service call panics | **Software defect** | The run aborts. This is not a statistical event — it is a bug. |

Contract violations are the raw material of probabilistic testing. Defects are
not. `feotest` does not catch panics in trial closures — if your code panics,
the test aborts with a clear diagnostic. This is deliberate: a defective program
must not be statistically analysed.

### Trial closures

In most cases, you will not use `ServiceContractOutcome` directly. Instead, you pass a
**trial closure** to the experiment or test builder. The closure receives an
input string and returns a `TrialOutcome`:

```rust
use feotest::model::{ContractViolation, TrialOutcome};
use std::time::Instant;

let trial = |input: &str| -> TrialOutcome {
    let start = Instant::now();
    let response = my_service.call(input);
    let elapsed = start.elapsed();

    match validate(&response) {
        Ok(()) => TrialOutcome::success(elapsed),
        Err(reason) => TrialOutcome::failure(
            ContractViolation::new("validation", reason),
            elapsed,
        ),
    }
};
```

---

## Part 6: Experiments

### Measure

Establishes an empirical baseline. Runs a large number of samples (1000+
recommended) and writes a spec file containing the observed rate, confidence
interval, and derived threshold.

See [Part 2](#step-2-measure) for a full example.

### Explore

Compares multiple configurations with small sample sizes. Each configuration
is executed independently with the same inputs and trial closure. Use this
before committing to a configuration for measurement.

See [Part 2](#step-1-explore-optional) for a full example.

### Optimize

Iteratively refines a single factor to maximise or minimise a scoring
function. The API shape mirrors `ExploreExperiment`: a factor is a
user-defined type, a `service_contract(factory)` builds an instance from a
factor, and the trial closure runs against the instance. The only
structural difference is how factors are supplied — optimize takes a
single `initial_factor` plus a `FactorMutator` that drives subsequent
factors from history; explore takes them all upfront as `factors(vec)`.

Requires a `Scorer` (evaluates each iteration) and a `FactorMutator<F>`
(produces the next factor from the current one and the history).

```rust
use feotest::experiment::{
    ExecutionResult, FactorMutator, IterationRecord, Objective,
    OptimizeExperiment, Scorer,
};
use feotest::model::TrialOutcome;
use serde::Serialize;

// The factor: what varies between iterations. `Serialize` lets it
// round-trip into the optimization YAML artefact.
#[derive(Clone, Serialize)]
struct Temperature(f64);

// The service contract: what the factory produces from each factor.
struct MyService { temperature: f64 }
impl MyService {
    fn new(temperature: f64) -> Self { Self { temperature } }
    fn call(&self, _instruction: &str) -> TrialOutcome {
        // invoke the service using `self.temperature` and return a TrialOutcome
        TrialOutcome::success(std::time::Duration::ZERO)
    }
}

struct SuccessRateScorer;
impl Scorer for SuccessRateScorer {
    fn score(&self, result: &ExecutionResult) -> f64 {
        result.summary().observed_pass_rate()
    }
}

struct StepMutator;
impl FactorMutator<Temperature> for StepMutator {
    fn mutate(
        &self,
        current: &Temperature,
        _history: &[IterationRecord<Temperature>],
    ) -> Temperature {
        Temperature(current.0 - 0.1)
    }
}

let inputs = vec!["instruction".to_string()];

let result = OptimizeExperiment::builder()
    .service_contract_id("my-service")
    .initial_factor(Temperature(0.9))
    .service_contract(|f: &Temperature| MyService::new(f.0))
    .scorer(SuccessRateScorer)
    .mutator(StepMutator)
    .samples_per_iteration(20)
    .inputs(&inputs)
    .trial(|uc: &MyService, input| uc.call(input))
    .objective(Objective::Maximize)
    .max_iterations(20)
    .no_improvement_window(5)
    .experiment_id("temp-tune-v1")
    .build()
    .run();

if let (Some(iter), Some(score)) = (result.best_iteration(), result.best_score()) {
    println!("Best: iteration {} → score {:.4}", iter, score);
}
```

Factor types are yours to design. Anything with `Clone + Serialize`
works: newtype wrappers around scalars, structs with multiple fields,
enums over variants. The YAML output captures whatever shape the factor
has — scalars render as scalars, strings as strings (block scalars when
multi-line), structs as mappings.

---

## Part 7: Baseline specs

Measure experiments write baseline specs to YAML files. Probabilistic tests
read these files to derive thresholds. The spec format is designed for human
readability and version-control friendliness.

### Format

```yaml
schemaVersion: feotest-spec-1
useCaseId: my-service
generatedAt: 2026-03-27T10:00:00Z
experimentId: baseline-v1

execution:
  samplesPlanned: 1000
  samplesExecuted: 1000
  terminationReason: COMPLETED

requirements:
  minPassRate: 0.9234

statistics:
  successRate:
    observed: 0.9480
    standardError: 0.0070
    confidenceInterval95: [0.9234, 0.9726]
  successes: 948
  failures: 52

cost:
  totalTimeMs: 5200
  avgTimePerSampleMs: 5
  totalTokens: 197000
  avgTokensPerSample: 197
```

### Spec resolution

The `SpecResolver` searches for spec files by service contract ID:

1. The directory specified by `FEOTEST_SPEC_DIR` (if set)
2. The directory passed to `SpecResolver::new` or `SpecResolver::with_dir`

The spec file is expected at `{spec_dir}/{service_contract_id}.yaml`.

### Spec lifecycle

Baseline specs should be committed to version control. They represent a
measured truth about the service at a point in time. When the service changes
materially (new model, new prompt, new infrastructure), re-run the measure
experiment to produce a new baseline.

---

## Part 8: Verdict records

Every probabilistic test produces a `VerdictRecord` — the single source of
truth consumed by all rendering paths (JUnit XML, future HTML reports, console
output).

A verdict record contains:

| Field | Content |
|---|---|
| `identity` | Service contract ID and test name |
| `verdict` | Pass, Fail, or Inconclusive |
| `intent` | Verification or Smoke |
| `execution` | Samples planned/executed, successes, failures, cost |
| `functional` | Pass rate, failure distribution |
| `statistical_analysis` | Confidence level, CI, threshold, z-test, p-value |
| `spec_provenance` | Baseline filename, threshold origin, contract reference |
| `warnings` | Undersized samples, smoke caveats, etc. |

### JUnit XML output

Verdict records can be serialised to JUnit XML for CI integration:

```rust
use feotest::reporting::JunitXmlWriter;

let verdicts = vec![result.verdict_record().clone()];
JunitXmlWriter::write_to_file(Path::new("verdicts.xml"), &verdicts).unwrap();
```

The XML is compatible with GitHub Actions, GitLab CI, Jenkins, and
`cargo-nextest`.

---

## Part 9: Operational controls

### Warmup

Services with cold-start effects (cache warming, connection pool initialisation)
can declare a warmup count. Warmup invocations are executed and discarded before
counted samples begin:

```rust
let config = ExecutionConfig::new(100).with_warmup(5);
// 105 total invocations: 5 discarded, 100 counted
```

### Budgets

Time and token budgets prevent runaway experiments:

```rust
use std::time::Duration;

let config = ExecutionConfig::new(1000)
    .with_time_budget(Duration::from_secs(60))
    .with_token_budget(100_000);
```

Both budgets are checked between samples. The first one to exhaust wins —
time is checked before tokens on each iteration, so ties go to the time
budget. Zero or negative budgets are rejected at configuration time.

#### Budget exhaustion behaviour

When a budget is exhausted, two outcomes are available:

- **`Fail` (default)** — the test is force-failed. No statistical verdict
  is produced from the partial sample set; the failure is signalled by a
  `BUDGET_EXHAUSTED` warning naming the exhausted budget, how much was
  consumed, and how many samples completed versus planned. This is the
  safe default: running out of budget means the test could not complete
  as specified.
- **`EvaluatePartial`** — the framework evaluates the samples completed
  so far with the normal statistical machinery and produces a verdict
  from the partial results. A `BUDGET_EXHAUSTED_PARTIAL` warning
  accompanies the verdict. Choose this for cost-constrained LLM work
  where a statistically valid (if less powerful) answer on 60 samples
  beats a hard fail after 60 of 100 requested.

Zero completed samples always force `Fail` regardless of policy — there
is nothing to evaluate. A `BUDGET_EXHAUSTED_NO_SAMPLES` warning is
emitted in that case.

The policy is settable directly on both the simplified API and the
builder:

```rust
use feotest::{BudgetExhaustedBehavior, ptest::ProbabilisticTest};

let record = ProbabilisticTest::new("my-service", &inputs, trial)
    .samples(1000)
    .threshold(0.95)
    .time_budget(Duration::from_secs(60))
    .on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial)
    .run();
```

#### Run-scoped budgets

A run-scoped budget caps cumulative time or tokens across every test in
a single `cargo test` invocation, on top of any per-method budgets the
individual tests carry. Configure it with environment variables:

```bash
FEOTEST_RUN_TIME_BUDGET_MS=600000 \
FEOTEST_RUN_TOKEN_BUDGET=1000000 \
    cargo test
```

Either variable may be set independently; absence of both leaves the run
uncapped at this scope.

Enrolment is opt-out: when a run-scoped budget is configured, every test
the framework executes is automatically subject to it. Tests do not
declare enrolment or reference the run budget in their own
configuration. Removing the environment variable removes the cap; no
code change is needed.

Per-method and run-scoped budgets compose by first-exhausted-wins. A
test whose own time budget exhausts before the run budget terminates
with the existing per-method exhaustion warning; a test whose run budget
exhausts first terminates with a run-scoped variant of the same warning.
The exhaustion behaviour policy (`Fail` vs `EvaluatePartial`) applies
identically to both.

A test that begins execution after the run budget is already exhausted
short-circuits to zero samples and emits `BUDGET_EXHAUSTED_NO_SAMPLES`.
Tests already mid-flight terminate at their next per-sample check;
`feotest` cannot abort a trial that has already started.

For programmatic configuration — for example, a custom test harness that
derives the budget from a CI parameter — set the budget once before the
first test runs:

```rust
use std::time::Duration;
use feotest::RunBudget;

feotest::controls::run::init(
    RunBudget::new(Some(Duration::from_secs(600)), Some(1_000_000)),
)
.expect("run budget already initialised");
```

`init` errors if the run budget has already been materialised — either
by a prior `init` call or by a test that already consulted the
environment. The budget is set once per process and frozen for the
remainder of the run.

### Token tracking

Trial closures can report token consumption via a `TokenRecorder`:

```rust
use feotest::controls::TokenRecorder;

let recorder = TokenRecorder::new();
recorder.record(150);  // report 150 tokens from this trial
assert_eq!(recorder.total(), 150);
```

For simpler cases, a static `token_charge` per sample can be configured on the
`ExecutionConfig`.

### Pacing

Rate-limiting prevents overwhelming the service under test:

```rust
use feotest::controls::PacingConfig;

let pacing = PacingConfig::new()
    .max_requests_per_second(5.0)
    .min_ms_per_sample(100);
// Floors compose most-restrictive-wins: 5 rps → 200ms, 100ms min →
// effective delay = 200ms.
```

No pacing delay is applied before the first sample; the delay is
inserted between each sample and the next.

#### Capping the delay

For runs where aggressive rate composition could stall progress,
`max_delay_per_sample` caps the proactive pacing delay from above:

```rust
let pacing = PacingConfig::new()
    .max_requests_per_second(1.0)     // 1000ms floor
    .max_delay_per_sample(100);       // cap at 100ms
// Effective delay: min(1000, 100) = 100ms.
```

The cap only activates when a floor is configured — a cap alone
leaves the run unpaced.

---

## Part 10: What's next

`feotest` is in active development. The roadmap includes:

- **Latency dimension** — multi-dimensional verdicts (pass rate and latency)
- **Covariate-aware baseline selection** — matching test conditions to baselines
- **Early termination** — stopping when success or failure is inevitable
- **Sentinel binary** — a standalone CLI for production reliability monitoring
- **`#[feotest]` proc-macro** — ergonomic test declaration
- **Transparent statistics** — detailed statistical reasoning in verdict output
- **HTML reports** — standalone report generation from verdict XML

See [feotest-examples](https://github.com/mavai-org/feotest-examples) for
worked examples demonstrating the framework's current capabilities.
