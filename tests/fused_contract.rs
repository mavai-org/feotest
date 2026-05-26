//! Integration tests for the fused service-contract surface.
//!
//! A contract is authored in a single `impl ServiceContract` block carrying
//! its input/output types, its invocation, and its criteria. These tests pin
//! the two load-bearing properties of that surface:
//!
//! 1. The output is the *raw* response. A malformed-but-received response is
//!    `Ok` from `invoke` and is failed by a parsing criterion; only an
//!    unobtainable response is a `Defect` that aborts.
//! 2. Covariate context is built from a generic contract reference — the
//!    associated types make the trait non-object-safe, so there is no `&dyn`.
//!    A `Configurable` contract still drives factor get/set.

use feotest::controls::Cost;
use feotest::criteria::{Criteria, Criterion, CriterionOutcome};
use feotest::model::{ContractViolation, Defect};
use feotest::service_contract::{
    Configurable, CovariateCategory, CovariateContext, CovariateDeclaration, FactorError,
    FactorValue, ServiceContract,
};

/// A contract whose service returns a raw text response. The response may be
/// well-formed (a positive integer), malformed (non-numeric), or unobtainable
/// (a transport-class failure) depending on the input — exactly the three
/// cases the fused surface must distinguish.
struct CountingService {
    model: String,
}

impl ServiceContract for CountingService {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        "counting.service"
    }

    fn covariates(&self) -> Vec<CovariateDeclaration> {
        vec![CovariateDeclaration::new(
            "model",
            CovariateCategory::ExternalDependency,
        )]
    }

    fn invoke(&self, input: &String, cost: &mut Cost) -> Result<String, Defect> {
        cost.record_tokens(7);
        match input.as_str() {
            // No response obtainable — the only case that aborts.
            "boom" => Err(Defect::new("transport failure")),
            // A malformed response still came back: return it raw and let the
            // criteria judge it.
            "bad" => Ok("not-a-number".to_string()),
            // A well-formed response.
            other => Ok(other.to_string()),
        }
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criterion::empirical()
            .pass_rate()
            .transforming(|r: &String| {
                r.parse::<u32>()
                    .map_err(|_| ContractViolation::new("parse", "response is not an integer"))
            })
            .name("positive count")
            .satisfies("count is positive", |n: &u32| {
                if *n > 0 {
                    Ok(())
                } else {
                    Err(ContractViolation::new("non-positive", "count was zero"))
                }
            })
            .build()])
    }
}

impl Configurable for CountingService {
    fn get_factor(&self, name: &str) -> Option<FactorValue> {
        match name {
            "model" => Some(FactorValue::String(self.model.clone())),
            _ => None,
        }
    }

    fn set_factor(&mut self, name: &str, value: FactorValue) -> Result<(), FactorError> {
        match (name, value) {
            ("model", FactorValue::String(s)) => {
                self.model = s;
                Ok(())
            }
            ("model", _) => Err(FactorError::new("model", "expected string")),
            _ => Err(FactorError::new(name, "unknown factor")),
        }
    }

    fn factor_names(&self) -> Vec<&str> {
        vec!["model"]
    }
}

#[test]
fn well_formed_response_passes_the_parsing_criterion() {
    let contract = CountingService {
        model: "gpt-4o".into(),
    };
    let mut cost = Cost::new();

    let output = contract
        .invoke(&"42".to_string(), &mut cost)
        .expect("a well-formed response is obtainable");

    let results = contract.criteria().evaluate(&output);
    assert_eq!(results[0].outcome(), CriterionOutcome::Pass);
}

#[test]
fn malformed_response_is_ok_but_fails_the_parsing_criterion() {
    let contract = CountingService {
        model: "gpt-4o".into(),
    };
    let mut cost = Cost::new();

    // A malformed response is received, so invoke succeeds — it is not a defect.
    let output = contract
        .invoke(&"bad".to_string(), &mut cost)
        .expect("a malformed response is still a response, not a defect");

    // The parsing criterion fails that sample, carrying the transform's reason.
    let results = contract.criteria().evaluate(&output);
    assert_eq!(results[0].outcome(), CriterionOutcome::Fail);
    assert_eq!(results[0].reason().unwrap().check(), "parse");
}

#[test]
fn unobtainable_response_is_a_defect() {
    let contract = CountingService {
        model: "gpt-4o".into(),
    };
    let mut cost = Cost::new();

    let result = contract.invoke(&"boom".to_string(), &mut cost);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().message(), "transport failure");
}

#[test]
fn invoke_records_its_token_cost() {
    let contract = CountingService {
        model: "gpt-4o".into(),
    };
    let mut cost = Cost::new();

    contract.invoke(&"42".to_string(), &mut cost).unwrap();
    contract.invoke(&"7".to_string(), &mut cost).unwrap();

    assert_eq!(cost.tokens_recorded(), 14);
}

#[test]
fn covariate_context_builds_from_a_generic_contract_reference() {
    // Generic over the concrete contract type: no `&dyn` is possible now that
    // the trait carries associated types, and none is needed.
    fn context_of<C: ServiceContract>(contract: &C) -> Option<CovariateContext> {
        CovariateContext::from_contract(contract)
    }

    let contract = CountingService {
        model: "gpt-4o".into(),
    };
    let ctx = context_of(&contract).expect("the contract declares a covariate");
    assert_eq!(ctx.declarations().len(), 1);
    assert_eq!(ctx.declarations()[0].key(), "model");
}

#[test]
fn configurable_drives_factor_get_and_set() {
    let mut contract = CountingService {
        model: "gpt-4o".into(),
    };

    assert_eq!(
        contract.get_factor("model"),
        Some(FactorValue::String("gpt-4o".into()))
    );

    contract
        .set_factor("model", FactorValue::String("claude-sonnet".into()))
        .unwrap();
    assert_eq!(
        contract.get_factor("model"),
        Some(FactorValue::String("claude-sonnet".into()))
    );
}
