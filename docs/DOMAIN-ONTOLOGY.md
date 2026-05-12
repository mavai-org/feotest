# feotest Domain Ontology

> **Status: TO BE DONE.** This is a placeholder scaffold. Focus is
> on punit for the time being; feotest's per-project ontology
> will be drafted once punit's lands and is corrected.
>
> When this document is filled in, treat it as the Rust-side
> counterpart to
> [`punit/docs/DOMAIN-ONTOLOGY.md`](../../punit/docs/DOMAIN-ONTOLOGY.md):
> map the family domain ontology
> ([`javai-orchestrator/inventory/DOMAIN-ONTOLOGY.md`](../../inventory/DOMAIN-ONTOLOGY.md))
> onto feotest's Rust idioms, modules, and types. This document is
> **downstream** of the family ontology — when this document and
> the family ontology disagree, the family ontology wins and this
> document is corrected.

---

## How to read

For each family-level concept the finished document will carry:

- **family**: the family ontology section name.
- **rust type**: the Rust identifier in feotest (trait, struct,
  enum, type alias, …).
- **module**: where it lives.
- **role notes**: idiom-level notes — what's a struct vs enum vs
  trait, what's `pub` vs `pub(crate)`, what's mandatory vs
  defaulted via blanket impls.
- **gotchas / drift to fix**: feotest-specific points an agent
  must preserve when modifying or generating Rust code.

Family-level invariants and policies are not restated here; the
family ontology is authoritative on those. Where feotest *enforces*
a family-level invariant via a specific test, lint, or module-
visibility rule, the enforcement mechanism is named.

---

## Sections to populate

The structure mirrors `punit/docs/DOMAIN-ONTOLOGY.md`. The Rust
mappings are TBD; the section list is fixed so that the eventual
fill-in is a sweep, not a re-architecture.

- [ ] Subject under test
  - [ ] Use Case → trait + factor type binding
  - [ ] Use Case ID
  - [ ] Factor (and the canonical Rust analogue of `NoFactors`)
  - [ ] Covariate + Covariate Profile + per-category implementations
  - [ ] Input Source

- [ ] Judgement under test
  - [ ] Service Contract
  - [ ] Postcondition (the Rust equivalent of `ContractBuilder`'s
        `ensure` / `deriving` pattern)
  - [ ] Duration Constraint
  - [ ] Expected-Output Match
  - [ ] Conformance

- [ ] The act of testing
  - [ ] Sample
  - [ ] Outcome — **map to idiomatic `Result<T, E>`**, not a
        custom Outcome type. Per family ontology + orchestrator
        CLAUDE.md.
  - [ ] Probabilistic Test
  - [ ] Verdict

- [ ] Methodology and statistics
  - [ ] Parameter Triangle
  - [ ] Threshold (pass-rate)
  - [ ] Threshold (latency)
  - [ ] Threshold Origin
  - [ ] Threshold Provenance
  - [ ] Feasibility Gate
  - [ ] Wilson Score Bound
  - [ ] Latency Population
  - [ ] Empirical Percentile

- [ ] Cost, pacing, and budget
  - [ ] Token (cost-proxy)
  - [ ] Budget (note: punit has `RC12 / RC13` exception
        handling; feotest does not — the inventory marks them
        `n/a` for feotest)
  - [ ] Pacing

- [ ] Experimentation
  - [ ] Experiment (MEASURE / EXPLORE / OPTIMIZE)
  - [ ] Experiment Configuration
  - [ ] Stepper (Factors)
  - [ ] Empirical Baseline
  - [ ] Footprint
  - [ ] Content Fingerprint

- [ ] Test intent and policy
  - [ ] Test Intent
  - [ ] Compliance Testing
  - [ ] Conformance Testing

- [ ] Reporting
  - [ ] Verdict XML (RP07)
  - [ ] Verdict Sink

- [ ] Sentinel
  - [ ] Sentinel runtime
  - [ ] Reliability Specification

- [ ] Feotest-only concepts (not at the family level)
  - [ ] Proc-macro / decorator ergonomics (TH04 — distinctive
        feotest authoring surface)
  - [ ] Module visibility regime (the Rust counterpart to
        punit's api / api.spec / engine boundary)

- [ ] Lifecycles in feotest terms
- [ ] Feotest-specific invariants and enforcement
  - [ ] How feotest enforces statistical isolation (Cargo /
        crate-visibility / clippy lints — fill in)
  - [ ] How feotest enforces requirement-code isolation
  - [ ] How feotest enforces sentinel deployability (no
        test-harness deps in the sentinel binary)
- [ ] Drift to fix (feotest-side)
  - [ ] Known: token-budget pre-sample double-count for static
        charges (memory: `feotest_token_budget_double_count`)
  - [ ] Known: feotest predates 2026-05 catalog amendments to
        EX04 / EX07 / EX10 (per-sample resultProjection in
        MEASURE baselines, `inputIndex` replacing `input` in
        projections, soft-warning rather than hard-abort on
        integrity failure). See
        `plan/followups/feotest-RP01-EX04-EX07-EX10-amendment-alignment.md`
        in the orchestrator.
  - [ ] Known: PT09 / PT10 are *done* in feotest but partial
        in punit — when punit catches up, verify feotest's
        evaluator surface stays aligned.

---

## What this document does not do

When filled in, this document does **not** restate family
invariants, duplicate the glossary, or restate methodology. See
`punit/docs/DOMAIN-ONTOLOGY.md`'s closing section for the same
discipline applied to punit; the same applies here.
