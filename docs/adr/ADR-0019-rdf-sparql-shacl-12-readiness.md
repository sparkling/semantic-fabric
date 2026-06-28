---
status: accepted
date: 2026-06-27
tags: [rdf-1.2, sparql-1.2, shacl, oxigraph, rudof, jena-replacement, conformance, gap-register, feature-flags, upstream-contribution]
depends-on:
  - ADR-0001
  - ADR-0004
implements:
  - ADR-0001
---

# RDF 1.2 / SPARQL 1.2 / SHACL readiness — replacing Jena (JVM) with the Rust stack

## Context and Problem Statement

The ontology and its validation/query layer use **RDF 1.2 and SPARQL 1.2** (triple terms, `rdf:reifies` reification, directional language strings; SPARQL 1.2 triple-term patterns and accessor functions) plus **SHACL**, served today on **Apache Jena (Fuseki + jena-shacl)** — a JVM stack. The no-JVM charter (ADR-0001) requires the Rust stack — **Oxigraph** (RDF/SPARQL crates) + **rudof** (SHACL) — to be a complete replacement.

**Acceptance is two questions:** (1) will the RDF-1.2 ontology **load** into the virtualiser? (2) will it support the **SPARQL 1.2 features** needed to query the RDF 1.2 constructs the ontology encodes? **Scope assumption:** do not scope to current usage — assume we will eventually use the **full** RDF 1.2 + SPARQL 1.2 standards, and SHACL up to **SHACL 1.2** as it matures. This ADR is the standing register of what the Rust stack supports, what it does not, and how each gap is closed.

## Considered Options

- **Apache Jena (Fuseki + jena-shacl), the JVM baseline** — keep the existing stack; rejected because the no-JVM charter (ADR-0001) forbids a JVM stack, and Jena 6.1.0 holds nothing the Rust stack lacks (G1/G7 it also lacks; G2/G5 already shipped in the Rust pins; G4 neutralised by Native mode; G3/G8 architectural).
- **The Rust stack — Oxigraph crates (RDF/SPARQL) + rudof (SHACL)** — chosen; a complete replacement once 1.2 feature flags are enabled and the crates are wired.
- **A standing fork of Oxigraph/rudof** — rejected in favour of upstream contribution; both projects are dual MIT/Apache with no CLA/DCO and responsive, so a thin temporary `cargo [patch]` against our fork (carrying the exact submitted PR) is used only when a fix is needed before an upstream release, then dropped once merged.

## Decision Outcome

**Yes — the Rust stack replaces Jena with no capability loss for our architecture, once 1.2 feature flags are enabled and the crates are wired.** Oxigraph passes the full W3C RDF 1.2 and SPARQL 1.2 test suites (empty skip-lists); rudof `shacl` 0.3.4 covers SHACL 1.0 Core (Native mode) plus `sh:sparql`; and Jena 6.1.0 — the baseline — has nothing the Rust stack lacks that is either (a) still unshipped everywhere (SHACL 1.2), (b) already shipped in our Rust pins, or (c) unreachable in our configuration.

1. **Adopt the Rust stack (Oxigraph crates + rudof) as the Jena replacement.**
2. **Enable the feature-flag / wiring matrix above**, pin exact patch versions (specs pre-final), and run **rudof SHACL in Native mode** (its `sparql` feature is on by default — pin Native so G4's panic path is unreachable).
3. **Build our own SPARQL 1.2 Protocol serve endpoint** (G8) — Oxigraph's is server-binary-only at 1.1.
4. **Upstream contribution is the gap-closing strategy:** contribute **G4** (S) and **G6 (a)+(b)** (M); G2/G5 already done; **park G7** until SHACL 1.2 reaches ≥ CR. **No standing fork** — both projects are dual MIT/Apache, no CLA/DCO, and responsive; if a fix is needed before an upstream release, use a thin temporary `cargo [patch]` pointing at our fork carrying the exact submitted PR, dropped once merged. Oxigraph is effectively single-maintainer — keep local patches minimal and upstream promptly.
5. **Triple-term graphs serialise as Turtle / N-Triples 1.2** (G1); JSON-LD is for triple-term-free graphs.

### Standards status (2026-06-27)

| Standard | W3C status |
|---|---|
| RDF 1.2 (Concepts, Semantics) | Candidate Recommendation (7 Apr 2026) |
| SPARQL 1.2 (Query / Update / Federated / Protocol / Results / Entailment / GSP) | Working Drafts (Rec-track) |
| SHACL 1.0 | Recommendation (20 Jul 2017) |
| SHACL 1.2 (Core / Node Expressions / Rules / SPARQL Ext) | Working / First Public Working Drafts |

### Library-level support (evidence: W3C suites with empty skip-lists; pinned-crate source)

- **RDF 1.2 — oxrdf / oxttl (`rdf-12`):** triple terms (object position), `rdf:reifies` reification, directional language strings (`BaseDirection` / `rdf:dirLangString`), `rdf:JSON`, RDF Dataset Canonicalization (`rdfc-10`), and 1.2 read+write for Turtle / TriG / N-Triples / N-Quads / RDF-XML.
- **SPARQL 1.2 — spargebra parser/algebra (`sparql-12`):** full grammar — `VERSION`, triple-term / reifier / annotation syntax, `TRIPLE`/`SUBJECT`/`PREDICATE`/`OBJECT`/`isTRIPLE`, `LANGDIR`/`hasLANG`/`hasLANGDIR`/`STRLANGDIR`; Query + Update; `SERVICE` parsed; results JSON/XML/CSV-TSV (sparesults). semantic-fabric uses the parser, not Oxigraph's evaluator.
- **SHACL — rudof `shacl` 0.3.4 (Native mode):** complete SHACL 1.0 Core constraint set, plus the `sh:sparql` SPARQL-based constraint component.

Both acceptance questions are therefore **YES**.

### Required configuration — the #1 actionable

Today the engine builds with **all 1.2 capability compiled out**: `oxrdf`/`spargebra` are declared with no feature flags (so `rdf-12`/`sparql-12`/`rdfc-10` are off); `oxttl`/`sparesults`/`sparopt`/`oxsdatatypes` are declared-but-unwired; `oxjsonld`/`oxrdfio`/`oxrdfxml` are not declared. To realise 1.2:

| Crate | Action | Feature flags |
|---|---|---|
| oxrdf | wired — add flags | `rdf-12`, `rdfc-10` |
| spargebra | wired — add flags | `sparql-12`, **`sep-0002`** (ADJUST), **`sep-0006`** (LATERAL — opt-in extension, ADR-0007), `standard-unicode-escaping` (strict 1.2 `\u`) |
| oxttl | wire | `rdf-12` (+ `async-tokio`) |
| sparesults | wire | `sparql-12` (+ `async-tokio`) |
| sparopt, oxsdatatypes | wire | current |
| oxjsonld | **add dep** (≥ 0.2.5) | `rdf-12` |
| oxrdfio (facade), oxrdfxml | add if needed | `rdf-12` |

Two traps the sweep surfaced: **`sparql-12` alone is not full 1.2** — `ADJUST` is gated behind `sep-0002`, and strict whole-string `\u` unescaping behind `standard-unicode-escaping`. **`LATERAL` (`sep-0006`) is enabled** as a documented opt-in extension (it maps directly to SQL `LATERAL`/`CROSS APPLY` and unlocks top-N-per-group; ADR-0007) — but it is a non-standard SPARQL extension (absent from the SPARQL 1.2 Query WD), so it is **kept out of, and reported as outside, the 1.2 conformance surface**.

> **Reconciliation note (2026-06-28 — corrected; supersedes an earlier same-day claim).** An earlier version of this note asserted `sparopt` "cannot be wired with the 1.2 feature set / fails to compile against `spargebra` with `sparql-12`/`sep-0006`." That is **empirically false** (verified via `cargo build -p sparopt` + `cargo tree -e features`): `sparopt` 0.3.6 compiles cleanly with `spargebra` 0.4.6 + `sparql-12`/`sep-0002`/`sep-0006` and is already a live transitive dependency (pulled by the `spareval` oracle → `oxigraph`/`rudof`). Corrected status for the matrix row "`sparopt` … wire … current": `sparopt` is **not wired into the engine optimizer by choice** — the ADR-0007 order-disciplined cascade is the sole optimiser, so the opt-in pre-rewrite stage is unnecessary (no loss). It is not a compatibility block. Companion notes: ADR-0007 §pipeline step 2, ADR-0004 §substrate matrix.

### Gap register

| # | Gap | Disposition |
|---|---|---|
| G1 | Triple terms unrepresentable in **JSON-LD and RDF/XML** (syntax-level; Jena has the same limit) | Serialise triple-term graphs as **Turtle / N-Triples 1.2**. Not contributable (spec-level). |
| G2 | JSON-LD `@direction` | **Resolved upstream** — oxjsonld ≥ 0.2.5 implements it; the skipped W3C tests are an intentional RDF-1.2-forward deviation. Add + wire oxjsonld with `rdf-12`. |
| G3 | No entailment-regime engine in the store | **By design** — entailment lives in the rewriter (ADR-0008). Non-issue. |
| G4 | rudof property-pair `validate_sparql()` = `unimplemented!()` (panics in SPARQL mode) | We run **Native** (no panic). rudof's `sparql` feature is on by default → **pin Native explicitly**. Contribute the fix (effort S). |
| G5 | rudof `sh:sparql` SPARQL-based constraint component | **Resolved upstream** — shipped rudof 0.3.2, present in our 0.3.4. Use default features. |
| G6 | rudof user-reachable panics: (a) report rendering on complex SHACL paths, (b) min/max on uncommon numeric datatypes | Contribute targeted fixes (effort M); avoid trigger shapes meanwhile. |
| G7 | SHACL 1.2 (node expressions, rules) | **Wait-for-spec** (WD/FPWD; Jena lacks it too). |
| G8 | SPARQL **Protocol** / **Graph Store Protocol** / `SERVICE` execution | Oxigraph provides these only in its server binary at SPARQL 1.1. Our serve layer is ours to build → **implement the SPARQL 1.2 Protocol ourselves**. External `SERVICE` is out of scope (ADR-0002); cross-RDBMS federation is our semi-join. |

### Jena baseline (regression check)

Jena 6.1.0 (2026-05-11): stable RDF 1.2 + SPARQL 1.2; jena-shacl = SHACL Core + SHACL-SPARQL; no SHACL 1.2 or Advanced Features. Per gap: G1/G7 — Jena lacks them too (**no regression**); G2/G5 — already shipped in our Rust pins; G4 — Jena has it, **neutralised by Native mode**; G3/G8 — architectural (our rewriter / our serve layer). **Net: no functional regression in switching off Jena.**

### Consequences

* Good, because the Rust stack replaces Jena with no capability loss for our architecture once 1.2 feature flags are enabled and the crates are wired, and there is no functional regression in switching off Jena.
* Good, because gaps G2 and G5 are already resolved upstream and present in our Rust pins, and G4 is neutralised by running rudof in Native mode.
* Neutral, because triple-term graphs must serialise as Turtle / N-Triples 1.2 (G1); JSON-LD is reserved for triple-term-free graphs (a syntax-level limit Jena shares).
* Neutral, because the SPARQL 1.2 Protocol / Graph Store Protocol serve endpoint (G8) is ours to build — Oxigraph ships those only in its server binary at 1.1 — and external `SERVICE` stays out of scope (ADR-0002).
* Neutral, because exact patch versions must be pinned while the specs are pre-final, and `LATERAL` (`sep-0006`) is kept out of, and reported as outside, the 1.2 conformance surface.
* Bad, because Oxigraph is effectively single-maintainer, requiring local patches to be kept minimal and upstreamed promptly; G4 (S) and G6 (a)+(b) (M) remain to be contributed, and G7 (SHACL 1.2) is parked until the spec reaches ≥ CR.

### Confirmation

- The ontology (Turtle 1.2, incl. `rdf:reifies <<( … )>>`) parses via oxttl `rdf-12` with no loss; the `sh:sparql` / `PREDICATE()` constructs parse via spargebra `sparql-12`.
- `cargo build` with the feature matrix compiles in triple terms, dir-lang strings, RDFC-1.0, and the full SPARQL 1.2 grammar.
- rudof Native-mode validation of the meta-shapes passes with no `unimplemented!()` path hit.
- Evidence basis: W3C RDF 1.2 / SPARQL 1.2 test suites (Oxigraph CI, empty skip-lists) and the ADR-0005 conformance / SHACL-Native gates.

## More Information

- **Charter / no-JVM:** ADR-0001. **Substrate (Oxigraph crates):** ADR-0004 — this ADR fixes the 1.2 feature flags it pins. **Conformance / SHACL Native:** ADR-0005. **Reasoning (entailment in the rewriter):** ADR-0008. **Scope (SERVICE out):** ADR-0002.
- **Evidence basis:** W3C RDF 1.2 / SPARQL 1.2 test suites (Oxigraph CI, empty skip-lists); pinned-crate source (oxrdf 0.3.3, oxttl 0.2.3, spargebra 0.4.6, oxjsonld 0.2.5, rudof `shacl` 0.3.4); Apache Jena 6.1.0 (CHANGES, jena-shacl source); W3C spec-status pages. Upstream health: rudof (WESO, ~biweekly releases, dual MIT/Apache, no CLA); Oxigraph (single-maintainer, very responsive, dual MIT/Apache, no CLA).
