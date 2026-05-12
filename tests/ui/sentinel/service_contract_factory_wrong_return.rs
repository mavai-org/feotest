use feotest::{sentinel, service_contract_factory};

#[sentinel]
#[derive(Default)]
struct Spec;

impl Spec {
    #[service_contract_factory]
    fn not_a_factory(&self) -> String {
        String::new()
    }
}

fn main() {}
