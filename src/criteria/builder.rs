//! Builders for criteria.
//!
//! A criterion is authored by choosing a target origin
//! ([`Criterion::meeting`](crate::criteria::Criterion::meeting) for a normative
//! rate, [`Criterion::empirical`](crate::criteria::Criterion::empirical) for a
//! baseline-derived one), then a kind (`pass_rate` / `zero_failures`), then a
//! name, optionally a `transforming` step, and one or more `satisfies`
//! postconditions. The chain ends in `build()`, which collapses the criterion
//! into a type-erased [`Criterion`] ready to drop into
//! [`Criteria::of`](crate::criteria::Criteria::of).
//!
//! The `transforming` step changes the value the postconditions judge, so it
//! is type-state: postconditions added after it judge the transformed value,
//! and Rust enforces that ordering at compile time. Postcondition order among
//! themselves is preserved; everything else is set by independent calls.

use std::marker::PhantomData;

use crate::criteria::criterion::{Criterion, CriterionTarget};
use crate::criteria::result::CriterionSampleResult;
use crate::model::{ContractViolation, Outcome};

/// A postcondition check over a value type `V`.
type Check<V> = Box<dyn Fn(&V) -> Outcome + Send + Sync>;

/// A postcondition over a value type `V`: a name plus its check.
type NamedCheck<V> = (String, Check<V>);

/// A transform from the output `O` to a value `T` the postconditions judge.
type Transform<O, T> = Box<dyn Fn(&O) -> Result<T, ContractViolation> + Send + Sync>;

/// A matcher judging the actual output against the expected value:
/// `(expected, actual) -> Outcome`.
type Matcher<O> = Box<dyn Fn(&O, &O) -> Outcome + Send + Sync>;

impl<O: 'static> Criterion<O> {
    /// Begins a **normative** criterion — a target asserted from a document.
    #[must_use]
    pub const fn meeting() -> NormativeCriterion<O> {
        NormativeCriterion::new()
    }

    /// Begins an **empirical** criterion — a target derived from a baseline.
    #[must_use]
    pub const fn empirical() -> EmpiricalCriterion<O> {
        EmpiricalCriterion::new()
    }
}

/// Entry builder for a **normative** criterion — a target asserted from a
/// document. Returned by [`Criterion::meeting`](crate::criteria::Criterion::meeting).
pub struct NormativeCriterion<O> {
    marker: PhantomData<O>,
}

impl<O: 'static> NormativeCriterion<O> {
    pub(crate) const fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }

    /// A criterion that must meet an explicit pass rate.
    ///
    /// # Panics
    ///
    /// Panics if `rate` is not in the open interval `(0, 1)`.
    #[must_use]
    pub fn pass_rate(self, rate: f64) -> CriterionBuild<O> {
        assert!(
            rate > 0.0 && rate < 1.0,
            "pass rate must be in (0, 1), got {rate}"
        );
        CriterionBuild::new(CriterionTarget::NormativeRate(rate))
    }

    /// An observational criterion that must show zero failures.
    #[must_use]
    pub fn zero_failures(self) -> CriterionBuild<O> {
        CriterionBuild::new(CriterionTarget::ZeroFailures)
    }
}

/// Entry builder for an **empirical** criterion — a target derived from a
/// measured baseline. Returned by
/// [`Criterion::empirical`](crate::criteria::Criterion::empirical).
pub struct EmpiricalCriterion<O> {
    marker: PhantomData<O>,
}

impl<O: 'static> EmpiricalCriterion<O> {
    pub(crate) const fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }

    /// A criterion whose pass-rate target is derived from a baseline.
    #[must_use]
    pub fn pass_rate(self) -> CriterionBuild<O> {
        CriterionBuild::new(CriterionTarget::EmpiricalRate)
    }

    /// An observational criterion that must show zero failures.
    #[must_use]
    pub fn zero_failures(self) -> CriterionBuild<O> {
        CriterionBuild::new(CriterionTarget::ZeroFailures)
    }
}

/// Builder-state marker: no `satisfies` postcondition yet, so the
/// reference-matching terminals (`matching` / `matching_equality`) are still
/// reachable. The initial state of every [`CriterionBuild`].
pub struct Open;

/// Builder-state marker: a `satisfies` postcondition is present.
///
/// The criterion now judges intrinsic properties of the output, and the
/// reference-matching terminals are no longer offered. A matching criterion is
/// terminal and exclusive; the type-state makes mixing the two a compile error.
pub struct Constrained;

/// Builder for a criterion whose postconditions judge the output `O` directly.
///
/// Add a name and `satisfies` postconditions, optionally switch to a
/// transformed value with `transforming`, then `build`. Before the first
/// `satisfies`, the criterion may instead be turned into a reference-matching
/// one with `matching` / `matching_equality`; these terminals are reachable
/// only in the [`Open`] state, so the two styles cannot be mixed on one
/// criterion (a compile error, not a runtime check).
// javai-ref: JVI-BD4F1AB — do not remove (resolves in javai-orchestrator)
pub struct CriterionBuild<O, S = Open> {
    target: CriterionTarget,
    name: Option<String>,
    postconditions: Vec<NamedCheck<O>>,
    _state: PhantomData<S>,
}

impl<O: 'static> CriterionBuild<O, Open> {
    fn new(target: CriterionTarget) -> Self {
        Self {
            target,
            name: None,
            postconditions: Vec::new(),
            _state: PhantomData,
        }
    }

    /// Judges the actual output against the per-sample expected value with a
    /// custom matcher. The matcher receives `(expected, actual)` and returns
    /// `Ok(())` on equivalence or `Err(ContractViolation)` on mismatch; the
    /// violation's check name flows into the failure distribution.
    ///
    /// Terminal and exclusive: a reference-matching criterion is purely an
    /// equivalence judgement, so it carries no `satisfies` / `transforming`
    /// clauses. The builder enforces this through its type-state — `matching`
    /// is offered only before any `satisfies`, and the returned
    /// [`MatchingBuild`] offers only `name` / `build` — so mixing the two
    /// styles is a compile error. Pair a match with an intrinsic check by
    /// bundling a separate criterion in
    /// [`Criteria::of`](crate::criteria::Criteria::of).
    #[must_use]
    pub fn matching(
        self,
        matcher: impl Fn(&O, &O) -> Outcome + Send + Sync + 'static,
    ) -> MatchingBuild<O> {
        MatchingBuild {
            target: self.target,
            name: self.name,
            matcher: Box::new(matcher),
        }
    }

    /// Shorthand for [`matching`](Self::matching) with an equality matcher.
    ///
    /// On mismatch the sample fails as
    /// `ContractViolation::new("not-equal", "expected … but got …")`.
    #[must_use]
    pub fn matching_equality(self) -> MatchingBuild<O>
    where
        O: PartialEq + std::fmt::Debug,
    {
        self.matching(|expected: &O, actual: &O| {
            if expected == actual {
                Ok(())
            } else {
                Err(ContractViolation::new(
                    "not-equal",
                    format!("expected {expected:?} but got {actual:?}"),
                ))
            }
        })
    }
}

impl<O: 'static, S> CriterionBuild<O, S> {
    /// Names the criterion. Required before `build`.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Adds a named postcondition judging the output `O`.
    ///
    /// The first `satisfies` moves the builder into the [`Constrained`] state,
    /// past which the reference-matching terminals are no longer offered.
    #[must_use]
    pub fn satisfies(
        self,
        name: impl Into<String>,
        check: impl Fn(&O) -> Outcome + Send + Sync + 'static,
    ) -> CriterionBuild<O, Constrained> {
        let mut postconditions = self.postconditions;
        postconditions.push((name.into(), Box::new(check)));
        CriterionBuild {
            target: self.target,
            name: self.name,
            postconditions,
            _state: PhantomData,
        }
    }

    /// Switches the criterion to judge a transformed value `T`.
    ///
    /// The transform receives the output and returns the value the subsequent
    /// `satisfies` postconditions judge, or a [`ContractViolation`] if it
    /// cannot — a failed transform fails the whole criterion for that sample
    /// (counted in its denominator), it does not abort the run. Postconditions
    /// already added (on `O`) are retained and checked before the transform.
    #[must_use]
    pub fn transforming<T: 'static>(
        self,
        transform: impl Fn(&O) -> Result<T, ContractViolation> + Send + Sync + 'static,
    ) -> TransformingBuild<O, T> {
        TransformingBuild {
            target: self.target,
            name: self.name,
            pre: self.postconditions,
            transform: Box::new(transform),
            postconditions: Vec::new(),
        }
    }

    /// Collapses the chain into a type-erased [`Criterion`].
    ///
    /// # Panics
    ///
    /// Panics if no name was set (`name(..)`).
    #[must_use]
    pub fn build(self) -> Criterion<O> {
        let name = require_name(self.name);
        let post_names = names_of(&self.postconditions);
        let checks: Vec<_> = self.postconditions.into_iter().map(|(_, c)| c).collect();
        let report_name = name.clone();
        let evaluate = Box::new(move |output: &O, _expected: Option<&O>| {
            run_checks(&report_name, output, &checks).unwrap_or_else(|| pass(&report_name))
        });
        Criterion::new(name, self.target, post_names, evaluate)
    }
}

/// Builder for a criterion that transforms the output into `T` before judging.
/// Reached via [`CriterionBuild::transforming`].
pub struct TransformingBuild<O, T> {
    target: CriterionTarget,
    name: Option<String>,
    pre: Vec<NamedCheck<O>>,
    transform: Transform<O, T>,
    postconditions: Vec<NamedCheck<T>>,
}

impl<O: 'static, T: 'static> TransformingBuild<O, T> {
    /// Names the criterion. Required before `build`.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Adds a named postcondition judging the transformed value `T`.
    #[must_use]
    pub fn satisfies(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&T) -> Outcome + Send + Sync + 'static,
    ) -> Self {
        self.postconditions.push((name.into(), Box::new(check)));
        self
    }

    /// Collapses the chain into a type-erased [`Criterion`]; the transformed
    /// type `T` does not escape.
    ///
    /// # Panics
    ///
    /// Panics if no name was set (`name(..)`).
    #[must_use]
    pub fn build(self) -> Criterion<O> {
        let name = require_name(self.name);
        let mut post_names = names_of(&self.pre);
        post_names.extend(names_of(&self.postconditions));

        let pre_checks: Vec<_> = self.pre.into_iter().map(|(_, c)| c).collect();
        let post_checks: Vec<_> = self.postconditions.into_iter().map(|(_, c)| c).collect();
        let transform = self.transform;
        let report_name = name.clone();

        let evaluate = Box::new(move |output: &O, _expected: Option<&O>| {
            if let Some(failed) = run_checks(&report_name, output, &pre_checks) {
                return failed;
            }
            match transform(output) {
                Err(violation) => CriterionSampleResult::fail(&report_name, violation),
                Ok(value) => run_checks(&report_name, &value, &post_checks)
                    .unwrap_or_else(|| pass(&report_name)),
            }
        });
        Criterion::new(name, self.target, post_names, evaluate)
    }
}

/// Builder for a reference-matching criterion.
///
/// It judges the output against the per-sample expected value through a matcher,
/// rather than via free-form postconditions. Reached via
/// [`CriterionBuild::matching`] / [`CriterionBuild::matching_equality`] and
/// **terminal** — only `name` and `build` remain, so a matching criterion
/// cannot also carry `satisfies` / `transforming` clauses.
// javai-ref: JVI-3P2SEQ4 — do not remove (resolves in javai-orchestrator)
pub struct MatchingBuild<O> {
    target: CriterionTarget,
    name: Option<String>,
    matcher: Matcher<O>,
}

impl<O: 'static> MatchingBuild<O> {
    /// Names the criterion. Required before `build`.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Collapses the chain into a type-erased [`Criterion`].
    ///
    /// # Panics
    ///
    /// Panics if no name was set (`name(..)`).
    #[must_use]
    pub fn build(self) -> Criterion<O> {
        let name = require_name(self.name);
        let post_names = vec![name.clone()];
        let matcher = self.matcher;
        let report_name = name.clone();
        let evaluate = Box::new(move |output: &O, expected: Option<&O>| {
            let Some(expected) = expected else {
                panic!(
                    "criterion '{report_name}' is a reference-matching criterion but the contract supplied no expected value for this sample; override ServiceContract::expected or replace matching(..) with satisfies(..)"
                );
            };
            match matcher(expected, output) {
                Ok(()) => pass(&report_name),
                Err(violation) => CriterionSampleResult::fail(&report_name, violation),
            }
        });
        Criterion::new(name, self.target, post_names, evaluate)
    }
}

/// Runs checks in order; returns the first failure as a result, or `None` if
/// all passed.
fn run_checks<V>(criterion: &str, value: &V, checks: &[Check<V>]) -> Option<CriterionSampleResult> {
    for check in checks {
        if let Err(violation) = check(value) {
            return Some(CriterionSampleResult::fail(criterion, violation));
        }
    }
    None
}

fn pass(criterion: &str) -> CriterionSampleResult {
    CriterionSampleResult::pass(criterion)
}

fn names_of<V>(postconditions: &[NamedCheck<V>]) -> Vec<String> {
    postconditions.iter().map(|(n, _)| n.clone()).collect()
}

fn require_name(name: Option<String>) -> String {
    name.unwrap_or_else(|| panic!("a criterion requires a name — call name(..)"))
}
