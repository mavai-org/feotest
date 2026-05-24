//! Generates a sample HTML report showcasing all RP07 elements.
//!
//! Run with: cargo run --example sample_html_report
//!
//! Writes the report to `target/sample-report.html`.

use std::time::Duration;

use feotest::controls::PacingConfig;
use feotest::model::{
    CostSummary, ExecutionSummary, ExpirationInfo, ExpirationStatus, PacingSummary,
    TerminationInfo, TerminationReason, TestIdentity, TestIntent, ThresholdOrigin, Warning,
};
use feotest::reporting::HtmlReportWriter;
use feotest::verdict::{
    BaselineProvenance, CovariateStatus, CriterionRow, FunctionalAssessment, Misalignment,
    SpecProvenance, StatisticalAnalysis, Verdict, VerdictRecord,
};

fn execution(
    planned: u32,
    executed: u32,
    successes: u32,
    failures: u32,
    elapsed_ms: u64,
) -> ExecutionSummary {
    ExecutionSummary::new(
        planned,
        executed,
        successes,
        failures,
        TerminationInfo::new(TerminationReason::Completed),
        CostSummary::new(Duration::from_millis(elapsed_ms), 0, executed),
    )
}

fn full_pass_record() -> VerdictRecord {
    let analysis = StatisticalAnalysis::new(0.95, 0.014, 0.932, 0.920, ThresholdOrigin::Empirical)
        .with_test_results(3.14, 0.001);

    let provenance = SpecProvenance::new(ThresholdOrigin::Empirical)
        .with_spec_filename("translation-service.yaml")
        .with_contract_ref("SLA v2.0 §3.1")
        .with_expiration(ExpirationInfo::new(
            ExpirationStatus::Valid,
            Some("2026-12-31T23:59:59Z".into()),
        ));

    let baseline_prov = BaselineProvenance::new(
        "translation-service.yaml",
        "2026-03-15T08:00:00Z",
        500,
        0.9600,
        0.9200,
    );

    let pacing = PacingSummary::from_config(
        &PacingConfig::new()
            .max_requests_per_second(10.0)
            .max_requests_per_minute(300.0),
    );

    let covariates = CovariateStatus::new(
        true,
        vec![],
        vec![
            ("model".to_string(), "gpt-4o".to_string()),
            ("region".to_string(), "eu-central-1".to_string()),
            ("temperature".to_string(), "0.7".to_string()),
        ],
        vec![
            ("model".to_string(), "gpt-4o".to_string()),
            ("region".to_string(), "eu-central-1".to_string()),
            ("temperature".to_string(), "0.7".to_string()),
        ],
    );

    VerdictRecord::builder(
        TestIdentity::new("translation-service").with_test_name("test_en_to_de"),
        Verdict::Pass,
        TestIntent::Verification,
        execution(200, 200, 192, 8, 4200),
        FunctionalAssessment::single(CriterionRow::result(192, 8, vec![], Verdict::Pass)),
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance)
    .baseline_provenance(baseline_prov)
    .covariate_status(covariates)
    .correlation_id("ci-run-2026-04-19-001")
    .pacing(pacing)
    .environment(vec![
        ("cloud_provider".to_string(), "aws".to_string()),
        ("region".to_string(), "eu-central-1".to_string()),
        ("instance_type".to_string(), "m5.large".to_string()),
    ])
    .build()
}

fn fail_record() -> VerdictRecord {
    let analysis = StatisticalAnalysis::new(0.95, 0.040, 0.722, 0.900, ThresholdOrigin::Sla)
        .with_test_results(-1.50, 0.933);

    let provenance = SpecProvenance::new(ThresholdOrigin::Sla)
        .with_contract_ref("Payment SLA v1.0 §2.4")
        .with_expiration(ExpirationInfo::new(
            ExpirationStatus::ExpiringSoon,
            Some("2026-05-01T00:00:00Z".into()),
        ));

    let pacing = PacingSummary::from_config(&PacingConfig::new().max_requests_per_second(5.0));

    let covariates = CovariateStatus::new(
        false,
        vec![Misalignment::new("api_version", "2026-03-01", "2026-04-15")],
        vec![
            ("provider".to_string(), "stripe".to_string()),
            ("api_version".to_string(), "2026-03-01".to_string()),
            ("currency".to_string(), "EUR".to_string()),
        ],
        vec![
            ("provider".to_string(), "stripe".to_string()),
            ("api_version".to_string(), "2026-04-15".to_string()),
            ("currency".to_string(), "EUR".to_string()),
        ],
    );

    VerdictRecord::builder(
        TestIdentity::new("payment-gateway").with_test_name("test_charge_accuracy"),
        Verdict::Fail,
        TestIntent::Verification,
        execution(100, 100, 80, 20, 8500),
        FunctionalAssessment::single(CriterionRow::result(
            80,
            20,
            vec![
                ("amount_mismatch".to_string(), 12),
                ("currency_error".to_string(), 5),
                ("timeout".to_string(), 3),
            ],
            Verdict::Fail,
        )),
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance)
    .covariate_status(covariates)
    .pacing(pacing)
    .environment(vec![
        ("cloud_provider".to_string(), "aws".to_string()),
        ("region".to_string(), "eu-central-1".to_string()),
    ])
    .warning(Warning::new(
        "BASELINE_EXPIRING",
        "Baseline spec expires in 12 days",
    ))
    .warning(Warning::new(
        "COVARIATE_DRIFT",
        "Covariate 'api_version' changed from 2026-03-01 to 2026-04-15 since baseline",
    ))
    .build()
}

fn inconclusive_covariate_record() -> VerdictRecord {
    let analysis = StatisticalAnalysis::new(0.95, 0.031, 0.838, 0.850, ThresholdOrigin::Empirical)
        .with_test_results(1.61, 0.054);

    let provenance = SpecProvenance::new(ThresholdOrigin::Empirical)
        .with_spec_filename("sentiment-analyser.yaml");

    let baseline_prov = BaselineProvenance::new(
        "sentiment-analyser.yaml",
        "2026-02-10T09:00:00Z",
        300,
        0.9200,
        0.8500,
    );

    let covariates = CovariateStatus::new(
        false,
        vec![
            Misalignment::new("model", "gpt-4o", "gpt-4o-mini"),
            Misalignment::new("temperature", "0.3", "0.7"),
        ],
        vec![
            ("model".to_string(), "gpt-4o".to_string()),
            ("temperature".to_string(), "0.3".to_string()),
            ("prompt_version".to_string(), "v2.1".to_string()),
        ],
        vec![
            ("model".to_string(), "gpt-4o-mini".to_string()),
            ("temperature".to_string(), "0.7".to_string()),
            ("prompt_version".to_string(), "v2.1".to_string()),
        ],
    );

    VerdictRecord::builder(
        TestIdentity::new("sentiment-analyser").with_test_name("test_positive_detection"),
        Verdict::Inconclusive,
        TestIntent::Verification,
        execution(100, 100, 90, 10, 3800),
        FunctionalAssessment::single(CriterionRow::result(90, 10, vec![("misclassification".to_string(), 10)], Verdict::Inconclusive)),
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance)
    .baseline_provenance(baseline_prov)
    .covariate_status(covariates)
    .correlation_id("ci-run-2026-04-19-001")
    .environment(vec![
        ("cloud_provider".to_string(), "aws".to_string()),
        ("region".to_string(), "eu-central-1".to_string()),
    ])
    .warning(Warning::new(
        "COVARIATE_MISALIGNMENT",
        "2 covariates differ from baseline: model (gpt-4o -> gpt-4o-mini), temperature (0.3 -> 0.7)",
    ))
    .warning(Warning::new(
        "BASELINE_STALE",
        "Baseline is 68 days old — consider re-establishing",
    ))
    .build()
}

fn main() {
    let records = vec![
        full_pass_record(),
        fail_record(),
        inconclusive_covariate_record(),
    ];
    let timestamp = Some("2026-04-19T14:30:00Z");

    match HtmlReportWriter::generate(&records, timestamp) {
        Ok(html) => {
            let path = "target/sample-report.html";
            std::fs::write(path, &html).expect("failed to write report");
            println!("Report written to {path}");
            println!("Open with: open {path}");
        }
        Err(e) => {
            eprintln!("Failed to generate HTML report: {e}");
            eprintln!("Make sure xsltproc is installed.");
            std::process::exit(1);
        }
    }
}
