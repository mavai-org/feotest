# CLAUDE.md

This file provides guidance to Claude Code when working with code in this repository.

## Project Overview

See `README.md` for the full project description, positioning, and architectural direction.

`feotest` is a Rust library crate for probabilistic testing of stochastic services. It provides statistical inference machinery for determining whether a stochastic service meets a specified quality threshold, based on repeated empirical trials modelled as Bernoulli experiments.

The project is early-stage. The immediate priority is a correct, well-tested statistics and inference core.

## Build and Test Commands

```bash
# Build the project
cargo build

# Run all tests
cargo test

# Run a single test by name (substring match)
cargo test test_name

# Run tests in a specific module
cargo test module_name::

# Run tests with output shown
cargo test -- --nocapture

# Run only doc-tests
cargo test --doc

# Check without building (faster feedback)
cargo check

# Lint
cargo clippy

# Format
cargo fmt

# Format check (CI-friendly)
cargo fmt -- --check
```

## Architecture

### Module Structure

```
src/
├── lib.rs              # Crate root: module declarations and crate-level docs
├── statistics/         # Confidence intervals, threshold derivation, hypothesis testing
├── model/              # Domain types: trials, outcomes, sample aggregates
├── verdict/            # Mapping statistical results to pass/fail decisions
├── spec/               # Baseline specifications from empirical measurement
├── contract/           # Success/failure criteria for individual invocations
├── controls/           # Operational safeguards: warm-up, budgets, catastrophic halt
├── experiment/         # Experiment workflows for baseline establishment
├── reporting/          # Structured output of verdicts and diagnostics
└── usecase/            # The named unit of work under test
```

### Design Principles

- The statistics/inference core is the foundation; everything else builds on it.
- Statistics and domain logic must not depend on reporting or test-runner concerns. Dependencies point toward the core logic, not back out of it.
- Runner integration and proc-macro ergonomics come later.
- Module boundaries reflect domain concepts, not implementation convenience.
- Public API surface is minimal until designs stabilise.
- Start with a single library crate. Introduce a Cargo workspace only once there are genuinely separate packages to manage together.

### Statistics Library Strategy

- Use `statrs` for distribution math and quantiles (normal distribution CDF, inverse CDF).
- Implement Wilson score interval and Bernoulli-specific formulas directly in our code — these are core to the framework's semantics and must remain transparent and reviewable.
- This gives trusted numerical primitives without hiding the framework's defining logic inside a third-party crate.

## Conventions

### Language and Toolchain

- Rust edition 2024, minimum supported Rust version 1.85.
- All code must pass `cargo clippy` with the lint configuration in `Cargo.toml` (clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo all at warn level).
- All code must be formatted with `cargo fmt` (default rustfmt settings).
- `unsafe` code is forbidden (`unsafe_code = "forbid"` in `Cargo.toml`).

### Code Style

- **Idiomatic Rust**: prefer standard library types and patterns. Use `Option` for optional values. Prefer iterators over manual loops. Use `impl Trait` in argument position for flexibility, concrete types in return position for clarity.
- **Explicit naming**: names should be self-documenting. Avoid abbreviations except where universally understood (`ctx`, `config`). Prefer descriptive names over short ones. Public APIs should read like a domain model: `WilsonLowerBound`, `ThresholdOrigin`, `sample_size_for_confidence`, not abbreviated or overly clever names.
- **Small, coherent modules**: each module should have a single clear responsibility. Prefer many small files over few large ones.
- **Minimal public surface**: default to `pub(crate)` visibility. Only make items `pub` when they form part of the intended public API. Re-export key public types from `lib.rs` once the API stabilises.
- **Type-driven design**: use newtypes and enums to make invalid states unrepresentable. Prefer `struct` with named fields over tuples for domain concepts. Derive standard traits (`Debug`, `Clone`, `PartialEq`) where appropriate.
- **`Result` is for genuine runtime uncertainty only**: `Result` is reserved for conditions outside the program's control — a stochastic service that may not deliver, a network call that may fail, or a user-provided file that has not yet been placed in the requisite folder. The deciding question is *whose fault is it?* If a required application config file is missing because the developer failed to ship it, that is a defect — assert and abort. If a file is missing because an end user has not yet supplied it, that is a legitimate runtime condition — return a `Result`. A violated precondition (e.g., a confidence level outside (0, 1), successes exceeding trials) is always a programming error, not a runtime condition. There is no case for "handling" a defective program — it must abort with a clear message. Use `assert!` with descriptive messages for preconditions. Do not wrap deterministic logic in `Result` types.
- **`unwrap()` in library code**: acceptable only where failure is logically impossible and the invariant is self-evident (e.g., constructing a standard normal distribution with known-good constants). `unwrap()` is freely acceptable in tests.
- **Doc comments**: all public items must have `///` doc comments. Use `//!` module-level docs in `mod.rs` files. Include examples in doc comments where the usage is not obvious. Document `Panics` sections for functions with preconditions.

### Testing

- Tests live in a `#[cfg(test)] mod tests` block at the bottom of the file they test, or in a `tests/` directory for integration tests.
- Use `assert!`, `assert_eq!`, `assert_ne!` from the standard library. Use `approx` for floating-point comparisons.
- Test names should read as sentences: `fn rejects_negative_sample_count()`, `fn confidence_interval_contains_true_proportion()`.
- Statistical tests should use known analytical results or validated reference values, not approximate "looks right" checks.
- Use `proptest` for property-based testing of invariants: monotonicity, range constraints, convergence properties.
- Use `insta` for snapshot testing of report formatting only, not for probability logic.
- All non-trivial functionality must be covered by unit tests.
- Use `cargo-nextest` as the test runner in CI for faster execution and JUnit XML output. Use `quick-junit` only if the framework needs to emit its own report artifacts programmatically.
- Use `trybuild` for compile-time diagnostic testing once macros are introduced.

### Dependencies

- Add dependencies deliberately. Prefer the standard library where it suffices.
- Every dependency must justify its inclusion: correctness, significant complexity reduction, or ecosystem convention.
- Pin major versions in `Cargo.toml` to avoid unexpected breakage.

Recommended baseline:

```toml
[dependencies]
statrs = "0.18"

[dev-dependencies]
approx = "0.5"
proptest = "1"
insta = "1"
trybuild = "1"
```

### Documentation tone

- This is a Rust project. Documentation and comments should be written for a Rust audience. Do not reference Java, JUnit, punit, or the project's Java heritage in code comments or doc strings. The Rust developers using this framework do not need to know about its origins.

### Git

- Commit messages should be concise and describe the *why*, not just the *what*.
- Keep commits focused: one logical change per commit.
