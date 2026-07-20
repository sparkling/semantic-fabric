---
status: proposed
date: 2026-07-20
updated: 2026-07-20
tags: [graph-queries, named-graphs, quad-semantics, sparql-dataset, unfold, rdf-star]
supersedes: []
depends-on:
  - ADR-0007
  - ADR-0032
  - ADR-0034
implements: []
---

# Variable-graph querying: `GRAPH ?g` over the R2RML-declared named-graph structure

## Status note (2026-07-20)

Proposed under the Run-5 loop ("ledger must be empty — nothing deferred"). This
was the last ledger item deferred as "its own charter"; this ADR is that
charter plus the design, sized for implementation in the same loop.

## Context and Problem Statement

`GRAPH <g> { P }` works (the `current_graph` mechanism: `unfold.rs`'s
`GraphPattern::Graph` arm pins the graph, `graph_maps_match` filters candidate
POMs by their effective graph, R2RML §4.6 POM-overrides-subject precedence,
and — since Run 4 A2 — property paths honor it with graceful-empty). But
`GRAPH ?g { P }` — the variable form — is a sound 501: the `Graph` arm's
`name` match has no `Variable` case.

SPARQL §13.3: `GRAPH ?g` ranges over the dataset's **named graphs** (never the
default graph), binding `?g` to each named graph's name and evaluating `P`
inside it. Under R2RML, the named-graph structure is **statically declared in
the mapping**: a triple lands in named graph `g` iff its POM's effective graph
maps (POM's own `rr:graphMap`s, else the subject map's; empty ⇒ default graph)
produce `g`. Graph maps are `TermMap`s — constants in almost every real
mapping, templates occasionally, columns rarely.

## Decision

Implement `GRAPH ?g` as **per-branch graph enumeration + an ordinary variable
binding** — no quad-store, no dataset materialization, no new engine concept:

1. **The `Graph { name: Variable(v), inner }` arm** translates `inner` in a new
   mode: instead of filtering candidates by a pinned `current_graph`, each
   candidate POM contributes one branch **per effective graph map** (fan-out
   over `Vec<TermMap>`; the common case is 0 or 1), and each branch gains a
   binding for `v`:
   - constant graph map → `TermDef::Const(NamedNode)` — the dominant case;
   - template graph map → an ordinary `Derived` template binding;
   - column graph map → a `Derived` column binding (IRI-typed);
   - **no graph map ⇒ the POM's triples live in the default graph ⇒ that
     candidate is EXCLUDED** (§13.3: named graphs only).
2. **Same-graph correlation is free.** Two patterns inside one `GRAPH ?g`
   block share `v`, so the existing unification machinery (`unify`,
   `align_templates`, the Const/Const disjointness case, TemplateEq where
   shapes mismatch) enforces same-graph joining exactly as it enforces any
   shared-variable join. Projection of `?g`, `FILTER(?g = …)`, `VALUES ?g`,
   and joins on `?g` OUTSIDE the block all come along for free — `?g` is just
   a variable with per-branch definitions.
3. **Property paths under `GRAPH ?g`**: hop resolution needs ONE pinned graph
   per compiled closure. Since constant graph maps are statically enumerable,
   compile `GRAPH ?g { …path… }` as the **union over the mapping's declared
   constant named graphs** — one `GRAPH <g_i>` instance per declared constant
   graph (each already works post-Run-4), with `?g = Const(g_i)` per arm.
   Residual (sound 501, pinned): paths under a **template/column** graph map —
   the graph set is row-dependent, not enumerable at translate time.
4. **RDF-star inside named graphs**: the encoding's description maps must
   carry the SAME effective graph as the star map that quotes them —
   `sf-mapping` today emits description POMs with no graph maps (default
   graph), which is WRONG the moment a star map sits under `rr:graphMap`.
   Fix in the mapping compiler: description-map POMs inherit the quoted/outer
   map's effective graphs; `star_decode` and the oracle then see the
   annotation in the right graph. (Today this is unobservable — star + named
   graphs has no coverage — the differential cells land with this ADR.)
5. **Set semantics (ADR-0034) interaction**: `?g` participates in the
   solution tuple, so D1/D2 dedup keys and elision proofs extend unchanged
   (the graph binding is one more output-determining column; a POM with
   multiple graph maps multiplies branches, not rows-within-a-branch).

## Boundaries (each pinned, with cause)

- Paths under non-constant graph maps → 501 (row-dependent graph set).
- `GRAPH ?g` + `rr:sqlQuery` sources with no introspectable schema behave as
  elsewhere (no special interaction).
- Nested `GRAPH` (a `GRAPH` inside a `GRAPH ?g` body) follows SPARQL scoping:
  the inner pin wins for its subtree; the outer `?g` still binds from the
  inner pattern's graphs only if compatible (inner constant ≠ a candidate's
  graph ⇒ branch pruned — falls out of unification with `?g`'s Const).

## Test contract

Differential cells vs the spareval oracle (which evaluates the decoded
DATASET — verify `oracle::evaluate` builds named graphs from the fixture; if
today's fixtures/decoder are graph-blind, extend them first — that is part of
"tests sound and complete", not an excuse):
1. `GRAPH ?g { ?s ?p ?o }` over a mapping with 2 constant named graphs + a
   default-graph POM: only the named-graph triples, `?g` bound correctly.
2. Same-graph join correlation: two patterns, one `?g` — rows only where both
   triples share a graph; cross-graph combinations excluded.
3. Template graph map: `?g` from a rendered template; equality with a
   constant `?g` from another branch (exercises Const/Template unification).
4. `GRAPH ?g` + path over constant graphs (the enumeration union).
5. Star annotation under a named graph: reifier + description in the star
   map's graph; `GRAPH ?g { << … >> ex:p ?v }` binds `?g` and answers.
6. Pinned 501: path under a template graph map.
7. Both engines, `=_bag`, throughout; W3C suites untouched.
