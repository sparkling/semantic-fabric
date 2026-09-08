---
status: accepted
date: 2026-07-19
updated: 2026-09-08
tags: [property-paths, join-composition, derived-table, sql-emission, tree-ir, sound-501-lift]
supersedes: []
depends-on:
  - ADR-0007
  - ADR-0023
  - ADR-0025
  - ADR-0032
implements: []
---

# Join-onto-path composition: path branches as alias-preserving derived tables

## Current implementation (2026-09-08)

Joined paths now retain `ScanSource::Path` (the typed closure and a separate
internal CTE alias) through lowering. They become SQL only during catalog-aware
emission. This refines the original `LogicalSource::Query` storage decision below,
without changing source-native CTE execution, outer aliases, raw reconstruction,
correlated EXISTS/MINUS, or NPS bag semantics. Authored Table/Query sources use the
distinct `ScanSource::Logical` variant; generated paths cannot inherit table
constraint or portable-row-policy authority. Ordinary scan constructors wrap their
logical source with `.into()`; no second source representation is retained.

Live preflight probes and validates each physical path leaf in core, OPTIONAL,
EXISTS/NOT EXISTS and nested SubPlans before any cursor. Missing output columns
reject; retained recipes participate in existing compiler-clone/resource checks.
Required owned PostgreSQL/MySQL CLI evidence exercises two joined closures with
mapping `SRC`/`DST` references over native lowercase columns. The former frozen-SQL
implementation fails this check; typed live emission passes. Existing correlated
and multiplicity regression checks remain required.

The subsequent ADR-0049 text repair uses live varying-text facts for native
comparison decorations and a same-IR SQLite metadata twin preserving decoding.
External JOIN/OPTIONAL/EXISTS/NOT EXISTS/MINUS comparisons decorate both sides;
NULL/unbound behavior remains tested. SubPlan text facts follow the actual shared
aggregate projection layout (including SQLite AVG metadata), retain standalone
path aliases and visit each nested plan once per metadata walk.

CHARACTER decoder recipes now normalize each leaf before joins/UNION/closure,
preserving each endpoint's own width. Normalized path aliases become text;
raw SubPlans retain exact recipes only where all arms agree. An explicit compiler
flag propagates through nested emission; authored SQL never activates callbacks.
Required duplicate/connectivity, mixed-width and authenticated correlation tests
pass, with pinned PostgreSQL/MySQL CHAR path evidence. Ordinary non-native-consumer
text comparisons/dedup and live-proven text/CHAR template-based native Ref atoms
now have public/native evidence (ADR-0034); unknown Ref keys retain prior D1.
Ordinary text/CHAR policy consumers now filter before D1 dedup with public/native
evidence; general term keys remain open.
ADR-0034 retains D1/D2
wrappers as typed projections until emission; only original sources are probed,
closing ordinary native folding without exposing table/constraint authority.
ADR-0015's atom-local NULL guards now remove non-triples before projection and
correlation; supported OPTIONAL and anti-join behavior is preserved. Equal node shapes alone do not prove all SQL-key/RDF equality;
historical green fixtures are not total source-work bounds or release admission.

## Implementation status (2026-07-19, same day)

**Accepted, implemented, adversarially refuted.** Raw-SQL spike passed both
dialects before implementation. All test-contract cells green vs the spareval
oracle, including the mandatory `!p` duplicate-multiplicity bag fixture (4 rows
— dedup would have halved it) and the GROUP-BY-over-joined-path open question
(**works** via ordinary single-branch SQL aggregation — locked green, no
boundary needed). Three pre-existing sound-501 pins in `differential_tree`
lifted for free by the same conversion (path-as-OPTIONAL right/left,
path-left + SubPlan-OPTIONAL-right — the last hand-re-derived independently).
A dedicated adversarial refute pass **SURVIVED all 8 attack surfaces** with
executed evidence (18 permanent regression locks in
`adversarial_adr0033_refute.rs`), including the slice-drop suspicion (refuted
structurally: `Branch.path` and `Branch.limit` are mutually exclusive by
construction — slices always route through `lower_as_subplan`'s pre-existing
guard) and cascade interactions (every core-rewriting pass destructures
`LogicalSource::Table` and skips Query-sourced scans by construction).
Findings routed elsewhere: `hop_sql` NULL-poisoning over nullable columns
(pre-existing, standalone paths too) and the tree resolver dropping GRAPH
constraints entirely (pre-existing, all pattern kinds) — both to the F4
correctness wave; FILTER-on-path-endpoint remains the pre-existing generic
template-var FILTER boundary, unchanged.

**Update (2026-07-19, Run 4 A2) — no longer tree-only.** The flat engine now
runs the SAME `convert_path_branches` pre-conversion at its own
`GraphPattern::Join` arm (`unfold.rs`) before `join_branches` — one shared
conversion, not a second implementation (`convert_path_branches` widened to
`pub(crate)`). Every "tree-only lift" phrasing in this ADR is superseded on
that point: joined-path shapes now answer on BOTH engines, restoring the flat
engine as the `=_bag` oracle over these shapes (`differential_paths.rs::
closure_joined_with_class_pattern_now_matches_oracle_on_both_engines`,
`differential_star.rs::star_pattern_at_property_path_endpoint_now_answers_on_
both_engines`, and the `!p` multiplicity fixture asserted on flat too — 4
rows, dedup would halve it). Same wave: wrong-graph paths return graceful
EMPTY instead of a sound 501 (`resolve_pred_hop` two-pass restructure +
`empty_hop`'s statically-empty derived table, `path.rs`), lifting the
adversarial suite's named-graph 501 pin to an oracle-checked 0-row answer.

## Context and Problem Statement

Any `GraphPattern::Path` joined with anything 501s today — path-shape-agnostic,
both engines, pinned by `differential_paths.rs::closure_joined_with_class_
pattern_hits_the_identical_general_boundary_on_both_engines` and inherited by
RDF-star patterns at path endpoints (`ADR-0032` D6). The literal mechanism: a
path-carrying `Branch` (`core: []`, `path: Some(PathClosure)`) is emitted by
`emit_path_branch` as one self-contained `WITH [RECURSIVE] … SELECT` statement
via an unconditional early return in `emit_branch_with` — it structurally
cannot also carry a join. `unfold::merge` (which the tree's `InnerJoin` arm
calls verbatim — one code path, not two) rejects path-presence before any
unification.

`ADR-0025` item 4's "path-as-derived-table, ~800 lines, M2 milestone" estimate
attached to a *different, harder* problem (multi-branch UNION-pooled
aggregation over paths — sidestepped at the time via `rust_group` routing).
The plain join case was never actually designed until now. Prior art in-tree:
`SqlCond::PathExists` (ADR-0025 gap 1) already proves the WITH-prelude works
nested inside a parenthesized subquery; `lower_as_subplan` proves derived-table
joins generally.

## Decision Outcome

**Convert a path-carrying branch, at the two tree join sites, into an ordinary
branch whose `core` holds one `Scan` with `LogicalSource::Query(<self-contained
path SQL>)` — keeping the outer scan alias identical to `PathClosure.alias`.**

* A new `emit::path_as_derived_table_sql(pc, cte_alias, dialect, catalog)`
  (~15 lines) reuses the proven `path_with_prelude` verbatim with a *fresh
  internal* CTE alias, wrapping it as
  `WITH … SELECT sf_s, sf_o FROM t{cte_alias}`.
* A new `lower::convert_path_branches` (~25–40 lines) takes `b.path`, pushes
  `Scan { alias: pc.alias, source: LogicalSource::Query(sql) }` onto `b.core`.
  Because the **outer alias is unchanged**, every pre-existing
  `TermDef::Derived{Column("sf_s"/"sf_o"), pc.alias}` reference in the
  enclosing `Construction.subst` / conditions resolves to the derived table's
  identically-named output columns — **zero cross-tree rewriting**.
* Call sites, both in `iq/lower.rs`, tree-only: the `IqNode::InnerJoin` arm
  (safe unconditionally — normalize collapses 1-child joins, so standalone
  paths never pass through it and keep today's top-level-`WITH` SQL shape) and
  the `IqNode::LeftJoin` arm (left operand always; right operand *before* the
  `is_single_subplan_branch` check, after which a bare-path right side falls
  naturally into `build_left_join`'s single-scan fast path — gaining OPTIONAL
  inner-FILTER support for free).
* The flat engine's path guards (`merge`, `minus_branches`, `lower_exists`,
  `leftjoin.rs`) stay **unchanged** — a tree-exceeds-flat lift, precedented by
  ADR-0025 gap 3. `SqlCond::PathExists` stays live for the sole-path-in-EXISTS
  case (the conversion fires only inside genuine multi-child joins).

### Rejected alternative: `SubPlanJoin`-based wrapping

Two reasons, both structural. (1) Post-normalize, `lift_inner_join` has already
hoisted variable bindings into one enclosing `Construction`, so the SPARQL
variable names `lower_as_subplan`'s positional remap machinery needs are gone
at the point of wrapping. (2) A real emission-order hazard: `render_from`'s
core-less path renders all `.opts` before any `.subplan_joins` regardless of
insertion order — a path-as-SubPlan followed by an ordinary OPTIONAL
correlating on the path's output would emit a `LEFT JOIN … ON` referencing an
alias introduced later in the same FROM clause (a crash). The
ordinary-`Scan`-in-`core` design renders core scans first, unconditionally —
the hazard is unreachable.

### Soundness: multiplicity

The CTE emits `SELECT DISTINCT sf_s, sf_o` for every kind except bare `!p` at
`PathKind::One` (`UNION ALL`). SPARQL path semantics for `+`/`*`/`?` is a
**set** over endpoint pairs (the ALP algorithm); DISTINCT is exactly that set
at the raw-key level (sound under the node-shape invariant `path.rs` already
enforces). SQL join and SPARQL Join agree on "preserve each operand's own
multiplicity, correlate by equality" — so the set case cannot multiply and the
bag `!p` case (length-1 path ⇒ ordinary triple-pattern bag semantics)
reproduces exactly the right multiplicity. **This argument must be verified
empirically, adversarially** — a duplicate-multiplicity `!p` join fixture
checked against the spareval oracle's own bag counts is a required test, not
an optional one.

### Coverage and remaining sound-501s

Covered: InnerJoin of any arity and mix (incl. two separate paths sharing a
variable); OPTIONAL with a path on either side; EXISTS/NOT EXISTS/MINUS bodies
containing a joined path (falls out of `lower_iq_exists` reusing `lower_node`;
`SqlCond::Exists` CROSS-JOINs scans generically — first `Query`-sourced use,
gets its own test). Stays 501: everything flat-side (by design); a sliced path
as a join operand (inherits the existing SubPlan slice guard — same reason);
GROUP BY over a *joined* path is expected to work via the ordinary single-
branch aggregation path but is **unverified — an open question the
implementation must answer or explicitly gate**.

## Risks (the adversarial-review checklist)

1. **Planner pushdown**: a recursive CTE inside a derived table may not receive
   outer-correlation pushdown (dialect/version-dependent) — a real perf-
   regression risk on large closures joined selectively. Bench before done.
2. **Catalog-blindness** of the embedded SQL (`ColumnCatalog::default()` at
   LOWER time) — a pre-existing, shipped limitation of all derived-table
   rendering, inherited not introduced; recorded, not a blocker.
3. **Raw-SQL validity spike**: two independent `WITH RECURSIVE` blocks, each
   inside its own nested derived table, on both SQLite and PostgreSQL —
   confirm by executing the literal generated SQL, not by assumption.

## Test contract

The pinned boundary test's tree-side assertion deliberately flips to "correct
rows, matching spareval" (flat side stays 501 — the pin's purpose inverts into
documenting the tree-exceeds-flat lift). New differential cells: plain
InnerJoin (PJ_* fixtures exist), OPTIONAL-right-path, OPTIONAL-left-path,
two-separate-paths-joined, EXISTS-with-joined-path-body, the `!p`
duplicate-multiplicity fixture, and the star-at-path-endpoint cell from
`ADR-0032` D6 (which this lift finally makes answerable — update that D6 note
when it lands). `differential_paths`/`differential_tree`/`differential_star`
suites hold throughout; RED-first; adversarial refute-review before commit.
