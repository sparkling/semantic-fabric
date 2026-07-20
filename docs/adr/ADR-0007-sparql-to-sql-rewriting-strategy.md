---
status: accepted
date: 2026-06-27
updated: 2026-07-19
tags: [obda, virtualization, sparql-to-sql, rewriting, intermediate-query, optional, null-semantics, optimizer-cascade, correctness, term-construction-lifting, plan-cache, cost-driven]
supersedes: []
depends-on:
  - ADR-0003
  - ADR-0004
  - ADR-0006
implements:
  - ADR-0001
---

# SPARQL→SQL rewriting strategy and cascade correctness (virtualization / OBDA)

## Context and Problem Statement

The virtualizer (ADR-0003) turns a SPARQL 1.2 query over the virtual graph into SQL over the live source. Scope is R2RML-only (no OWL reasoning on this path; tier-1 hierarchy via T-mapping UNION expansion is ADR-0008), which bounds the rewriting blow-up by the number of triples-map references in the query rather than by ontology entailment.

The deep difficulty is a semantic mismatch: **SPARQL solution mappings are partial functions under bag + three-valued semantics, whereas SQL tuples are total functions where "absent" is NULL under a *different* three-valued logic.** Every correctness bug on this path is a leak between those two NULL regimes, and SPARQL `OPTIONAL` → SQL `LEFT JOIN` is the largest leak (it took the reference system, Ontop, roughly a decade to stabilise self-left-join elimination with nullable determinants).

## Decision Drivers

* Correctness first — a sound, complete base translation (Chebotko/Pérez semantics) with OPTIONAL/NULL handled exactly.
* Performance — an Ontop-style structural + semantic optimizer cascade is where the real-world speed lives.
* Reuse — `spargebra` (parse/algebra), `sqlparser` (dialect emission, ADR-0004), `tokio-postgres` (execution, ADR-0006).

## Considered Options

* **Xiao/Kontchakov et al. (ISWC 2018) OPTIONAL-to-SQL base translation** — adopt as the unoptimized ground truth (`LEFT JOIN` + `COALESCE` of shared variables + explicit compatibility filter), proven bag- and 3VL-faithful.
* **Invent a bespoke base translation** — rejected; getting OPTIONAL/NULL exact across the two different three-valued NULL regimes from scratch is the largest correctness leak (Ontop took ~a decade to stabilise it).
* **Wire the `sparopt` algebra optimizer into `sf-sparql` (path A)** — not pursued; the order-disciplined IQ→IQ cascade is the sole optimiser (filter-pushdown = pass 5, join-elimination = passes 2/4), so the opt-in pre-stage adds nothing needed and whether its API integrates at our use-site is unproven.

## Decision Outcome

### Base translation = the unoptimized ground truth

Adopt the **Xiao/Kontchakov et al. (ISWC 2018)** OPTIONAL-to-SQL translation as the *unoptimized ground truth* — `LEFT JOIN` + `COALESCE` of shared variables + an explicit compatibility filter, proven bag- and 3VL-faithful. Do **not** invent a base translation; the cascade is then only semantics-preserving IQ→IQ rewrites on a translation that is already correct.

NULL / left-join rules the base translation obeys:

* **R1** — a shared OPTIONAL variable is never a plain `ON a = b`; the condition is `(a = b OR a IS NULL OR b IS NULL)` (an unbound variable is compatible with any value).
* **R2** — a shared variable is one SPARQL variable but two SQL columns after a LEFT JOIN; project it as `COALESCE(left, right)`.
* **R3** — R2RML mappings cannot emit NULL, so the mapping filters NULLs out; OPTIONAL re-introduces them (the padding effect / `IS NOT NULL` guards).
* **R4** — preserve bag semantics / multiplicities everywhere except inside an established DISTINCT/uniqueness context.
* **R5** — FILTER *inside* OPTIONAL belongs in the LEFT JOIN `ON` condition; FILTER *after* OPTIONAL is a later WHERE — an outer FILTER must never be pushed onto the preserved (left) side.

### Pipeline (`sf-sparql`)

1. **Parse** — `spargebra::SparqlParser` → `GraphPattern`.
2. **Algebra optimize (opt-in)** — `sparopt` (filter pushdown, join reorder); bypassable if it regresses. *(Reconciliation, 2026-06-28 — **supersedes an earlier same-day note**, empirically re-verified via `cargo build -p sparopt` + `cargo tree`: this step is **unwired**, but NOT because of a compile failure. The earlier note claimed `sparopt` 0.3.6 "fails to compile against `spargebra` with `sparql-12`/`sep-0006`" — that is **false**: `sparopt` 0.3.6 compiles cleanly with `spargebra` 0.4.6 + `sparql-12`/`sep-0002`/`sep-0006` and is in fact a live transitive dependency here (via the `spareval` in-memory oracle → `oxigraph`/`rudof` → `sf-conformance`). The real status: `sparopt`'s optimizer is simply **not wired into `sf-sparql`'s pipeline** by choice — the order-disciplined cascade below is the sole optimiser (filter-pushdown = cascade pass 5; join-elimination = passes 2/4), so the opt-in pre-stage adds nothing we need and there is no correctness/perf loss. Wiring it (path A) was not pursued; whether its API integrates at our use-site is unproven. The dead `[workspace.dependencies]` catalog entry was removed (hygiene; `sparopt` remains in the tree transitively via `spareval`). Companion notes: ADR-0004 §substrate matrix, ADR-0019 §config matrix.)*
3. **Unfold** — replace each triple pattern with the SQL sub-expressions of the matching mapping-IR entries → an IQ-style relational tree (the ISWC-2018 base translation).
4. **Tier-0 elimination (up front)** — a refObjectMap with no `rr:joinCondition` ⇒ inline the parent's subject IRI (no join); parent == child triples-map on a PK ⇒ collapse to a scan (redundant self-join elimination).
5. **Optimizer cascade — order is load-bearing:** (i) IRI-template-mismatch pruning → (ii) self-join / self-left-join elimination → (iii) functional-dependency inference (transitive closure, through unions) → (iv) FK/PK join elimination → (v) selection pushdown → (vi) distinct removal.
6. **Emit** — translate the optimized tree to a `sqlparser` AST and render the target dialect; values are bound parameters only (ADR-0010).
7. **Execute & reconstruct** — run via `tokio-postgres`; map rows to bindings; serialise with `sparesults` (streamed, ADR-0010).

### Cascade order is load-bearing (invariants)

* IRI-template-mismatch pruning **must precede** self-join elimination — empty branches are pruned, and the IRI-term equalities that license a self-join merge are established first.
* FD inference (with transitive closure) **must precede** FK/PK join elimination — eliminating a join is sound only when uniqueness *and* match-guarantee hold; firing earlier drops rows and violates bag semantics.
* Every rule preserves `=_bag` w.r.t. the base translation; preserves the COALESCE/compatibility semantics; fires only when its integrity-constraint precondition is already established; left-join elimination preserves the right-side-bound provenance marker; the cascade runs to a fixpoint whose result is order-independent among commuting rules.

### Term-construction lifting (translation discipline)

IRI/literal construction (`concat`/`cast` over `rr:template` segments) is **lifted to the final projection**: joins and FILTERs are expressed over the **raw key columns**, never over constructed term strings, and RDF terms are materialised only in the outermost SELECT list. This is mandatory in the base translation, not a cascade pass — building terms inside join/filter predicates both defeats source indexes *and* blinds the source optimizer's row estimates (databases cannot see through IRI-template structure — the same blindness the cascade's IRI-template-mismatch pruning handles at the algebra level). Lifting keeps equi-joins on indexed key columns and keeps the source's own cardinality estimator accurate; it is the single-source half of the cost-driven design (the cross-source half is the semi-join cost model, ADR-0006), and it is costly to retrofit once the unfold/emit paths exist, so it is baked in from the start.

### v1 SPARQL coverage

**Supported:** BGP, `JOIN`, `FILTER`, `OPTIONAL` (null-safe), `UNION`, `BIND`, `VALUES`, projection, `DISTINCT`/`REDUCED`, `LIMIT`/`OFFSET`, `ORDER BY` (NULLS ordering pinned per dialect — SPARQL orders unbound first; never rely on the dialect default, e.g. PostgreSQL's `NULLS LAST`), aggregates (with the empty-group `SUM` NULL-vs-0 reconciliation), `GRAPH`, `MINUS`, transitive property paths `P+`/`P*`, and the **`LATERAL`** correlated-join extension (SEP-0006 → SQL `LATERAL` / `CROSS APPLY`; enables top-N-per-group; a documented opt-in extension, **out of the 1.2 conformance surface** — ADR-0019). Recursive paths compile to source-dialect recursive CTEs — PostgreSQL depth-counter + `CYCLE` (PG14+); a DuckDB *source* uses `USING KEY` (bounded-memory settled-set). **Deferred → `501`:** the `?` path operator, `SERVICE`, and OWL 2 QL tier-2 entailment (ADR-0008).

### Performance

**Plan cache (hot path).** Cache the compiled plan keyed on a **structural hash of the SPARQL algebra** (not the query string), invalidated by a monotonic **`⟨T, M⟩` + source-schema epoch** bumped on ontology reload, mapping reload, or a source-schema change — the cache is sized by `⟨T, M⟩`, never by data, so it cannot go stale against the live sources. Sharp keying rule: **parameterise *data* constants but key on *schema-selecting* constants** (predicate IRIs and IRI-template constants — the ones that decide which mapping entries and columns to unfold); otherwise a plan compiled for `:a` would wrongly serve a `:b` query. Implementation: a bounded `quick_cache` for compiled plans + `deadpool`'s `prepare_cached()` for source-side prepared statements. Declare available constraints (PK/FK/uniqueness from `sf-sql` introspection) so the source optimizer can finish the job — but never rely on it for template-aware pruning (databases cannot see through IRI-template structure; see *Term-construction lifting* above).

### Correctness anchor

Chebotko/Pérez relational-algebra semantics is the soundness/completeness reference (proof target: `eval_SQL(τ(Q,M),D) =_bag eval_SPARQL(Q, RDF(M,D))`).

### Consequences

* Good, because a proven base translation + an order-disciplined cascade captures Ontop-class speed, with blow-up bounded by R2RML-only scope.
* Bad, because the cascade is the single hardest correctness surface; NULL-across-left-joins demands the invariants above plus the ADR-0012 test strategy.
* Bad, because deferred features return `501` in v1 (documented, not silent).

### Confirmation

Verified by the layered strategy in ADR-0012 — the native in-memory Oxigraph oracle for rewriter correctness (the virtualiser's answer vs the expected graph loaded in-memory and queried), a NoREC-style internal differential (unoptimized vs optimized IQ) that pinpoints a faulty cascade rule, an MR1 constraint-toggling metamorphic test for unsound constraint-driven optimizations, and per-rule bounded equivalence checking (VeriEQL).

## More Information

* **Architecture:** ADR-0003. **Substrate:** ADR-0004. **Execution substrate:** ADR-0006. **Scope:** ADR-0002. **Reasoning:** ADR-0008. **Governance (injection-safety, recursion bounds, streaming):** ADR-0010. **Test strategy:** ADR-0012.
* **Research:** `docs/research/` — `cascade-correctness` (Xiao/Kontchakov ISWC-2018, Chebotko, Pérez, cascade order + invariants), `ontop`, `foundations-benchmarks`.
* **Cost-driven design (baked in here):** term-construction lifting + the plan cache are the rewriter-side half; the term-gen allocation discipline + cross-source semi-join cost are the ADR-0006 half. Both promoted from the ADR-0020 research register.

**Update (2026-07-19, Run 4 Wave B3) — a second condition-only exception to
term-construction lifting.** The lifting invariant ("the SELECT projects raw
key columns; RDF terms are built during reconstruction") is unchanged, but the
WHERE clause now has a second place where term-LEXICAL text is computed in SQL
(the first being `StrMatch`'s LIKE/regex pushdown): `SqlCond::TemplateEq`
renders two differently-shaped templates as dialect-correct CONCAT expressions
and compares them, resolving template-shape-mismatch equality (`unify.rs`,
`emit.rs::render_template_concat`). Boolean-condition-only — never SELECTed,
never reconstructed from — so the lifting economics are untouched. Restricted
to term classes where lexical equality IS term equality (IRIs, plain/
`xsd:string` literals) and to dialects whose NULL-propagation through concat
matches R2RML §11 term-absence (`||` on PG/SQLite, `CONCAT` on MySQL; every
other dialect stays a sound 501).
