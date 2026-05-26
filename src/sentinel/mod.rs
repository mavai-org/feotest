//! Reliability specifications.
//!
//! A reliability specification is a struct that declares the probabilistic
//! testing surface for one non-deterministic boundary of an application —
//! typically one per service or integration point. It bundles the service contracts,
//! experiments, and probabilistic tests that define what "reliable" means
//! for that boundary.
//!
//! Specifications are authored by annotating a struct with the `#[sentinel]`
//! attribute and marking its use-case factory methods with
//! `#[service_contract_factory]`. Both attributes are re-exported from the crate
//! root.
//!
//! ```ignore
//! use feotest::sentinel;
//! use feotest::service_contract_factory;
//! use feotest::service_contract::ServiceContract;
//!
//! #[sentinel]
//! #[derive(Default)]
//! struct PaymentGateway;
//!
//! impl PaymentGateway {
//!     #[service_contract_factory]
//!     fn payments(&self) -> impl ServiceContract {
//!         // construct and return a configured service contract
//!         # unimplemented!()
//!     }
//! }
//! ```
//!
//! At compile time each annotated struct is registered into a global
//! inventory of [`SpecDescriptor`] entries. Tooling that runs reliability
//! specifications iterates this inventory to discover available
//! specifications without requiring reflection, test-framework participation,
//! or a central manifest.
//!
//! This module provides only the authoring and registration surface. The
//! runtime that consumes registered descriptors — to execute probabilistic
//! tests or measure experiments against live services — is a separate
//! concern that will layer on top of the types defined here.

use core::any::Any;
use core::fmt;

pub mod content;
pub mod embedded;
pub mod resolver;
pub mod runner;
pub mod sinks;

pub use content::{
    ContentDescriptor, ContentInvoker, ContentKind, MeasureExperimentConfig,
    ProbabilisticTestConfig, content_for, registered_content,
};
pub use embedded::{DefaultEmbeddedRegistry, EmbeddedBaseline, registered_embedded_baselines};
pub use resolver::{
    BaselineQuery, BaselineResolutionError, EmbeddedBaselineLookup, OUTPUT_ENV_VAR, SOURCE_ENV_VAR,
    baseline_output_from_env, baseline_source_from_env, resolve_baseline,
};
pub use runner::{SentinelOutcome, SentinelResult, SentinelRunner, run_cli};
pub use sinks::{
    CompositeVerdictSink, ConsoleVerdictSink, FileVerdictSink, SinkError, VerdictSink,
};
#[cfg(feature = "webhook")]
pub use sinks::{WebhookVerdictSink, WebhookVerdictSinkBuilder};

/// The authoring contract every `#[sentinel]`-annotated struct satisfies.
///
/// Implementations are produced by the `#[sentinel]` attribute macro and
/// are not normally written by hand. The trait exposes the minimum surface
/// a runtime needs to identify, label, and invoke a specification: name,
/// description, and a type-erased downcast via [`as_any`](ReliabilitySpec::as_any).
// javai-ref: JVI-0PYEB09 — do not remove (resolves in javai-orchestrator)
pub trait ReliabilitySpec: Send + Sync {
    /// Stable symbolic identifier for this specification.
    ///
    /// Defaults to the snake-cased name of the annotated struct. May be
    /// overridden via `#[sentinel(name = "...")]`. Conventions encourage
    /// lowercase, dot- or underscore-separated identifiers.
    ///
    /// The identifier is sourced from a compile-time string literal in
    /// the `#[sentinel]` attribute, which is why the return type is
    /// `'static` — implementations do not need to borrow from `self`.
    fn name(&self) -> &'static str;

    /// Human-readable one-line description.
    ///
    /// Defaults to the empty string. Overridable via
    /// `#[sentinel(description = "...")]`. Intended for CLI listings and
    /// diagnostic output.
    fn description(&self) -> &'static str {
        ""
    }

    /// Type-erased upcast to `&dyn Any`, enabling the sentinel runner to
    /// downcast a trait-object spec reference to its concrete type before
    /// invoking content methods. The `#[sentinel]` attribute macro emits
    /// a trivial `self` return; bespoke implementations should do the
    /// same.
    fn as_any(&self) -> &dyn Any;
}

/// Metadata and constructor for a registered reliability specification.
///
/// One descriptor is submitted to the inventory for each `#[sentinel]`-
/// annotated struct. Descriptors are collected at link time and enumerated
/// via [`registered_specs`].
pub struct SpecDescriptor {
    /// Stable symbolic identifier — matches the instance's `name()`.
    pub name: &'static str,
    /// One-line human description — matches the instance's `description()`.
    pub description: &'static str,
    /// Constructor that yields an owned instance of the specification.
    ///
    /// The constructor is a plain function pointer so that the descriptor
    /// remains a `'static` value eligible for link-time inventory
    /// submission.
    pub constructor: fn() -> Box<dyn ReliabilitySpec>,
}

impl fmt::Debug for SpecDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpecDescriptor")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

inventory::collect!(SpecDescriptor);

/// Iterates every reliability specification descriptor registered in this
/// binary.
///
/// The order in which descriptors are yielded is unspecified and may vary
/// between runs; callers that need a stable order should sort by `name`.
pub fn registered_specs() -> impl Iterator<Item = &'static SpecDescriptor> {
    inventory::iter::<SpecDescriptor>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_contract::ServiceContract;
    use crate::spec::namer::CovariateProfile;
    use feotest_macros::{sentinel, service_contract_factory};

    /// Minimal service contract used by factory-compilation tests. The implementation
    /// is intentionally trivial — these tests exercise macro expansion, not
    /// service contract behaviour. The id is owned so the trait's `id(&self) -> &str`
    /// is satisfied by a genuine self-borrow rather than a dangling literal.
    struct TrivialServiceContract {
        id: String,
    }

    impl TrivialServiceContract {
        fn with_id(id: &str) -> Self {
            Self { id: id.to_owned() }
        }
    }

    impl ServiceContract for TrivialServiceContract {
        type Input = String;
        type Output = String;

        fn id(&self) -> &str {
            &self.id
        }

        fn invoke(
            &self,
            input: &String,
            _cost: &mut crate::controls::Cost,
        ) -> Result<String, crate::model::Defect> {
            Ok(input.clone())
        }

        fn criteria(&self) -> crate::criteria::Criteria<String> {
            crate::criteria::Criteria::of([crate::criteria::Criteria::meeting()
                .pass_rate(0.5)
                .name("response received")
                .satisfies("response received", |_: &String| Ok(()))
                .build()])
        }
    }

    #[sentinel]
    #[derive(Default)]
    struct SoloSpec;

    #[test]
    fn registers_single_spec_by_name() {
        let entry = registered_specs()
            .find(|d| d.name == "solo_spec")
            .expect("SoloSpec should register under its snake-cased name");

        let instance = (entry.constructor)();
        assert_eq!(instance.name(), "solo_spec");
        assert_eq!(instance.description(), "");
    }

    #[sentinel(name = "custom_id")]
    #[derive(Default)]
    struct NamedSpec;

    #[test]
    fn custom_name_via_attribute() {
        let entry = registered_specs()
            .find(|d| d.name == "custom_id")
            .expect("NamedSpec should register under the explicit name");

        assert_eq!(entry.name, "custom_id");
        let instance = (entry.constructor)();
        assert_eq!(instance.name(), "custom_id");
    }

    #[sentinel(description = "exercises the description override")]
    #[derive(Default)]
    struct DescribedSpec;

    #[test]
    fn description_defaults_empty_and_accepts_override() {
        let solo = registered_specs()
            .find(|d| d.name == "solo_spec")
            .expect("SoloSpec must be registered");
        assert_eq!(solo.description, "");

        let described = registered_specs()
            .find(|d| d.name == "described_spec")
            .expect("DescribedSpec must be registered");
        assert_eq!(described.description, "exercises the description override");
        let instance = (described.constructor)();
        assert_eq!(instance.description(), "exercises the description override");
    }

    #[sentinel]
    #[derive(Default)]
    struct FirstCoexisting;

    #[sentinel]
    #[derive(Default)]
    struct SecondCoexisting;

    #[test]
    fn multiple_specs_coexist_in_registry() {
        let names: Vec<&str> = registered_specs().map(|d| d.name).collect();
        assert!(
            names.contains(&"first_coexisting"),
            "FirstCoexisting should appear in the registry; saw {names:?}"
        );
        assert!(
            names.contains(&"second_coexisting"),
            "SecondCoexisting should appear in the registry; saw {names:?}"
        );
    }

    #[sentinel]
    #[derive(Default)]
    struct WithFactory {
        id_seed: String,
    }

    impl WithFactory {
        #[service_contract_factory]
        fn trivial(&self) -> impl ServiceContract {
            TrivialServiceContract::with_id(&format!("{}trivial", self.id_seed))
        }
    }

    #[test]
    fn factory_method_compiles_with_service_contract_return() {
        let spec = WithFactory {
            id_seed: String::new(),
        };
        assert_eq!(spec.trivial().id(), "trivial");
        // Covariate access via the trait is unaffected by the marker macro.
        assert!(spec.trivial().covariates().is_empty());
        let profile: CovariateProfile = spec.trivial().resolve_covariates();
        assert!(profile.is_empty());
    }
}
