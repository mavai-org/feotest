//! A reference-matching criterion is terminal and exclusive: `matching` /
//! `matching_equality` are offered only before any `satisfies`. Reaching for a
//! match after an intrinsic postcondition must not compile — the builder's
//! type-state (the `Constrained` state `satisfies` returns) removes the
//! matching terminals.

use feotest::criteria::Criterion;

fn main() {
    let _ = Criterion::<String>::meeting()
        .pass_rate(0.9)
        .name("x")
        .satisfies("non-empty", |_: &String| Ok(()))
        .matching_equality()
        .build();
}
