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
**explore experiment** to compare candidates:

```rust
use feotest::experiment::ExploreExperiment;
use feotest::model::TrialOutcome;

let inputs = vec!["Add 2 apples".to_string(), "Remove the milk".to_string()];
let mut use_case = MyUseCase::new();

let result = ExploreExperiment::new("my-service", 20, &inputs, |instruction| {
    use_case.invoke(instruction)
})
.config("model-a", || { /* set model A */ })
.config("model-b", || { /* set model B */ })
.run();

for config in result.configs() {
    let rate = config.execution().summary().observed_pass_rate();
    println!("{}: {:.1}%", config.name(), rate * 100.0);
}
```

Explore experiments use small sample sizes (10–20 per configuration). They are
not statistically rigorous — they are for rapid filtering.

### Step 2: Measure

Once you have chosen a configuration, run a **measure experiment** to establish
a statistical baseline:

```rust
use feotest::experiment::MeasureExperiment;
use feotest::spec::SpecResolver;

let inputs = standard_instructions();
let resolver = SpecResolver::with_dir("specs");
let mut use_case = MyUseCase::new();

let result = MeasureExperiment::new("my-service", 1000, &inputs, |instruction| {
    use_case.invoke(instruction)
})
.with_experiment_id("baseline-v1")
.with_spec_resolver(resolver)
.run();

let spec = result.spec();
println!("Observed rate:     {:.4}", spec.statistics.success_rate.observed);
println!("Derived threshold: {:.4}", spec.requirements.min_pass_rate);
```

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
let mut use_case = MyUseCase::new();

let result = ProbabilisticTestBuilder::new("my-service", &inputs, |instruction| {
    use_case.invoke(instruction)
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

Contracts are evaluated by `UseCaseOutcome::evaluate`, which times the service
call and runs all postconditions:

```rust
use feotest::contract::{ServiceContract, UseCaseOutcome};
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

let outcome = UseCaseOutcome::evaluate(&contract, &"request".into(), || {
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

In most cases, you will not use `UseCaseOutcome` directly. Instead, you pass a
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

Iteratively refines a single control factor to maximise or minimise a scoring
function. Requires a `Scorer` (evaluates each iteration) and a `FactorMutator`
(produces the next factor value).

```rust
use feotest::experiment::optimize::{Scorer, FactorMutator, Objective};
use feotest::experiment::{OptimizeExperiment, ExecutionResult};
use feotest::usecase::FactorValue;

struct SuccessRateScorer;
impl Scorer for SuccessRateScorer {
    fn score(&self, result: &ExecutionResult) -> f64 {
        result.summary().observed_pass_rate()
    }
}

struct StepMutator;
impl FactorMutator for StepMutator {
    fn mutate(&self, current: &FactorValue, _history: &[_]) -> FactorValue {
        if let FactorValue::Float(v) = current {
            FactorValue::Float(v - 0.1)
        } else {
            current.clone()
        }
    }
}
```

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

The `SpecResolver` searches for spec files by use case ID:

1. The directory specified by `FEOTEST_SPEC_DIR` (if set)
2. The directory passed to `SpecResolver::new` or `SpecResolver::with_dir`

The spec file is expected at `{spec_dir}/{use_case_id}.yaml`.

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
| `identity` | Use case ID and test name |
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

When a budget is exhausted, the framework either fails the test or evaluates the
partial results, depending on the configured `BudgetExhaustedBehavior`.

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
    .with_max_requests_per_second(5.0)
    .with_max_requests_per_minute(120.0);
// Most restrictive constraint wins: 200ms between requests
```

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

See [feotest-examples](https://github.com/javai-org/feotest-examples) for
worked examples demonstrating the framework's current capabilities.
