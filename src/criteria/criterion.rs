//! A single assembled criterion: a named, type-erased judgement on a sample.

use crate::criteria::result::CriterionSampleResult;

/// The target a criterion is judged against.
///
/// `meeting()` produces a normative target whose rate is stated explicitly;
/// `empirical()` produces a target whose rate is derived from a measured
/// baseline (resolved downstream, when the verdict is computed); the
/// zero-failures kind is observational and carries no rate.
#[derive(Debug, Clone, PartialEq)]
pub enum CriterionTarget {
    /// A normative pass rate asserted from a document (`meeting().pass_rate(r)`).
    NormativeRate(f64),
    /// A pass rate derived from a measured baseline (`empirical().pass_rate()`).
    EmpiricalRate,
    /// An observational zero-failures criterion (`*.zero_failures()`).
    ZeroFailures,
}

/// One assembled criterion over a contract output type `O`.
///
/// Built through [`Criteria::meeting`](crate::criteria::Criteria::meeting) or
/// [`Criteria::empirical`](crate::criteria::Criteria::empirical). Any
/// transform step is collapsed into the evaluation closure at build time, so
/// the transformed value type does not escape into this type — every criterion
/// over the same `O` has the same type and can sit in one collection.
pub struct Criterion<O> {
    name: String,
    target: CriterionTarget,
    postconditions: Vec<String>,
    #[allow(
        clippy::type_complexity,
        reason = "the evaluation closure is the criterion's whole behaviour, type-erased over any transform"
    )]
    evaluate: Box<dyn Fn(&O) -> CriterionSampleResult + Send + Sync>,
}

impl<O> Criterion<O> {
    pub(crate) fn new(
        name: String,
        target: CriterionTarget,
        postconditions: Vec<String>,
        evaluate: Box<dyn Fn(&O) -> CriterionSampleResult + Send + Sync>,
    ) -> Self {
        Self {
            name,
            target,
            postconditions,
            evaluate,
        }
    }

    /// The criterion's name (unique within a contract's criteria).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The target this criterion is judged against.
    #[must_use]
    pub const fn target(&self) -> &CriterionTarget {
        &self.target
    }

    /// The names of this criterion's postconditions, in declaration order.
    #[must_use]
    pub fn postconditions(&self) -> &[String] {
        &self.postconditions
    }

    /// Evaluates the criterion against one sample's output, yielding a
    /// two-valued result. Postconditions are checked in declaration order and
    /// the first failure (or a failed transform) determines the `Fail` reason;
    /// this short-circuit is *within* the criterion only.
    #[must_use]
    pub fn evaluate(&self, output: &O) -> CriterionSampleResult {
        (self.evaluate)(output)
    }
}

impl<O> std::fmt::Debug for Criterion<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Criterion")
            .field("name", &self.name)
            .field("target", &self.target)
            .field("postconditions", &self.postconditions)
            .finish_non_exhaustive()
    }
}
