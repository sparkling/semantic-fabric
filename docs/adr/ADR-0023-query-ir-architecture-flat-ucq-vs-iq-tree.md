---
status: accepted
date: 2026-06-30
tags: [ontop-parity, ir, architecture, optimizer, operator-tree, iq, substitution-lifting, normalization, t-mappings, saturation, aggregation, charter]
supersedes: []
depends-on:
  - ADR-0004
  - ADR-0006
  - ADR-0007
implements:
  - ADR-0021
---

# Query IR architecture — adopt a native operator-tree IR with substitution-lifting normalization (the optimal end state, built directly)

## Context and Problem Statement

semantic-fabric optimises a query as a **flat union of conjunctive queries**: a `Vec<Branch>` where each `Branch` (`crates/sf-sparql/src/iq.rs`) is a fixed two-level shape — inner-join `core` + one layer of `opts` LEFT JOINs + flat `where_conds` + a `bindings: var→TermDef` substitution + two mutually-exclusive escape slots `path`/`agg`. The cascade (`cascade/mod.rs:run`) is a single linear sweep of ~13 hand-written `=_bag`-preserving passes; union is the top-level `Vec`, executed one SQL `SELECT` per branch.

The reference OBDA engine, **Ontop**, uses a rooted **operator tree** (the *Intermediate Query*, IQ) — `ConstructionNode`/`Union`/`InnerJoin`/`LeftJoin`/`Filter`/`Aggregation`/`Distinct`/`Slice`/`OrderBy`/`Values`/`Flatten`/`Empty`/`True` over data leaves — normalised by **substitution-lifting to a fixpoint**. ADR-0021 recorded re-architecting to this model ("option B") as a deferred, charter-level decision. This ADR takes that decision.

A three-pillar, code-grounded review (2026-06-30 — independent steelman against the flat IR, a deep Ontop-IQ teardown, and a SOTA optimizer-architecture survey) established:

* **sf already owns IQ's hardest idea.** `TermDef` is exactly Ontop's `ConstructionNode` substitution, lifted to the outer projection (ADR-0007 term-construction lifting); `Vec<Branch>` is the "union of CQs with term constructors" normal form Ontop *normalises toward*. The regression-prone part of an IQ port — substitution composition under 3-valued logic — sf already has working and conformance-tested.
* **The fundamental limit is eager flattening.** `unfold.rs:join_branches` distributes `(A∪B)⋈(C∪D)` into independent branches *before* the cascade, erasing the union/nesting structure the residual optimizers and uniform composition need; worst case `mᵗ` branches.
* **The flat model is a structural ceiling for parity.** Verified disposition of Ontop's optimizer suite (33 classes / 656 scenarios): 79 oracle-green; 12 classes' signature transform implemented; **16 classes / 184 scenarios require IQ nodes the flat `Branch` lacks**; 2 SUBSUMED. Aggregate-over-UNION even *errors* end-to-end at HEAD (`BIND references unbound`) because the flat model has no `Aggregation`-over-`Union` composition.

The program goal (ADR-0021, owner directive) is **maximal Ontop parity within charter — optimizations and test-passing — benchmarked faster than Ontop, no deferrals.** The decision is which IR makes that goal *reachable*, built as the optimal end state rather than an incremental compromise.

## Decision Drivers

* **Charter (ADR-0004/0006/0007).** Own the rewriter in Rust, no JVM; stream / push down / bounded memory; `=_bag` is absolute; term-construction is lifted. Any IR must preserve all of these.
* **Division of optimization labour with the backend DB (corrected).** sf emits one SQL query that PostgreSQL/SQLite re-plan with their own statistics. Therefore: **physical optimization (join order, access paths, join algorithms) is the DB's job — sf must NOT do cost-based physical planning** (no stats; redundant). But **semantic/logical optimization is sf's job and is high-value** — the DB cannot perform OBDA-specific rewrites (self-join elimination via R2RML unique keys, redundant-FK-join elimination, empty/true propagation, union reduction, provable-`DISTINCT` removal, aggregation-through-union), and simpler emitted SQL plans faster and avoids pathological plans. Aggressive *logical* rewriting is exactly what a rich operator-tree IR enables.
* **Maximal parity is the goal, built directly.** Time/effort phasing is not a constraint. The architecture must make the full in-charter optimizer set and the full SPARQL surface *expressible*, not merely approachable by accretion.
* **Correctness is non-negotiable and is not "incrementalism."** Every rewrite holds the ADR-0005/0012/0013 gates (`=_bag` differential + W3C RDB2RDF floor + clippy + fmt). Verification checkpoints during the build are correctness, not timidity.

## Considered Options

* **Keep flat-UCQ + extend with special-case slots/passes.** Rejected: a structural ceiling — the 184-scenario OPTION_B set and uniform nesting (subqueries, nested aggregation, multi-scan OPTIONAL, agg-over-union) are *unreachable in principle* because eager flattening erases the structure; accretion re-implements an operator tree one bolt at a time, worse.
* **Native operator-tree IR with substitution-lifting normalization (Ontop-IQ-grade, in-charter).** **Chosen.** Makes the full in-charter optimizer set and SPARQL surface expressible; reuses sf's existing substitution machinery; lowers to SQL.
* **Cost-based / Cascades optimizer.** Rejected: physical planning is the DB's job (no stats in sf; OBDA join order is fixed by pattern+mapping). Orthogonal to the logical-rewrite IR; not built.
* **E-graph equality saturation as the normalization engine.** Deferred (not rejected forever): once the tree IR exists, an egg/egglog rule engine could replace heuristic fixpoint normalization and remove pass-ordering fragility — but each rule needs an independent bag-semantics soundness proof, and it presupposes the tree. Build the tree with heuristic substitution-lifting normalization first (Ontop's proven approach); revisit e-graphs as a later normalization-engine swap.
* **Java→Rust transliteration of Ontop.** Rejected (ADR-0004): Ontop is the specification/oracle, never source to port; entangled with OWLAPI/Guice/RDF4J/JDBC.

## Decision Outcome

**Adopt a native-Rust operator-tree IR with substitution-lifting normalization as THE query IR, built directly as the optimal end state.** It replaces the flat-`Vec<Branch>` *optimizer* model; the flat `Branch` (and `emit`) are retained only as the **SQL-lowering target** for a normalized leaf CQ. Goal: every in-charter Ontop optimization expressed as a tree rewrite, the full SPARQL surface composing uniformly, `=_bag` held throughout, benchmarked to stay faster than Ontop.

### The IR (in-charter node set)

`ConstructionNode` (variable→term `ImmutableSubstitution`, with projection), `FilterNode`, `InnerJoinNode` (n-ary), `LeftJoinNode`, `UnionNode` (n-ary, bag), `AggregationNode` (grouping + aggregate substitution), `DistinctNode`, `SliceNode`, `OrderByNode`, `ValuesNode`, `EmptyNode`, `TrueNode`; leaves `ExtensionalDataNode` (mapped relation, sparse columns) and `IntensionalDataNode` (pre-unfold triple pattern). Modelled as a Rust `enum` with exhaustive `match` (no JVM class hierarchy / DI). **Out of charter, not built now:** `FlattenNode`/JSON unnest (only if nested-data sources are targeted), cost-driven translation selection.

### Normalization (the engine)

Substitution-lifting to a fixpoint (compose `ConstructionNode` substitutions toward the root → "union of CQs with term constructors" normal form), plus the structural rewrites as tree-pattern rules, each carrying a `=_bag` soundness argument: self-join elimination (PK/UC/composite/FD), redundant-FK-join elimination, LeftJoin→InnerJoin downgrade (nullability + ancestor-filter null-rejection), aggregation-through-union, filter push-down/up across operator boundaries, Empty/True propagation, `DISTINCT` removal under proven uniqueness, union-branch merging, FD transitive closure. The existing cascade passes are re-expressed as these rules (not discarded — their `=_bag` arguments transfer).

### Lowering

Normalized IR → SQL: a leaf CQ (Construction over a join/filter of ExtensionalDataNodes) lowers to today's `Branch`/`emit` path (preserving ADR-0006 streaming, ADR-0007 term-construction lifting, ADR-0010 bound-parameter discipline); `UnionNode` → `UNION ALL` / multi-branch stream; `AggregationNode` → SQL `GROUP BY`; `Slice`/`OrderBy`/`Distinct` as today. Three-valued-logic in `LeftJoinNode` substitution composition is the designated correctness hotspot (Ontop's own regression history) — gated by the SPARQL OPTIONAL conformance + `=_bag` differential suites.

### Offline stage — T-mappings (ontology saturation + mapping consolidation)

In scope, matching Ontop §2.1. At startup: (1) fold the class/property hierarchy (subclass/subproperty, domain/range — sf's tier-1 entailment, today done per-query in `saturate.rs`) into the mapping set so a query over a class need not consult the ontology at runtime; (2) prune redundant union branches using integrity constraints (PK/FK/UC). This narrows per-query union width *before* unfolding, amortising what per-query normalization would otherwise redo on every query. It is **separable from the IR** (a mapping-preprocessing stage, not an IQ node), so it is built alongside the tree but does not gate the IR core. Tier-2 tree-witness entailment stays excluded (ADR-0008; Ontop ships it off by default).

### Execution model (the build)

Built directly on a dedicated branch, correctness-gated continuously — **not** shipped as timid increments, **not** without `=_bag` verification. Design is locked first (node set + normalization contract + lowering contract + the rule-by-rule `=_bag` arguments), then the IR + normalizer + lowering are implemented, the in-charter optimizer set + full SPARQL surface ported as tree rewrites, the offline T-mapping stage built, and the full ported Ontop optimizer suite + W3C RDB2RDF conformance + PG↔SQLite differential + the Ontop benchmark run to completion. Adversarial `=_bag` verification (refute-only reviewers) gates every rule.

### Alignment with Ontop (recorded point-by-point)

**Same (deliberate — the goal):** the IQ node set (§3), substitution-lifting normalization, the rule-based structural + semantic constraint-driven optimizations (§5), unfolding, SQL generation, and — now in scope — the offline T-mapping stage (§2.1).

**Deliberate deltas (charter / substrate, not oversights):** OWL 2 QL **tree-witness rewriting** (§4) excluded by ADR-0008 / ODR-0030 — Ontop ships it *off by default* and the research notes call it rarely exercised; **FlattenNode / JSON lenses** out of charter unless nested-data sources are targeted; **Rust-native DB drivers** (SQLite / PostgreSQL / MySQL) rather than Ontop's JDBC dialect universe (ADR-0006); **Rust `enum` + `match`** with `Branch`/`emit` lowering rather than Ontop's JVM class hierarchy + Guice DI + separate SQL-IQ / `NativeNode` stage (ADR-0004 — same behaviour). With T-mappings in scope, the only remaining *functional* gap vs Ontop is tier-2 tree-witness entailment — which Ontop itself disables by default.

### Consequences

* Good: the 184-scenario structural residue and the full SPARQL surface become *expressible*; aggregate-over-union and multi-scan OPTIONAL fall out of node composition; literal-optimization parity becomes reachable, not ceilinged.
* Good: aggressive logical rewriting (which the DB cannot do) is done well, emitting simpler SQL — consistent with staying faster than Ontop.
* Good: a uniform tree replaces the growing set of special-case slots/bypasses and near-duplicated pattern detectors in the flat cascade.
* Bad/cost: the IR is the engine's heart — this is the highest-stakes change in the codebase; the `=_bag` re-proof of every rewrite against the existing test corpus is the gating effort, and 3-valued-logic in LeftJoin normalization is the known regression hotspot.
* Neutral: `Branch`/`emit` persist as the lowering target rather than being deleted; the flat model lives on below the normalized leaf CQ.

### Confirmation

* `=_bag` differential (PG↔SQLite) + W3C RDB2RDF floor (≥82/0) + `cargo clippy --all-targets -D warnings` + `cargo fmt --check` hold at every gate; every commit standalone-compiles.
* The ported Ontop optimizer suite (ADR-0022 oracle + the per-class scenarios) and the SPARQL conformance/spareval differential are the parity measure; the GTFS benchmark confirms sf stays faster than Ontop @1× and @10× (no regression).
* Parity is reported as honest fractions (oracle-green + intent-green / total; documented out-of-charter residue), never "100%".

## More Information

* **Adversarial review (2026-06-30):** code-grounded steelman against the flat IR; Ontop IQ teardown (node taxonomy, substitution-lifting normalization, per-optimization mechanisms, integrity constraints, SQL generation); SOTA survey (Cascades/cost-based, e-graph equality saturation, relational-tree heuristic, Datalog, push-based dataflow).
* **Key sources:** Calvanese et al., *Ontop* (SWJ 2017); Xiao et al., *Efficient Handling of SPARQL OPTIONAL for OBDA* (ISWC 2018, arXiv:1806.05918); Ontop IQ internals/optimization docs (ontop-vkg.org); egg (POPL 2021) / egglog (PLDI 2023); Suciu et al., *Semantic Foundations of Equality Saturation* (ICDT 2025, arXiv:2501.02413); *Mixing Set and Bag Semantics* (arXiv:1905.02069); Lanti et al., *Cost-Driven OBDA* (ISWC 2017, arXiv:1707.06974).
* **Cross-refs:** ADR-0004 (own the rewriter, no JVM), ADR-0006 (execution/performance model — the lowering preserves it), ADR-0007 (rewriting strategy / `=_bag` / term-construction lifting — carried into the tree), ADR-0021 (parity program; this is the IR decision under it), ADR-0022 (WS-G oracle suite — the regression gate), ADR-0008 (reasoning/charter).
