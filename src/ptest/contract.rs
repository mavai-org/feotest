//! Contract-driven probabilistic test entry point.
//!
//! [`ProbabilisticTest::for_contract`] binds a test to a [`ServiceContract`]:
//! the engine invokes the contract and judges every criterion on every sample,
//! and the verdict decomposes per criterion with a composite over them.
//!
//! ```no_run
//! use feotest::ptest::ProbabilisticTest;
//! # fn run(contract: impl feotest::service_contract::ServiceContract<Input = String, Output = String>) {
//! let inputs = vec!["query".to_string()];
//! let result = ProbabilisticTest::for_contract(contract)
//!     .inputs(&inputs)
//!     .samples(200)
//!     .confidence(0.95)
//!     .run();
//! # let _ = result.verdict_record();
//! # }
//! ```

use crate::model::{TestIntent, ThresholdOrigin};
use crate::ptest::ProbabilisticTest;
use crate::ptest::runner::{self, ContractTestPlan, ProbabilisticTestResult};
use crate::service_contract::ServiceContract;
use crate::spec::BaselineSpec;
use crate::statistics::defaults::DEFAULT_CONFIDENCE;
use crate::statistics::types::ConfidenceLevel;

/// Default sample count when the caller does not set one.
const DEFAULT_SAMPLES: u32 = 100;

impl ProbabilisticTest<'_, ()> {
    /// Begins a contract-driven probabilistic test.
    ///
    /// The contract supplies the inputs' element type, the invocation, the
    /// acceptance criteria, and any latency commitment. Set the sampling plan
    /// with [`inputs`](ContractTest::inputs), [`samples`](ContractTest::samples),
    /// and [`confidence`](ContractTest::confidence); execute with
    /// [`run`](ContractTest::run).
    #[must_use]
    pub const fn for_contract<C: ServiceContract>(contract: C) -> ContractTest<'static, C> {
        ContractTest::new(contract)
    }
}

/// A contract-driven probabilistic test under construction.
///
/// Builder calls are order-independent; the sampling plan and contract are
/// assembled and validated at [`run`](Self::run).
pub struct ContractTest<'a, C: ServiceContract> {
    contract: C,
    inputs: Option<&'a [C::Input]>,
    samples: u32,
    confidence: f64,
    intent: TestIntent,
    threshold_origin: ThresholdOrigin,
    baseline: Option<BaselineSpec>,
    contract_ref: Option<String>,
}

impl<'a, C: ServiceContract> ContractTest<'a, C> {
    const fn new(contract: C) -> Self {
        Self {
            contract,
            inputs: None,
            samples: DEFAULT_SAMPLES,
            confidence: DEFAULT_CONFIDENCE,
            intent: TestIntent::Verification,
            threshold_origin: ThresholdOrigin::Empirical,
            baseline: None,
            contract_ref: None,
        }
    }

    /// Sets the inputs the contract is invoked against. Inputs are cycled
    /// round-robin when the sample count exceeds the input count.
    #[must_use]
    pub const fn inputs(mut self, inputs: &'a [C::Input]) -> Self {
        self.inputs = Some(inputs);
        self
    }

    /// Sets the number of counted samples.
    #[must_use]
    pub const fn samples(mut self, samples: u32) -> Self {
        self.samples = samples;
        self
    }

    /// Sets the confidence level for every criterion's verdict.
    #[must_use]
    pub const fn confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }

    /// Marks this as a smoke test rather than a verification.
    #[must_use]
    pub const fn smoke(mut self) -> Self {
        self.intent = TestIntent::Smoke;
        self
    }

    /// Records the origin of the criteria's targets (defaults to empirical).
    #[must_use]
    pub const fn threshold_origin(mut self, origin: ThresholdOrigin) -> Self {
        self.threshold_origin = origin;
        self
    }

    /// Supplies the baseline an empirical criterion derives its target from.
    #[must_use]
    pub fn baseline(mut self, baseline: BaselineSpec) -> Self {
        self.baseline = Some(baseline);
        self
    }

    /// Sets a human-readable contract reference for provenance.
    #[must_use]
    pub fn contract_ref(mut self, contract_ref: impl Into<String>) -> Self {
        self.contract_ref = Some(contract_ref.into());
        self
    }

    /// Executes the test and produces the verdict.
    ///
    /// # Panics
    ///
    /// Panics if no inputs were set, or if a service invocation yields a defect
    /// (a transport failure or a caught panic) — a defect aborts the run.
    #[must_use]
    pub fn run(self) -> ProbabilisticTestResult
    where
        C::Output: 'static,
    {
        let inputs = self
            .inputs
            .expect("a probabilistic test requires inputs — call .inputs(...)");
        let plan = ContractTestPlan {
            samples: self.samples,
            confidence: ConfidenceLevel::new(self.confidence),
            intent: self.intent,
            threshold_origin: self.threshold_origin,
            baseline: self.baseline,
            contract_ref: self.contract_ref,
        };
        runner::execute_contract(&self.contract, inputs, plan)
    }
}
