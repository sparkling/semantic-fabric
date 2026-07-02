# ADR-0023 — M4 OPTION_B Work-List (architect synthesis)

**Status:** M4 planning input — derived from the four family analyses + the empirical probe
(`crates/sf-conformance/tests/option_b_probe.rs`, `option_b_empirical_probe`). **Updated 2026-07-02**
(ADR-0023 optimizer-residue Wave B/D bucket-cleanup, see [[adr-0023-optimizer-residue-horizon]]): Group C
shipped and Group D reclassified from a feared correctness gap to a confirmed-`=_bag`-safe SQL-shape
cosmetic — see the Family 3 table and Roll-up section for the corrected dispositions; original
2026-06-30 figures kept alongside for history, not silently overwritten.
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
| union-structural | ValuesNodeOptimization::test1normalizationSlice | Slice(0,1) over Values(3) | needs-tree-rewrite [cosmetic] | NEW Slice-over-Values truncation (not in §4); bare-LIMIT non-det |
| union-structural | ValuesNodeOptimization::test2normalizationSlice | Slice(1,1) over Values | needs-tree-rewrite [cosmetic] | NEW Slice-over-Values w/ offset |
| union-structural | ValuesNodeOptimization::test3normalizationDistinct | Distinct over Values(dups) | needs-tree-rewrite [cosmetic] | §4.15 arm-dedup on Values leaf; SELECT DISTINCT already correct |
| union-structural | ValuesNodeOptimization::test4…SliceUnionValuesValues | Slice(0,4) over Union[Values,Values] | needs-tree-rewrite [cosmetic] | §4.15 Values-fold + NEW slice-over-Values |
| union-structural | ValuesNodeOptimization::test5…SliceUnionValuesNonValues | Slice(0,2) over Union[Values,ext], Values covers limit | needs-tree-rewrite [cosmetic] | NEW slice-over-union arm-drop |
| union-structural | ValuesNodeOptimization::test5…SliceUnionValuesValuesNonValues | Slice(0,4) over Union[Values,Values,ext] | needs-tree-rewrite [cosmetic] | §4.15 fold + NEW slice-over-union arm-drop |
| union-structural | ValuesNodeOptimization::test6…SliceUnionValuesNonValues | residual limit pushed to single non-Values arm | needs-tree-rewrite [cosmetic] | NEW residual-limit Slice-distribution-over-Union |
| union-structural | ValuesNodeOptimization::test7…SliceUnionValuesNonValues | residual limit per non-Values arm, outer Slice kept | needs-tree-rewrite [cosmetic] | NEW per-arm residual-limit pushdown |
| union-structural | ValuesNodeOptimization::test8…DistinctUnionValuesNonValues | Distinct over Union[distinct-Values,ext] no-op | needs-tree-rewrite [cosmetic] | §4.15 + Values-distinctness analysis |
| union-structural | ValuesNodeOptimization::test9…DistinctUnionValuesNonValues | Distinct over Union[Values(dups),ext] | needs-tree-rewrite [cosmetic] | §4.15 arm-dedup on Values leaf (distinct-dominated) |
| union-structural | ValuesNodeOptimization::test10…LimitDistinctUnionValues | Slice·Distinct·Union, Values non-distinct | needs-tree-rewrite [cosmetic] | §4.15 dedup + NEW slice-over-distinct-union arm-drop |
| union-structural | ValuesNodeOptimization::test11…LimitDistinctUnionValues | Slice·Distinct·Union, Values distinct | needs-tree-rewrite [cosmetic] | NEW slice-over-distinct-union arm-drop |
| union-structural | ValuesNodeOptimization::test12…LimitDistinctUnionDistinctTree | limit pushed onto both distinct arms | needs-tree-rewrite [cosmetic] | NEW slice-over-union + arm-distinctness (UC + IS NOT NULL) |
| union-structural | ValuesNodeOptimization::test13…LimitDistinctUnionNonDistinctTree | limit pushed onto the one distinct arm | needs-tree-rewrite [cosmetic] | NEW selective per-arm limit-pushdown |
| union-structural | ValuesNodeOptimization::test14…ConstructionUnionTrueTrue | Union[const-Construct/True ×2] → Values | needs-tree-rewrite [cosmetic] | §4.15 fold-constants-into-Values (probe: free-win) |
| union-structural | ValuesNodeOptimization::test15…ConstructionUnionTrueTrueDataNode | const arms fold, data arm kept | needs-tree-rewrite [cosmetic] | §4.15 partial Values-fold |
| union-structural | ValuesNodeOptimization::test17…DBConstant | same-DB-type DBConstant arms fold | needs-tree-rewrite [cosmetic] | §4.15 fold w/ homogeneous-cell-type gate |
| union-structural | ValuesNodeOptimization::test18…RDFConstant (diff datatypes) | heterogeneous RDF constants → NO fold | free-pass (negative) | §4.15 precondition forbids fold → matches Ontop |
| union-structural | ValuesNodeOptimization::test19…RDFConstant (same type) | same-type RDF consts → Construction(RDFLiteral) over Union[Values,data] | needs-tree-rewrite [cosmetic] | §4.15 binding-lift: split term, lift type, fold lexical |
| union-structural | ValuesNodeOptimization::test21…IRIConstant | mix foldable consts + IRI-template arm | needs-tree-rewrite [cosmetic] | §4.15 selective fold |
| union-structural | ValuesNodeOptimization::test22…NonConstant | mix consts + IS-NOT-NULL-expr arm | needs-tree-rewrite [cosmetic] | §4.15 fold gated on Constant value |
| union-structural | ValuesNodeOptimization::test23…RDFConstant (2-var) | multi-column Values under lifted Construction | needs-tree-rewrite [cosmetic] | §4.15 binding-lift, multi-projected-var generalisation |
| union-structural | ValuesNodeOptimization::test24…RDFConstantSub | heterogeneous → split term, lift wrapper, no Values fold | needs-tree-rewrite [cosmetic] | §4.15 RDF-term binding-lift (deepest UnionAndBindingLift) |
| union-structural | ValuesNodeOptimization::test25NoVariableTrueNodesAndValuesNodes | zero-var Union[True,True,Values] → counting Values | needs-tree-rewrite [cosmetic] | §4.15 + True≡single-0-arity-row identity |
| union-structural | ValuesNodeOptimization::test26MergeableCombination… | mixed True/Construction/Values arms, diff col order → one Values | needs-tree-rewrite [cosmetic] | §4.15 general arm-merge + col-order reconcile |
| union-structural | ValuesNodeSimpleQueryOptimization::testTranslatedSQLQuery1 | end-to-end LIMIT 2, assert SQL has no union/limit | needs-tree-rewrite [cosmetic] | Slice-over-Union/Values drivers; `=_bag` count already correct |
| union-structural | ValuesNodeComplexQueryOptimization::testTranslatedSQLQuery1 | end-to-end LIMIT 4 over wide mapping union | needs-tree-rewrite [cosmetic] | same Slice driver scaled |
| union-structural | BindingLiftTest::testUnionSubstitution | lift common URI-template binding into shared Construction above union | needs-tree-rewrite [cosmetic] | §4.15 binding-lift (load-bearing for signature parity, not `=_bag`) |

Family 1 totals: **6 free-pass, 27 needs-tree-rewrite (all [cosmetic] for `=_bag`), 0 M5, 0 charter.**

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
| join-elim | LeftJoinOpt::FDSimplification (testFDSimplification) | nested OPT chain + per-right DISTINCT/FILTER + FD + ancestor FILTER | free-pass — **confirmed 2026-07-02** (`option_b_probe.rs` MATCH, commit `9967884`) | Group C re-assoc already `=_bag`-correct; Ontop's multi-LJ FD-merge is a cheaper-SQL alternative, not a correctness prerequisite |
| join-elim | LeftJoinOpt::PaddingForUnsatisfiableRight single-Construction (testPaddingForUnsatisfiableRight1) | outer FILTER makes single-scan right unsat → NULL-pad | free-pass | §4.2 unsat-equality-prune + §4.13 |
| join-elim | LeftJoinOpt::PaddingForUnsatisfiableRight UNION-right (testPaddingForUnsatisfiableRight2/3) | right is a UNION, all arms unsat | free-pass — **confirmed 2026-07-02** (`option_b_probe.rs` MATCH, commit `54fd5e9`) | Group C's decomposition re-feeds the opts-free union into `left_join_branches`'s multi-branch NOT-EXISTS arm, which correctly NULL-pads rather than distributing the unsat-prune into the union |
| join-elim | LeftJoinOpt::LeftJoinUnionConstants/LeftJoinValues (testLeftJoinUnionConstants, testLeftJoinValues) | (L ⋈ Union{const}/Values) OPT R | free-pass | §4.15 fold + §4.16 + §4.4; Values is first-class leaf |
| join-elim | LeftJoinOpt::LeftJoinJoinLimit (testLeftJoinJoinLimit) | (L ⋈ SUBSELECT{…LIMIT 1}) OPT R | free-pass — **CLOSED (M5 Wave 2, `left_join_as_subplan`, `iq/lower.rs:620-632`)** | §5.1 SubPlan derived-table LEFT JOIN. Residual narrower boundary (still 501, not this row's scenario): a MULTI-branch right-side SubPlan, or a right branch that ITSELF still carries `opts` after decomposition (nested-OPTIONAL-inside-a-LIMIT-subselect) |
| join-elim | LeftJoinOpt::SelfLeftJoinWithProvenanceBlockedByDistinct (…1/3-10, SameVarsDistinct1, *NoOpt1/2) | DISTINCT over L OPT {BIND(prov).R} | free-pass | §4.4 + IfElseNull(IsNotNull) witness + §4.5/§4.15 distinct-bounded |
| join-elim | LeftJoinOpt::ImplicitVariableNonRemoval (testImplicitVariableNonRemoval) | OPT right var shared with core atom → no-op | free-pass | §4.5 global-deadness fails (sound no-op) |
| join-elim | MappingCQCOptimizer::test (FK redundant-join) | drop FK parent scan | free-pass | §4.6 fk-pk-join-elimination |
| join-elim | MappingCQCOptimizer::test_foreign_keys / ::test_optimisation_order | general containment chase (LIDs + homomorphism) | charter-excluded | semantic chase, not §4 syntactic (ADR-0023 keeps only §4.6) |
| join-elim | NRAJoinOptimizer (entire class, e.g. testFlattenLift1) | FlattenNode/NestedView/array-unnest lift | charter-excluded | FlattenNode/JSON out of charter; class disabled in Ontop |

Family 3 totals (updated 2026-07-02, see [[adr-0023-optimizer-residue-horizon]]): **25 free-pass, 0
needs-tree-rewrite, 0 M5 (closed), 2 charter** (27 rows total, unchanged). All 7 rows this table
originally marked needs-tree-rewrite (Group C: MergeLJs-right-nested, LJReductionWithLJOnTheRight,
PaddingUnsat-UNION-right; Group D: JoinTransfer PK/FK, JoinTransfer FD, FDOnRight, FDSimplification)
are now empirically confirmed
`=_bag`-correct (Group C's general `(L⋈R)∪(L¬∃R)` decomposition, `left_join_decomposed`, closes the whole
class regardless of FD/key structure — Ontop's FD-driven Group D rules turned out to be a cheaper-SQL
*alternative* strategy for the same correctness outcome, not a prerequisite for it). The
LeftJoinJoinLimit/M5 row is also closed (M5 Wave 2, `left_join_as_subplan`). What remains for all of
these is SQL-SHAPE-ONLY: the tree emits an extra `NOT EXISTS` correlated-subquery scan where Ontop emits
one collapsed join — a real, in-charter, but now-cosmetic backlog item (folds into the Family 1 cosmetic
set below, not a separate correctness milestone).

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

| disposition | union-structural | boolean-push | join-elim | projection-and-true | **total** | *(orig. join-elim / total)* |
|---|---|---|---|---|---|---|
| free-pass | 6 | 27 | 25 | 30 | **88** | *(17 / 80)* |
| needs-tree-rewrite | 27 | 0 | 0 | 0 | **27** | *(7 / 34)* |
| needs-SubPlan-M5 | 0 | 0 | 0 | 0 | **0** | *(1 / 1, now closed)* |
| charter-excluded | 0 | 0 | 2 | 4 | **6** | *(2 / 6)* |
| **enumerated rows** | 33 | 27 | 27 | 34 | **121** | *(same 121 rows as the original table — 7 needs-tree-rewrite + 1 M5 reclassified to free-pass within join-elim, none added/removed)* |

Row-count note: these are the **representative analysis rows**; several join-elim rows fold multiple
Ontop `@Test` methods (e.g. `testJoinTransfer1-14`) into one shape. They cover the full 16-class / 184-test
OPTION_B surface from the handover — the 184 figure is the Ontop method count, ~120 the distinct shapes.

**`=_bag` reality (2026-07-02, probe + adversarial-review-adjusted):** of the original 34 needs-tree-rewrite
rows, **all 27 union-structural (Family 1) are [cosmetic]** (the tree is already `=_bag`-correct; the
rewrite buys only Ontop SQL/node-signature parity) and the **7 join-elim (Group C + Group D) rows are now
ALSO cosmetic** — Group C's general decomposition closed them all, confirmed by `option_b_probe.rs`
(commits `9967884`, `54fd5e9`) plus a dedicated adversarial refute-only review (9 fixtures across nullable
FD determinants, DISTINCT/anti-join interaction, cyclic self-joins, multiplicity — zero mismatches found;
caveat: SQLite-only, live-PG/MySQL dialect-specific 3VL quirks on this exact shape not separately
re-verified). **Zero remaining genuine `=_bag` oracle-gaps in this table.** What's left everywhere is
SQL-shape/signature parity only — 27 union-structural rewrites + the join-elim group's extra `NOT EXISTS`
scan vs Ontop's collapsed join.

## Proposed implementation waves (needs-tree-rewrite, dependency order)

- ~~**Wave 3 — Group C: LeftJoin multi-node-right re-association**~~ **SHIPPED** (M4 Wave 3, commit
  `45ae36c`). Unblocked LJReductionWithLJOnTheRight, MergeLJs-right-nested, PaddingUnsat-UNION-right — all
  confirmed `=_bag`-correct.
- ~~**Wave 4 — Group D: atom/FD transfer at the LeftJoin-over-InnerJoin boundary**~~ **RECLASSIFIED
  2026-07-02**: not needed for correctness (Group C already closes the whole class independent of FD/key
  reasoning — see Roll-up above). What remains is a SQL-shape/performance optimization (collapse the
  `Union`+`NOT EXISTS` decomposition back into a cheaper single-scan join when a key/FD condition proves
  it's safe) — folded into the cosmetic backlog below, no longer a separate correctness-critical wave.
- **Wave 5 — Group B: UnionAndBindingLift + Values constant-fold** *(independent; signature parity only,
  NEXT UP — `=_bag` parity is now exhausted per the finding above)*. §4.15 fold-constants-into-Values,
  RDF-term split/lift, multi-column Values. Unblocks: test14-26, test19/23/24,
  BindingLiftTest::testUnionSubstitution, end-to-end SQL-shape tests. Cosmetic for `=_bag`.
- **Wave 6 — Group A: Slice/Values/Distinct folding drivers** *(pure SQL-shape, all cosmetic)*.
  New Slice-over-Values/Union truncation + arm-drop + arm-distinctness analysis. Unblocks: test1-13,
  Simple/Complex SQL-shape tests.
- **Wave 7 (new) — join-elim SQL-shape collapse** *(the former Group D scope, now understood as cosmetic)*.
  Collapse the Group C `Union`+`NOT EXISTS` decomposition back to a single-scan join/`OptJoin` when a
  provable key/FD match makes the anti-join branch unreachable. Lowest urgency of the four (existing
  correctness is already proven; this is pure SQL-shape/perf).

**Recommended next: Wave 5/6 (Group A+B cosmetic rewrites)** — per the 2026-07-02 finding, `=_bag` parity
is exhausted (zero remaining oracle-gaps), so the M4 worklist's own sequencing rule ("defer [cosmetic
rewrites] until `=_bag` parity is exhausted") now applies: cosmetic SQL-shape work is unblocked. All three
remaining waves (5, 6, 7) are lower-risk than what was previously assumed of Wave 4 — none is a live
correctness gap.
