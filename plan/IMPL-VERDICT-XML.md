# IMPL-VERDICT-XML — Complete RP07 verdict XML for feotest

## Feature reference

Inventory code: **RP07** — Verdict XML interchange format

Specification:
[`inventory/catalog/reporting/RP07-verdict-xml-interchange/README.md`](../../inventory/catalog/reporting/RP07-verdict-xml-interchange/README.md)

Schema:
[`inventory/catalog/reporting/RP07-verdict-xml-interchange/verdict-1.0.xsd`](../../inventory/catalog/reporting/RP07-verdict-xml-interchange/verdict-1.0.xsd)

## Current state

feotest already serialises verdicts to RP07 XML (`src/reporting/verdict_xml.rs`)
covering: identity, execution, functional dimension, latency dimension,
statistics, covariates, provenance, baseline, termination, cost, warnings,
and verdict.

Three elements added to the RP07 standard are missing:

| Element | RP07 section | feotest status |
|---------|-------------|----------------|
| `<pacing>` | 12 | `PacingConfig` exists in `controls/config.rs` but is not carried to the verdict record or XML |
| `<environment>` | 13 | No model exists |
| `<expiration>` | 14 (inside `<provenance>`) | No model exists (EX08 is `not started`) |
| `correlation-id` attribute | Root element | Not emitted |

## Goal

Emit the full RP07 schema from feotest, including pacing summary,
environment metadata, baseline expiration, and correlation ID. After this
work, feotest's RP07 status moves from `partial` to `done`.

## Scope

This document covers only the verdict XML serialisation path. It does not
implement the features themselves end-to-end (e.g., it does not implement
EX08 baseline expiration policy enforcement). It adds the data structures
needed to carry these values through the verdict record to XML, and wires
up the serialisation. Populating these fields from the test runner is a
separate concern — the fields are optional in RP07, so emitting them only
when data is available is correct behaviour.

## Design

### 1. Correlation ID

Add an optional `correlation_id` field to `VerdictRecord`.

**VerdictRecord** (`src/verdict/record.rs`):

```rust
pub struct VerdictRecord {
    // ... existing fields ...
    correlation_id: Option<String>,
}
```

Builder method:

```rust
pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
    self.correlation_id = Some(id.into());
    self
}
```

Accessor:

```rust
pub fn correlation_id(&self) -> Option<&str> {
    self.correlation_id.as_deref()
}
```

**XML writer** — emit on the root element when present:

```rust
if let Some(id) = record.correlation_id() {
    write!(xml, " correlation-id=\"{}\"", escape_attr(id)).unwrap();
}
```

### 2. Pacing summary

Add a `PacingSummary` struct to `src/model/types.rs` that captures the
resolved pacing state at the point of verdict construction.

```rust
/// Resolved pacing constraints recorded on a verdict.
///
/// Captures both the configured limits and the effective values after
/// constraint resolution. All fields reflect the state at verdict time,
/// not the configuration input.
#[derive(Debug, Clone)]
pub struct PacingSummary {
    max_rps: f64,
    max_rpm: f64,
    max_concurrent: u32,
    effective_min_delay_ms: u64,
    effective_concurrency: u32,
    effective_rps: f64,
}
```

This is a reporting-oriented summary, not the `PacingConfig` itself. The
runner resolves `PacingConfig` into effective values and constructs a
`PacingSummary` for the verdict record.

**VerdictRecord** — add optional field + builder method:

```rust
pacing: Option<PacingSummary>,
```

**Mapping from `PacingConfig`** — a `From<&PacingConfig>` or constructor
that computes the effective values:

```rust
impl PacingSummary {
    pub fn from_config(config: &PacingConfig) -> Self {
        let effective_delay = config.effective_delay_ms();
        let effective_rps = if effective_delay > 0 {
            1000.0 / effective_delay as f64
        } else {
            f64::INFINITY
        };
        Self {
            max_rps: config.max_requests_per_second().unwrap_or(0.0),
            max_rpm: config.max_requests_per_minute().unwrap_or(0.0),
            max_concurrent: 1, // feotest is single-threaded today
            effective_min_delay_ms: effective_delay,
            effective_concurrency: 1,
            effective_rps,
        }
    }
}
```

**Note:** feotest does not yet support concurrent requests (RC11 is
`not started`), so `max_concurrent` and `effective_concurrency` are
both 1. The fields exist to match the RP07 schema; they will become
meaningful when RC11 is implemented.

**XML writer** — new `write_pacing` function:

```rust
fn write_pacing(w: &mut String, record: &VerdictRecord) {
    let Some(pacing) = record.pacing() else { return };
    write!(w, "  <pacing").unwrap();
    write!(w, " max-rps=\"{}\"", pacing.max_rps()).unwrap();
    write!(w, " max-rpm=\"{}\"", pacing.max_rpm()).unwrap();
    write!(w, " max-concurrent=\"{}\"", pacing.max_concurrent()).unwrap();
    write!(w, " effective-min-delay-ms=\"{}\"", pacing.effective_min_delay_ms()).unwrap();
    write!(w, " effective-concurrency=\"{}\"", pacing.effective_concurrency()).unwrap();
    write!(w, " effective-rps=\"{}\"", pacing.effective_rps()).unwrap();
    writeln!(w, "/>").unwrap();
}
```

### 3. Environment metadata

Add an optional environment metadata map to `VerdictRecord`.

**VerdictRecord** (`src/verdict/record.rs`):

```rust
environment: Vec<(String, String)>,
```

Use `Vec<(String, String)>` rather than `HashMap` to preserve insertion
order and avoid a dependency on `indexmap`. This matches feotest's existing
pattern for covariate profiles.

Builder method:

```rust
pub fn environment(mut self, entries: Vec<(String, String)>) -> Self {
    self.environment = entries;
    self
}
```

Accessor:

```rust
pub fn environment(&self) -> &[(String, String)] {
    &self.environment
}
```

**XML writer** — new `write_environment` function:

```rust
fn write_environment(w: &mut String, record: &VerdictRecord) {
    if record.environment().is_empty() { return }
    writeln!(w, "  <environment>").unwrap();
    for (key, value) in record.environment() {
        writeln!(w, "    <entry key=\"{}\" value=\"{}\"/>",
            escape_attr(key), escape_attr(value)).unwrap();
    }
    writeln!(w, "  </environment>").unwrap();
}
```

**Population:** the runner or test builder can attach environment
metadata. Likely sources:

- Environment variables prefixed with `FEOTEST_ENV_` (convention-based
  auto-capture, similar to punit's `@EnvironmentMetadata`)
- Explicit builder method on `ProbabilisticTestBuilder`

For the initial implementation, the builder method is sufficient. Auto-
capture is a follow-up.

### 4. Baseline expiration

Add an `ExpirationStatus` model to `src/model/types.rs`.

```rust
/// Baseline freshness status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpirationStatus {
    /// No expiration policy defined.
    NoExpiration,
    /// Baseline is within its validity period.
    Valid,
    /// Baseline is approaching expiration.
    ExpiringSoon,
    /// Baseline is very close to expiration.
    ExpiringImminently,
    /// Baseline has expired.
    Expired,
}

impl ExpirationStatus {
    /// Whether this status warrants a warning in reports.
    pub const fn requires_warning(&self) -> bool {
        matches!(self, Self::ExpiringSoon | Self::ExpiringImminently | Self::Expired)
    }
}
```

```rust
/// Expiration information for a baseline spec.
#[derive(Debug, Clone)]
pub struct ExpirationInfo {
    status: ExpirationStatus,
    expires_at: Option<String>,  // ISO 8601
}
```

**SpecProvenance** — add optional expiration:

```rust
pub struct SpecProvenance {
    // ... existing fields ...
    expiration: Option<ExpirationInfo>,
}
```

With builder method:

```rust
pub fn with_expiration(mut self, info: ExpirationInfo) -> Self {
    self.expiration = Some(info);
    self
}
```

**XML writer** — nest `<expiration>` inside `<provenance>`:

```rust
fn write_provenance(w: &mut String, record: &VerdictRecord) {
    let Some(prov) = record.spec_provenance() else { return };

    let has_expiration = prov.expiration().is_some();
    write!(w, "  <provenance origin=\"{}\"", prov.threshold_origin()).unwrap();
    // ... spec-filename, contract-ref as before ...

    if has_expiration {
        writeln!(w, ">").unwrap();
        let exp = prov.expiration().unwrap();
        write!(w, "    <expiration status=\"{}\"", exp.status().xml_name()).unwrap();
        if let Some(at) = exp.expires_at() {
            write!(w, " expires-at=\"{}\"", escape_attr(at)).unwrap();
        }
        write!(w, " requires-warning=\"{}\"", exp.status().requires_warning()).unwrap();
        writeln!(w, "/>").unwrap();
        writeln!(w, "  </provenance>").unwrap();
    } else {
        writeln!(w, "/>").unwrap();
    }
}
```

**Population:** feotest does not yet implement EX08 (baseline expiration
policy). Until EX08 is implemented, the `expiration` field on
`SpecProvenance` will always be `None` and the `<expiration>` element
will be omitted from the XML. This is correct RP07 behaviour — the
element is optional.

## Wiring into the XML writer

Update `write_record` to call the new element writers in RP07 element
order:

```rust
write_identity(&mut xml, record);
write_execution(&mut xml, record);
write_functional(&mut xml, record);
write_latency(&mut xml, record);
write_statistics(&mut xml, record);
write_covariates(&mut xml, record);
write_provenance(&mut xml, record);   // now emits <expiration> when present
write_baseline(&mut xml, record);
write_termination(&mut xml, record);
write_cost(&mut xml, record);
write_warnings(&mut xml, record);
write_pacing(&mut xml, record);       // new
write_environment(&mut xml, record);  // new
write_verdict(&mut xml, record);
```

## File changes

| File | Change |
|------|--------|
| `src/model/types.rs` | Add `PacingSummary`, `ExpirationStatus`, `ExpirationInfo` |
| `src/model/mod.rs` | Re-export new types |
| `src/verdict/record.rs` | Add `correlation_id`, `pacing`, `environment` fields + builder methods; add `expiration` to `SpecProvenance` |
| `src/reporting/verdict_xml.rs` | Add `correlation-id` to root; add `write_pacing`, `write_environment`; update `write_provenance` for expiration |
| `src/reporting/verdict_xml.rs` (tests) | Add snapshot tests for pacing, environment, expiration |
| `src/ptest/runner.rs` | Populate `PacingSummary` on the verdict record when pacing is configured |

## Testing

### Snapshot tests (extend existing suite in `verdict_xml.rs`)

| Snapshot | Scenario |
|----------|----------|
| `xml_with_pacing` | Verdict with pacing summary attached |
| `xml_with_environment` | Verdict with environment metadata entries |
| `xml_with_expiration` | Verdict with provenance containing expiration |
| `xml_with_correlation_id` | Verdict with correlation ID on root element |
| `xml_full_record` | Verdict with all optional elements present |

### Unit tests

- `PacingSummary::from_config` computes effective values correctly
- `ExpirationStatus::requires_warning` returns correct values
- `VerdictRecord` builder accepts and exposes all new fields
- Existing snapshots remain unchanged (new fields default to `None`/empty)

### Integration test

Extend `tests/html_report.rs` (or add `tests/verdict_xml.rs`) to verify
that a complete verdict with all fields validates structural expectations
(contains expected elements, namespace, no external references).

## Implementation order

1. Add `PacingSummary`, `ExpirationStatus`, `ExpirationInfo` to
   `src/model/types.rs` with unit tests
2. Re-export from `src/model/mod.rs`
3. Add `correlation_id`, `pacing`, `environment` to `VerdictRecord` +
   builder; add `expiration` to `SpecProvenance`
4. Update `write_record` to emit `correlation-id`
5. Add `write_pacing` and `write_environment`
6. Update `write_provenance` for nested `<expiration>`
7. Add snapshot tests for all new elements
8. Wire `PacingSummary` construction into the runner when pacing is
   configured
9. Verify all existing snapshots still pass (run `cargo test`, review
   any `insta` diffs)
10. Update `inventory/FEATURES.md`: feotest RP07 `partial` → `done`
