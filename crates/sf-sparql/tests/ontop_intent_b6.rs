//! Ontop-parity intent file — batch 6 of 8 (ADR-0022).
//!
//! Companion to `ontop_port_b6.rs`.  That file contains the per-scenario
//! classification rationale; this file captures the *optimization intent* of
//! each Ontop class and explains why the intent cannot yet be expressed against
//! sf's cascade IR.
//!
//! ---
//! ## Summary
//!
//! | Class                          | Scenarios | Converted GREEN | Converted RED |
//! |--------------------------------|-----------|-----------------|---------------|
//! | `UriTemplateTest`              |  1        |  0              |  0            |
//! | `ValuesNodeOptimizationTest`   | 25        |  0              |  0            |
//! | `ValuesNodeTest`               | 22        |  0              |  0            |
//! | **Total**                      | **48**    |  **0**          |  **0**        |
//!
//! All 48 scenarios are BOUNDARY or NEEDS_IMPL.  No test functions appear in
//! this file — that is the honest outcome, not a gap.
//!
//! ---
//!
//! ## `UriTemplateTest` — 1 scenario — BOUNDARY
//!
//! **Optimization intent:** When two `ConstructionNode`s bind the same variable
//! `?x` via *compatible* URI templates (one with a prefix, one with no prefix)
//! and are joined under a `LeftJoinNode`, Ontop's
//! `UNION_AND_BINDING_LIFT_OPTIMIZER` merges the join into an `InnerJoinNode`
//! whose filter is the derived strict-equality
//! `CONCAT("http://example.org/ds1/", a) = c`.  The right arm (incompatible
//! template prefix) is eliminated entirely.
//!
//! **Why BOUNDARY in sf:** The scenario requires `ConstructionNode`,
//! `LeftJoinNode`, and `InnerJoinNode` — Ontop IQ-tree nodes (ADR-0004) that
//! have no counterpart in sf's `Branch`/`Scan` IR.  sf's pass-1 does prune
//! conflicting IRI-template constants on a single scan column, but that is a
//! *two-conflicting-constants-on-a-scan* shape, not a lifting of bindings
//! across a left-join over construction nodes.  The test is also `@Ignore`d in
//! Ontop 5.5.0 and asserts nothing (body is `System.out.println` only), so
//! even Ontop itself does not currently enforce this behaviour.
//!
//! ---
//!
//! ## `ValuesNodeOptimizationTest` — 25 scenarios
//!
//! **Optimization intent (NEEDS_IMPL, 14 scenarios):** Ontop materialises
//! inline SPARQL `VALUES` data as a `ValuesNode` in the IQ tree and then
//! propagates structural knowledge about that node upward:
//!
//!   * A `SliceNode(offset=0, limit=k)` over a `ValuesNode` whose row-count
//!     is <= k collapses the slice to a `ConstructionNode`-over-`TrueNode`
//!     binding the first row (tests 1-2).
//!   * A `DistinctNode` over a `ValuesNode` deduplicates inline rows in place
//!     (test 3).
//!   * A `SliceNode` or `DistinctNode` over a `UnionNode` whose arms include
//!     one or more `ValuesNode`s: merge the values arms, push the limit or
//!     dedup through (tests 4-13).
//!
//! **Why NEEDS_IMPL:** sf's `unfold.rs` (the `GraphPattern::Values` branch)
//! explodes a SPARQL `VALUES` clause into a bag union of coreless branches
//! whose cells are `TermDef::Const` bindings *before* the cascade runs.  By
//! the time `cascade::run` is called there is no `ValuesNode` to rewrite; the
//! inline data is already in the branch bag.  Implementing these optimizations
//! would require either (a) a `ValuesNode` variant in `Branch` so the cascade
//! can see the inline data, or (b) a pre-cascade SLICE/DISTINCT pass over the
//! branch bag that detects all-const branches and prunes/deduplicates them.
//! Either constitutes a new IR feature, tracked separately from ADR-0022 batch
//! conformance.
//!
//! **Optimization intent (BOUNDARY, 11 scenarios):** The remaining tests
//! exercise `normalizeForOptimization` over a `ConstructionNode`-over-`TrueNode`
//! being folded into a `ValuesNode`, including RDF-term-type decomposition
//! (`RDFTermTypeConstant`, split `f0`/`f1` lift variables) and constant-row
//! merging across heterogeneous arm types.  This is Ontop's internal IQ-node
//! normalization framework and has no analogue in sf's cascade.
//!
//! ---
//!
//! ## `ValuesNodeTest` — 22 scenarios
//!
//! **Optimization intent (NEEDS_IMPL, 15 scenarios):**
//!
//!   * Constant-column lift (tests 1-3, 5): when a column in a `ValuesNode`
//!     has the same value in every row, Ontop lifts it into a
//!     `ConstructionNode` substitution so downstream joins and filters see a
//!     simpler binding.
//!   * Filter push-down into `ValuesNode` (test 12): a `FILTER(?x < 2)`
//!     is pushed into the `ValuesNode`, dropping non-satisfying rows at the
//!     relational level before any join.
//!   * IRI-template join with `ValuesNode` rows (tests `testJoinIRITemplateString1`
//!     .`testJoinIRITemplateString9`): a `ValuesNode` holding IRI *string*
//!     constants is joined with a `ConstructionNode` that binds a variable via
//!     an IRI template.  Ontop decomposes each string back into the template's
//!     placeholder columns, prunes rows whose string does not match the template
//!     structure (yielding `EmptyNode` when none match -- test 3), and emits a
//!     strict-equality filter for the non-injective case (test 5).
//!
//!   The IRI-template-join shape is conceptually close to sf's pass-1 pruning
//!   of conflicting IRI-template constants, but it operates over *rows* of
//!   inline VALUES data rather than two scan-column equalities -- so it cannot
//!   be expressed against sf's cascade without the VALUES IR.
//!
//! **Optimization intent (BOUNDARY, 7 scenarios):**
//!
//!   * An empty `ValuesNode` (one empty tuple) normalizes to a `TrueNode`
//!     (test 4).
//!   * Tests 6-11 exercise `ValuesNode.applyDescendingSubstitution(...)` --
//!     Ontop's down-propagation machinery that rewrites a `ValuesNode` in
//!     place when a parent substitution is pushed down.  sf resolves all
//!     bindings at `unfold` time; there is no descending-substitution pass.
//!
//! ---
//!
//! ## Implementation notes for future NEEDS_IMPL work
//!
//! The two most tractable entry points if sf ever gains an inline-VALUES IR:
//!
//! 1. **All-const branch dedup / slice** -- after `unfold` produces the branch
//!    bag, a dedicated pre-cascade pass could detect branches that are entirely
//!    `TermDef::Const` (no scan), group them, apply limit/offset, and
//!    deduplicate.  This would cover the NEEDS_IMPL scenarios in
//!    `ValuesNodeOptimizationTest` and `ValuesNodeTest` tests 1-3/5/12 without
//!    requiring a new `Branch` variant.
//!
//! 2. **IRI-template-string row filtering** -- extend pass-1's
//!    `prune_template_contradictions` to also compare IRI-string constants from
//!    all-const branches against the IRI template of a paired scan branch,
//!    dropping rows that cannot match.  This directly generalises the existing
//!    two-constant contradiction check.
//!
//! Neither is part of ADR-0022 batch-6 scope; they belong to a future ADR or
//! NEEDS_IMPL backlog entry.
