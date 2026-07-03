# ADR-0023 — M4 OPTION_B Work-List (architect synthesis)

**Status:** M4 planning input — derived from the four family analyses + the empirical probe
(`crates/sf-conformance/tests/option_b_probe.rs`, `option_b_empirical_probe`). **Updated 2026-07-02**
(ADR-0023 optimizer-residue Wave B/D bucket-cleanup, see [[adr-0023-optimizer-residue-horizon]]): Group C
shipped and Group D reclassified from a feared correctness gap to a confirmed-`=_bag`-safe SQL-shape
cosmetic — see the Family 3 table and Roll-up section for the corrected dispositions; original
2026-06-30 figures kept alongside for history, not silently overwritten.
**RE-CORRECTED 2026-07-03:** the 2026-07-02 Group D "confirmed" verdict was wrong for FDSimplification —
its probe fixture's inner FILTER was a no-op, so the MATCH it recorded was vacuous, not evidence. A real
bug existed (`leftjoin.rs`'s anti-join branch dropped the OPTIONAL's own inner FILTER — ADR-0007 silent
wrong answer), now fixed on the `leftjoin-antijoin-filter` branch. See the Family 3 table + Roll-up
section below for the corrected per-row disposition; do not trust the "Group D fully closed" framing
above without reading the correction.
**Authority:** `docs/adr/ADR-0023-query-ir-architecture-flat-ucq-vs-iq-tree.md`,
`docs/design/ADR-0023-design-lock.md` §4 (rule catalogue) / §5 (lowering),
`docs/HANDOVER-2026-06-30-ir-architecture-decision.md` (79 oracle-green / 16 classes / 184 scenarios = OPTION_B),
`docs/research/ontop.md` §5.
**Oracle law:** `=_bag` is ABSOLUTE. A verdict is trusted ONLY where the empirical probe
(`translate_tree` vs spareval) confirms it; conceptual `free-pass-likely` predictions are recorded as
*predicted* and upgraded to *confirmed* only where the probe exercised that shape.

## Verdict legend

- **free-pass** — already `=_bag`-correct under the locked architecture (n-ary Union + `Vec<Branch>`
  bag-concat lowering, eager InnerJoin/Filter flattening, substitution-lift normal form, single-scan-right
  `OptJoin`, symbolic conds to LOWER). No new code. Ontop's exact tree/SQL shape is an internal artifact, not a `=_bag` constraint.
- **needs-tree-rewrite** — a new §4 tree-level rewrite is required during `normalize` (the flat cascade
  cannot see the shape post-lowering). Sub-tagged **[cosmetic]** when the probe/analysis shows the tree is
  *already* `=_bag`-correct and the rewrite only buys Ontop SQL/node-signature parity, or **[oracle-gap]**
  when the tree currently mis-evaluates or errors.
- **needs-SubPlan-M5** — requires the derived-table / SubPlan facility deferred to M5 (§5.1).
- **charter-excluded** — outside ADR-0023 (general CQ-containment chase, FlattenNode/JSON/NRA).

## Empirical anchor (probe, verbatim verdicts)

| family | probed scenario | FLAT | TREE | SPAREVAL | empirical verdict |
|---|---|---|---|---|---|
| union-structural | flattenUnion (nested UNION) | Ok(6) | Ok(6) | Ok(6) | **free-pass confirmed** |
| union-structural | ValuesNode constant-fold (BIND-union) | Ok(2) | Ok(2) | Ok(2) | **free-win** (predicted needs-rewrite → already `=_bag`; cosmetic) |
| boolean-push | JoiningCondition (join + FILTER pushdown) | Ok(2) | Ok(2) | Ok(2) | **free-pass confirmed** |
| join-elim | self-leftjoin on PK (single-scan right) | Ok(3) | Ok(3) | Ok(3) | **free-pass confirmed** |
| join-elim | JoinTransfer (OPTIONAL over multi-atom InnerJoin right, single-projected) | Ok(3) | Ok(3) | Ok(3) | **free-win** (predicted needs-rewrite → already `=_bag`; cosmetic) |
| join-elim | LJReductionWithLJOnTheRight (right-nested OPTIONAL) | Err 501 | Err 501 | Ok(3) | **oracle-gap** (genuine TREE-ERR — the real backlog item) |
| projection-and-true | projection-shrink over UNION | Ok(4) | Ok(4) | Ok(4) | **free-pass confirmed** |
| projection-and-true | PullOutVariable (shared-var self join) | Ok(3) | Ok(3) | Ok(3) | **free-pass confirmed** |

Probe consequence applied below: scenarios the analysis flagged needs-tree-rewrite but explicitly
"non-load-bearing for `=_bag`" are classified **needs-tree-rewrite [cosmetic]**. Only shapes the probe
shows mis-evaluating/erroring are **[oracle-gap]**.

---

## Family 1 — union-structural

| family | Ontop class::method | sparql shape | verdict | §4 rule / tree node |
|---|---|---|---|---|
| union-structural | FlattenUnionOptimizer::flattenUnionTest1 | union-of-union (3-deep), root projects X | free-pass (confirmed) | §4 n-ary Union + §5 `Vec<Branch>` bag-concat + projection pushdown |
| union-structural | FlattenUnionOptimizer::flattenUnionTest2 | flat union, arms are unions-under-identity-Construction | free-pass | identity-Construction collapse + n-ary Union |
| union-structural | FlattenUnionOptimizer::flattenUnionTest3 | union arm = InnerJoin(data, union); Ontop asserts no-op | free-pass (divergent tree, `=_bag`) | §4.16 join-over-union distribution (bag-exact) |
| union-structural | FlattenUnionOptimizer::flattenUnionTest4 | innermost union under Construction inside InnerJoin | free-pass | identity collapse + n-ary Union (+ §4.16) |
| union-structural | FlattenUnionOptimizer::flattenUnionTest5 | identity-Construction over union2[Construction(InnerJoin), data] | free-pass | identity collapse + n-ary Union + projection pushdown |
| union-structural | ValuesNodeOptimization::test1normalizationSlice | Slice(0,1) over Values(3) | **DONE** (Wave C, commit `d313f26`) | `normalize_slice` (`iq/normalize.rs`): Slice-over-Values truncation, incl. through the identity-projection Construction wrapper |
| union-structural | ValuesNodeOptimization::test2normalizationSlice | Slice(1,1) over Values | **DONE** (Wave C, commit `d313f26`) | same `normalize_slice`, offset half |
| union-structural | ValuesNodeOptimization::test3normalizationDistinct | Distinct over Values(dups) | **DONE** (Wave C, commit `5611138`) | `normalize_distinct`/`dedup_rows` (`iq/normalize.rs`); guarded by `same_var_set` (a real =_bag bug an adversarial review caught pre-merge — see commit message) |
| union-structural | ValuesNodeOptimization::test4…SliceUnionValuesValues | Slice(0,4) over Union[Values,Values] | **DONE** (Wave C, commit `1e5dd60`) | covered FREE by composing test1/test2's `normalize_slice` + test14's `try_fold_constant_union` (no new production code, confirmed with its own named test + rule-sensitivity check) |
| union-structural | ValuesNodeOptimization::test5…SliceUnionValuesNonValues | Slice(0,2) over Union[Values,ext], Values covers limit | **DONE** (Wave C, commit `38a3f07`) | `try_slice_over_union` (`iq/normalize.rs`): drops the unreachable data arm entirely |
| union-structural | ValuesNodeOptimization::test5…SliceUnionValuesValuesNonValues | Slice(0,4) over Union[Values,Values,ext] | **DONE** (Wave C, commit `38a3f07`) | same rule, multiple known arms in sequence (adversarially verified up to 4 mixed arms) |
| union-structural | ValuesNodeOptimization::test6…SliceUnionValuesNonValues | residual limit pushed to single non-Values arm | **DONE** (Wave C, commit `38a3f07`) | same rule: `=_bag`-correct residual `Slice(offset,limit)` wrapping the surviving arms — an adversarial review caught the initial `offset` computation was wrong (hardcoded 0 instead of `offset.saturating_sub(cursor)`), fixed + re-reviewed clean |
| union-structural | ValuesNodeOptimization::test7…SliceUnionValuesNonValues | residual limit per non-Values arm, outer Slice kept | **DONE, `=_bag`-equivalent shape** (Wave C, commit `38a3f07`) | sf's IR has no per-arm Slice (a Slice is spine-only), so multiple surviving non-Values arms are bundled under ONE residual Slice over the whole reconstructed Union rather than Ontop's literal per-arm LIMIT distribution — same correctness/pruning outcome, a narrower SQL-shape match than Ontop's own internal representation |
| union-structural | ValuesNodeOptimization::test8…DistinctUnionValuesNonValues | Distinct over Union[distinct-Values,ext] no-op | **DONE** (Wave C, commit `3380d44`) | already-duplicate-free Values arm inside a mixed Union correctly declines under the EXISTING single-arm dispatch (no Union recursion needed) — confirmed with its own named test |
| union-structural | ValuesNodeOptimization::test9…DistinctUnionValuesNonValues | Distinct over Union[Values(dups),ext] | **DONE** (Wave C, commit `3380d44`) | `dedup_one_arm` (`iq/normalize.rs`): per-arm dedup, outer `Distinct` + data arm untouched (cross-arm dedup isn't provable) |
| union-structural | ValuesNodeOptimization::test10…LimitDistinctUnionValues | Slice·Distinct·Union, Values non-distinct | **documented boundary** (not attempted) | confirmed empirically `Slice{Distinct{Union{...}}}` — `normalize_slice`'s dispatch doesn't recognize `Distinct` as pass-through, so the Slice-side arm-drop (`try_slice_over_union`) never reaches through it; making it safe needs reasoning about CROSS-arm content overlap under DISTINCT (a later arm's contribution to a LIMIT window is no longer a fixed row count once earlier output can silently absorb/dedupe it) — qualitatively harder than any rule built this wave, not a same-pattern extension |
| union-structural | ValuesNodeOptimization::test11…LimitDistinctUnionValues | Slice·Distinct·Union, Values distinct | **documented boundary** (not attempted) | same boundary as test10 |
| union-structural | ValuesNodeOptimization::test12…LimitDistinctUnionDistinctTree | limit pushed onto both distinct arms | **documented boundary** (not attempted) | needs DATABASE SCHEMA-level uniqueness reasoning (UC + IS NOT NULL on the underlying table) — `normalize` runs purely on the IR tree with NO catalog/schema access at all (schema-aware passes live in the flat path's POST-lowering `cascade` module); this is an architectural gap, not an effort gap |
| union-structural | ValuesNodeOptimization::test13…LimitDistinctUnionNonDistinctTree | limit pushed onto the one distinct arm | **documented boundary** (not attempted) | same architectural gap as test12 |
| union-structural | ValuesNodeOptimization::test14…ConstructionUnionTrueTrue | Union[const-Construct/True ×2] → Values | **DONE** (Wave C, commit `d3139f5`) | `try_fold_constant_union` (`iq/normalize.rs`): folds an all-constant-BIND Union to one Values leaf; foundation for test15/17-26 below (not itself implemented — see note) |
| union-structural | ValuesNodeOptimization::test15…ConstructionUnionTrueTrueDataNode | const arms fold, data arm kept | **DONE** (Wave C, commit `9bb21b8`) | new `try_partial_fold_constant_union` (`iq/normalize.rs`), sharing a `const_rows_of` helper extracted from `try_fold_constant_union` (test14): folds 2+ constant arms into one `Values`, keeps data arm(s) as sibling `Union` arms; adversarial review found a pre-existing, `=_bag`-safe missed-optimization gap composing with test26's column reordering when a data arm widens the union's own variable scope (declines to fold rather than mishandling — not fixed, not blocking) |
| union-structural | ValuesNodeOptimization::test17…DBConstant | same-DB-type DBConstant arms fold | **DONE** (Wave C, commit `9052100`) | covered FREE, no new production code: Ontop needs a homogeneous-cell-type gate for its own SQL-VALUES-clause column-type constraint (the SAME reason test18 declines on heterogeneous types); semantic-fabric's `Values` IR node stores `Option<TermDef>` cells directly with no such constraint, so `try_fold_constant_union` already folds constant arms of ANY types unconditionally — confirmed empirically (int+string, IRI+lang-literal both fold with no gate) and end-to-end via `diff_p`; test17's homogeneous case is a strict subset of what already happens |
| union-structural | ValuesNodeOptimization::test18…RDFConstant (diff datatypes) | heterogeneous RDF constants → NO fold | free-pass (negative) | §4.15 precondition forbids fold → matches Ontop |
| union-structural | ValuesNodeOptimization::test19…RDFConstant (same type) | same-type RDF consts → Construction(RDFLiteral) over Union[Values,data] | **DONE for `=_bag`** (Wave C, test bundled into commit `84365ff`; SQL-shape hoist itself NOT implemented — see note below) | Ontop's own hoist (splitting a term into lexical value + datatype, lifting a shared wrapper above the whole union) collapses ITS SQL shape; semantic-fabric's `IqNode::Union` always explodes into independent branches and `Construction` folds its own substitution into each branch separately, REGARDLESS of what (if anything) wraps the union — confirmed via representative-scenario `diff_p` (not a line-for-line Ontop source port) AND an adversarial review's direct IR-level inspection, which corrected an overclaim in the first pass: the tree is NOT literally "unchanged" (the pre-existing test15/17 `try_partial_fold_constant_union` still fires on these same shapes), but the SPECIFIC binding-lift wrapper-hoist never fires and isn't needed for correctness |
| union-structural | ValuesNodeOptimization::test21…IRIConstant | mix foldable consts + IRI-template arm | **DONE for `=_bag`** (Wave C — same finding as test19, verified with its own representative scenario) | see test19's note; the SQL-shape hoist itself is not implemented |
| union-structural | ValuesNodeOptimization::test22…NonConstant | mix consts + IS-NOT-NULL-expr arm | **DONE for `=_bag`** (Wave C — same finding as test19, verified with its own representative scenario) | see test19's note; adversarial review flagged the author's `FILTER(BOUND(?n))` approximation as a WEAK reading of "IS-NOT-NULL-expr arm" (R2RML resolution already injects an equivalent `IsNotNull` on that column, making the FILTER redundant) — a stronger construction (`OPTIONAL{...} FILTER(BOUND(?n))`, doing real exclusionary work) was also verified correct |
| union-structural | ValuesNodeOptimization::test23…RDFConstant (2-var) | multi-column Values under lifted Construction | **DONE for `=_bag`** (Wave C — same finding as test19, verified with a 2-variable representative scenario) | see test19's note; the SQL-shape hoist itself is not implemented |
| union-structural | ValuesNodeOptimization::test24…RDFConstantSub | heterogeneous → split term, lift wrapper, no Values fold | **DONE for `=_bag`** (Wave C — same finding as test19, verified with a representative heterogeneous-type scenario, including a 4-way type mix in adversarial review) | see test19's note; even Ontop's own optimizer declines the Values-fold half here (heterogeneous types), and semantic-fabric's `try_fold_constant_union`/`try_partial_fold_constant_union` fold heterogeneous types regardless (test17's finding) — correctness never depended on the fold OR the hoist |
| union-structural | ValuesNodeOptimization::test25NoVariableTrueNodesAndValuesNodes | zero-var Union[True,True,Values] → counting Values | **DONE** (Wave C, commit `2d492f2`) | lifted the `project.is_empty()` early-decline in `try_fold_constant_union`; a new bare-`IqNode::True` arm (guarded to `project.is_empty()`) contributes one empty-tuple row -- adversarial review found it's not strictly load-bearing (a second fold opportunity via `lift_construction` rescues correctness either way) but kept for tree-shape consistency; one arm-shape combination (True mixed with a bare zero-column Values arm) is rescued by NEITHER fold pass and still executes correctly via ordinary branch cross-product -- a missed optimization, not a bug, not pursued further |
| union-structural | ValuesNodeOptimization::test26MergeableCombination… | mixed True/Construction/Values arms, diff col order → one Values | **DONE** (Wave C, commit `487b4fb`) | `same_var_set`/`reorder_row` in `try_fold_constant_union` — Values/Construction-True arms in ANY column order fold correctly; a genuinely bare zero-var `IqNode::True` arm mixed into a non-empty-project union is not separately exercised (unclear it's reachable at all with `project` non-empty — not attempted) |
| union-structural | ValuesNodeSimpleQueryOptimization::testTranslatedSQLQuery1 | end-to-end LIMIT 2, assert SQL has no union/limit | **DONE for `=_bag`** (Wave C — verified with a representative pure-data-union-under-LIMIT scenario, commit `a393953`) | `=_bag` count was already correct (pre-Wave-C note); the "assert SQL has no union/limit" half is a SQL-shape/signature assertion, not `=_bag`, and is NOT implemented — same distinction as `BindingLiftTest` below |
| union-structural | ValuesNodeComplexQueryOptimization::testTranslatedSQLQuery1 | end-to-end LIMIT 4 over wide mapping union | **DONE for `=_bag`** (Wave C — same finding, same representative scenario) | see the Simple row above; the SQL-shape assertion itself is not implemented |
| union-structural | BindingLiftTest::testUnionSubstitution | lift common URI-template binding into shared Construction above union | **charter-excluded** (Wave C, reclassified — team-lead directive) | §4.15 binding-lift is purely SQL-signature-shape (matching Ontop's exact collapsed SQL text), explicitly NOT `=_bag` from its own original note — this wave's stated charter is `=_bag` cosmetic rewrites only, so this row is out of scope for it entirely, not deferred or left undone within scope |

Family 1 totals (2026-07-03, Wave C COMPLETE for its own `=_bag` charter): **6 free-pass, 22 DONE
(test1/test2/test3/test4/test5/test5b/test6/test7/test8/test9/test14/test15/test17/test19/test21/
test22/test23/test24/test25/test26/testTranslatedSQLQuery1-Simple/testTranslatedSQLQuery1-Complex),
4 documented-boundary (test10/11/12/13 -- see Family 1 table for why each is a genuine architectural
gap, not an effort gap), 0 needs-tree-rewrite, 0 needs-SubPlan-M5, 1 charter-excluded
(`BindingLiftTest::testUnionSubstitution`, reclassified per team-lead directive — purely SQL-
signature-shape, explicitly not `=_bag`, out of THIS wave's stated scope entirely).** (Original:
6/27/0/0 — kept for history; the 22 DONE + 4 documented-boundary rows are no longer counted in the
27 needs-tree-rewrite.)

**Wave C progress note (2026-07-03, branch `fix/adr-0023-residue-waves`, worktree
`adr-0023-residue-waves`):** test1/test2 (Slice-over-Values truncation, commit `d313f26`),
test14 (constant-Union-to-Values fold, commit `d3139f5`), and test3 (Distinct-over-Values
dedup, commit `5611138`) shipped — each with unit + differential-tree tests (spareval-gated
where deterministic, `diff_p_bag` where an oracle's tie-breaking is legitimately
implementation-defined), revert-proofs, and adversarial refute-only review. **test3's first
adversarial review pass REFUTED the rule as first fixed**: `SELECT DISTINCT ?x WHERE { VALUES
(?x ?y) {(1 2)(1 3)(1 2)} }` wrongly returned 2 rows instead of 1 (deduped the Values leaf's
full pre-projection tuple through an identity-projection wrapper, before the wrapper's own
`project` narrowed away the unselected `?y` — SPARQL applies DISTINCT after Project). Fixed
with a `same_var_set` guard (decline whenever `project` narrows the Values leaf's own `vars`);
a second adversarial pass re-verified the fix across 7 further angles and found nothing. This
is the process working as intended, not a caveat to hide.

test14's fold is a real capability (not a stub) but did NOT at the time yet subsume test15/17-26
(partial fold with a DATA arm kept, homogeneous-DB-type gating, RDF-term binding-lift/split,
zero-var counting, column-order reconciliation) — those remained open, each verified to safely
DECLINE at the time (confirmed by the implementer's own tests and two adversarial reviewers)
rather than mis-fold. UPDATE: test15 (partial fold, sharing a `const_rows_of` helper extracted
from test14's own logic), test25 (zero-var counting), and test26 (column-order reconciliation)
are since DONE — see the Family 1 table for each. test4 (Slice over Union[Values,Values]) needed
NO new code — covered by composing
test1/test2 + test14's rules, confirmed with its own named/rule-sensitivity-checked test (commit
`1e5dd60`), no separate adversarial round (no new production logic). test5/test5b/test6/test7
(Slice-over-Union arm-drop + residual-limit, commit `38a3f07`) are a genuinely NEW, materially
more complex mechanism (arm-by-arm cursor tracking) — a real `=_bag` bug (residual offset
hardcoded to 0 instead of carrying forward the unconsumed skip) was caught by adversarial review
and fixed; see that commit's message. test7's coverage is `=_bag`-equivalent but not a literal
SQL-shape match (sf's IR has no per-arm Slice, so multiple surviving non-Values arms share ONE
residual Slice rather than Ontop's per-arm LIMIT distribution). test8/test9 (Distinct-over-Union
per-arm dedup, `dedup_one_arm`, commit `3380d44`) close the rest of Group A's Slice/Distinct-only
rows — test8 free (an already-distinct Values arm inside a mixed Union is a genuine no-op under
the existing single-arm dispatch), test9 needing the new per-arm-dedup helper (built with the
narrowing-projection guard from the start, having been bitten by that exact bug on the sibling
single-Values rule earlier this session). **test10-13 are DOCUMENTED BOUNDARIES, not attempted**:
test10/11 need the Slice-side arm-drop to see THROUGH an enclosing `Distinct` — confirmed
empirically the tree shape is `Slice{Distinct{Union{..}}}` and `normalize_slice` doesn't recognize
`Distinct` as pass-through — and doing so safely requires reasoning about CROSS-arm content
overlap under DISTINCT (a later arm's contribution to a LIMIT window stops being a fixed count
once earlier arms' output can silently absorb it), qualitatively harder than anything built this
wave, not a same-pattern extension. test12/13 need DATABASE SCHEMA-level uniqueness reasoning (UC
+ IS NOT NULL) that plain tree-level `normalize` has no access to at all (schema-aware passes live
in the flat path's POST-lowering `cascade` module) — an architectural gap, not an effort gap.
test26 (mixed-order arm-merge, `same_var_set`/`reorder_row` in `try_fold_constant_union`, commit
`487b4fb`) closes the column-order-reconciliation half of Group B's general arm-merge scope; test25
(zero-var counting, lifting `try_fold_constant_union`'s `project.is_empty()` guard, commit
`2d492f2`) and test15 (partial fold — 2+ constant arms combine even alongside a genuine DATA arm,
`try_partial_fold_constant_union` sharing a `const_rows_of` helper extracted from test14's own
logic, commit `9bb21b8`) close the remaining non-binding-lift items. test17 (commit `9052100`) is
ALSO covered FREE: Ontop needs a homogeneous-cell-type gate before folding constants into a SQL
VALUES clause (a real column-type constraint, the same reason test18 declines on heterogeneous
types); semantic-fabric's `Values` IR node has no such constraint (it stores `Option<TermDef>`
cells directly), so the fold already runs unconditionally on any mix of constant types — confirmed
empirically, no new code needed.

**test19/21/22/23/24 and the two end-to-end composition rows are ALSO DONE for `=_bag`** (not
implemented as a rewrite, but verified as not needing one): the RDF-term binding-lift proper
(splitting a term into lexical value + datatype, lifting a shared datatype/wrapper Construction
above the union) collapses ONTOP's OWN SQL shape, but `IqNode::Union` always explodes into
independent branches and `IqNode::Construction` folds its own substitution into each branch
separately — neither behavior is conditioned on whether a shared wrapper was hoisted, confirmed via
representative-scenario `diff_p`/`diff_p_bag` for each (same-type consts + data arm; constant IRIs +
an IRI-template arm; consts + a nullable-column-gated arm; a 2-variable generalization; heterogeneous
types up to a 4-way mix) PLUS the two end-to-end rows' own pure-data-union-under-LIMIT shape — an
adversarial review independently re-verified this and corrected an overclaim in the first pass (the
tree is NOT literally "unchanged": the pre-existing test15/17 fold still fires on these shapes; the
SPECIFIC binding-lift wrapper-hoist is what never fires and isn't needed). **Caveat stated plainly:**
these are representative constructions matching each test's one-line worklist description, not a
line-for-line port of Ontop's Java test source (unavailable in this environment); the underlying
reasoning is shape-invariant (not scenario-specific), which is why one architectural argument covers
all of them, but exact Ontop fixtures were not cross-checked. The SQL-shape/signature-parity goal
itself (collapsing to Ontop's exact SQL shape) is NOT implemented for any of these — same
"signature parity, not `=_bag`" category `BindingLiftTest::testUnionSubstitution` was already in from
the start (that one row was not independently re-verified and is left as originally classified).

**A genuine bug was found and fixed during this investigation, in ALREADY-COMMITTED Wave-C code**
(not a pre-existing, unrelated bug like the ones below): the broader adversarial review above,
while checking the binding-lift claim, incidentally caught `try_partial_fold_constant_union`
(test15/test17, commit `9bb21b8`) unconditionally PREPENDING its folded Values arm to position 0,
regardless of where the constant arms originally sat relative to a data arm — a real flat-vs-tree
`=_bag` divergence under a bare `LIMIT` (no `ORDER BY`): `SELECT ?n WHERE {{data}} UNION {{c1}}
UNION {{c2}} LIMIT 2` returned the data arm's rows on the flat side but the folded constants on the
tree side. Independently reproduced, then fixed (commit `84365ff`) by folding each maximal
CONTIGUOUS run of constant arms at its own starting position instead — constant arms separated by a
data arm no longer combine at all, a narrow loss of optimization scope in exchange for correctness.
Revert-proven against the exact old buggy code; a second adversarial review (including the
reviewer's own non-vacuity safeguard: confirming their own new probes genuinely fail against the
reinstated old bug before trusting a pass against the fix) found nothing further.

**Wave C is COMPLETE for its own `=_bag` charter: 0 genuinely open, 4 documented-boundary
(test10-13, genuine architectural gaps, not effort gaps), 1 charter-excluded
(`BindingLiftTest::testUnionSubstitution` — per team-lead directive, reclassified from
needs-tree-rewrite to charter-excluded: it was already known from the start to be a pure
SQL-signature-shape concern, not `=_bag`, and this wave's stated charter is `=_bag` cosmetic
rewrites only, not "match Ontop's exact SQL text" — so it is genuinely OUT OF SCOPE for this wave,
not deferred work still owed within it).**

**Correctness backlog (post-Wave-C, team-lead handoff) — COMPLETE.** Adversarial review
incidentally surfaced five pre-existing, Wave-C-unrelated bugs — none touched by any Wave C diff
itself (the original report named what turned out to be the SAME bug twice, as
"core-less-branch OptJoin" and separately as "zero-var-Union-as-LeftJoin-left" — item 3 below
covers both; item 5 was found DURING item 3's own adversarial review). Four are now fixed (each
with its own RED-first/revert-proof/adversarial-review gate); one was assessed and found to be NOT
a bug (no fix needed — a documented, `=_bag`-safe asymmetry, not a gap). Zero items remain
genuinely open from this original handoff — though item 5's own adversarial review surfaced two
FURTHER related-but-out-of-scope items, flagged (not fixed) at the end of item 5 below, for a
future follow-up round:

1. ~~`FILTER NOT EXISTS { ... }` with no variable shared with the outer scope silently returns the
   WRONG answer on the tree path~~ — **FIXED, commit `45b395c`** (priority-escalated ahead of Wave C
   by the team lead the moment it was flagged: a silent wrong answer beats cosmetics). Root cause:
   a build-time conflation, not just a lowering bug — `GraphPattern::Minus` and
   `Expression::Not(Expression::Exists(..))` both compiled to the identical `IqCond::NotExists` node
   (`build.rs`), and `lower_iq_exists` applied MINUS's SPARQL §8.3.2 disjoint-domain no-op exception
   to FILTER NOT EXISTS too (SPARQL §11.4.7 — a pure existence test with no such exception). Fixed
   with an `is_minus: bool` field on `IqCond::NotExists`, threaded through resolve/normalize's
   pass-through recursion, gating the skip on it instead of the shared `negated` flag. Adversarial
   review (9 angles, NOT REFUTED).
2. ~~A bare `OFFSET n` with no `LIMIT` fails at SQL emission~~ — **FIXED, commit `d29e550`**.
   Confirmed live against a real `sqlite3` CLI and a live MySQL server (both reject a bare
   `OFFSET`; PostgreSQL doesn't and needed no change). Fixed with a new
   `Dialect::bare_offset_limit_sentinel()` emitting an explicit "no limit" `LIMIT` before the
   `OFFSET` for exactly the 2 confirmed-broken dialects. Adversarial review (8 angles, live
   SQLite/MySQL/Postgres evidence, NOT REFUTED).
3. ~~A core-less `LeftJoin` LEFT operand (a bare `{}` or a `BIND(...)`-only Construction, no real
   scan) mis-aliases the OPTIONAL's own columns~~ — **FIXED, commit `b1148dc`**. `Branch.opts` was
   non-empty but `Branch.core` was empty, and two separate FROM-decision guards
   (`emit_branch_with`, `emit_agg_branch`) checked only `core`/`subplan_joins`, never `opts` — so no
   FROM clause was rendered at all, yet the SELECT list still projected the opt's own column,
   referencing an alias no FROM clause ever introduced (confirmed live: `no such column: t0.name`
   on every dialect — this is the SAME bug regardless of whether the core-less left side is a bare
   `{}`, a zero-var Union, or a `BIND(...)`-only Construction). Fixed with a synthetic `(SELECT 1)`
   single-row anchor (deliberately not promoting the opt to a hard anchor, which would break
   OPTIONAL's "guaranteed at least one row" semantics — concretely proven wrong via the independent
   spareval oracle during review) plus rendering every `opts` entry as its own `LEFT JOIN`.
   Adversarial review (8 angles, mostly via genuine revert-and-recheck, live PG/MySQL
   verification, NOT REFUTED).
4. ~~A flat-oracle limitation aggregating over a BIND-only union~~ — **ASSESSED, commit
   `533d839`: NOT a bug, no fix needed.** `COUNT`/`SUM` over a `UNION` whose every arm is a bare
   `BIND` makes flat's own aggregation-over-UNION mechanism introduce an internal synthetic
   variable it cannot itself bind, so flat honestly defers (`Unsupported("BIND references unbound
   ?<synthetic>")`) rather than risk a wrong answer — its own 501 discipline working as designed.
   TREE computes this correctly via `rust_group`, confirmed directly against the independent
   `spareval` oracle (bypassing flat, since the standard harness's "both sides must 501 together"
   check would otherwise misreport "tree succeeds where flat defers" as a mismatch even though
   tree's answer is genuinely correct) — `=_bag`-safe strengthening, not a regression. Documented
   with a permanent regression test asserting BOTH flat's 501 and tree's spareval-verified
   correctness, so a future flat-side capability change surfaces rather than silently invalidating
   the premise.
5. ~~A genuine, pre-existing Path bug found incidentally during bug 3's own adversarial review~~
   — **FIXED (crash → sound 501), commit `d849f46`**. `Branch` can only ever represent ONE of
   {a plain core/opts scan model, a path closure} at a time (`emit_branch_with` dispatches
   unconditionally to `emit_path_branch` whenever `path.is_some()`, which has zero awareness of
   `core`/`opts`), and `leftjoin.rs` has THREE separate composition functions that each build a
   merged `Branch` from a left+right pair, none of which accounted for a path on either side:
   `inner_join_one` (the `P ⋈ R` half) always carried only `left.path` forward, silently dropping
   `right.path`; `not_exists_cond_for` (the `P − R` half) built a `NotExists` with empty `scans` for
   a path branch while its `conds` still referenced the path's own CTE-only columns;
   `build_left_join` (the single-scan fast path, reachable regardless of the LEFT side's own shape)
   never touches `left.path` at all, so a path-shaped left ends up with `path:Some(_)` AND non-empty
   `opts` simultaneously. A naive "carry the other side's path across instead" fix would not be
   sound (it would just trade one dropped side for the other), so fixed with three new guards
   (matching this file's own pre-existing convention for the analogous `right.subplan_joins` 501
   boundary) rather than a real fix, which would need a new composition mechanism (e.g. wrapping
   the path's own rendering as an opaque SubPlan derived table) — explicitly out of scope for a
   crash-to-501 fix. Adversarial review (8 angles, NOT REFUTED) surfaced two further
   related-but-out-of-scope items, flagged for separate follow-up: (a) `MINUS` with a path-shaped
   LEFT operand diverges between flat (501s) and tree (computes correctly, hand-verified) — tree is
   MORE capable here, not wrong, matching item 4's own "tree surpasses an inherent flat limitation"
   pattern; (b) a cosmetic "EXISTS"-worded error message on a `MINUS`-triggered 501 in
   `lower_iq_exists`.

---

## Family 2 — boolean-push

| family | Ontop class::method | sparql shape | verdict | §4 rule / tree node |
|---|---|---|---|---|
| boolean-push | PushDownBoolean::testJoiningCondition1 | conjunctive join cond split to deepest InnerJoin | free-pass (confirmed) | §4.8 filter-pushdown/conjunct-split (eager flatten) |
| boolean-push | PushDownBoolean::testJoiningCondition2 | InnerJoin cond distributed into every Union branch | free-pass | §4.16 join-over-union + §4.8 |
| boolean-push | PushDownBoolean::testJoiningCondition3 | filter on right/nullable LJ vars → no-op | free-pass | §4.8 LeftJoin caveat |
| boolean-push | PushDownBoolean::testJoiningCondition4 | cond not pushed into nested LJ chain | free-pass | LeftJoin boundary preserved (`OptJoin`) |
| boolean-push | PushDownBoolean::testJoiningCondition5 | cond on right-only LJ var → no-op | free-pass | §4.8 LeftJoin caveat |
| boolean-push | PushDownBoolean::testLeftJoinCondition1 | preserved-only ON not pushed into left | free-pass | ON kept on `OptJoin.on` |
| boolean-push | PushDownBoolean::testLeftJoinCondition2 | right-only ON pushed into right operand | free-pass | §4.8 LeftJoin exception (`=_bag`-neutral) |
| boolean-push | PushDownBoolean::testLeftJoinAndFilterCondition1 | preserved-only filter moves across LJ | free-pass | §4.9(b1) preserved-side movement |
| boolean-push | PushDownBoolean::testLeftJoinAndFilterCondition2 | right-only filter above LJ → no-op | free-pass | §4.8 caveat (§4.10 downgrade is separate) |
| boolean-push | PushUpBoolean::testPropagationFomInnerJoinProvider | nested InnerJoin cond lifted to merged parent | free-pass | eager InnerJoin flatten (normalize default) |
| boolean-push | PushUpBoolean::testNoPropagationFomInnerJoinProvider | InnerJoin cond inside Union arm not lifted | free-pass | per-arm Branch cond |
| boolean-push | PushUpBoolean::testPropagationFomFilterNodeProvider | Filter under InnerJoin merged into cond | free-pass | §4.8 Filter-under-InnerJoin fold |
| boolean-push | PushUpBoolean::testNoPropagationFomFilterNodeProvider | Filter in Union arm not lifted | free-pass | per-arm Branch cond |
| boolean-push | PushUpBoolean::testNoPropagationFomLeftJoinProvider | LJ ON not lifted | free-pass | ON stays on `OptJoin.on` |
| boolean-push | PushUpBoolean::testPropagationToExistingFilterRecipient | ancestor Filter absorbed into InnerJoin cond | free-pass | §4.8 fold |
| boolean-push | PushUpBoolean::testRecursivePropagation | recursive nested cond+filter to one InnerJoin | free-pass | §3 substitution-lift + recursive flatten |
| boolean-push | PushUpBoolean::testPropagationToLeftJoinRecipient | right InnerJoin cond → LJ ON | free-pass | `L ⟕ σ_θ(R)=L ⟕_θ R` (right-only θ) |
| boolean-push | PushUpBoolean::testPropagationThroughLeftJoin | preserved-side cond lifted above LJ | free-pass | §4.9(b1) preserved movement |
| boolean-push | PushUpBoolean::testCompletePropagationThroughUnion (@Ignore) | common predicate lifted from all arms | free-pass | §4.9(a) cosmetic; Ontop ignores too |
| boolean-push | PushUpBoolean::testNoPropagationThroughUnion | distinct per-arm predicates → no-op | free-pass | per-arm Branch cond |
| boolean-push | PushUpBoolean::testPartialPropagationThroughUnion (@Ignore) | common conjunct lifted, residuals kept | free-pass | common-factor lift cosmetic; Ontop ignores |
| boolean-push | PushUpBoolean::testMultiplePropagationsThroughUnion (@Ignore) | multi-level common-predicate lift + IRI Construction | free-pass | §3 subst-lift; cosmetic |
| boolean-push | RegexCaseOptimization::testLCase | REGEX(LCASE(x),p,'i') fold away LCASE | free-pass | SqlCond expr-simplification, non-§4, `=_bag` identical |
| boolean-push | RegexCaseOptimization::testUCase | symmetric UCASE elision | free-pass | expr-simplification |
| boolean-push | RegexCaseOptimization::testMixedCases | fold case on input + constant pattern | free-pass | expr-simplification |
| boolean-push | RegexCaseOptimization::testNonCaseSensitiveRegex | no 'i' flag → LOWER preserved | free-pass (negative) | no fold; default-correct |
| boolean-push | RegexCaseOptimization::testMultipleFlags | fold fires with 'is' flags | free-pass | expr-simplification |

Family 2 totals: **27 free-pass, 0 needs-tree-rewrite, 0 M5, 0 charter.** (Whole family unlocked by the architecture.)

---

## Family 3 — join-elim

| family | Ontop class::method | sparql shape | verdict | §4 rule / tree node |
|---|---|---|---|---|
| join-elim | LeftJoinOpt::self-leftjoin-elim on PK (testSelfJoinElimination1/2/3, …WithCondition) | OPTIONAL of same table on PK | free-pass (confirmed) | §4.4 self-LJ-elim (cascade `joinelim.rs`) |
| join-elim | LeftJoinOpt::self-LJ contradiction/non-unification (testSelfLeftJoinNonUnification*, …IfElseNull1/2) | conflicting PK const → right NULL / Empty | free-pass | §4.4 lj-contradiction + §4.13 empty-right propagation |
| join-elim | LeftJoinOpt::self/left-join NON-elim (testNoSelfLeftJoin1/2/3, testLeftJoinNonElimination1, …Elimination3) | join not on key / nullable FK → no-op | free-pass | §4.3/§4.4 precondition fail (sound no-op) |
| join-elim | LeftJoinOpt::FK LJ→InnerJoin downgrade (testLeftJoinElimination1/2/4, …WithFilterCondition2) | NOT-NULL FK=PK ON → InnerJoin | free-pass | §4.10 lj→ij downgrade disjunct-2 (FK) |
| join-elim | LeftJoinOpt::LJ-elim w/ surviving cond (testLeftJoinElimination5, …WithFilterCondition4, …ImplicitFilterCondition, testSelfLeftJoinWithJoinOnLeft1/2) | residual ON → IfElseNull binding | free-pass | §4.4/§4.10 + §4.13 |
| join-elim | LeftJoinOpt::nullable-unique self-LJ guard (testSelfJoinNullableUniqueConstraint) | self-LJ on nullable UC | free-pass | §4.4 nullable-determinant (synth IS NOT NULL) |
| join-elim | LeftJoinOpt::DISTINCT prune unused right (testDistinctPruneUnusedRight1/2/3/5/6/7, testDistinctNoPrune*, testNoDistinctNoPrune1) | DISTINCT drops dead-var OPTIONAL right | free-pass | §4.5 distinct-prune (cascade pass 2d) |
| join-elim | LeftJoinOpt::DISTINCT prune over UNION-of-LJs (testDistinctPruneUnusedRight4) | per-arm dead-right prune under DISTINCT | free-pass | §4.16 (left-operand distribute) + §4.5 |
| join-elim | LeftJoinOpt::ProjectionAway no-DISTINCT key-guaranteed (testProjectionAway2/3/5, testPartialProjectionAway1/2, testNonProjectionAway*) | key ⇒ ≤1 match, right unprojected → drop | free-pass | §4.5 + §4.7 uniqueness oracle |
| join-elim | LeftJoinOpt::ProjectionAway stacked LJs (testProjectionAway1, testNonRequirement1/2, testRequirement1/2) | left-deep OPT chain, drop unused rights | free-pass | left-deep → `Branch.core` + `Vec<OptJoin>` |
| join-elim | LeftJoinOpt::MergeLJs same-table left-deep (testMergeLJs1/3/…, testNonMergeLJs1/2/3) | ((L OPT R1) OPT R2), R1=R2 same table | free-pass | within-CQ self-LJ-elim between opts (§4.4 ext) |
| join-elim | LeftJoinOpt::MergeLJs right-nested/multi-scan (testMergeLJs2[ignored], higher MergeLJs) | OPT whose right is OPT/multi-atom needing re-assoc | free-pass — **CLOSED (Group C, commit `45ae36c`)** | §4 LeftJoin re-association (Group C, `left_join_decomposed`); same target shape as LJReductionWithLJOnTheRight below — not separately probed under this exact Ontop test name, but structurally identical per this table's own rule mapping |
| join-elim | NRAJoin/LeftJoinOpt::JoinTransfer PK/FK (testJoinTransfer1-14) | L OPT {R1.R2}, share key with inner atom → transfer | free-pass — **confirmed** (`option_b_probe.rs` MATCH) | Group C decomposition (NOT Ontop's FD-transfer shortcut) already `=_bag`-correct; SQL-shape-only cosmetic gap remains (extra `NOT EXISTS` scan vs Ontop's collapsed join) |
| join-elim | LeftJoinOpt::JoinTransfer FD/NullableDet (testJoinTransferFD1-7, testFDOnNullableDeterminant1-10, testNonJoinTransferFD1-4) | FD-determinant-driven transfer, IS-NOT-NULL synth | free-pass — **reclassified 2026-07-02** (adversarial refute-only review, nullable-determinant angle, zero mismatches found) | Group C decomposition is sound independent of FD/key structure (see [[adr-0023-optimizer-residue-horizon]]); SQL-shape-only cosmetic gap remains; exact Ontop test methods not individually probed, only the representative shape |
| join-elim | LeftJoinOpt::JoinTransfer/SameTerms single-scan (testLJSameTerms1, testJoinTransferSameTerms1/2, testNonJoinTransferSameTerms1) | DISTINCT over L OPT R, R single same-table scan | free-pass | §4.4 sameterm/FD branch + §4.5 (cascade pass 2c) |
| join-elim | LeftJoinOpt::LJReductionWithLJOnTheRight (…1/2/3/5-12, testNon…1/2) | L OPT (R1 OPT R2) — right is itself OPTIONAL | free-pass — **CLOSED (Group C, commit `45ae36c`)** | §4 LJ re-association/reduction (Group C) — **probe: was Err 501 on flat, now `Ok(3)` MATCH on tree** |
| join-elim | LeftJoinOpt::FDOnRight (testFDOnRight1-7) | DISTINCT over L OPT (R1 OPT R2) on shared FD-det | free-pass — **confirmed 2026-07-02** (`option_b_probe.rs` MATCH, commit `9967884`) | Group C re-assoc already `=_bag`-correct; Ontop's FD-collapse is a cheaper-SQL alternative, not a correctness prerequisite |
| join-elim | LeftJoinOpt::FDSimplification (testFDSimplification) | nested OPT chain + per-right DISTINCT/FILTER + FD + ancestor FILTER | **needs-tree-rewrite — CORRECTED 2026-07-02** (the 2026-07-02 "confirmed MATCH" was a NO-OP-filter probe artifact, not evidence; see [[adr-0023-optimizer-residue-horizon]]) | `not_exists_cond_for` (`leftjoin.rs`) omitted the OPTIONAL's own inner FILTER from its NOT-EXISTS condition — a left row whose only right candidate is filtered out vanished instead of NULL-padding (ADR-0007 violation). Fixed on the `leftjoin-antijoin-filter` branch; re-verify MATCH here once merged |
| join-elim | LeftJoinOpt::PaddingForUnsatisfiableRight single-Construction (testPaddingForUnsatisfiableRight1) | outer FILTER makes single-scan right unsat → NULL-pad | free-pass | §4.2 unsat-equality-prune + §4.13 |
| join-elim | LeftJoinOpt::PaddingForUnsatisfiableRight UNION-right (testPaddingForUnsatisfiableRight2/3) | right is a UNION, all arms unsat | free-pass — **confirmed 2026-07-02** (`option_b_probe.rs` MATCH, commit `54fd5e9`) | Group C's decomposition re-feeds the opts-free union into `left_join_branches`'s multi-branch NOT-EXISTS arm, which correctly NULL-pads rather than distributing the unsat-prune into the union |
| join-elim | LeftJoinOpt::LeftJoinUnionConstants/LeftJoinValues (testLeftJoinUnionConstants, testLeftJoinValues) | (L ⋈ Union{const}/Values) OPT R | free-pass | §4.15 fold + §4.16 + §4.4; Values is first-class leaf |
| join-elim | LeftJoinOpt::LeftJoinJoinLimit (testLeftJoinJoinLimit) | (L ⋈ SUBSELECT{…LIMIT 1}) OPT R | free-pass — **CLOSED (M5 Wave 2, `left_join_as_subplan`, `iq/lower.rs:620-632`)** | §5.1 SubPlan derived-table LEFT JOIN. Residual narrower boundary (still 501, not this row's scenario): a MULTI-branch right-side SubPlan, or a right branch that ITSELF still carries `opts` after decomposition (nested-OPTIONAL-inside-a-LIMIT-subselect) |
| join-elim | LeftJoinOpt::SelfLeftJoinWithProvenanceBlockedByDistinct (…1/3-10, SameVarsDistinct1, *NoOpt1/2) | DISTINCT over L OPT {BIND(prov).R} | free-pass | §4.4 + IfElseNull(IsNotNull) witness + §4.5/§4.15 distinct-bounded |
| join-elim | LeftJoinOpt::ImplicitVariableNonRemoval (testImplicitVariableNonRemoval) | OPT right var shared with core atom → no-op | free-pass | §4.5 global-deadness fails (sound no-op) |
| join-elim | MappingCQCOptimizer::test (FK redundant-join) | drop FK parent scan | free-pass | §4.6 fk-pk-join-elimination |
| join-elim | MappingCQCOptimizer::test_foreign_keys / ::test_optimisation_order | general containment chase (LIDs + homomorphism) | charter-excluded | semantic chase, not §4 syntactic (ADR-0023 keeps only §4.6) |
| join-elim | NRAJoinOptimizer (entire class, e.g. testFlattenLift1) | FlattenNode/NestedView/array-unnest lift | charter-excluded | FlattenNode/JSON out of charter; class disabled in Ontop |

Family 3 totals (**RE-CORRECTED 2026-07-03**, see [[adr-0023-optimizer-residue-horizon]]): **24
free-pass, 1 needs-tree-rewrite (FDSimplification), 0 M5 (closed), 2 charter** (27 rows total). The
2026-07-02 update below moved all 7 originally-needs-tree-rewrite rows to free-pass; ONE of those
(FDSimplification) has since been moved BACK — its "confirmed MATCH" rested on a probe fixture whose
inner FILTER was a no-op (never excluded a candidate), so it never actually exercised the anti-join
branch's own filter and the MATCH verdict was vacuous, not a real confirmation. A genuine bug existed:
`not_exists_cond_for` (`crates/sf-sparql/src/leftjoin.rs`) omitted the OPTIONAL's inner FILTER from its
`NOT EXISTS` condition, so a right row that exists-but-fails-the-filter still counted as "a match
exists" — a left row whose only candidate is filtered out vanished from BOTH branches instead of
NULL-padding (a silent wrong answer, ADR-0007). Fixed + adversarially re-reviewed on the
`leftjoin-antijoin-filter` branch (not yet merged here); the probe scenario's filter has been corrected
to a match-removing one (`docs` + `option_b_probe.rs` both updated) — re-run once the fix merges and
this should flip back to free-pass with REAL evidence. The other 6 rows this table originally marked
needs-tree-rewrite (Group C: MergeLJs-right-nested, LJReductionWithLJOnTheRight, PaddingUnsat-UNION-right;
Group D: JoinTransfer PK/FK, JoinTransfer FD, FDOnRight) remain empirically confirmed `=_bag`-correct —
none of their probe scenarios exercise a match-removing FILTER inside a multi-scan OPTIONAL right (the
specific shape the bug needed), so they are NOT suspected to share FDSimplification's gap, but this has
not been independently re-verified per-scenario beyond that structural check (see the caveat in the
Roll-up section below). Group C's general `(L⋈R)∪(L¬∃R)` decomposition
(`left_join_decomposed`) remains sound independent of FD/key structure — Ontop's FD-driven Group D rules
are a cheaper-SQL *alternative* strategy, not a prerequisite, for the 6 rows this still holds for. The
LeftJoinJoinLimit/M5 row is also closed (M5 Wave 2, `left_join_as_subplan`) and unaffected (its scenario
has no inner FILTER either). What remains for the 6 still-confirmed rows is SQL-SHAPE-ONLY: the tree
emits an extra `NOT EXISTS` correlated-subquery scan where Ontop emits one collapsed join — a real,
in-charter, but cosmetic backlog item (folds into the Family 1 cosmetic set below).

---

## Family 4 — projection-and-true

| family | Ontop class::method | sparql shape | verdict | §4 rule / tree node |
|---|---|---|---|---|
| projection-and-true | ProjectionShrinking::testUnion | projection-shrink over Union | free-pass (confirmed) | §4.12 projection driver; lift_construction into arms |
| projection-and-true | ProjectionShrinking::testUnionAndImplicitJoinCondition1 | shared join-key retained (no-shrink) | free-pass | §4.12 + ColEq retention |
| projection-and-true | ProjectionShrinking::testUnionAndImplicitJoinCondition2 | shrink to join key, drop dead Y | free-pass | §4.16 + projection push |
| projection-and-true | ProjectionShrinking::testUnionAndExplicitJoinCondition1 | cond-referenced vars retained (no-shrink) | free-pass | symbolic cond at LOWER |
| projection-and-true | ProjectionShrinking::testUnionAndExplicitJoinCondition2 | partial shrink, cond var kept | free-pass | §4.12 + symbolic-cond invariant |
| projection-and-true | ProjectionShrinking::testUnionAndFilter | filter-over-union shrink, drop dead X | free-pass | §4.12 |
| projection-and-true | ProjectionShrinking::testConstructionNode | Construction∘Construction fold + shrink | free-pass | Construction-fold (design-lock l.163) |
| projection-and-true | ProjectionShrinking::testConstructionNodeAndImplicitJoinCondition2 | subst-lift through InnerJoin → synthesized equality | free-pass | lift_construction + merge_into (synth ColEq) |
| projection-and-true | TrueNodesRemoval::testSingleTrueNodeRemoval_innerJoinParent1 | InnerJoin(True,D1)→D1 | free-pass | §4.13 (implemented) |
| projection-and-true | TrueNodesRemoval::…innerJoinParent2 | InnerJoin[A≠B](True,D3)→Filter[A≠B](D3) | free-pass | §4.13 residual-cond preserve |
| projection-and-true | TrueNodesRemoval::…innerJoinParent3 | InnerJoin(True,D1,D2)→InnerJoin(D1,D2) | free-pass | §4.13 |
| projection-and-true | TrueNodesRemoval::…leftJoinParent | LeftJoin(D1,True)→D1 | free-pass (`=_bag`; shape via optional 1-line §4.13 ext) | True→`Branch::empty()`; LJ vs empty-match = D1 |
| projection-and-true | TrueNodesRemoval::testSingleTrueNodeChainRemoval | no-True no-op | free-pass | identity |
| projection-and-true | TrueNodesRemoval::…NonRemoval_leftJoinParent | LeftJoin(True,D1) preserved | free-pass (correct non-removal) | §4.13 reduces only Empty-left |
| projection-and-true | TrueNodesRemoval::…NonRemoval_UnionParent | Union(D,True) keeps True arm | free-pass | normalize_union prunes only Empty |
| projection-and-true | TrueNodesRemoval::testMultipleTrueNodesRemoval1 | nested InnerJoin all-True→D1 | free-pass | bottom-up §4.13 |
| projection-and-true | TrueNodesRemoval::testMultipleTrueNodesRemoval2 | InnerJoin(True, LeftJoin(D1,True))→D1 | free-pass (`=_bag`) | §4.13 + True-right empty-match |
| projection-and-true | TrueNodesRemoval::testTrueNodesPartialRemoval1 | keep inner LeftJoin(True,D1) | free-pass | §4.13 unwrap + preserve True-left |
| projection-and-true | TrueNodesRemoval::testTrueNodesPartialRemoval2 | keep inner Union(True,D) | free-pass | §4.13 + Union keep True |
| projection-and-true | PullOutVariable::testDataNode | within-atom repeated var → self-equality | free-pass | §5 LOWER self-ColEq |
| projection-and-true | PullOutVariable::testJoiningConditionTest1 | InnerJoin shared var → explicit eq | free-pass | §5 ColEq in where_conds |
| projection-and-true | PullOutVariable::testJoiningConditionTest2 | LeftJoin shared vars → ON conjunction | free-pass | §5 NullSafeEq ON |
| projection-and-true | PullOutVariable::testJoin3 | n-ary InnerJoin many shared vars | free-pass | §5 per-var ColEq |
| projection-and-true | PullOutVariable::testJoin4 | n-ary InnerJoin single shared var | free-pass | §5 ColEq |
| projection-and-true | PullOutVariable::testJoiningConditionTest3 | intra-atom repeats + cross-shared (LJ) | free-pass | §5 R1/R5 lowering split |
| projection-and-true | PullOutVariable::testJoiningConditionTest4 | InnerJoin-over-LeftJoin shared vars | free-pass | §5 ColEq + NullSafeEq ON |
| projection-and-true | PullOutVariable::testJoiningConditionTest5 | InnerJoin two shared vars | free-pass | §5 ColEq conjuncts |
| projection-and-true | PullOutVariable::testLJUnnecessaryConstructionNode1 | redundant pure-projection Construction on LJ right | free-pass | lift_construction + NullSafeEq ON |
| projection-and-true | PullOutVariable::testDistinctProjection | DISTINCT over InnerJoin shared var | free-pass | §5 ColEq + Distinct slot preserved |
| projection-and-true | PullOutVariable::testUnionDistinctProjection | per-arm pull-out under Union | free-pass | §4.16 + §5 |
| projection-and-true | PullOutVariable::testFlattenOutputVariable (@Ignore) | flatten output-var pull-out | charter-excluded | FlattenNode/JSON; @Ignore in Ontop |
| projection-and-true | PullOutVariable::testFlattenOutputVariable2 (@Ignore) | flatten output-var w/ index-bound | charter-excluded | FlattenNode/JSON |
| projection-and-true | PullOutVariable::testFlattenIndexVariable (@Ignore) | flatten index-var pull-out | charter-excluded | FlattenNode/JSON |
| projection-and-true | PullOutVariable::testFlattenIndexAndOutputVariable (@Ignore) | both index+output pull-out | charter-excluded | FlattenNode/JSON |

Family 4 totals: **30 free-pass, 0 needs-tree-rewrite, 0 M5, 4 charter.**

---

## Roll-up

**Updated 2026-07-02** (see [[adr-0023-optimizer-residue-horizon]] for evidence/commits) — Group C
shipped (M4 Wave 3, commit `45ae36c`) and its whole needs-tree-rewrite join-elim bucket (Group C's own 3
rows + Group D's 4 rows, 7 total) is now empirically confirmed `=_bag`-correct, so it moves from
needs-tree-rewrite to free-pass below. The table below reflects that; the ORIGINAL (2026-06-30, M4
planning) counts are kept alongside for history.

**RE-CORRECTED 2026-07-03**: FDSimplification's free-pass move rested on a vacuous (no-op-filter) probe
and has been reverted to needs-tree-rewrite pending the `leftjoin-antijoin-filter` fix merging here (see
the correction note above the Family 3 table). The counts below are updated accordingly; the 2026-07-02
88/27/0/6 figures are superseded by this row, not deleted (see git history for that intermediate state).

| disposition | union-structural | boolean-push | join-elim | projection-and-true | **total** | *(orig. join-elim / total)* |
|---|---|---|---|---|---|---|
| free-pass | 6 | 27 | 24 | 30 | **87** | *(17 / 80)* |
| **DONE (Wave C, implemented or verified free)** | 22 | 0 | 0 | 0 | **22** | *(new bucket, not in the original 121; most rows implement new logic, several -- test17/19/21/22/23/24 and the two end-to-end rows -- are verified as needing no rewrite instead, see Family 1 table for the precise per-row rationale)* |
| **documented-boundary (Wave C)** | 4 | 0 | 0 | 0 | **4** | *(new bucket, not in the original 121 — architectural gaps, see Family 1 table)* |
| needs-tree-rewrite | 0 | 0 | 1 | 0 | **1** | *(7 / 34)* |
| needs-SubPlan-M5 | 0 | 0 | 0 | 0 | **0** | *(1 / 1, now closed)* |
| charter-excluded | 1 | 0 | 2 | 4 | **7** | *(2 / 6, +1 reclassified from needs-tree-rewrite within union-structural — `BindingLiftTest::testUnionSubstitution`, team-lead directive)* |
| **enumerated rows** | 33 | 27 | 27 | 34 | **121** | *(same 121 rows as the original table — 7 needs-tree-rewrite + 1 M5 reclassified to free-pass within join-elim, minus FDSimplification's 2026-07-03 revert, none added/removed; the 22 Wave C DONE + 4 documented-boundary + 1 charter-excluded rows are a subset of union-structural's 27, not additional rows)* |

**Wave C update (2026-07-03):** 9 of union-structural's 24 needs-tree-rewrite rows
(test1/test2/test3/test4/test5/test5b/test6/test7/test14) are now DONE, not merely
cosmetic-and-pending — see the Family 1 table above and the Wave C progress note there.
`union-structural`'s needs-tree-rewrite count in the table above (18) already reflects this; the
roll-up's own historical "27" column headers elsewhere in this doc predate Wave C and are not
restated as errors, just superseded by this row.

Row-count note: these are the **representative analysis rows**; several join-elim rows fold multiple
Ontop `@Test` methods (e.g. `testJoinTransfer1-14`) into one shape. They cover the full 16-class / 184-test
OPTION_B surface from the handover — the 184 figure is the Ontop method count, ~120 the distinct shapes.

**`=_bag` reality (RE-CORRECTED 2026-07-03, see [[adr-0023-optimizer-residue-horizon]]):** of the original
34 needs-tree-rewrite rows, **all 27 union-structural (Family 1) are [cosmetic]** (the tree is already
`=_bag`-correct; the rewrite buys only Ontop SQL/node-signature parity). Of the 7 join-elim (Group C +
Group D) rows, **6 are confirmed cosmetic** (Group C: MergeLJs-right-nested, LJReductionWithLJOnTheRight,
PaddingUnsat-UNION-right; Group D: JoinTransfer PK/FK, JoinTransfer FD, FDOnRight — `option_b_probe.rs`
commits `9967884`, `54fd5e9`), but **FDSimplification is a REAL, now-fixed `=_bag` gap, not cosmetic**:
its 2026-07-02 "confirmed MATCH" probe used a no-op inner FILTER (never excluded a candidate against
fixture P), so the verdict was vacuous — it never exercised `not_exists_cond_for`'s own filter
application. The actual bug: a left row whose only right candidate is filtered out vanished from BOTH
the match branch (excluded by the filter) and the no-match branch (`NOT EXISTS` wrongly false, since the
unfiltered join still exists) instead of being NULL-padded — a silent wrong answer (ADR-0007). Fixed +
revert-proven + adversarially re-reviewed on the `leftjoin-antijoin-filter` branch (not yet merged here);
the probe scenario now uses a match-removing filter and correctly shows `Mismatch` until that merge.
The 2026-07-02 "dedicated adversarial refute-only review (9 fixtures... zero mismatches found)" that
this row's prior confirmation also leaned on did NOT include a match-removing-filter-on-multi-scan-
OPTIONAL-right angle among its 4 (nullable FD determinant / DISTINCT-anti-join / cyclic self-join /
multiplicity) — that blind spot is why it missed this bug; the other 6 rows' scenarios were individually
re-checked (see the Family 3 table note above) and none of them exercise an inner FILTER on a multi-scan
OPTIONAL right, so they are not currently suspected to share this gap, but that is a structural argument,
not a re-run adversarial pass — treat the 6 as probe-confirmed-good, NOT as freshly adversarially
re-cleared. Also unchanged caveat: SQLite-only, live-PG/MySQL dialect-specific 3VL quirks on these shapes
not separately re-verified. **One remaining genuine `=_bag` oracle-gap in this table (FDSimplification,
fixed elsewhere, pending merge here) — not zero.** What's left for the other 33 is SQL-shape/signature
parity only — 27 union-structural rewrites + the join-elim group's extra `NOT EXISTS` scan vs Ontop's
collapsed join.

## Proposed implementation waves (needs-tree-rewrite, dependency order)

- ~~**Wave 3 — Group C: LeftJoin multi-node-right re-association**~~ **SHIPPED** (M4 Wave 3, commit
  `45ae36c`). Unblocked LJReductionWithLJOnTheRight, MergeLJs-right-nested, PaddingUnsat-UNION-right — all
  confirmed `=_bag`-correct.
- ~~**Wave 4 — Group D: atom/FD transfer at the LeftJoin-over-InnerJoin boundary**~~ **PARTIALLY
  RECLASSIFIED 2026-07-02, CORRECTED 2026-07-03**: 3 of 4 Group D rows (JoinTransfer PK/FK, JoinTransfer
  FD, FDOnRight) are not needed for correctness (Group C already closes them independent of FD/key
  reasoning — see Roll-up above); for those, what remains is a SQL-shape/performance optimization
  (collapse the `Union`+`NOT EXISTS` decomposition back into a cheaper single-scan join when a key/FD
  condition proves it's safe) — folded into the cosmetic backlog below. The 4th (FDSimplification) is
  NOT cosmetic: it was a real `=_bag` gap (the anti-join-FILTER bug, see Roll-up above), now fixed on the
  `leftjoin-antijoin-filter` branch — pending that merge, do not fold FDSimplification's residual (if any,
  beyond the correctness fix itself) into the Wave 7 cosmetic-only scope below without re-probing it first.
- **Wave 5 — Group B: UnionAndBindingLift + Values constant-fold** *(independent; signature parity only)*.
  §4.15 fold-constants-into-Values, RDF-term split/lift, multi-column Values. Unblocks: test14-26,
  test19/23/24, BindingLiftTest::testUnionSubstitution, end-to-end SQL-shape tests. Cosmetic for `=_bag`.
  STATUS (Wave C COMPLETE, 2026-07-03): test14/15/17/19/21/22/23/24/25/26 and both end-to-end rows
  are DONE for `=_bag` (test17/19/21-24 and the end-to-end rows covered free — no rewrite needed,
  verified via representative-scenario differential testing, see Family 1 table);
  `BindingLiftTest::testUnionSubstitution` reclassified charter-excluded per team-lead directive
  (signature-parity-only, not `=_bag` — out of scope for THIS wave entirely, not deferred). The
  SQL-shape/signature-parity goal itself (Ontop's exact collapsed SQL, byte-for-byte) is NOT
  implemented for any of these — genuinely out of this wave's `=_bag`-only charter, a separate
  undertaking if ever wanted.
  **STARTED 2026-07-03** (branch `fix/adr-0023-residue-waves`, "Wave C" in that session's own commit
  naming): test14's core mechanism (`try_fold_constant_union`, commit `d3139f5`) — an all-constant-BIND
  Union folds to one Values leaf. Does NOT yet subsume test15/17-26/BindingLiftTest (partial fold with a
  kept DATA arm, homogeneous-type gating, RDF-term binding-lift/split, zero-var counting, column-order
  reconciliation) — each confirmed to safely DECLINE today rather than mis-fold.
- **Wave 6 — Group A: Slice/Values/Distinct folding drivers** *(pure SQL-shape, all cosmetic)*.
  New Slice-over-Values/Union truncation + arm-drop + arm-distinctness analysis. Unblocks: test1-13,
  Simple/Complex SQL-shape tests. **STARTED 2026-07-03** (same branch/session): test1/test2's slice
  (`normalize_slice`, commit `d313f26`) and test3's dedup (`normalize_distinct`/`dedup_rows`, commit
  `5611138`, `same_var_set`-guarded after an adversarial review caught a narrowing-projection `=_bag`
  bug) — Slice/Distinct-over-Values truncation/dedup, plus test4 (Slice over Union[Values,Values],
  commit `1e5dd60`) confirmed covered for free by composing with Wave 5's fold, plus
  test5/test5b/test6/test7 (Slice-over-Union arm-drop + residual limit, `try_slice_over_union`,
  commit `38a3f07`, `offset.saturating_sub(cursor)`-corrected after an adversarial review caught a
  residual-offset `=_bag` bug), plus test8/test9 (Distinct-over-Union per-arm dedup,
  `dedup_one_arm`, commit `3380d44`). test10-13 are DOCUMENTED BOUNDARIES (see the Family 1 table
  and the Roll-up progress note): test10/11 need Slice-side arm-drop to see through an enclosing
  Distinct with cross-arm-overlap-aware reasoning; test12/13 need database schema (UC/IS NOT NULL)
  access `normalize` doesn't have. Not attempted, not silently skipped.
- **Wave 7 (new) — join-elim SQL-shape collapse** *(the former Group D scope, now understood as cosmetic)*.
  Collapse the Group C `Union`+`NOT EXISTS` decomposition back to a single-scan join/`OptJoin` when a
  provable key/FD match makes the anti-join branch unreachable. Lowest urgency of the four (existing
  correctness is already proven; this is pure SQL-shape/perf).

**Recommended next: Wave 5/6 (Group A+B cosmetic rewrites)** — per the 2026-07-02 finding, `=_bag` parity
is exhausted (zero remaining oracle-gaps), so the M4 worklist's own sequencing rule ("defer [cosmetic
rewrites] until `=_bag` parity is exhausted") now applies: cosmetic SQL-shape work is unblocked. All three
remaining waves (5, 6, 7) are lower-risk than what was previously assumed of Wave 4 — none is a live
correctness gap.
