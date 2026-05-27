//! Sentinel runner and CLI entry point.
//!
//! [`SentinelRunner`] is the execution core the CLI delegates to. It
//! enumerates registered reliability specifications and their
//! content (tests and experiments), dispatches to test or measure mode,
//! and aggregates results into a [`SentinelResult`]. [`run_cli`] is the
//! single public entry point downstream sentinel binaries wire into
//! `main`.

use core::any::Any;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::sentinel::content::{ContentInvoker, content_for};
use crate::sentinel::embedded::DefaultEmbeddedRegistry;
use crate::sentinel::resolver::{baseline_output_from_env, baseline_source_from_env};
use crate::sentinel::sinks::{CompositeVerdictSink, ConsoleVerdictSink, VerdictSink};
use crate::sentinel::{SpecDescriptor, registered_specs};
use crate::spec::BaselineSpec;
use crate::verdict::{Verdict, VerdictRecord};

/// Environment variable that points the default CLI at a webhook
/// endpoint.
///
/// When set — and the `webhook` feature is compiled in — the CLI
/// attaches a [`crate::sentinel::sinks::WebhookVerdictSink`] to the
/// runner in addition to the default console sink.
pub const WEBHOOK_ENV_VAR: &str = "FEOTEST_WEBHOOK_URL";

/// Outcome summary for a full sentinel run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentinelOutcome {
    /// All content completed successfully.
    Pass,
    /// At least one probabilistic test produced a non-pass verdict.
    Fail,
    /// A transient error prevented completion (not a test failure).
    Error,
}

impl SentinelOutcome {
    /// Maps the summary onto a process exit code.
    ///
    /// The sentinel binary uses the simple `0` / `1` pairing. The richer
    /// exit-code taxonomy (`2` inconclusive, `3` error, `4` usage) layers on later.
    #[must_use]
    pub const fn exit_code(self) -> ExitCode {
        match self {
            Self::Pass => ExitCode::SUCCESS,
            Self::Fail | Self::Error => ExitCode::FAILURE,
        }
    }
}

/// Structured result of a single `run` or `measure` invocation.
#[derive(Debug, Clone)]
pub struct SentinelResult {
    /// Verdicts collected during test mode; empty for measure mode.
    pub verdicts: Vec<VerdictRecord>,
    /// Baselines emitted during measure mode; empty for test mode.
    pub emitted_baselines: Vec<BaselineSpec>,
    /// Overall outcome summary.
    pub outcome: SentinelOutcome,
}

/// Execution core that test and measure subcommands delegate to.
///
/// Holds the configuration (baseline source / output destination) the
/// subcommands resolve once at startup and pass to each invocation,
/// plus any verdict sinks to fan each produced verdict out to.
// javai-ref: JVI-PYF4FP9 — do not remove (resolves in javai-orchestrator)
pub struct SentinelRunner {
    source: Option<PathBuf>,
    output: Option<String>,
    sinks: Vec<Box<dyn VerdictSink>>,
}

impl SentinelRunner {
    /// Builds a runner with explicit source / output paths — useful for
    /// tests that need to isolate from the environment.
    #[must_use]
    pub const fn new(source: Option<PathBuf>, output: Option<String>) -> Self {
        Self {
            source,
            output,
            sinks: Vec::new(),
        }
    }

    /// Builds a runner from the ambient environment
    /// ([`baseline_source_from_env`] and [`baseline_output_from_env`]).
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(baseline_source_from_env(), baseline_output_from_env())
    }

    /// Attaches a verdict sink to this runner.
    ///
    /// Sinks are invoked in registration order for every verdict the
    /// runner produces during [`run_tests`](Self::run_tests). A sink
    /// that fails to deliver does not abort the run; the failure is
    /// logged to stderr and the remaining sinks still see the verdict.
    ///
    /// When no sinks are attached, [`run_tests`](Self::run_tests)
    /// installs a default [`ConsoleVerdictSink`] so the CLI's historical
    /// "one tab-separated line per verdict" shape is preserved.
    ///
    /// ```no_run
    /// use feotest::sentinel::{
    ///     ConsoleVerdictSink, FileVerdictSink, SentinelRunner,
    /// };
    ///
    /// let runner = SentinelRunner::from_env()
    ///     .with_sink(Box::new(ConsoleVerdictSink::new()))
    ///     .with_sink(Box::new(FileVerdictSink::new("verdicts.jsonl")));
    /// let _ = runner.run_tests(&[]);
    /// ```
    #[must_use]
    pub fn with_sink(mut self, sink: Box<dyn VerdictSink>) -> Self {
        self.sinks.push(sink);
        self
    }

    /// Runs probabilistic tests for the selected specs.
    ///
    /// An empty `specs` slice runs every registered spec; non-empty
    /// filters by stable name (matching [`Sentinel::name`]).
    ///
    /// Every produced verdict is dispatched to each attached sink
    /// before being pushed onto the returned [`SentinelResult`], so
    /// programmatic callers who don't configure any sink still get the
    /// full collection back.
    ///
    /// # Panics
    ///
    /// Panics via the resolver when an EMPIRICAL-origin test cannot
    /// resolve its baseline through the
    /// external → embedded → panic chain. A test that requires a
    /// baseline but cannot resolve one is a misconfiguration, not a
    /// survivable runtime condition.
    #[must_use]
    pub fn run_tests(mut self, specs: &[String]) -> SentinelResult {
        if self.sinks.is_empty() {
            self.sinks.push(Box::new(ConsoleVerdictSink::new()));
        }
        let mut composite = CompositeVerdictSink::new();
        for sink in self.sinks.drain(..) {
            composite = composite.push(sink);
        }
        let _ = composite.initialize();

        let mut verdicts = Vec::new();
        let mut outcome = SentinelOutcome::Pass;
        for_each_selected_spec(specs, |spec_desc| {
            let spec = (spec_desc.constructor)();
            let any_ref: &dyn Any = spec.as_any();
            for content in content_for(any_ref.type_id()) {
                if let ContentInvoker::Test(invoke) = &content.invoker {
                    let record = invoke(any_ref);
                    if !matches!(record.verdict(), Verdict::Pass) {
                        outcome = SentinelOutcome::Fail;
                    }
                    let _ = composite.accept(&record);
                    verdicts.push(record);
                }
            }
        });
        let _ = composite.finalize();

        SentinelResult {
            verdicts,
            emitted_baselines: Vec::new(),
            outcome,
        }
    }

    /// Runs measure experiments for the selected specs and emits their
    /// baselines to the configured destination.
    ///
    /// # Panics
    ///
    /// Panics if the output destination cannot be opened for writing.
    #[must_use]
    pub fn run_experiments(&self, specs: &[String]) -> SentinelResult {
        let mut baselines = Vec::new();
        for_each_selected_spec(specs, |spec_desc| {
            let spec = (spec_desc.constructor)();
            let any_ref: &dyn Any = spec.as_any();
            for content in content_for(any_ref.type_id()) {
                if let ContentInvoker::Experiment(invoke) = &content.invoker {
                    let baseline = invoke(any_ref);
                    baselines.push(baseline);
                }
            }
        });
        self.emit_baselines(&baselines);
        SentinelResult {
            verdicts: Vec::new(),
            emitted_baselines: baselines,
            outcome: SentinelOutcome::Pass,
        }
    }

    fn emit_baselines(&self, baselines: &[BaselineSpec]) {
        let target = self.output.as_deref().unwrap_or("-");
        if target == "-" {
            for baseline in baselines {
                let yaml = baseline.to_yaml().expect("serialise baseline");
                println!("---");
                println!("{yaml}");
            }
            return;
        }
        let Some(dir) = crate::sentinel::resolver::parse_file_location(target) else {
            eprintln!("unsupported baseline-output URI: {target}");
            return;
        };
        let resolver = crate::spec::SpecResolver::with_dir(&dir);
        for baseline in baselines {
            match resolver.write(
                baseline,
                &[],
                &crate::spec::namer::CovariateProfile::empty(),
            ) {
                Ok(path) => eprintln!("wrote baseline: {}", path.display()),
                Err(err) => eprintln!(
                    "failed to write baseline for {}: {err}",
                    baseline.service_contract_id
                ),
            }
        }
    }

    /// Checks every EMPIRICAL-origin probabilistic test has an embedded
    /// default baseline, or (if `external_override` is set) a baseline
    /// available at that external source. Returns `true` when every
    /// required baseline is present.
    #[must_use]
    pub fn check_baselines(&self, external_override: Option<&Path>) -> bool {
        let embedded = DefaultEmbeddedRegistry;
        let mut all_present = true;
        let source = external_override
            .map(PathBuf::from)
            .or_else(|| self.source.clone());
        for spec_desc in registered_specs() {
            let spec = (spec_desc.constructor)();
            let any_ref: &dyn Any = spec.as_any();
            for content in content_for(any_ref.type_id()) {
                if !content.requires_external_baseline() {
                    continue;
                }
                let profile = crate::spec::namer::CovariateProfile::empty();
                let service_contract_id = format!("{}.{}", spec.name(), content.method_name);
                let query = crate::sentinel::resolver::BaselineQuery {
                    spec_name: spec.name(),
                    method_name: content.method_name,
                    covariate_profile: &profile,
                    service_contract_id: &service_contract_id,
                };
                let resolved = crate::sentinel::resolver::resolve_baseline(
                    &query,
                    source.as_deref(),
                    &embedded,
                );
                if let Err(err) = resolved {
                    eprintln!("missing baseline: {err}");
                    all_present = false;
                }
            }
        }
        all_present
    }
}

fn for_each_selected_spec<F: FnMut(&SpecDescriptor)>(selectors: &[String], mut f: F) {
    for spec_desc in registered_specs() {
        if !selectors.is_empty() && !selectors.iter().any(|s| s == spec_desc.name) {
            continue;
        }
        f(spec_desc);
    }
}

/// Parsed CLI.
#[derive(Parser, Debug)]
#[command(
    name = "sentinel",
    about = "Run reliability specifications outside a test harness"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Test mode (default workload): run probabilistic tests and emit
    /// verdicts.
    Run {
        /// Optional specification names. Empty = run every registered
        /// specification.
        specs: Vec<String>,
    },
    /// Measure mode (opt-in): run measure experiments and emit candidate
    /// baseline specifications to the configured output destination.
    Measure {
        /// Where to write emitted candidate baselines. Accepts a URI
        /// (`file:///…`) or `-` for stdout. Falls back to
        /// `FEOTEST_BASELINE_OUTPUT`, then to stdout.
        #[arg(long, short)]
        output: Option<String>,
        /// Optional specification names. Empty = run every registered
        /// specification's measure experiments.
        specs: Vec<String>,
    },
    /// Enumerate every registered reliability specification.
    List,
    /// Verify every EMPIRICAL-origin probabilistic test has a baseline
    /// (external or embedded). Exits non-zero when any are missing.
    Check {
        /// Override the external baseline directory for this check only.
        #[arg(long)]
        baselines: Option<PathBuf>,
    },
}

/// Parses the process args and dispatches to the runner. Downstream
/// sentinel binaries wire this directly into `main`:
///
/// ```ignore
/// fn main() -> std::process::ExitCode {
///     feotest::sentinel::run_cli()
/// }
/// ```
#[must_use]
pub fn run_cli() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run { specs } => {
            let runner = configure_cli_runner(SentinelRunner::from_env());
            let result = runner.run_tests(&specs);
            eprintln!(
                "{} verdict(s); outcome = {:?}",
                result.verdicts.len(),
                result.outcome
            );
            result.outcome.exit_code()
        }
        Commands::Measure { output, specs } => {
            let runner = SentinelRunner::new(
                baseline_source_from_env(),
                output.or_else(baseline_output_from_env),
            );
            let result = runner.run_experiments(&specs);
            eprintln!("emitted {} baseline(s)", result.emitted_baselines.len());
            result.outcome.exit_code()
        }
        Commands::List => {
            for spec_desc in registered_specs() {
                if spec_desc.description.is_empty() {
                    println!("{}", spec_desc.name);
                } else {
                    println!("{}\t{}", spec_desc.name, spec_desc.description);
                }
            }
            ExitCode::SUCCESS
        }
        Commands::Check { baselines } => {
            let runner = SentinelRunner::from_env();
            if runner.check_baselines(baselines.as_deref()) {
                eprintln!("all empirical-origin tests have resolvable baselines");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

/// Attaches the default CLI sinks to a runner.
///
/// Always installs a [`ConsoleVerdictSink`] so the CLI's historical
/// "one tab-separated line per verdict" output shape is preserved. When
/// `FEOTEST_WEBHOOK_URL` is set in the environment and the `webhook`
/// Cargo feature is compiled in, also attaches a
/// [`crate::sentinel::sinks::WebhookVerdictSink`]. When the env var is
/// set but the feature is not compiled in, warns on stderr and proceeds
/// without the webhook sink.
fn configure_cli_runner(runner: SentinelRunner) -> SentinelRunner {
    let runner = runner.with_sink(Box::new(ConsoleVerdictSink::new()));
    match std::env::var(WEBHOOK_ENV_VAR) {
        Ok(url) if !url.is_empty() => attach_webhook_sink(runner, &url),
        _ => runner,
    }
}

#[cfg(feature = "webhook")]
fn attach_webhook_sink(runner: SentinelRunner, url: &str) -> SentinelRunner {
    use crate::sentinel::sinks::WebhookVerdictSink;
    let sink = WebhookVerdictSink::builder(url).build();
    runner.with_sink(Box::new(sink))
}

#[cfg(not(feature = "webhook"))]
#[allow(
    clippy::needless_pass_by_value,
    reason = "signature mirrors the webhook-enabled variant"
)]
fn attach_webhook_sink(runner: SentinelRunner, url: &str) -> SentinelRunner {
    eprintln!(
        "{WEBHOOK_ENV_VAR}={url} ignored: the `webhook` feature is not compiled into this binary"
    );
    runner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_outcome_exit_codes() {
        assert_eq!(
            format!("{:?}", SentinelOutcome::Pass.exit_code()),
            format!("{:?}", ExitCode::SUCCESS)
        );
        assert_eq!(
            format!("{:?}", SentinelOutcome::Fail.exit_code()),
            format!("{:?}", ExitCode::FAILURE)
        );
        assert_eq!(
            format!("{:?}", SentinelOutcome::Error.exit_code()),
            format!("{:?}", ExitCode::FAILURE)
        );
    }

    #[test]
    fn cli_parses_run_subcommand() {
        let parsed = Cli::try_parse_from(["sentinel", "run", "my_spec"]).expect("parse");
        match parsed.command {
            Commands::Run { specs } => assert_eq!(specs, vec!["my_spec"]),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_measure_with_output() {
        let parsed = Cli::try_parse_from([
            "sentinel",
            "measure",
            "--output",
            "file:///tmp/out",
            "my_spec",
        ])
        .expect("parse");
        match parsed.command {
            Commands::Measure { output, specs } => {
                assert_eq!(output.as_deref(), Some("file:///tmp/out"));
                assert_eq!(specs, vec!["my_spec"]);
            }
            other => panic!("expected Measure, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_list_subcommand() {
        let parsed = Cli::try_parse_from(["sentinel", "list"]).expect("parse");
        assert!(matches!(parsed.command, Commands::List));
    }

    #[test]
    fn cli_parses_check_subcommand() {
        let parsed = Cli::try_parse_from(["sentinel", "check"]).expect("parse");
        assert!(matches!(
            parsed.command,
            Commands::Check { baselines: None }
        ));
    }

    #[test]
    fn cli_parses_check_with_override() {
        let parsed =
            Cli::try_parse_from(["sentinel", "check", "--baselines", "/tmp/b"]).expect("parse");
        match parsed.command {
            Commands::Check {
                baselines: Some(path),
            } => assert_eq!(path, PathBuf::from("/tmp/b")),
            other => panic!("expected Check with path, got {other:?}"),
        }
    }
}
