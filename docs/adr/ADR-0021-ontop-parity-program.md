---
status: accepted
date: 2026-06-28
tags: [ontop-parity, program, umbrella, roadmap, optimizer, sparql-surface, mapping, dialects, hardening, charter, spec-oracle]
supersedes: []
depends-on:
  - ADR-0004
  - ADR-0005
  - ADR-0007
  - ADR-0008
  - ADR-0012
  - ADR-0020
implements:
  - ADR-0001
---

# Ontop-parity program — reach Ontop 5.5.0 parity within charter

## Context and Problem Statement

semantic-fabric is built and working (rewriter, SQLite + PostgreSQL executors, `serve`, property paths, W3C RDB2RDF conformance 81/82 · 80/81, 181 tests). The reference OBDA system is **Ontop 5.5.0**. `COMPARISON.md` shows semantic-fabric is competitive but has measured gaps — most sharply the **Q5 / OPTIONAL latency degradation** (sf 0.81→8.72 ms @1×→10× vs Ontop's flat 1.84→2.93 ms) and a **SPARQL surface that `501`s** everything past the wired operators (`unfold.rs:142`).

This ADR is the **umbrella** for a program to close those gaps and reach parity **within charter** — i.e. parity on the surfaces semantic-fabric is chartered to own (optimizer, SPARQL surface, mapping, *relational* dialects, hardening), explicitly **not** the surfaces excluded by ADR-0001/0008 (OWL 2 QL tier-2 entailment) or N/A to a no-JVM Rust engine (Protégé / `.obda`).

## Decision Drivers

* **Own the rewriter (ADR-0004).** Ontop is the *specification and oracle*, never source to transliterate — its Java is entangled with OWLAPI/Guice/RDF4J/JDBC (`docs/research/ontop.md §13`).
* **Correctness first.** Every parity gain must hold the ADR-0005/0012 correctness gates; the OPTIONAL/NULL surface is the hardest in the codebase (ADR-0007).
* **Highest measured value first**, de-risked. The optimizer (Q5) is the biggest measured win; tests-first de-risks the reimplementation.
* **Charter discipline.** Scope stays bounded by ADR-0008 (tier-2 entailment deferred) and the external ODR-0030.

## Considered Options

* **Spec + oracle reimplementation** — reimplement Ontop's algorithms against semantic-fabric's *existing* architecture + Oxigraph, and **port Ontop's JUnit tests as the oracle**. Chosen.
* **Java→Rust port of Ontop** — rejected: entangled with OWLAPI/Guice/RDF4J/JDBC (`ontop.md §13`); violates ADR-0004 ("own the rewriter").
* **Full parity including OWL 2 QL tier-2 entailment + Protégé/`.obda`** — rejected/out of charter: tier-2 entailment is held by ADR-0008 + ODR-0030; Protégé is a Java GUI (N/A to a no-JVM engine).

## Decision Outcome

Chosen: **spec + oracle, parity within charter, Wave 1 = WS-G (tests first)** — the three operator decisions below, executed one workstream per wave through the gated wave loop.

### The three locked decisions (operator, 2026-06-28)

1. **Approach = spec + oracle, NOT a Java→Rust port.** Reimplement Ontop's algorithms following semantic-fabric's architecture; port Ontop's tests as the oracle. Honors ADR-0004 and `ontop.md §13`.
2. **Charter = parity *within charter*.** Hold ADR-0008 / ODR-0030 (no OWL 2 QL tree-witness entailment; tier-2 stays depth-0 / 501). Protégé / `.obda` out (Java GUI, N/A). Pursue parity on optimizer, SPARQL surface, mapping, relational dialects, hardening.
3. **Wave 1 = WS-G (tests first).** Port Ontop's IQ-optimizer tests → Rust equivalence/oracle tests, *then* drive WS-A/WS-B against them.

### Workstreams

| WS | Scope | Charter |
|----|-------|---------|
| **A. Optimizer + Q5 fix** | self-/self-left-join elim, IRI-template-mismatch pruning, redundant-union elim, FD inference; the **Q5 OPTIONAL fix** | in-charter — highest measured value |
| **B. SPARQL surface** | wire the `unfold.rs:142` `501`s: MINUS, aggregates/GROUP BY, BIND, VALUES, GRAPH, ORDER BY, (NOT) EXISTS, subqueries | in-charter — unblocks full GTFS-18 |
| **C. Mapping completeness** | named graphs (`rr:graphMap`), RefObjectMap join conditions | in-charter |
| **D. Dialect breadth** | MySQL next; long tail **as Rust drivers allow** (no JDBC universe) | in-charter |
| **E. Entailment** | OWL 2 QL T-mapping saturation / tree-witness | **EXCLUDED** — held by ADR-0008 + ODR-0030 |
| **F. Hardening** | the ADR-0014 backlog (TLS, retries, rate-limiting, k8s/probes, hot-reload, observability) | in-charter — extends ADR-0014 |
| **G. Test port** | Ontop JUnit IQ-optimizer tests → Rust oracle/equivalence tests | in-charter — **Wave 1** (ADR-0022) |

### Charter exclusions (recorded, deliberate)

* **OWL 2 QL tier-2 entailment** (RHS-existential / tree-witness saturation) — **excluded**, held by **ADR-0008** (tier-2 evidence-gated/deferred) and the external **ODR-0030** (in the `semantic-modelling` repo; cross-corpus reference). Tier-2 queries stay depth-0 / `501`.
* **Protégé / `.obda`** — N/A: Java GUI, no place in a no-JVM Rust engine.

### Execution model — gated wave loop

One workstream per wave: **implement → verify → adversarial review → conditional promotion.** The verify gate is `=_bag` differential + W3C RDB2RDF floor + `cargo clippy --all-targets -D warnings` + `cargo fmt --check` (ADR-0005/0012). The engine-perf Path-B sweep is reserved for the closing **optimise** milestone, never auto-run in CI. No speed for correctness: cost may choose only among `=_bag`-equivalent plans.

### Incremental ADRs (avoid design-target drift)

The umbrella (this ADR) and **WS-G (ADR-0022)** are created now. **WS-A…F each get their ADR at the start of their wave**, so each describes the code as actually built — avoiding the kind of "design-target vs wired" drift recorded below.

### ADR-0007 correction (recorded here; fixed by WS-B)

ADR-0007 §"v1 SPARQL coverage" lists `BIND, VALUES, ORDER BY, aggregates, GRAPH, MINUS` as **Supported**, but `unfold.rs:142` `501`s every one (the code comment at `unfold.rs:139–141` lists them as deferred). It also lists the `?` path operator as deferred, though commit `0cdbdd1` wired full property paths and `unfold.rs:132` handles `Path`. The wording is a v1 *contract/target*, not what is wired — **WS-B reconciles ADR-0007 when it wires the `501`s.**

### Consequences

* Good, because tests-first (WS-G) de-risks the hardest reimplementation surface (OPTIONAL/NULL) before any optimizer change.
* Good, because the charter keeps scope honest — excluded surfaces are recorded against ADR-0008 / ODR-0030, not silently attempted.
* Good, because the gated verify loop forbids trading correctness for speed (`=_bag`-equivalent plans only).
* Bad, because Wave 1 produces an oracle, not user-visible features — slower to visible parity.
* Neutral, because WS-A…F ADRs are authored per-wave rather than all up front.

### Confirmation

* Each wave holds the verify gate (`=_bag` + W3C floor + clippy + fmt) before promotion.
* Parity is measured by re-running `COMPARISON.md` (GTFS-Madrid OBDA track) and the W3C RDB2RDF conformance suite; the **benchmark** milestone re-measures Q5 and the full GTFS-18 against Ontop 5.5.0.
* The `horizon-tracker` agent owns the program objective and milestone checkpoints until *validated, benchmarked, and optimised*.

## More Information

* **Ontop spec/oracle:** `~/source/ontop` (HEAD just past tag `ontop-5.5.0`; `git checkout ontop-5.5.0` for an exact match); `docs/research/ontop.md` (§5 optimizer, §10 modules, §13 reuse-vs-reimplement).
* **Charter exclusion:** ODR-0030 (`semantic-modelling` repo — cross-corpus; not in this `docs/adr/` index).
* **Gates / loop / registers:** ADR-0005 (conformance + bench), ADR-0012 (test strategy), ADR-0020 (outstanding SOTA optimisations), ADR-0007 (rewriter cascade), ADR-0008 (reasoning / charter), ADR-0004 (own the rewriter).
* **Workstream ADRs:** ADR-0022 (WS-G, Wave 1). WS-A…F created at the start of their waves.
* **Theory anchor (WS-A):** Xiao/Kontchakov et al., *Efficient Handling of SPARQL OPTIONAL for OBDA* (ISWC 2018).
