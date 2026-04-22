use feotest::{sentinel, use_case_factory};

#[sentinel]
#[derive(Default)]
struct Spec;

impl Spec {
    #[use_case_factory]
    fn not_a_factory(&self) -> String {
        String::new()
    }
}

fn main() {}
