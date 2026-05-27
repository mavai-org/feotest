//! Method-level content descriptors for sentinels.
//!
//! A [`ContentDescriptor`] represents one probabilistic test or measure
//! experiment method declared inside a `#[sentinel_impl]` block. The
//! descriptors are submitted to an `inventory` registry at link time and
//! enumerated by the sentinel runner at execution time.

use core::any::{Any, TypeId};

use crate::model::ThresholdOrigin;
use crate::spec::BaselineSpec;
use crate::verdict::VerdictRecord;

/// A descriptor for one invokable unit of work inside a reliability
/// specification — either a probabilistic test or a measure experiment.
pub struct ContentDescriptor {
    /// Returns the [`TypeId`] of the spec struct this descriptor belongs to.
    ///
    /// Used by the runner to filter the global inventory down to descriptors
    /// that match a given spec instance.
    pub spec_type_id: fn() -> TypeId,
    /// Name of the method this descriptor wraps. Stable identifier for CLI
    /// selection, reporting, and cross-referencing.
    pub method_name: &'static str,
    /// What kind of content this is — test or experiment — plus its
    /// configuration.
    pub kind: ContentKind,
    /// Invoker that runs the method given the owning spec.
    pub invoker: ContentInvoker,
}

impl ContentDescriptor {
    /// Returns `true` when this descriptor represents a probabilistic test
    /// whose threshold origin requires an externally-provided baseline
    /// specification.
    #[must_use]
    pub const fn requires_external_baseline(&self) -> bool {
        match &self.kind {
            ContentKind::ProbabilisticTest(cfg) => cfg.requires_external_baseline(),
            ContentKind::MeasureExperiment(_) => false,
        }
    }
}

/// The two kinds of content a sentinel can declare.
pub enum ContentKind {
    /// A probabilistic test: runs a trial closure many times and produces
    /// a [`VerdictRecord`].
    ProbabilisticTest(ProbabilisticTestConfig),
    /// A measure experiment: runs a trial closure many times and produces
    /// a [`BaselineSpec`] that can be used as a baseline by a probabilistic
    /// test.
    MeasureExperiment(MeasureExperimentConfig),
}

/// Configuration for a probabilistic-test method.
///
/// Mirrors the shape of arguments accepted by the existing free-function
/// `#[probabilistic_test]` macro, narrowed to the subset used by the sentinel
/// runtime.
#[derive(Debug, Clone)]
pub struct ProbabilisticTestConfig {
    /// Origin of the test's threshold — determines whether the test needs
    /// an external baseline (`Empirical`) or not (`Sla` / `Slo` / `Policy` /
    /// `Unspecified`).
    pub origin: ThresholdOrigin,
    /// Explicit minimum pass-rate threshold, for threshold-first tests.
    pub threshold: Option<f64>,
    /// Number of samples (only meaningful for threshold-first and
    /// sample-size-first approaches).
    pub samples: Option<u32>,
    /// Baseline-spec resolver key — the method name of the paired measure
    /// experiment, if any.
    pub baseline_method: Option<&'static str>,
}

impl ProbabilisticTestConfig {
    /// Returns `true` exactly when this test's threshold is derived from a
    /// baseline produced by a measure experiment. Normative origins
    /// inline their threshold in the declaration and do not require an
    /// external baseline.
    #[must_use]
    pub const fn requires_external_baseline(&self) -> bool {
        matches!(self.origin, ThresholdOrigin::Empirical)
    }
}

/// Configuration for a measure-experiment method.
#[derive(Debug, Clone)]
pub struct MeasureExperimentConfig {
    /// Number of samples the experiment collects.
    pub samples: u32,
    /// Name of the probabilistic test method whose baseline this experiment
    /// produces, if declared.
    pub baseline_for: Option<&'static str>,
}

/// Invoker that runs a content method against a spec instance.
///
/// The variants close over method-specific configuration at code-generation
/// time, so the runner only needs to hand over the spec reference. The spec
/// is passed as `&dyn Any` via [`crate::sentinel::Sentinel::as_any`];
/// the invoker downcasts to the concrete type before invoking.
pub enum ContentInvoker {
    /// Runs a probabilistic test and returns its verdict.
    Test(fn(&dyn Any) -> VerdictRecord),
    /// Runs a measure experiment and returns its baseline specification.
    Experiment(fn(&dyn Any) -> BaselineSpec),
}

inventory::collect!(ContentDescriptor);

/// Iterates every content descriptor registered in this binary, across all
/// specs. Most callers should prefer [`content_for`].
pub fn registered_content() -> impl Iterator<Item = &'static ContentDescriptor> {
    inventory::iter::<ContentDescriptor>()
}

/// Iterates every content descriptor belonging to a particular spec type.
pub fn content_for(type_id: TypeId) -> impl Iterator<Item = &'static ContentDescriptor> {
    registered_content().filter(move |d| (d.spec_type_id)() == type_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sla_origin_does_not_require_baseline() {
        let cfg = ProbabilisticTestConfig {
            origin: ThresholdOrigin::Sla,
            threshold: Some(0.95),
            samples: Some(200),
            baseline_method: None,
        };
        assert!(!cfg.requires_external_baseline());
    }

    #[test]
    fn slo_origin_does_not_require_baseline() {
        let cfg = ProbabilisticTestConfig {
            origin: ThresholdOrigin::Slo,
            threshold: Some(0.95),
            samples: Some(200),
            baseline_method: None,
        };
        assert!(!cfg.requires_external_baseline());
    }

    #[test]
    fn policy_origin_does_not_require_baseline() {
        let cfg = ProbabilisticTestConfig {
            origin: ThresholdOrigin::Policy,
            threshold: Some(0.95),
            samples: Some(200),
            baseline_method: None,
        };
        assert!(!cfg.requires_external_baseline());
    }

    #[test]
    fn empirical_origin_requires_baseline() {
        let cfg = ProbabilisticTestConfig {
            origin: ThresholdOrigin::Empirical,
            threshold: None,
            samples: Some(200),
            baseline_method: Some("calibrate"),
        };
        assert!(cfg.requires_external_baseline());
    }

    #[test]
    fn unspecified_origin_does_not_require_baseline() {
        let cfg = ProbabilisticTestConfig {
            origin: ThresholdOrigin::Unspecified,
            threshold: Some(0.95),
            samples: Some(200),
            baseline_method: None,
        };
        assert!(!cfg.requires_external_baseline());
    }
}
