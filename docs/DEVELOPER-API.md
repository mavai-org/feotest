# feotest Developer API

> **Status: TO BE DONE.** This is a placeholder scaffold. Focus is
> on punit for the time being; feotest's developer-facing API spec
> will be drafted once punit's lands and is corrected.
>
> When this document is filled in, treat it as the Rust-side
> counterpart to
> [`punit/docs/DEVELOPER-API.md`](../../punit/docs/DEVELOPER-API.md):
> the surface a Rust developer types when authoring tests,
> experiments, sentinel reliability specifications, and consumers
> against feotest. It is the operational counterpart to
> [`DOMAIN-ONTOLOGY.md`](DOMAIN-ONTOLOGY.md).

---

## Discipline that carries across (already settled at the family level)

The family ontology
([`javai-orchestrator/inventory/DOMAIN-ONTOLOGY.md`](../../inventory/DOMAIN-ONTOLOGY.md))
binds feotest as much as it binds punit. The following are
non-negotiable when filling in this document:

- **Statistics isolation rule.** Statistical calculations live
  exclusively in feotest's dedicated statistics module. No
  reimplementation outside the module. No reaching for the
  inverse-normal CDF, no inline Wilson, no order-statistic
  arithmetic outside the module — even small helpers belong in
  the dedicated home, with their own conformance tests against
  the javai-R fixtures. Enforcement mechanism in feotest TBD
  (Cargo workspace, crate visibility, clippy lints — pick one
  or compose).
- **Statistical engine has no other dependencies.** This document
  must record (when filled in) the precise Rust mechanism that
  enforces the isolation. The architectural maxim is the same as
  punit's — the implementation differs.
- **Outcome vs exception discipline → idiomatic `Result<T, E>`.**
  Use `Result::Err(...)` for an expected business-level failure;
  use `panic!` (or propagate a panic-equivalent) only for genuine
  defects. The framework treats a panic from the service contract under
  test as "investigate this bug", not as a counted sample failure.
  Per-language note in orchestrator CLAUDE.md confirms this is
  the equivalent of punit's `Outcome` channel.
- **Sentinel deployability — zero test-harness deps.** The
  sentinel binary has no `cargo test` / proc-macro test surface
  dependencies. Equivalent to punit's
  `RuntimeArchitectureTest` + `SentinelArchitectureTest`.
- **Requirement-code isolation.** Orchestrator-internal codes
  (CT, EX, LT, PT, RC, RP, SC, SN, TH, UC, XM, DG) MUST NOT
  appear anywhere in feotest source — production OR test, in
  doc-comments, identifiers, string literals, or `#[test]`
  function names. Enforcement mechanism in feotest TBD.
- **Verdict XML wire format.** Same canonical XSD as punit:
  `inventory/catalog/reporting/RP07-verdict-xml-interchange/verdict-1.0.xsd`.
  feotest currently emits `ci-lower` / `ci-upper`; alignment to
  `wilson-lower` is in flight under
  `DIR-RP07-WILSON-LOWER-VERDICT-XML` (in 0.7.x cleanup scope on
  the punit side; feotest's half lands independently).

---

## Sections to populate

The structure mirrors `punit/docs/DEVELOPER-API.md`. The Rust
shape of each section is TBD.

- [ ] Audience and scope
- [ ] The authoring entry-points
  - [ ] Rust equivalent of `@ProbabilisticTest` / `@Experiment`
        (TH04 — proc-macro / decorator ergonomics; the inventory
        marks this `done` for feotest)
  - [ ] Rust equivalent of `cargo test` integration (TH02)
- [ ] The feotest entry point
  - [ ] Rust equivalent of `PUnit.testing / measuring / exploring / optimizing`
- [ ] The Sampling primitive
  - [ ] How the empirical pair pattern manifests in Rust (the
        same structural guarantee as punit's shared `Sampling`
        reference, expressed in Rust ownership / borrowing
        idioms)
- [ ] Service Contract and Contract
- [ ] Postconditions (the Rust analogue of `ContractBuilder`'s
      `ensure` / `deriving` chain)
- [ ] Criteria
- [ ] The empirical pair pattern (Rust)
- [ ] Spec terminals
- [ ] Public module / crate contract
  - Which crates / modules an author may `use`
  - Which are `pub(crate)` / internal
  - The boundary equivalent to punit's
    `api / api.spec / engine` discipline
- [ ] Architecture-test catalogue (feotest's specific tests / lints)
- [ ] Statistics isolation rule (with feotest's enforcement
      mechanism named — see "Discipline that carries across" above)
- [ ] Sentinel deployability (with the Rust enforcement mechanism)
- [ ] `Result<T, E>` convention (the family Outcome invariant in
      feotest's idiom)
- [ ] Verdict XML wire format
- [ ] Versioning and binary compatibility
  - Cargo SemVer, public-API exposure, MSRV policy

---

## What this document will not do (when filled in)

- Enumerate every public function on every type — Rustdoc + the
  IDE / `cargo doc` are authoritative on signatures.
- Duplicate the user guide ([`USER-GUIDE.md`](USER-GUIDE.md)).
- Document the methodology — the Statistical Companion and the
  family ontology own that.
- Invent abstractions. Concepts are owned by the family ontology;
  feotest's per-project ontology
  ([`DOMAIN-ONTOLOGY.md`](DOMAIN-ONTOLOGY.md)) maps them onto
  Rust idioms; this document maps them onto a Rust developer's
  fingertips.

---

## Order of work when this is unblocked

1. Review the published `punit/docs/DEVELOPER-API.md` end-to-end so
   the cross-language conceptual mapping is current.
2. Sweep the feotest source tree (`src/`, `tests/`, `examples/`,
   `feotest-macros/`) for the public surface — every `pub` item
   plus the proc-macro entry points.
3. Fill `feotest/docs/DOMAIN-ONTOLOGY.md` first (the conceptual
   mapping), then this document (the API surface).
4. Cross-check against the family ontology's *Implementation
   Mapping* section — that's where the feotest-side type names
   should appear once they are settled.
5. Surface drift items — bugs the documentation surfaces, not
   bugs it fixes — into follow-up directives.
