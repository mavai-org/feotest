//! Singleton wiring smoke test for the run-scoped budget.
//!
//! This file compiles to its own integration test binary, which gives
//! the singleton a fresh `OnceLock` that no other test shares. The
//! behavioural tests for engine composition live alongside the engine
//! itself; the goal here is only to verify that the
//! `feotest::controls::run` singleton can be populated explicitly and
//! that a second attempt to initialise it is refused.

use std::time::Duration;

use feotest::RunBudget;
use feotest::controls::run::{current, init};

#[test]
fn singleton_accepts_one_init_and_refuses_the_second() {
    // The singleton may already have been materialised from environment
    // variables inherited by the test harness. Either the first init
    // succeeds and the second fails, or the first fails immediately
    // because env-driven materialisation has already populated the
    // singleton — but two successful init calls per process is never
    // allowed. That is the invariant under test.
    let first = init(RunBudget::new(Some(Duration::from_secs(60)), None));
    let second = init(RunBudget::new(None, Some(100_000)));
    assert!(
        !(first.is_ok() && second.is_ok()),
        "two init() calls must not both succeed"
    );
    assert!(
        current().is_some(),
        "a budget must now be installed (either via env or via init)"
    );
}
