//! Ontop-parity oracle port — batch 6 of 8 (ADR-0022).
//!
//! Assigned slice `[30, 35)` over the combined, path-sorted `*Test.java` listing
//! of `~/source/ontop/core/optimization/src/test/java/it/unibz/inf/ontop/iq/{executor,optimizer}/`
//! (33 files, indices 0..=32). The slice resolves to the three tail classes:
//!
//!   * index 30 — `iq/optimizer/UriTemplateTest.java`            ( 1 scenario)
//!   * index 31 — `iq/optimizer/ValuesNodeOptimizationTest.java` (25 scenarios)
//!   * index 32 — `iq/optimizer/ValuesNodeTest.java`             (22 scenarios)
//!
//! **No oracle tests are written here — and this is the honest outcome, not a
//! stub.** All 48 scenarios are BOUNDARY or NEEDS_IMPL; *none* are SUPPORTED by
//! sf's cascade. The uniform root cause: sf's optimizer cascade
//! (`crates/sf-sparql/src/cascade/mod.rs`, six passes over [`Branch`] base-table
//! `Scan`s) has **no inline-VALUES IR** and **no IQ-tree-node normalization
//! framework**. A SPARQL `VALUES` clause is exploded at *unfold* time
//! (`crates/sf-sparql/src/unfold.rs`, `GraphPattern::Values`) into a bag union of
//! **core-less branches** whose cells are `TermDef::Const` bindings — i.e. the
//! inline data is gone *before* the cascade ever runs. There is no `ValuesNode`,
//! `SliceNode`, `DistinctNode`, `UnionNode`, or `ConstructionNode` for a pass to
//! rewrite, and no `applyDescendingSubstitution` / `normalizeForOptimization`
//! machinery. Every scenario in these three classes asserts a *normalization of
//! such nodes*, which sf neither models nor can faithfully express. Fabricating a
//! cascade test here (feeding `run()` const-bound branches and asserting a no-op,
//! or re-purposing pass-1 IRI-template pruning on a contrived non-VALUES input)
//! would test the *absence* of the optimization or a *different* scenario — pure
//! coverage theater, which the charter forbids. See `src/cascade/ws_g.rs` /
//! `ws_fk.rs` / `ws_st.rs` for the port pattern used by the batches whose Ontop
//! classes *do* land on the relational base-scan cascade.
//!
//! Per-class classification (provenance cited against the Ontop 5.5.0 source):
//!
//! ## index 30 — `UriTemplateTest` — 1 scenario — BOUNDARY
//!
//! * `testCompatibleUriTemplates1` — **BOUNDARY.** `@Ignore`d in Ontop *and*
//!   asserts nothing (the body only `System.out.println`s; the optimizer result is
//!   never compared). It exercises `UNION_AND_BINDING_LIFT_OPTIMIZER`: a LEFT JOIN
//!   over (INNER JOIN of two `ConstructionNode`s binding `?x` via *compatible* URI
//!   templates) is lifted to an inner join carrying a derived strict-equality
//!   `CONCAT("http://example.org/ds1/", a) = c`. That is an IQ-tree binding-lift +
//!   compatible-template merge over `ConstructionNode`/`UnionNode`/`LeftJoinNode`
//!   — Ontop IQ scaffolding (ADR-0004), with no analogue in sf's base-scan
//!   cascade (sf has no construction/union node to lift bindings across).
//!
//! ## index 31 — `ValuesNodeOptimizationTest` — 25 scenarios
//!
//! NEEDS_IMPL (14) — data-reducing normalizations that presuppose an inline-VALUES
//! IR sf lacks; each would be a genuine, mostly cardinality-changing optimization
//! (so NOT `=_bag`-preserving — they belong to SPARQL `VALUES`/`SLICE`/`DISTINCT`
//! semantics, not the cascade's `=_bag` rewrites):
//!
//! * `test1normalizationSlice`, `test2normalizationSlice` — SLICE (LIMIT/OFFSET)
//!   over a `ValuesNode` collapses to a single-row `CONSTRUCT`.
//! * `test3normalizationDistinct` — DISTINCT over a `ValuesNode` dedups its rows.
//! * `test4normalizationSliceUnionValuesValues`,
//!   `test5normalizationSliceUnionValuesNonValues`,
//!   `test5normalizationSliceUnionValuesValuesNonValues`,
//!   `test6normalizationSliceUnionValuesNonValues`,
//!   `test7normalizationSliceUnionValuesNonValues` — SLICE over a UNION with
//!   `ValuesNode` arms: merge the values arms and push the limit through.
//! * `test8normalizationDistinctUnionValuesNonValues`,
//!   `test9normalizationDistinctUnionValuesNonValues` — DISTINCT over a UNION with
//!   a `ValuesNode` arm: dedup the arm / push a DISTINCT down.
//! * `test10normalizationLimitDistinctUnionValues`,
//!   `test11normalizationLimitDistinctUnionValues`,
//!   `test12normalizationLimitDistinctUnionDistinctTree`,
//!   `test13normalizationLimitDistinctUnionNonDistinctTree` — SLICE·DISTINCT·UNION
//!   limit push-down through (non-)distinct arms.
//!
//! BOUNDARY (11) — Ontop `normalizeForOptimization` term/binding-lift framework
//! over `ConstructionNode`-over-`TrueNode` folded into a `ValuesNode`, plus
//! RDF-term-type decomposition (`RDFTermTypeConstant`, split `f0`/`f1` lift
//! variables). Pure IQ scaffolding (ADR-0004); sf has no such node IR:
//!
//! * `test14normalizationConstructionUnionTrueTrue`,
//!   `test15normalizationConstructionUnionTrueTrueDataNode`,
//!   `test17normalizationConstructionUnionTrueTrueDBConstant`,
//!   `test18normalizationConstructionUnionTrueTrueRDFConstant`,
//!   `test19normalizationConstructionUnionTrueTrueRDFConstant`,
//!   `test21normalizationConstructionUnionTrueTrueIRIConstant`,
//!   `test22normalizationConstructionUnionTrueTrueNonConstant`,
//!   `test23normalizationConstructionUnionTrueTrueRDFConstant`,
//!   `test24normalizationConstructionUnionTrueTrueRDFConstantSub`,
//!   `test25NoVariableTrueNodesAndValuesNodes`,
//!   `test26MergeableCombinationOfTrueConstructionValuesNodes`.
//!
//! ## index 32 — `ValuesNodeTest` — 22 scenarios
//!
//! NEEDS_IMPL (15) — presuppose an inline-VALUES IR sf lacks:
//!
//! * `test1normalization`, `test2normalization`, `test3normalization`,
//!   `test5normalization` — lift constant columns out of a `ValuesNode` into a
//!   `ConstructionNode` substitution (a value identical across all rows).
//! * `test12propagateDownConstraint` — push a FILTER (`?x < 2`) into the
//!   `ValuesNode`, dropping non-satisfying rows.
//! * `testJoinIRITemplateString1`..`testJoinIRITemplateString9` — join a
//!   `ValuesNode` of IRI *string* constants with an IRI-template `ConstructionNode`:
//!   decompose each string back into the template placeholder columns, prune rows
//!   that do not match the template (→ `EmptyNode` when none match —
//!   `testJoinIRITemplateString3`), keep a strict-equality for the non-injective
//!   case (`...5`), and handle NULL / cast / `%2F`-encoded / multi-template rows.
//!   This *generalizes* sf's pass-1 IRI-template-mismatch pruning, but over inline
//!   `VALUES` rows rather than two conflicting `=` constants on a scan column — so
//!   it cannot be expressed against sf's cascade without the VALUES IR. (Porting it
//!   onto pass-1's two-conflicting-constants shape would be a *different* scenario:
//!   coverage theater, refused.)
//!
//! BOUNDARY (7) — Ontop IQ-node unit tests of the substitution / normalization
//! framework, not cascade optimizations (ADR-0004):
//!
//! * `test4normalization` — an empty `ValuesNode` (one empty tuple) normalizes to a
//!   `TrueNode`.
//! * `test6substitutionNoChange`, `test7substitutionConstant`,
//!   `test8substitutionFunction`, `test9substitutionVariable`,
//!   `test10trivialSubstitutionVariable`, `test11substitutionTriple` — exercise
//!   `ValuesNode.applyDescendingSubstitution(...)` (Ontop's down-propagation
//!   machinery). sf has no descending-substitution pass; bindings are resolved at
//!   unfold, not propagated through an IQ tree.
//!
//! (`testJoinIRITemplateString10` is `@Ignore`d in Ontop — counted in the 22 above,
//! classified NEEDS_IMPL with its siblings — pending a type-specific cast mockup.)
//!
//! ---
//! scenarios_total = 1 + 25 + 22 = 48; converted_green = 0; converted_red = 0
//! (no SUPPORTED-claimed scenario diverged, because none is SUPPORTED).
