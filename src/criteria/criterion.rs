//! A single assembled criterion: a named, type-erased judgement on a sample.

use crate::criteria::result::CriterionSampleResult;

/// A criterion's type-erased evaluation: the sample's output and the optional
/// per-sample expected value in, a two-valued sample result out. Any transform
/// is collapsed into this closure at build time, so the criterion's whole
/// behaviour — postconditions or a reference matcher — lives behind one type.
type Evaluate<O> = Box<dyn Fn(&O, Option<&O>) -> CriterionSampleResult + Send + Sync>;

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
/// Built through [`Criterion::meeting`](crate::criteria::Criterion::meeting) or
/// [`Criterion::empirical`](crate::criteria::Criterion::empirical). Any
/// transform step is collapsed into the evaluation closure at build time, so
/// the transformed value type does not escape into this type — every criterion
/// over the same `O` has the same type and can sit in one collection.
///
/// The evaluation closure takes the sample's output and the optional per-sample
/// expected value. Postcondition criteria ignore the expected value; a
/// reference-matching criterion routes it through its matcher (and treats a
/// missing expected value as a defect).
// javai-ref: JVI-JGG2K8= — do not remove (resolves in javai-orchestrator)
// javai-ref: JVI-K90P6S1 — do not remove (resolves in javai-orchestrator)
pub struct Criterion<O> {
    name: String,
    target: CriterionTarget,
    postconditions: Vec<String>,
    evaluate: Evaluate<O>,
}

impl<O> Criterion<O> {
    pub(crate) fn new(
        name: String,
        target: CriterionTarget,
        postconditions: Vec<String>,
        evaluate: Evaluate<O>,
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

    /// Evaluates the criterion against one sample's output and the optional
    /// per-sample expected value, yielding a two-valued result. Postconditions
    /// are checked in declaration order and the first failure (or a failed
    /// transform) determines the `Fail` reason; this short-circuit is *within*
    /// the criterion only. Postcondition criteria ignore `expected`; a
    /// reference-matching criterion routes it through its matcher.
    ///
    /// # Panics
    ///
    /// A reference-matching criterion panics if `expected` is `None` — a
    /// contract that declares such a criterion must supply a reference value
    /// for every sample (via [`ServiceContract::expected`]). This is a defect,
    /// not a sample failure.
    ///
    /// [`ServiceContract::expected`]: crate::service_contract::ServiceContract::expected
    #[must_use]
    pub fn evaluate(&self, output: &O, expected: Option<&O>) -> CriterionSampleResult {
        (self.evaluate)(output, expected)
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
