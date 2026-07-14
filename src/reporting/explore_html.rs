//! Exploration comparison HTML report.
//!
//! Renders a single, self-contained HTML page comparing the configurations of
//! one or more explore experiments — overall and per criterion — so a reader
//! can see at a glance which configuration performs best without reading the
//! raw exploration YAML.
//!
//! The input is a directory laid out as `<root>/<service>/*.yaml`, as written
//! by [`ExploreSpecWriter`](crate::spec::explore::ExploreSpecWriter). Files
//! that are not `feotest-spec-1` exploration specs are skipped defensively.
//! Grouping and labelling use the YAML body only (`useCaseId`,
//! `configuration`) — filenames are never parsed.
//!
//! The report introduces no statistics of its own: every number shown is read
//! from the spec or is a nearest-rank percentile over the recorded passing
//! latencies. Ranking is a plain ordered sort, deliberately not a statistical
//! claim — no confidence intervals and no cross-configuration significance
//! testing. The one comment the report makes on the *comparison* is the
//! bounded "too close to call" marker between equally-reliable adjacent
//! configurations whose median latencies differ by less than a fixed
//! presentational margin.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io;
use std::path::Path;

use crate::spec::explore::{ExplorationSpec, FactorYamlValue};
use crate::statistics::latency::{min_samples_for, nearest_rank_percentile};

/// Relative median-latency margin below which two equally-reliable adjacent
/// configurations are flagged "too close to call". A presentational margin on
/// the ordering — not a significance test.
const NEAR_TIE_LATENCY_RELATIVE: f64 = 0.05;

/// Most latency points drawn per distribution strip; denser recordings are
/// decimated evenly so the SVG stays lightweight.
const STRIP_MAX_POINTS: usize = 150;

/// Generates the exploration comparison report from a directory of
/// exploration specs.
pub struct ExploreHtmlReportWriter;

impl ExploreHtmlReportWriter {
    /// Generates the comparison page over every exploration spec beneath
    /// `root` (one sub-directory per service contract, one YAML file per
    /// configuration).
    ///
    /// # Errors
    ///
    /// Returns an error if `root` cannot be read. Individual files that fail
    /// to parse, or that carry a schema other than `feotest-spec-1`, are
    /// skipped rather than failing the report.
    pub fn generate(root: &Path) -> io::Result<String> {
        let services = collect_services(root)?;
        Ok(render(&services))
    }

    /// Generates the comparison page and writes it to `output`, creating
    /// parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an error if `root` cannot be read or the file cannot be
    /// written.
    pub fn write_to_file(root: &Path, output: &Path) -> io::Result<()> {
        let html = Self::generate(root)?;
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, html)
    }
}

// ── the comparison model ────────────────────────────────────────────────────

/// All configurations explored for one service contract.
struct ServiceComparison {
    service: String,
    variants: Vec<Variant>,
}

/// One explored configuration, distilled from its spec.
struct Variant {
    label: String,
    observed: f64,
    successes: u32,
    failures: u32,
    samples_executed: u32,
    termination: String,
    avg_time_per_sample_ms: Option<u64>,
    avg_tokens_per_sample: Option<u64>,
    criteria: BTreeMap<String, CriterionCell>,
    /// Passing-trial durations, sorted ascending, as recorded in the spec.
    latencies_ms: Vec<u64>,
    factors: Vec<(String, String)>,
}

/// One criterion's tally within a configuration.
struct CriterionCell {
    observed: f64,
    successes: u32,
    failures: u32,
}

impl Variant {
    /// The median passing latency, when enough passing samples were recorded
    /// to state one.
    fn p50_ms(&self) -> Option<f64> {
        percentile_ms(&self.latencies_ms, 0.50)
    }
}

/// A variant with its leaderboard rank (competition ranking: near-ties share
/// a rank) and near-tie marker.
struct RankedVariant<'a> {
    rank: usize,
    near_tie: bool,
    variant: &'a Variant,
}

// ── collection ──────────────────────────────────────────────────────────────

/// Reads every exploration spec beneath `root` and groups the configurations
/// by service contract, in name order.
fn collect_services(root: &Path) -> io::Result<Vec<ServiceComparison>> {
    let mut by_service: BTreeMap<String, Vec<Variant>> = BTreeMap::new();
    for dir in sorted_entries(root)? {
        if !dir.is_dir() {
            continue;
        }
        for file in sorted_entries(&dir)? {
            if file.extension().is_none_or(|ext| ext != "yaml") {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(spec) = ExplorationSpec::from_yaml(&body) else {
                continue;
            };
            if spec.schema_version != "feotest-spec-1" {
                continue;
            }
            let service = spec.service_contract_id.clone();
            let variants = by_service.entry(service).or_default();
            let variant = build_variant(spec, variants.len());
            variants.push(variant);
        }
    }
    Ok(by_service
        .into_iter()
        .map(|(service, variants)| ServiceComparison { service, variants })
        .collect())
}

/// The entries of `dir`, sorted by path for a deterministic reading order.
fn sorted_entries(dir: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    Ok(entries)
}

/// Distils one spec into a comparison variant. `ordinal` labels a spec that
/// carries neither a configuration name nor factors.
fn build_variant(spec: ExplorationSpec, ordinal: usize) -> Variant {
    let factors: Vec<(String, String)> = spec
        .execution_context
        .iter()
        .map(|(key, value)| (key.clone(), factor_display(value)))
        .collect();
    let label = spec
        .configuration
        .clone()
        .or_else(|| factor_label(&factors))
        .unwrap_or_else(|| format!("configuration-{}", ordinal + 1));
    let criteria = spec
        .statistics
        .criteria
        .as_ref()
        .map(|criteria| {
            criteria
                .iter()
                .map(|(name, block)| {
                    (
                        name.clone(),
                        CriterionCell {
                            observed: block.observed,
                            successes: block.successes,
                            failures: block.failures,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Variant {
        label,
        observed: spec.statistics.observed,
        successes: spec.statistics.successes,
        failures: spec.statistics.failures,
        samples_executed: spec.execution.samples_executed,
        termination: spec
            .execution
            .termination_reason
            .clone()
            .unwrap_or_else(|| "UNKNOWN".to_owned()),
        avg_time_per_sample_ms: spec.cost.as_ref().map(|cost| cost.avg_time_per_sample_ms),
        avg_tokens_per_sample: spec.cost.as_ref().map(|cost| cost.avg_tokens_per_sample),
        criteria,
        latencies_ms: spec
            .latency
            .map(|latency| latency.sorted_passing_latencies_ms)
            .unwrap_or_default(),
        factors,
    }
}

/// A `key=value` label from the factor map, or `None` when there are no
/// factors.
fn factor_label(factors: &[(String, String)]) -> Option<String> {
    if factors.is_empty() {
        return None;
    }
    Some(
        factors
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A factor value rendered for display.
fn factor_display(value: &FactorYamlValue) -> String {
    match value {
        FactorYamlValue::String(s) => s.clone(),
        FactorYamlValue::Float(f) => f.to_string(),
        FactorYamlValue::Int(i) => i.to_string(),
        FactorYamlValue::Bool(b) => b.to_string(),
    }
}

// ── ranking ─────────────────────────────────────────────────────────────────

/// Ranks a service's variants: observed rate descending, then median passing
/// latency ascending, then average cost ascending. Adjacent equally-reliable
/// variants whose medians are within the near-tie margin share a rank
/// (competition ranking) and carry the near-tie marker.
fn ranked(variants: &[Variant]) -> Vec<RankedVariant<'_>> {
    let mut order: Vec<&Variant> = variants.iter().collect();
    order.sort_by(|a, b| {
        b.observed
            .total_cmp(&a.observed)
            .then_with(|| p50_for_ranking(a).total_cmp(&p50_for_ranking(b)))
            .then_with(|| avg_cost_for_ranking(a).cmp(&avg_cost_for_ranking(b)))
    });

    let mut result: Vec<RankedVariant<'_>> = Vec::with_capacity(order.len());
    for (index, variant) in order.into_iter().enumerate() {
        let tied_with_previous = result
            .last()
            .is_some_and(|previous| is_near_tie(previous.variant, variant));
        let rank = if tied_with_previous {
            let previous_rank = result.last().map_or(1, |previous| previous.rank);
            if let Some(previous) = result.last_mut() {
                previous.near_tie = true;
            }
            previous_rank
        } else {
            index + 1
        };
        result.push(RankedVariant {
            rank,
            near_tie: tied_with_previous,
            variant,
        });
    }
    result
}

/// The median used for ordering; a variant with no stateable median sorts
/// last among equally-reliable variants.
fn p50_for_ranking(variant: &Variant) -> f64 {
    variant.p50_ms().unwrap_or(f64::MAX)
}

/// The average per-sample cost used as the final tie-break.
fn avg_cost_for_ranking(variant: &Variant) -> u64 {
    variant.avg_time_per_sample_ms.unwrap_or(u64::MAX)
}

/// Whether two adjacent ranked variants are too close to call: identical
/// observed pass rate and medians within the presentational margin. A
/// difference in pass rate is never softened into a near-tie, and a variant
/// with no median never ties.
fn is_near_tie(a: &Variant, b: &Variant) -> bool {
    if a.observed.to_bits() != b.observed.to_bits() {
        return false;
    }
    match (a.p50_ms(), b.p50_ms()) {
        (Some(p50_a), Some(p50_b)) => {
            let larger = p50_a.max(p50_b);
            larger > 0.0 && (p50_a - p50_b).abs() < NEAR_TIE_LATENCY_RELATIVE * larger
        }
        _ => false,
    }
}

// ── percentiles ─────────────────────────────────────────────────────────────

/// The nearest-rank percentile of the recorded passing latencies, or `None`
/// when fewer samples passed than the percentile's minimum-sample floor.
fn percentile_ms(latencies_ms: &[u64], fraction: f64) -> Option<f64> {
    if latencies_ms.len() < min_samples_for(fraction) as usize {
        return None;
    }
    let values: Vec<f64> = latencies_ms.iter().map(|&ms| ms_to_f64(ms)).collect();
    Some(nearest_rank_percentile(&values, fraction))
}

/// A latency in milliseconds as `f64`.
#[allow(
    clippy::cast_precision_loss,
    reason = "trial durations in ms are far below 2^52"
)]
const fn ms_to_f64(ms: u64) -> f64 {
    ms as f64
}

// ── rendering ───────────────────────────────────────────────────────────────

/// Renders the full page.
fn render(services: &[ServiceComparison]) -> String {
    let mut out = String::new();
    append_head(&mut out);
    let _ =
        out.write_str("<body>\n<header>\n<h1>feotest Exploration Report</h1>\n</header>\n<main>\n");
    if services.is_empty() {
        let _ = out.write_str(
            "<p class=\"empty\">No explorations found. Run an explore experiment to \
             produce configuration data, then regenerate this report.</p>\n",
        );
    } else {
        append_overview(&mut out, services);
        if services
            .iter()
            .any(|service| ranked(&service.variants).iter().any(|r| r.near_tie))
        {
            append_near_tie_legend(&mut out);
        }
        for service in services {
            append_service(&mut out, service);
        }
    }
    let _ = out.write_str("</main>\n<footer>\n<p class=\"timestamp\">Generated by feotest</p>\n</footer>\n</body>\n</html>\n");
    out
}

/// Renders the document head with the embedded stylesheet.
fn append_head(out: &mut String) {
    let _ = out.write_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>feotest Exploration Report</title>\n");
    let _ = write!(out, "<style>{CSS}</style>\n</head>\n");
}

/// Renders the overview table: one row per service with its best performer.
fn append_overview(out: &mut String, services: &[ServiceComparison]) {
    let _ = out.write_str(
        "<section class=\"overview\">\n<h2>Overview</h2>\n<table>\n<thead>\n<tr>\
         <th>Service</th><th>Configurations</th><th>Best overall</th></tr>\n</thead>\n<tbody>\n",
    );
    for service in services {
        let ranked = ranked(&service.variants);
        let leaders: Vec<&str> = ranked
            .iter()
            .filter(|r| r.rank == 1)
            .map(|r| r.variant.label.as_str())
            .collect();
        let _ = writeln!(
            out,
            "<tr><td><a href=\"#{anchor}\">{service}</a></td><td class=\"num\">{count}</td><td>{best}</td></tr>",
            anchor = escape(&service.service),
            service = escape(&service.service),
            count = service.variants.len(),
            best = escape(&leaders.join(" \u{2248} ")),
        );
    }
    let _ = out.write_str("</tbody>\n</table>\n</section>\n");
}

/// Renders the legend explaining the near-tie marker.
fn append_near_tie_legend(out: &mut String) {
    let _ = out.write_str(
        "<p class=\"legend\">\u{2248} too close to call \u{2014} equally reliable, medians \
         within 5%. A presentational margin on the ordering, not a significance test: \
         this report makes no claim that one configuration statistically beats another.</p>\n",
    );
}

/// Renders one service's section: leaderboard, per-criterion matrix, and
/// latency distribution strips.
fn append_service(out: &mut String, service: &ServiceComparison) {
    let _ = write!(
        out,
        "<section class=\"service\" id=\"{anchor}\">\n<h2>{name}</h2>\n",
        anchor = escape(&service.service),
        name = escape(&service.service),
    );
    let ranked = ranked(&service.variants);
    append_leaderboard(out, &ranked);
    append_criterion_matrix(out, &ranked);
    append_latency_strips(out, &ranked);
    let _ = out.write_str("</section>\n");
}

/// Renders the ranked leaderboard table.
fn append_leaderboard(out: &mut String, ranked: &[RankedVariant<'_>]) {
    let max_stated = ranked
        .iter()
        .flat_map(|r| {
            [0.50, 0.95, 0.99]
                .into_iter()
                .filter_map(|fraction| percentile_ms(&r.variant.latencies_ms, fraction))
        })
        .fold(0.0_f64, f64::max);
    let _ = out.write_str(
        "<h3>Leaderboard</h3>\n<table class=\"leaderboard\">\n<thead>\n<tr>\
         <th>#</th><th>Configuration</th><th>Pass rate</th><th>p50</th><th>p95</th><th>p99</th>\
         <th>Avg cost</th><th>Samples</th><th>Termination</th></tr>\n</thead>\n<tbody>\n",
    );
    for row in ranked {
        let variant = row.variant;
        let marker = if row.near_tie { " \u{2248}" } else { "" };
        let _ = write!(
            out,
            "<tr>\n<td class=\"rank\">{rank}{marker}</td>\n<td>{label}</td>\n",
            rank = row.rank,
            label = variant_cell(variant),
        );
        let _ = writeln!(
            out,
            "<td class=\"passrate\">{bar} {rate:.3} ({pass}/{total})</td>",
            bar = rate_bar(variant.observed),
            rate = variant.observed,
            pass = variant.successes,
            total = variant.successes + variant.failures,
        );
        append_latency_cell(out, variant, 0.50, max_stated);
        append_latency_cell(out, variant, 0.95, max_stated);
        append_latency_cell(out, variant, 0.99, max_stated);
        let _ = write!(
            out,
            "<td class=\"cost\">{cost}</td>\n<td class=\"num\">{samples}</td>\n<td>{badge}</td>\n</tr>\n",
            cost = cost_cell(variant),
            samples = variant.samples_executed,
            badge = termination_badge(&variant.termination),
        );
    }
    let _ = out.write_str("</tbody>\n</table>\n");
}

/// Renders a leaderboard latency cell: a bar scaled to the service's largest
/// stated percentile, or a dash when the percentile cannot be stated.
fn append_latency_cell(out: &mut String, variant: &Variant, fraction: f64, max_stated: f64) {
    match percentile_ms(&variant.latencies_ms, fraction) {
        Some(value) if max_stated > 0.0 => {
            let width = (value / max_stated * 100.0).min(100.0);
            let _ = writeln!(
                out,
                "<td class=\"latency\"><span class=\"bar-track narrow\">\
                 <span class=\"bar-fill muted\" style=\"width:{width:.0}%\"></span></span> \
                 {value:.0}ms</td>",
            );
        }
        Some(value) => {
            let _ = writeln!(out, "<td class=\"latency\">{value:.0}ms</td>");
        }
        None => {
            let _ = out.write_str("<td class=\"latency muted\">-</td>\n");
        }
    }
}

/// The configuration label, with the factor map in a collapsed details block
/// when factors were recorded.
fn variant_cell(variant: &Variant) -> String {
    if variant.factors.is_empty() {
        return escape(&variant.label);
    }
    let mut cell = format!(
        "<details class=\"factor-list\"><summary>{}</summary><dl>",
        escape(&variant.label)
    );
    for (key, value) in &variant.factors {
        let _ = write!(
            cell,
            "<dt>{key}</dt><dd>{value}</dd>",
            key = escape(key),
            value = escape(value),
        );
    }
    cell.push_str("</dl></details>");
    cell
}

/// A pass-rate bar, filled proportionally.
fn rate_bar(rate: f64) -> String {
    let width = (rate * 100.0).clamp(0.0, 100.0);
    format!(
        "<span class=\"bar-track\"><span class=\"bar-fill pass\" style=\"width:{width:.1}%\"></span></span>"
    )
}

/// The average per-sample cost, when recorded.
fn cost_cell(variant: &Variant) -> String {
    match (
        variant.avg_time_per_sample_ms,
        variant.avg_tokens_per_sample,
    ) {
        (Some(ms), Some(tokens)) if tokens > 0 => {
            format!("{ms}ms \u{b7} {tokens} tok")
        }
        (Some(ms), _) => format!("{ms}ms"),
        (None, _) => "-".to_owned(),
    }
}

/// A termination badge; anything other than `COMPLETED` is flagged.
fn termination_badge(termination: &str) -> String {
    let class = if termination == "COMPLETED" {
        "badge ok"
    } else {
        "badge flagged"
    };
    format!("<span class=\"{class}\">{}</span>", escape(termination))
}

/// Renders the per-criterion comparison matrix over the union of criteria.
fn append_criterion_matrix(out: &mut String, ranked: &[RankedVariant<'_>]) {
    let criteria: BTreeSet<&str> = ranked
        .iter()
        .flat_map(|r| r.variant.criteria.keys().map(String::as_str))
        .collect();
    if criteria.is_empty() {
        return;
    }
    let _ = out.write_str("<h3>Per-criterion comparison</h3>\n<table class=\"criterion-matrix\">\n<thead>\n<tr><th>Configuration</th>");
    for name in &criteria {
        let _ = write!(out, "<th class=\"criterion-name\">{}</th>", escape(name));
    }
    let _ = out.write_str("</tr>\n</thead>\n<tbody>\n");
    for row in ranked {
        let _ = write!(out, "<tr><td>{}</td>", escape(&row.variant.label));
        for name in &criteria {
            match row.variant.criteria.get(*name) {
                Some(cell) => {
                    let _ = write!(
                        out,
                        "<td class=\"cell\">{bar} {rate:.3} ({pass}/{total})</td>",
                        bar = rate_bar(cell.observed),
                        rate = cell.observed,
                        pass = cell.successes,
                        total = cell.successes + cell.failures,
                    );
                }
                None => {
                    let _ = out.write_str("<td class=\"cell muted\">n/a</td>");
                }
            }
        }
        let _ = out.write_str("</tr>\n");
    }
    let _ = out.write_str("</tbody>\n</table>\n");
}

/// Renders one latency-distribution strip per configuration, on a shared
/// scale, with the p50 / p95 / p99 percentiles marked.
fn append_latency_strips(out: &mut String, ranked: &[RankedVariant<'_>]) {
    let max_latency = ranked
        .iter()
        .filter_map(|r| r.variant.latencies_ms.last().copied())
        .max();
    let Some(max_latency) = max_latency else {
        return;
    };
    let _ = out.write_str("<h3>Latency distribution</h3>\n<div class=\"latency-strips\">\n");
    for row in ranked {
        append_latency_strip(out, row.variant, max_latency);
    }
    let _ = writeln!(
        out,
        "<p class=\"strip-axis\">0ms \u{2013} {max_latency}ms \u{b7} \
         <span class=\"mark-p50\">\u{258d}p50</span> \
         <span class=\"mark-p95\">\u{258d}p95</span> \
         <span class=\"mark-p99\">\u{258d}p99</span> \
         \u{b7} percentiles are computed from every passing sample; the dots are an \
         evenly-thinned subset, plotted for shape only</p>",
    );
    let _ = out.write_str("</div>\n");
}

/// Renders one configuration's strip: its passing latencies as dots on a
/// shared 0..max axis, with markers at each stateable percentile.
fn append_latency_strip(out: &mut String, variant: &Variant, max_latency_ms: u64) {
    let _ = write!(
        out,
        "<div class=\"strip\"><span class=\"strip-label\">{}</span>",
        escape(&variant.label)
    );
    if variant.latencies_ms.is_empty() {
        let _ = out.write_str("<span class=\"muted\">no passing samples</span></div>\n");
        return;
    }
    let scale = ms_to_f64(max_latency_ms).max(1.0);
    let x = |ms: u64| -> f64 { (ms_to_f64(ms) / scale).mul_add(304.0, 8.0) };
    let _ = out.write_str(
        "<svg class=\"latency-strip-svg\" width=\"320\" height=\"24\" viewBox=\"0 0 320 24\" \
         role=\"img\">",
    );
    let _ = write!(
        out,
        "<line class=\"strip-range\" x1=\"{min:.1}\" y1=\"14\" x2=\"{max:.1}\" y2=\"14\"/>",
        min = x(variant.latencies_ms[0]),
        max = x(*variant.latencies_ms.last().unwrap_or(&0)),
    );
    let step = (variant.latencies_ms.len() / STRIP_MAX_POINTS).max(1);
    for &ms in variant.latencies_ms.iter().step_by(step) {
        let _ = write!(
            out,
            "<circle class=\"strip-dot\" cx=\"{cx:.1}\" cy=\"14\" r=\"2.5\"/>",
            cx = x(ms),
        );
    }
    for (fraction, class) in [
        (0.50, "strip-p50"),
        (0.95, "strip-p95"),
        (0.99, "strip-p99"),
    ] {
        if let Some(value) = percentile_ms(&variant.latencies_ms, fraction) {
            let cx = (value / scale).mul_add(304.0, 8.0);
            let _ = write!(
                out,
                "<line class=\"{class}\" x1=\"{cx:.1}\" y1=\"4\" x2=\"{cx:.1}\" y2=\"22\"/>",
            );
        }
    }
    let _ = out.write_str("</svg>");
    let _ = write!(
        out,
        "<span class=\"strip-note\">{count} passed</span>",
        count = variant.latencies_ms.len(),
    );
    let _ = out.write_str("</div>\n");
}

/// Escapes text for HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The embedded stylesheet. Shares the verdict report's palette and idioms so
/// the two reports read as one family.
const CSS: &str = "\
:root {\
  --pass-color: #2e7d32; --fail-color: #c62828; --inconclusive-color: #6a1b9a;\
  --advisory-color: #f9a825; --border-color: #dee2e6; --bg-light: #f8f9fa;\
  --bg-white: #ffffff; --text-color: #212529; --text-muted: #6c757d;\
}\
* { box-sizing: border-box; margin: 0; padding: 0; }\
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;\
  color: var(--text-color); background: var(--bg-light); line-height: 1.5; padding: 2rem; }\
header, main, footer { max-width: 1200px; margin: 0 auto; }\
h1 { font-size: 1.5rem; margin-bottom: 1rem; }\
h2 { font-size: 1.15rem; margin: 1.5rem 0 0.5rem; border-bottom: 2px solid var(--border-color); }\
h3 { font-size: 0.95rem; margin: 1rem 0 0.35rem; color: var(--text-muted); }\
table { width: 100%; border-collapse: collapse; background: var(--bg-white); font-size: 0.875rem; }\
th, td { text-align: left; padding: 0.4rem 0.6rem; border: 1px solid var(--border-color); }\
th { background: var(--bg-light); }\
td.num, td.rank { text-align: right; width: 1%; white-space: nowrap; }\
.muted { color: var(--text-muted); }\
.empty { color: var(--text-muted); }\
.legend { font-size: 0.8rem; color: var(--text-muted); margin: 0.5rem 0 1rem; }\
.timestamp { color: var(--text-muted); font-size: 0.8rem; margin-top: 1.5rem; }\
.badge { display: inline-block; padding: 0 0.4rem; border-radius: 3px; font-size: 0.75rem; }\
.badge.ok { background: #e8f5e9; color: var(--pass-color); }\
.badge.flagged { background: #fff8e1; color: var(--advisory-color); }\
.bar-track { display: inline-block; width: 90px; height: 8px; background: var(--bg-light);\
  border: 1px solid var(--border-color); border-radius: 4px; vertical-align: middle; }\
.bar-track.narrow { width: 60px; }\
.bar-fill { display: block; height: 100%; border-radius: 4px; }\
.bar-fill.pass { background: var(--pass-color); }\
.bar-fill.fail { background: var(--pass-color); opacity: 0.8; }\
.bar-fill.muted { background: var(--text-muted); }\
.factor-list summary { cursor: pointer; }\
.factor-list dl { margin: 0.25rem 0 0 0.75rem; font-size: 0.8rem; }\
.factor-list dt { float: left; clear: left; margin-right: 0.4rem; color: var(--text-muted); }\
.latency-strips .strip { display: flex; align-items: center; gap: 0.75rem; margin: 0.15rem 0; }\
.strip-label { width: 8rem; font-size: 0.8rem; }\
.strip-range { stroke: var(--border-color); stroke-width: 6; stroke-linecap: round; }\
.strip-dot { fill: var(--pass-color); fill-opacity: 0.25; }\
.strip-p50 { stroke: var(--fail-color); stroke-width: 2; }\
.strip-p95 { stroke: var(--advisory-color); stroke-width: 2; }\
.strip-p99 { stroke: var(--inconclusive-color); stroke-width: 2; }\
.strip-axis { font-size: 0.75rem; color: var(--text-muted); margin-left: 8.75rem; }\
.strip-note { font-size: 0.75rem; color: var(--text-muted); }\
.mark-p50 { color: var(--fail-color); }\
.mark-p95 { color: var(--advisory-color); }\
.mark-p99 { color: var(--inconclusive-color); }\
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::baseline::{CostBlock, ExecutionBlock};
    use crate::spec::explore::{
        ExplorationCriterionBlock, ExplorationLatencyBlock, ExplorationStatisticsBlock,
    };

    fn spec(
        service: &str,
        configuration: &str,
        observed: f64,
        successes: u32,
        failures: u32,
        latencies: Vec<u64>,
    ) -> ExplorationSpec {
        ExplorationSpec {
            schema_version: "feotest-spec-1".to_owned(),
            service_contract_id: service.to_owned(),
            generated_at: "2026-07-14T00:00:00Z".to_owned(),
            experiment_id: Some("comparison".to_owned()),
            configuration: Some(configuration.to_owned()),
            execution_context: BTreeMap::new(),
            execution: ExecutionBlock {
                samples_planned: successes + failures,
                samples_executed: successes + failures,
                termination_reason: Some("COMPLETED".to_owned()),
            },
            statistics: ExplorationStatisticsBlock {
                observed,
                successes,
                failures,
                failure_distribution: None,
                criteria: Some(BTreeMap::from([(
                    "primary".to_owned(),
                    ExplorationCriterionBlock {
                        observed,
                        successes,
                        failures,
                        failure_distribution: None,
                    },
                )])),
            },
            latency: (!latencies.is_empty()).then_some(ExplorationLatencyBlock {
                sorted_passing_latencies_ms: latencies,
            }),
            cost: Some(CostBlock {
                total_time_ms: 100,
                avg_time_per_sample_ms: 3,
                total_tokens: 10,
                avg_tokens_per_sample: 1,
            }),
        }
    }

    fn variant_from(spec: ExplorationSpec) -> Variant {
        build_variant(spec, 0)
    }

    fn write_spec(root: &Path, spec: &ExplorationSpec, filename: &str) {
        let dir = root.join(&spec.service_contract_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(filename), spec.to_yaml().unwrap()).unwrap();
    }

    #[test]
    fn ranks_by_rate_then_median_then_cost() {
        let slow = variant_from(spec("svc", "slow", 0.9, 90, 10, vec![9; 20]));
        let fast = variant_from(spec("svc", "fast", 0.9, 90, 10, vec![3; 20]));
        let best = variant_from(spec("svc", "best", 0.95, 95, 5, vec![9; 20]));
        let variants = vec![slow, fast, best];

        let ranked = ranked(&variants);
        let labels: Vec<&str> = ranked.iter().map(|r| r.variant.label.as_str()).collect();
        assert_eq!(labels, ["best", "fast", "slow"]);
    }

    #[test]
    fn equal_rates_with_close_medians_share_a_rank() {
        let a = variant_from(spec("svc", "a", 0.9, 90, 10, vec![100; 20]));
        let b = variant_from(spec("svc", "b", 0.9, 90, 10, vec![102; 20]));
        let variants = vec![a, b];

        let ranked = ranked(&variants);
        assert_eq!(ranked[0].rank, 1);
        assert_eq!(ranked[1].rank, 1);
        assert!(ranked[0].near_tie && ranked[1].near_tie);
    }

    #[test]
    fn a_pass_rate_difference_is_never_a_near_tie() {
        let a = variant_from(spec("svc", "a", 0.91, 91, 9, vec![100; 20]));
        let b = variant_from(spec("svc", "b", 0.9, 90, 10, vec![100; 20]));
        assert!(!is_near_tie(&a, &b));
    }

    #[test]
    fn a_variant_without_a_median_never_ties() {
        let a = variant_from(spec("svc", "a", 0.9, 90, 10, vec![100; 20]));
        let b = variant_from(spec("svc", "b", 0.9, 90, 10, Vec::new()));
        assert!(!is_near_tie(&a, &b));
    }

    #[test]
    fn percentiles_below_their_sample_floor_are_unstated() {
        assert!(percentile_ms(&[1, 2, 3, 4], 0.50).is_none());
        assert!(percentile_ms(&[1, 2, 3, 4, 5], 0.50).is_some());
        assert!(percentile_ms(&(1..=19).collect::<Vec<u64>>(), 0.95).is_none());
        assert!(percentile_ms(&(1..=20).collect::<Vec<u64>>(), 0.95).is_some());
    }

    #[test]
    fn generates_a_comparison_over_a_spec_directory() {
        let dir = tempfile::tempdir().unwrap();
        write_spec(
            dir.path(),
            &spec("svc.alpha", "candidate-1", 0.95, 95, 5, vec![3; 20]),
            "candidate-1.yaml",
        );
        write_spec(
            dir.path(),
            &spec("svc.alpha", "candidate-2", 0.85, 85, 15, vec![5; 20]),
            "candidate-2.yaml",
        );

        let html = ExploreHtmlReportWriter::generate(dir.path()).unwrap();
        assert!(html.contains("svc.alpha"));
        assert!(html.contains("candidate-1"));
        assert!(html.contains("candidate-2"));
        assert!(html.contains("Leaderboard"));
        assert!(html.contains("Per-criterion comparison"));
        assert!(html.contains("Latency distribution"));
        // Ranked order: candidate-1 (higher rate) precedes candidate-2.
        assert!(html.find("candidate-1").unwrap() < html.find("candidate-2").unwrap());
        // Self-contained: no external references.
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn a_missing_criterion_renders_as_not_applicable() {
        let dir = tempfile::tempdir().unwrap();
        let mut with_extra = spec("svc", "rich", 0.9, 90, 10, vec![3; 20]);
        if let Some(criteria) = with_extra.statistics.criteria.as_mut() {
            criteria.insert(
                "secondary".to_owned(),
                ExplorationCriterionBlock {
                    observed: 1.0,
                    successes: 100,
                    failures: 0,
                    failure_distribution: None,
                },
            );
        }
        write_spec(dir.path(), &with_extra, "rich.yaml");
        write_spec(
            dir.path(),
            &spec("svc", "plain", 0.9, 90, 10, vec![3; 20]),
            "plain.yaml",
        );

        let html = ExploreHtmlReportWriter::generate(dir.path()).unwrap();
        assert!(html.contains("n/a"));
        assert!(html.contains("secondary"));
    }

    #[test]
    fn a_large_sample_states_p99_in_leaderboard_and_strip() {
        let dir = tempfile::tempdir().unwrap();
        let latencies: Vec<u64> = (1..=200).collect();
        write_spec(
            dir.path(),
            &spec("svc", "big", 0.9, 180, 20, latencies),
            "big.yaml",
        );

        let html = ExploreHtmlReportWriter::generate(dir.path()).unwrap();
        assert!(html.contains("<th>p99</th>"));
        assert!(html.contains("198ms")); // nearest-rank p99 of 1..=200
        assert!(html.contains("strip-p99"));
        assert!(html.contains("strip-p95"));
    }

    #[test]
    fn a_small_sample_leaves_p99_unstated() {
        let dir = tempfile::tempdir().unwrap();
        write_spec(
            dir.path(),
            &spec("svc", "small", 0.9, 18, 2, vec![3; 20]),
            "small.yaml",
        );

        let html = ExploreHtmlReportWriter::generate(dir.path()).unwrap();
        // The p99 column exists but the cell (and the strip marker) are absent.
        assert!(html.contains("<th>p99</th>"));
        assert!(!html.contains("<line class=\"strip-p99\""));
    }

    #[test]
    fn an_empty_root_reports_no_explorations() {
        let dir = tempfile::tempdir().unwrap();
        let html = ExploreHtmlReportWriter::generate(dir.path()).unwrap();
        assert!(html.contains("No explorations found"));
    }

    #[test]
    fn non_spec_yaml_is_skipped_defensively() {
        let dir = tempfile::tempdir().unwrap();
        let service_dir = dir.path().join("svc");
        std::fs::create_dir_all(&service_dir).unwrap();
        std::fs::write(service_dir.join("junk.yaml"), "not: a spec").unwrap();
        write_spec(
            dir.path(),
            &spec("svc", "real", 0.9, 90, 10, vec![3; 20]),
            "real.yaml",
        );

        let html = ExploreHtmlReportWriter::generate(dir.path()).unwrap();
        assert!(html.contains("real"));
        assert!(!html.contains("junk"));
    }
}
