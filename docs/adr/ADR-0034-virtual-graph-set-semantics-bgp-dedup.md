---
status: proposed
date: 2026-07-19
updated: 2026-07-19
tags: [set-semantics, bgp-dedup, duplicate-rows, soundness, union-dedup, key-elision]
supersedes: []
depends-on:
  - ADR-0007
  - ADR-0025
  - ADR-0032
  - ADR-0033
implements: []
---

# Virtual-graph set semantics: BGP-level dedup for duplicate rows and cross-map same-triple emission

## Status note (2026-07-19)

Proposed, scheduled as Run 4 Wave C0. The red phase already exists: 9 intentional
failing cells in `differential_star.rs` (A3's oracle investigation,
`duplicate_source_row_*` ×6, `cross_source_*_bag_multiplicity_*` ×3), measured
engine-vs-oracle divergences from 4v3 (plain baseline) to 514v3 (nested star) and
405v2 (cross-product case).

## Context and Problem Statement

R2RML defines the output dataset as an RDF **graph — a set of triples**. SPARQL
§18.3 evaluates a BGP over that set: each distinct solution mapping μ with
μ(BGP) ⊆ G has cardinality **1** (the instance-mapping multiplicity clause
concerns blank-node instance mappings, not repeated triples — a duplicate source
row does not create a second triple, and two maps emitting the same triple
still describe one triple). The engines instead return one solution per
**source-row combination**: a duplicate row in a logical table, or two candidate
maps producing the identical triple, inflate the answer bag. The spareval oracle
(evaluating the decoded graph, which materializes as a set) is right; the
engines are wrong. A3 proved this is **general R2RML behavior, not
star-specific** — the plain-pattern baseline diverges 4v3 with one duplicated
row; star's extra shared-variable join positions only amplify the same
mechanism multiplicatively (66v3, 130v3, 514v3).

Every prior `=_bag` gate passed only because no fixture ever contained (D1) a
logical source with duplicate rows over the projected columns, or (D2) two
candidate maps agreeing on a triple.

## Decision

Dedup at the **BGP-block boundary**, where SPARQL's own semantics puts it —
never at the final result (projection/UNION above the BGP create *legitimate*
duplicates that must survive).

**D1 — within-branch (duplicate rows).** A branch whose joined tables do not
all contribute a declared key over the branch's output-determining columns gets
`SELECT DISTINCT`, reusing the existing single-branch DISTINCT pushdown
discipline (`iq.rs` — SELECT list restricted to output-determining columns,
per-branch, already proven for query-level DISTINCT).

**D2 — cross-branch (same triple from two maps).** A multi-branch pattern
relation joins its arms with `UNION` (set) instead of `UNION ALL`, under the
already-stated precondition (`emit_subplan_sql`, ADR-0025 Tier-2 gap 2): SQL
raw-column dedup equals SPARQL term dedup **only when cross-arm reconstruction
is injective**. Where arm reconstructions are not provably injective-compatible,
phase 1 refuses (sound 501, pinned); the general fallback (dedup over rendered
term expressions — the same fully-rendered-lexical lesson as the Fix-1 `pf:` id
repair) is phase 2 if a real mapping ever needs it.

**Elision — the performance story (this is why this is cheap in practice).**
Introspection already captures `TableSchema.primary_key` and `.unique`:

- D1 elides when every joined table's projected columns are covered by a
  declared PK/UNIQUE key (duplicate rows impossible) — the overwhelmingly
  common case (PK-templated subjects).
- D2 elides when the arms' subject/object templates are pairwise **provably
  disjoint** (`unify::templates_provably_disjoint` — existing machinery, ADR-0032
  D6): disjoint arms cannot produce the same mapping, so `UNION ALL` is already
  set-correct.

A well-keyed, disjointly-templated mapping — the norm — emits byte-identical
SQL to today. The DISTINCT/UNION cost lands only on mappings that can actually
produce duplicates, where it is the price of a correct answer.

**Interactions.**
- Aggregates: the BGP block sits below GROUP BY, so dedup-before-aggregation is
  automatic (COUNT over a duplicate-carrying source becomes correct, not just
  cosmetically deduped).
- Property paths: closure relations already dedup internally
  (`SELECT DISTINCT sf_s, sf_o`, iq.rs); the NPS `UNION ALL` bag exception is
  arm-disjoint by construction (a triple's predicate matches exactly one arm),
  so D2-elision applies to it verbatim; D1 still applies to its underlying
  scans.
- Both engines: the mechanism lives in branch emission + the shared
  branch-union seam, below the flat/tree fork — one implementation, two
  engines, same as ADR-0033's conversion.

## Consequences

- The 9 red cells go green; `=_bag` vs the oracle becomes unconditional rather
  than fixture-lucky. This closes a **soundness** gap in the project's own
  definition (answer equivalence with the native evaluator over the decoded
  graph).
- SQL shape changes only where duplicates are possible; elision cells must pin
  the common case emitting NO DISTINCT (SQL-shape assertions), and the criterion
  bench suite gates the perf claim (target: zero measurable regression on the
  existing PK-covered fixtures).
- The phase-1 non-injective cross-arm 501 is a new, honest, pinned boundary
  (expected to be unreachable for realistic mappings; revisit only on evidence).

## Test contract

1. All 9 `differential_star` set-semantics cells green, `=_bag` with spareval.
2. New plain-pattern (non-star) duplicate-row cells in `differential_tree` —
   the bug is general; its regression lock must be too.
3. Elision SQL-shape cells: PK-covered fixture emits no DISTINCT; disjoint-arm
   fixture emits UNION ALL.
4. Full suites: differential_tree/paths/star, adversarial_adr0033_refute, no
   regressions; bench before/after receipts on the standard suite.
