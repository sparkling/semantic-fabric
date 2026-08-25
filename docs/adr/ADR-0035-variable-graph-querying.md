---
status: accepted
date: 2026-07-20
updated: 2026-08-25
tags: [graph-queries, named-graphs, quad-semantics, sparql-dataset, unfold, rdf-star]
supersedes: []
depends-on:
  - ADR-0007
  - ADR-0032
  - ADR-0034
implements: []
---

# Variable-graph querying over R2RML graph sets

## Implementation status

**Accepted; corrective implementation landed, full acceptance evidence
partial.** Commit `10dedd4` makes incompatible ordinary and `rr:class` binds
prune their branches. Commit `5218874` introduces a normalized distinct
subject-map/POM graph union and threads it through mapping, ordinary unfolding,
paths, RDF-star, dump/materialization, and execution.

Eight graph-union and three RDF-star regression cases pass with the targeted
flat/tree/oracle suites; the locked/offline workspace and W3C gates also pass
with one adjudicated documented deviation and no unexpected failure.
The decision is not marked fully implemented because the planned live
PostgreSQL matrix, mutation-lite receipt, and accepted dual-host MetaHarness
transaction were not completed. Those are evidence gaps, not a reversal of the
landed semantic correction.

## Context and problem statement

SPARQL `GRAPH ?g` ranges over named graphs in the active dataset, never the
default graph, and binds `?g` to each matching graph name. R2RML declares the
dataset graph structure through graph maps on both a subject map and its
predicate-object maps.

The original implementation treated a predicate-object map's graph maps as an
override of its subject map's graph maps. That is incorrect. Each generated
triple is placed in every graph in the **distinct union** of both graph-map
sets. An empty union places it in the default graph; `rr:defaultGraph` is an
explicit default-graph member. The materializer already follows this rule, but
ordinary unfolding and property paths do not, creating query/materialization
disagreement.

The same area also exposes a separate binding invariant: an incompatible
subject, predicate, object, class, or graph binding must prune its branch.
Ignoring a failed bind can leak impossible solutions, especially when one
variable is reused across positions.

## Decision

1. **One canonical graph-set function.** Query unfolding, property paths,
   RDF-star description-map inheritance, and quad materialization use the same
   normalized, duplicate-free union of `SubjectMap.graphs` and
   `PredicateObjectMap.graphs`.
2. **Dataset matching follows union membership.**
   - A default BGP accepts a triple when the union is empty or contains
     `rr:defaultGraph`.
   - `GRAPH <g>` accepts it when the union contains `g`.
   - `GRAPH ?g` emits one branch per distinct named-graph member and never
     binds `rr:defaultGraph`.
3. **Dynamic graph maps stay sound.** A pinned constant graph against a
   template or column graph map must add a runtime equality constraint or
   return an honest `Unsupported`; silently returning no rows is not sound.
   Variable-graph paths over a row-dependent graph set retain the same rule.
4. **Graph variables are ordinary correlated bindings.** Projection,
   filtering, `VALUES`, joins, and nested `GRAPH` reuse the existing unifier.
   Every bind operation in ordinary triple and `rr:class` unfolding must check
   its result and prune incompatible branches.
5. **Paths use identical graph semantics.** Constant filtering and variable
   enumeration for property paths operate over the normalized union, not an
   override approximation.
6. **RDF-star preserves the complete graph set.** Description maps inherit the
   normalized union in both quoted-subject and quoted-object positions.
7. **Set semantics are unchanged.** `?g` is part of the solution tuple;
   duplicate graph declarations normalize to one destination and one solution.

## Consequences

- Good: direct query results and materialized-quad results share one semantic
  rule and can serve as differential oracles for each other.
- Good: subject and POM graph declarations compose as R2RML requires, including
  mixed default/named placement and RDF-star descriptions.
- Good: one checked binding invariant prevents a family of repeated-variable
  wrong-result bugs rather than patching one subject call site.
- Cost: graph handling crosses `sf-mapping` and `sf-sparql`; issue #9 needs one
  semantic owner and sequential integration with issue #8 where both touch
  unfolding.
- Neutral: the virtual graph architecture remains push-down based; no quad store
  or dataset materialization is introduced into the runtime path.

## Corrective test contract

All applicable cells run through flat and tree plans, SQLite and PostgreSQL,
`=_bag`, and the spareval/materialized-quad oracle:

1. Two constant named graphs plus a default-only map: `GRAPH ?g` returns only
   named-graph triples and binds each graph correctly.
2. Two patterns sharing one graph variable correlate within the same graph;
   cross-graph combinations are absent.
3. Distinct subject and POM named graphs make the triple visible in both.
4. The same graph declared at both levels yields one solution, not a duplicate.
5. Subject `rr:defaultGraph` plus a POM named graph is visible in default and
   named evaluation, while `GRAPH ?g` binds only the named graph.
6. Constant `GRAPH <g>` and property paths match either half of the union.
7. Constant, template, and column graph-map cases either answer correctly or
   return the documented `Unsupported`; none silently return a false empty.
8. RDF-star description maps in subject and object position inherit every
   member of the complete union.
9. Removing or negating either half of the union/default-sentinel logic is
   killed by mutation-lite checks.
10. Incompatible constants and repeated variables across S/P/O/GRAPH,
    including `?x ?x ?o`, `?x ?p ?p`, and
    `GRAPH ?s { ?s :p ?o }`, prune their branches.
11. `rr:class` atoms and inverse-predicate subject/object swaps obey the same
    checked-bind invariant, with no increase in unsupported queries.

The W3C RDB2RDF suites remain green after the targeted graph, path, star, and
binding-prune suites.

## Historical landing note

The 2026-07-20 implementation correctly established variable-graph branch
enumeration, widened path graph patterns, preserved nested-`GRAPH` scoping, and
threaded graph context into standalone RDF-star description maps. Those facts
remain useful history. The former POM-falls-back-to-subject assumption and the
"all eight cells prove implementation" conclusion are withdrawn.

## Rules

- **R1** — effective triple placement is the distinct subject-map/POM graph
  union; POM graph maps never override subject graph maps.
- **R2** — `rr:defaultGraph` is never emitted as a `GRAPH ?g` binding.
- **R3** — dynamic graph matching is constrained at runtime or rejected
  explicitly, never approximated as empty.
- **R4** — every incompatible S/P/O/class/GRAPH bind prunes its branch.
- **R5** — flat, tree, path, RDF-star, and materialization semantics stay in
  differential agreement.
