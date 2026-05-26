//! Covariate-aware baseline selection.
//!
//! Given a set of candidate baselines and the current runtime covariate
//! profile, selects the candidate that best matches. The algorithm is a
//! two-phase approach: hard gates first, then soft scoring.

use std::fmt;

use crate::service_contract::CovariateDeclaration;
use crate::spec::BaselineSpec;
use crate::spec::matching::{ConformanceDetail, MatchResult, match_covariate};
use crate::spec::namer::CovariateProfile;

/// A parsed baseline spec together with its source filename.
#[derive(Debug, Clone)]
pub struct BaselineCandidate {
    /// The filename (not full path) of the spec file.
    #[allow(dead_code, reason = "filename retained for future diagnostics")]
    pub(crate) filename: String,
    /// The parsed baseline spec.
    pub(crate) spec: BaselineSpec,
}

/// The result of covariate-aware baseline selection.
#[derive(Debug)]
pub struct SelectionResult {
    /// The selected baseline spec.
    selected: BaselineSpec,
    /// Per-covariate conformance details.
    conformance: Vec<ConformanceDetail>,
    /// Whether the selection was ambiguous (multiple equally-scored candidates).
    ambiguous: bool,
    /// Total number of candidates considered.
    candidate_count: usize,
}

impl SelectionResult {
    /// The selected baseline spec.
    #[must_use]
    pub const fn selected(&self) -> &BaselineSpec {
        &self.selected
    }

    /// Consumes the result and returns the selected spec.
    #[must_use]
    pub fn into_selected(self) -> BaselineSpec {
        self.selected
    }

    /// Per-covariate conformance details.
    #[must_use]
    pub fn conformance(&self) -> &[ConformanceDetail] {
        &self.conformance
    }

    /// Whether the selection was ambiguous.
    #[must_use]
    pub const fn ambiguous(&self) -> bool {
        self.ambiguous
    }

    /// Total number of candidates considered.
    #[must_use]
    pub const fn candidate_count(&self) -> usize {
        self.candidate_count
    }

    /// Returns non-conforming covariate details (soft mismatches).
    #[must_use]
    pub fn non_conforming(&self) -> Vec<&ConformanceDetail> {
        self.conformance.iter().filter(|d| !d.conforms()).collect()
    }

    /// Creates a `SelectionResult` from a single spec with no conformance
    /// issues. For use in tests that need a clean selection result.
    #[cfg(test)]
    pub(crate) fn from_single(spec: BaselineSpec) -> Self {
        Self {
            selected: spec,
            conformance: Vec::new(),
            ambiguous: false,
            candidate_count: 1,
        }
    }

    /// Creates a `SelectionResult` with custom conformance and ambiguity.
    /// For use in tests that need to exercise warning paths.
    #[cfg(test)]
    pub(crate) fn with_details(
        spec: BaselineSpec,
        conformance: Vec<ConformanceDetail>,
        ambiguous: bool,
        candidate_count: usize,
    ) -> Self {
        Self {
            selected: spec,
            conformance,
            ambiguous,
            candidate_count,
        }
    }
}

/// Errors that can occur during baseline selection.
#[derive(Debug)]
pub enum SelectionError {
    /// No candidate baselines found at all.
    NoCandidates {
        /// The service contract ID that was searched.
        service_contract_id: String,
    },
    /// Candidates exist but none match the required configuration covariates.
    ConfigurationMismatch {
        /// The service contract ID.
        service_contract_id: String,
        /// The configuration covariate values from the test profile that
        /// could not be matched.
        required: Vec<(String, String)>,
        /// The configuration covariate values available across candidates.
        available: Vec<Vec<(String, String)>>,
    },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCandidates {
                service_contract_id,
            } => {
                write!(
                    f,
                    "no baseline candidates found for service contract '{service_contract_id}'"
                )
            }
            Self::ConfigurationMismatch {
                service_contract_id,
                required,
                available,
            } => {
                write!(
                    f,
                    "no baseline matches the configuration for service contract '{service_contract_id}'\n\
                     Required: {}\n\
                     Available configurations:",
                    format_kv_pairs(required),
                )?;
                if available.is_empty() {
                    write!(f, " (none)")?;
                } else {
                    for config in available {
                        write!(f, "\n  - {}", format_kv_pairs(config))?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SelectionError {}

/// Selects the best-matching baseline from candidates.
///
/// # Phase 1: Hard gates
///
/// Filters candidates by `Configuration` covariates (exact match required).
/// If no candidates survive, returns a `ConfigurationMismatch` error listing
/// available configurations to guide the developer.
///
/// # Phase 2: Soft matching
///
/// Scores surviving candidates by non-hard-gate covariates. Each conforming
/// covariate adds one point. Ties are broken by:
/// 1. Match count (higher wins)
/// 2. Declaration order (first diverging covariate decides)
/// 3. Recency (`generated_at` timestamp, newer wins)
///
/// # Errors
///
/// Returns `NoCandidates` if the candidate list is empty, or
/// `ConfigurationMismatch` if hard-gate filtering eliminates all candidates.
// javai-ref: JVI-YN3BJ6U — do not remove (resolves in javai-orchestrator)
pub fn select(
    candidates: &[BaselineCandidate],
    test_profile: &CovariateProfile,
    declarations: &[CovariateDeclaration],
) -> Result<SelectionResult, SelectionError> {
    if candidates.is_empty() {
        return Err(SelectionError::NoCandidates {
            service_contract_id: String::new(),
        });
    }

    let service_contract_id = &candidates[0].spec.service_contract_id;

    // Separate hard-gate and soft-match declarations
    let hard_gate_keys: Vec<&str> = declarations
        .iter()
        .filter(|d| d.category().is_hard_gate())
        .map(CovariateDeclaration::key)
        .collect();

    let soft_keys: Vec<&str> = declarations
        .iter()
        .filter(|d| !d.category().is_hard_gate())
        .map(CovariateDeclaration::key)
        .collect();

    // Phase 1: Hard-gate filtering
    let survivors: Vec<&BaselineCandidate> = if hard_gate_keys.is_empty() {
        candidates.iter().collect()
    } else {
        candidates
            .iter()
            .filter(|c| {
                hard_gate_keys.iter().all(|key| {
                    let test_val = test_profile.get(key).unwrap_or("");
                    let baseline_val = c.spec.covariates.get(*key).map_or("", String::as_str);
                    match_covariate(key, baseline_val, test_val) == MatchResult::Conforms
                })
            })
            .collect()
    };

    if survivors.is_empty() {
        let required: Vec<(String, String)> = hard_gate_keys
            .iter()
            .map(|k| {
                (
                    (*k).to_string(),
                    test_profile.get(k).unwrap_or("").to_string(),
                )
            })
            .collect();

        let available: Vec<Vec<(String, String)>> = candidates
            .iter()
            .map(|c| {
                hard_gate_keys
                    .iter()
                    .map(|k| {
                        (
                            (*k).to_string(),
                            c.spec.covariates.get(*k).cloned().unwrap_or_default(),
                        )
                    })
                    .collect()
            })
            .collect();

        return Err(SelectionError::ConfigurationMismatch {
            service_contract_id: service_contract_id.clone(),
            required,
            available,
        });
    }

    // Phase 2: Soft-match scoring
    let scored: Vec<ScoredCandidate<'_>> = survivors
        .iter()
        .map(|c| score_candidate(c, test_profile, &soft_keys))
        .collect();

    // Sort: highest score first, then by declaration-order tie-breaking, then recency
    let best = select_best(&scored, &soft_keys, test_profile);

    // Check for ambiguity (multiple candidates with same top score)
    let top_score = best.score;
    let top_count = scored.iter().filter(|s| s.score == top_score).count();

    // Build conformance details for the selected candidate
    let conformance = build_conformance(best.candidate, test_profile, declarations);

    Ok(SelectionResult {
        selected: best.candidate.spec.clone(),
        conformance,
        ambiguous: top_count > 1,
        candidate_count: candidates.len(),
    })
}

/// A candidate with its soft-match score.
struct ScoredCandidate<'a> {
    candidate: &'a BaselineCandidate,
    score: usize,
}

/// Scores a candidate by counting conforming soft-match covariates.
fn score_candidate<'a>(
    candidate: &'a BaselineCandidate,
    test_profile: &CovariateProfile,
    soft_keys: &[&str],
) -> ScoredCandidate<'a> {
    let score = soft_keys
        .iter()
        .filter(|key| {
            let test_val = test_profile.get(key).unwrap_or("");
            let baseline_val = candidate
                .spec
                .covariates
                .get(**key)
                .map_or("", String::as_str);
            match_covariate(key, baseline_val, test_val) == MatchResult::Conforms
        })
        .count();

    ScoredCandidate { candidate, score }
}

/// Selects the best candidate using the full tie-breaking chain.
fn select_best<'a>(
    scored: &'a [ScoredCandidate<'a>],
    soft_keys: &[&str],
    test_profile: &CovariateProfile,
) -> &'a ScoredCandidate<'a> {
    scored
        .iter()
        .max_by(|a, b| {
            // 1. Higher score wins
            a.score.cmp(&b.score).then_with(|| {
                // 2. Declaration-order tie-breaking: compare left-to-right
                for key in soft_keys {
                    let test_val = test_profile.get(key).unwrap_or("");
                    let a_val = a
                        .candidate
                        .spec
                        .covariates
                        .get(*key)
                        .map_or("", String::as_str);
                    let b_val = b
                        .candidate
                        .spec
                        .covariates
                        .get(*key)
                        .map_or("", String::as_str);
                    let a_matches = match_covariate(key, a_val, test_val) == MatchResult::Conforms;
                    let b_matches = match_covariate(key, b_val, test_val) == MatchResult::Conforms;
                    match (a_matches, b_matches) {
                        (true, false) => return std::cmp::Ordering::Greater,
                        (false, true) => return std::cmp::Ordering::Less,
                        _ => {}
                    }
                }
                // 3. Recency: newer generated_at wins (lexicographic ISO 8601)
                a.candidate
                    .spec
                    .generated_at
                    .cmp(&b.candidate.spec.generated_at)
            })
        })
        .expect("scored list is non-empty after survivor filtering")
}

/// Builds conformance details for all declared covariates against a candidate.
fn build_conformance(
    candidate: &BaselineCandidate,
    test_profile: &CovariateProfile,
    declarations: &[CovariateDeclaration],
) -> Vec<ConformanceDetail> {
    declarations
        .iter()
        .map(|decl| {
            let key = decl.key();
            let test_val = test_profile.get(key).unwrap_or("");
            let baseline_val = candidate
                .spec
                .covariates
                .get(key)
                .map_or("", String::as_str);
            let result = match_covariate(key, baseline_val, test_val);
            ConformanceDetail::new(key, baseline_val, test_val, result)
        })
        .collect()
}

/// Formats key-value pairs for error messages.
fn format_kv_pairs(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_contract::CovariateCategory;
    use crate::spec::baseline::{
        BaselineSpec, ExecutionBlock, RequirementsBlock, StatisticsBlock, SuccessRateBlock,
    };
    use std::collections::BTreeMap;

    fn make_spec(
        service_contract_id: &str,
        generated_at: &str,
        covariates: &[(&str, &str)],
    ) -> BaselineSpec {
        let mut spec = BaselineSpec::new(
            service_contract_id,
            generated_at,
            ExecutionBlock {
                samples_planned: 100,
                samples_executed: 100,
                termination_reason: Some("COMPLETED".to_string()),
            },
            RequirementsBlock {
                min_pass_rate: 0.85,
            },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: 0.90,
                    standard_error: 0.03,
                    confidence_interval95: [0.85, 0.95],
                },
                successes: 90,
                failures: 10,
                failure_distribution: None,
                latency_distribution: None,
                per_criterion: None,
            },
        );
        spec.covariates = covariates
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect::<BTreeMap<_, _>>();
        spec
    }

    fn make_candidate(spec: BaselineSpec) -> BaselineCandidate {
        let filename = format!("{}.yaml", spec.service_contract_id);
        BaselineCandidate { filename, spec }
    }

    #[test]
    fn single_candidate_selected() {
        let spec = make_spec("uc", "2026-01-01T00:00:00Z", &[]);
        let candidates = vec![make_candidate(spec)];
        let profile = CovariateProfile::empty();

        let result = select(&candidates, &profile, &[]).unwrap();
        assert_eq!(result.selected().service_contract_id, "uc");
        assert_eq!(result.candidate_count(), 1);
        assert!(!result.ambiguous());
    }

    #[test]
    fn no_candidates_returns_error() {
        let result = select(&[], &CovariateProfile::empty(), &[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SelectionError::NoCandidates { .. }));
    }

    #[test]
    fn hard_gate_filters_mismatches() {
        let spec_a = make_spec("uc", "2026-01-01T00:00:00Z", &[("model", "gpt-4o")]);
        let spec_b = make_spec("uc", "2026-01-01T00:00:00Z", &[("model", "claude-sonnet")]);
        let candidates = vec![make_candidate(spec_a), make_candidate(spec_b)];

        let profile = CovariateProfile::builder()
            .put("model", "claude-sonnet")
            .build();
        let declarations = vec![CovariateDeclaration::new(
            "model",
            CovariateCategory::Configuration,
        )];

        let result = select(&candidates, &profile, &declarations).unwrap();
        assert_eq!(
            result.selected().covariates.get("model").unwrap(),
            "claude-sonnet"
        );
    }

    #[test]
    fn hard_gate_mismatch_returns_error() {
        let spec = make_spec("uc", "2026-01-01T00:00:00Z", &[("model", "gpt-4o")]);
        let candidates = vec![make_candidate(spec)];

        let profile = CovariateProfile::builder()
            .put("model", "claude-sonnet")
            .build();
        let declarations = vec![CovariateDeclaration::new(
            "model",
            CovariateCategory::Configuration,
        )];

        let result = select(&candidates, &profile, &declarations);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SelectionError::ConfigurationMismatch { .. }));
        assert!(err.to_string().contains("claude-sonnet"));
    }

    #[test]
    fn soft_match_scores_by_count() {
        let spec_a = make_spec(
            "uc",
            "2026-01-01T00:00:00Z",
            &[("day-of-week", "WEEKDAY"), ("region", "EU")],
        );
        let spec_b = make_spec(
            "uc",
            "2026-01-01T00:00:00Z",
            &[("day-of-week", "WEEKEND"), ("region", "EU")],
        );
        let candidates = vec![make_candidate(spec_a), make_candidate(spec_b)];

        let profile = CovariateProfile::builder()
            .put("day-of-week", "WEEKDAY")
            .put("region", "EU")
            .build();
        let declarations = vec![
            CovariateDeclaration::new("day-of-week", CovariateCategory::Temporal),
            CovariateDeclaration::new("region", CovariateCategory::Infrastructure),
        ];

        let result = select(&candidates, &profile, &declarations).unwrap();
        // spec_a matches both, spec_b matches only region
        assert_eq!(
            result.selected().covariates.get("day-of-week").unwrap(),
            "WEEKDAY"
        );
        assert!(!result.ambiguous());
    }

    #[test]
    fn tie_broken_by_declaration_order() {
        // Both match on 1 of 2 soft covariates, but different ones.
        // The earlier-declared covariate wins.
        let spec_a = make_spec(
            "uc",
            "2026-01-01T00:00:00Z",
            &[("day-of-week", "WEEKDAY"), ("region", "US")],
        );
        let spec_b = make_spec(
            "uc",
            "2026-01-01T00:00:00Z",
            &[("day-of-week", "WEEKEND"), ("region", "EU")],
        );
        let candidates = vec![make_candidate(spec_a), make_candidate(spec_b)];

        let profile = CovariateProfile::builder()
            .put("day-of-week", "WEEKDAY")
            .put("region", "EU")
            .build();
        // day-of-week declared first → it has higher priority
        let declarations = vec![
            CovariateDeclaration::new("day-of-week", CovariateCategory::Temporal),
            CovariateDeclaration::new("region", CovariateCategory::Infrastructure),
        ];

        let result = select(&candidates, &profile, &declarations).unwrap();
        assert_eq!(
            result.selected().covariates.get("day-of-week").unwrap(),
            "WEEKDAY"
        );
    }

    #[test]
    fn tie_broken_by_recency() {
        let spec_old = make_spec("uc", "2026-01-01T00:00:00Z", &[("day-of-week", "WEEKDAY")]);
        let spec_new = make_spec("uc", "2026-06-15T00:00:00Z", &[("day-of-week", "WEEKDAY")]);
        let candidates = vec![make_candidate(spec_old), make_candidate(spec_new)];

        let profile = CovariateProfile::builder()
            .put("day-of-week", "WEEKDAY")
            .build();
        let declarations = vec![CovariateDeclaration::new(
            "day-of-week",
            CovariateCategory::Temporal,
        )];

        let result = select(&candidates, &profile, &declarations).unwrap();
        assert_eq!(result.selected().generated_at, "2026-06-15T00:00:00Z");
    }

    #[test]
    fn ambiguous_flag_set_on_tie() {
        // Two identical candidates
        let spec_a = make_spec("uc", "2026-01-01T00:00:00Z", &[("day-of-week", "WEEKDAY")]);
        let spec_b = make_spec("uc", "2026-01-01T00:00:00Z", &[("day-of-week", "WEEKDAY")]);
        let candidates = vec![make_candidate(spec_a), make_candidate(spec_b)];

        let profile = CovariateProfile::builder()
            .put("day-of-week", "WEEKDAY")
            .build();
        let declarations = vec![CovariateDeclaration::new(
            "day-of-week",
            CovariateCategory::Temporal,
        )];

        let result = select(&candidates, &profile, &declarations).unwrap();
        assert!(result.ambiguous());
    }

    #[test]
    fn conformance_details_reported() {
        let spec = make_spec(
            "uc",
            "2026-01-01T00:00:00Z",
            &[("day-of-week", "WEEKDAY"), ("region", "US")],
        );
        let candidates = vec![make_candidate(spec)];

        let profile = CovariateProfile::builder()
            .put("day-of-week", "WEEKDAY")
            .put("region", "EU")
            .build();
        let declarations = vec![
            CovariateDeclaration::new("day-of-week", CovariateCategory::Temporal),
            CovariateDeclaration::new("region", CovariateCategory::Infrastructure),
        ];

        let result = select(&candidates, &profile, &declarations).unwrap();
        assert_eq!(result.conformance().len(), 2);
        assert!(result.conformance()[0].conforms()); // day-of-week matches
        assert!(!result.conformance()[1].conforms()); // region differs
        assert_eq!(result.non_conforming().len(), 1);
        assert_eq!(result.non_conforming()[0].key(), "region");
    }

    #[test]
    fn no_declarations_selects_first_by_recency() {
        let spec_a = make_spec("uc", "2026-01-01T00:00:00Z", &[]);
        let spec_b = make_spec("uc", "2026-06-01T00:00:00Z", &[]);
        let candidates = vec![make_candidate(spec_a), make_candidate(spec_b)];

        let result = select(&candidates, &CovariateProfile::empty(), &[]).unwrap();
        assert_eq!(result.selected().generated_at, "2026-06-01T00:00:00Z");
    }

    #[test]
    fn selection_error_display_no_candidates() {
        let err = SelectionError::NoCandidates {
            service_contract_id: "uc".to_string(),
        };
        assert!(err.to_string().contains("no baseline candidates"));
    }

    #[test]
    fn selection_error_display_config_mismatch() {
        let err = SelectionError::ConfigurationMismatch {
            service_contract_id: "uc".to_string(),
            required: vec![("model".to_string(), "claude".to_string())],
            available: vec![vec![("model".to_string(), "gpt-4o".to_string())]],
        };
        let msg = err.to_string();
        assert!(msg.contains("model=claude"));
        assert!(msg.contains("model=gpt-4o"));
    }
}
