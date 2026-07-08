---
status: accepted
date: 2026-07-07
ratified: 2026-07-07
tags: [ontop-parity, residue-closure, correctness, feature-completeness, cosmetic, charter, sound-501, =_bag, deferral]
supersedes: []
depends-on:
  - ADR-0004
  - ADR-0005
  - ADR-0007
  - ADR-0008
  - ADR-0012
  - ADR-0021
  - ADR-0022
  - ADR-0023
  - ADR-0024
implements: ADR-0021
---

# Ontop-parity residue closure — catalog outstanding work and document deferral decisions

## Context and Problem Statement

As of 2026-07-07 (main == 7a4f88d), semantic-fabric has achieved **=_bag CORRECTNESS parity** with Ontop 5.5.0 on everything verified:

- **Row parity** on all 15 GTFS feature-class queries at scales 1/100/1000/10000
- **Performance parity or faster** on all 15 (geomean ~3× on small queries, up to ~11× on large joins/paths; q9/q6/q13 tiny-aggregate residual closed 2026-07-03)
- **W3C RDB2RDF conformance floor**: 81/82 on SQLite, 80/81 on PostgreSQL
- **Differential oracle** `differential_tree`: 117/117 queries vs. the SQLite+oxigraph-spareval reference

Milestones M0–M8 of the operator-tree IR program (ADR-0023) are complete. The =_bag-absolute rule (ADR-0007) — where sf cannot answer soundly, it emits an honest `Error::Unsupported` ("sound 501") rather than risk a wrong answer — is both the correctness anchor and the reason this ADR is needed: **everything that remains is either a genuine bug with bounded blast radius, a capability that *requires* architectural change to handle soundly, or a cosmetic SQL-shape parity that does not affect =_bag correctness**.

This ADR catalogs all three tiers, documents acceptance criteria for fixing tier-1 bugs, and records deferral rationale and stale-branch warnings for future sessions.

## Decision Drivers

* **Correctness anchor (ADR-0007).** No `=_bag` violations on verified queries; any remaining bugs must be fixed with the same rigor (oracle proof, revert-test, adversarial review, regression gate).
* **Honest roadmap (ADR-0021).** A documented deferral is more valuable than a silent gap — every item here is actionable by a future session with known effort size and gating criteria.
* **Charter discipline.** The program is **within charter** (ADR-0004/0021); the residue stays within ADR-0008 (tier-2 entailment excluded).

## Decision Outcome

The outstanding work is organized into three tiers (detailed below), with recommended execution order: Tier 1 (correctness risk) → Tier 2 (feature completeness) → Tier 3 (cosmetics). Each tier has acceptance criteria, architectural blocking points, and per-item effort estimates.

## Progress log (updated 2026-07-07)

The tiers below are the ORIGINAL catalog; this log records what has actually shipped. Each item was RED-tested → adversarially refute-reviewed → fully gated (`differential_tree` + workspace + W3C floor + clippy `-D` + fmt) → merged `--no-ff` and pushed.

**DONE — merged to `main`:**

| Item | Commit (merge) | Outcome / correction to the plan below |
|---|---|---|
| Tier-1 bug #1 (opts-nullability) | `502029c` | Compatible-merge fix; the adversarial review found **2 further bugs + 1 residual**, all closed (flat join-order drop; DISTINCT-over-COALESCE; both-nullable → sound 501). |
| Tier-1 bug #2 (`lower_as_subplan` slice drop) | `9ff2ea0` | Sound-501 both paths; two coincidentally-passing adversarial tests updated. |
| **Tier-2 gap 3** (COUNT(DISTINCT *)) | `43b5506` | Tree computes it via `rust_agg` whole-solution dedup; flat stays 501 (tree-exceeds-flat). |
| **Tier-2 gap 2** (multi-branch UNION SubPlan pooling) | `b10b3b4` | Unified single/multi-branch `lower_as_subplan`; the refuter found an **injectivity regression** (non-injective DISTINCT pooling), now gated to a sound 501. |
| **Tier-2 gap 5** (post-GROUP-BY expr over UNION aggregate) | `7f2a15d` | **NARROWED vs the plan**: `force_rust_group` + a Rust-side `eval_expr` evaluator, restricted to INTEGER-safe arithmetic (`+ - *`, unary `-` over COUNT + int literals). The refuter found that decimal aggregates (AVG/SUM) / division corrupt the XSD type in `eval_expr`'s f64 core → those stay sound 501. |
| **Tier-2 gap 4** (GROUP BY over a property-path closure) | `c43fe54` | **Much simpler than the dossier feared**: NOT path-as-SubPlan — just routing a single-branch path aggregation to the Rust group path (which runs the path branch's own SQL and groups by variable name). |
| **C.3** (SELECT DISTINCT over a non-injective template) | `4d47dc9` | A **pre-existing** silent-wrong-answer bug surfaced by the gap-2 refuter (both paths, no SubPlan). Fixed: `emit_branch_with` refuses SQL-DISTINCT emission over a non-injective binding → sound 501. |

**Corrections to this ADR's plan, learned by shipping:** the "gaps 2/4/5 collapse into one UNION-pooling milestone" framing (from the dossier, §Recommended Sequencing) proved **wrong** — gap 4 was a routing change, and gap 5's blocker was `bind_term_def` arithmetic + `eval_expr`'s numeric types, not pooling ("widen the pooling" is backwards there). Item-5/7/8 must-stay-501 tests were superseded to tree-superset/now-works tests.

**Two NEW pre-existing bugs discovered by adversarial review (not in the original catalog):**
- **C.3** — non-injective-template DISTINCT — **FIXED** `4d47dc9`.
- **C.4** — `AVG(?missing)` over an always-unbound operand var returned `"0"^^xsd:integer` vs spareval's UNBOUND (`rust_agg` `AggKind::Avg` conflated empty-group with all-unbound-operand). **FIXED** `3bcb2a8` — discriminate on `rows`: empty group ⇒ 0 (SPARQL §11, like SUM), non-empty-but-all-unbound ⇒ UNBOUND. (C.4's parenthetical claim that "SUM + MIN/MAX were already correct" was WRONG — see C.5.)
- **C.6** — SQL-PUSHDOWN aggregate over a NULLABLE operand. The C.5 fix corrected `rust_agg` (the in-process path), but a SINGLE-branch aggregate over an OPTIONAL/nullable operand is PUSHED to SQL, where SQL's NULL semantics diverge from SPARQL §11: reconstruction mapped `SUM(all-NULL)`→`"0"` (wrong — a GROUP-BY group is never empty, so all-NULL means non-empty-all-unbound ⇒ UNBOUND), and SQL silently skips NULL rows in a MIXED group where SPARQL propagates the error to the whole aggregate (SUM/AVG/MIN/MAX alike). **FIXED** — `lower_aggregation` now forces the Rust group path (`force_rust_group`) whenever a SUM/AVG/MIN/MAX operand reads a nullable alias (`def_reads_opt_alias` over `branch.nullable_aliases()`), reusing C.5's correct discrimination; COUNT is exempt (SQL `COUNT(col)` skips NULLs = SPARQL COUNT filters). Also fixed a pre-existing rust_agg decimal-canonicalisation bug the routing exposed: `decimal_term` rendered integral decimals as `"30.0"`; now canonicalised via `oxsdatatypes::Decimal` (the oracle's own library) to `"30"` (RDF term equality is lexical, so this was a real `=_bag` divergence). Regression-locked (`adr0025_c6_*`). Found by self-adversarial probing of the C.5 merge.
- **C.7** — AVG over an EMPTY group (implicit grouping, all rows filtered out) returned UNBOUND on the single-branch SQL-pushdown path, but SPARQL §11 (and the oracle) says `"0"^^xsd:integer` (like SUM). The reconstruction's NULL-aggregate branch mapped SUM/COUNT→"0" but AVG/MIN/MAX→UNBOUND. **FIXED** — AVG joins the "0" branch. This is sound ONLY because C.6 now routes nullable-operand aggregates to `rust_group`: on the SQL path the operand is therefore mandatory, so a NULL AVG value means 0 rows (empty group), never "non-empty but all-unbound" (which `rust_agg` C.4/C.5 handle as UNBOUND). MIN/MAX of an empty multiset stay UNBOUND (correct). Regression-locked (`adr0025_c7_*`). Found by the C.6 refuter (surface 7).
- **C.8** — `rust_agg` SUM/AVG **ignored `DISTINCT` entirely** (only COUNT read `agg.distinct`), silently double-counting duplicate operand values whenever an aggregation routed to the in-process Rust-group path (any UNION that can't self-join-eliminate, including anything hitting the C.6 nullable-operand gate). The SQL-pushdown sibling (`emit.rs::agg_expr_sql`) already renders `SUM(DISTINCT col)` correctly — only this path had the gap. **FIXED** — new `dedup_if_distinct` helper (canonicalises on the same lexical key the existing `COUNT(DISTINCT …)` branches use) applied to SUM/AVG's operand multiset before aggregating. MIN/MAX deliberately left untouched — dedup cannot change a minimum/maximum, so `MIN(DISTINCT ?v)` was already correct without reading `distinct`. Regression-locked (`adr0025_c8_*`). Found by a full-workspace test-coverage sweep (pass 6, 2026-07-08) that swept `sf-sparql`'s `iq`/`cascade` internals for the first time.
- **AVG decimal-division PRECISION (tracked residue, NOT fixed)** — AVG of integer/decimal operands that don't divide evenly (e.g. `(1+2+4)/3`) computes in `f64` (`2.3333333333333335`) but the oracle does exact `xsd:decimal` division (`2.333333333333333333`). PRE-EXISTING on BOTH the SQL path (SQLite AVG is f64) and `rust_agg`, independent of C.5/C.6 (refuter-confirmed: reproduces on `af2e4aa` with a mandatory operand, no OPTIONAL). A sound fix requires reworking AVG (and SUM/division) onto arbitrary-precision `oxsdatatypes::Decimal` arithmetic instead of `f64` — a substantial, focused change deserving its own ADR item, not a rider here. Bounded blast radius: only non-even AVG/division of decimals; integer/double aggregates and even divisions are unaffected.
- **C.6b** — `rust_agg` SUM/AVG must PRESERVE the `xsd:double` operand type (SPARQL §11.4 / XPath numeric promotion). C.6's routing of a nullable double-operand aggregate to `rust_group` exposed that `rust_agg`'s Sum/Avg always emitted `decimal_term` (⇒ `xsd:decimal`), a datatype `=_bag` regression vs the SQL path (which preserves double via `avg_result_code`). **FIXED** — added `is_xsd_double` + `double_term`; SUM/AVG now promote to `xsd:double` when any operand is double. (The residual `"7.5E0"` vs oracle `"7.5"` lexical form is the DELIBERATE R2RML §10 / ADR-0015 canonical-double representation, NOT a bug — oxigraph's f64 Display is non-canonical, so computed-double results are asserted against sf's own canonical form, not `assert_vs_spareval`.) Found by the C.6 adversarial refuter (surface 5); the refuter also confirmed two sibling probe failures are PRE-EXISTING and NOT C.6-attributable (AVG f64-division precision `2.333…35` vs exact decimal; AVG-over-empty-implicit-group reconstructs unbound but oracle wants "0" — both reproduce on the parent `af2e4aa` with no nullable operand, tracked as separate residue).
- **C.5** — `rust_agg` SUM/AVG/MIN/MAX **error-propagation**. Surfaced by the test-coverage pass (§ below) with a spareval-gated differential: SUM over an all-unbound operand returned `0` (should be UNBOUND — C.4's `rows`-discrimination was never applied to SUM), and — the broader bug — a **MIXED** group (some rows bind the operand, some leave it unbound) wrongly aggregated only the bound rows for ALL of SUM/AVG/MIN/MAX. Per SPARQL §11 (spareval-confirmed), a single unbound-operand row anywhere in a non-empty group makes the WHOLE aggregate UNBOUND (only COUNT filters errors). **FIXED** — each of SUM/AVG/MIN/MAX now returns UNBOUND when `!rows.is_empty() && rows.iter().any(|r| r.get(var).is_none())`, which subsumes C.4's all-unbound case and fixes the mixed case. Regression-locked (`adr0025_gap2_*`).

**Tier-2 gap 1 — DONE `ab26dfb`.** Property path inside EXISTS/NOT EXISTS/MINUS, via a new correlated `SqlCond::PathExists` over the path's recursive-CTE distinct-pairs derived table `t{alias}(sf_s, sf_o)`. `lower_iq_exists` reuses the existing correlation-building loop (unifying outer bindings with the path branch's sf_s/sf_o bindings); extracted `path_with_prelude` shared with `emit_path_branch`. Covers P+, length-1 composites (p/q, ^p, p|q), all three operators, one- and two-endpoint correlation. Reflexive P*/P? sound-501 (their prelude calls the fallible `reflexive_sql`, which the infallible `render_cond` can't propagate — bounded follow-up). Adversarially refute-reviewed across 7 soundness surfaces (anti-join over cyclic graphs, MINUS disjoint-domain no-op, same-var/two-endpoint correlation, unbound-OPTIONAL correlation, semi-join multiplicity preservation, mixed-case empty-catalog) — **no wrong-answer found**. Original capability: Ontop has no general recursive-path-in-EXISTS, so sf is ahead here.

**ALL TIER-1 + TIER-2 + the two new bugs (C.3, C.4) are now CLOSED — =_bag CORRECTNESS and FEATURE parity with Ontop 5.5.0 is complete on everything verified.**

**Tier-3 — RESOLVED (2026-07-07).** Investigated against the M4 worklist (`docs/design/ADR-0023-M4-optionb-worklist.md`) and empirically re-verified on current main. Finding: the `=_bag`-meaningful content of Tier-3 was **already closed** — Wave 6 (Group A Slice/Values/Distinct folding) by Wave C (`d313f26`..`487b4fb`), and the Wave-7 join-elim shapes (right-nested OPTIONAL `LJReductionWithLJOnTheRight`, `MergeLJs`-right-nested, `PaddingUnsat`-UNION-right) by Group C (`45ae36c`). The top-of-doc summary row that marked `LJReductionWithLJOnTheRight` as a "genuine TREE-ERR / real backlog item" was **superseded** by the doc's own later Group-C rows (was flat-501, now tree-`Ok` MATCH). All six representative shapes now compute the correct `=_bag` result on current main and are locked by the regression test `adr0025_tier3_bag_content_closed` (commit on branch `chore/adr-0025-tier3-verify`).

  The **only** UNimplemented Tier-3 items are pure SQL-*signature*-shape with **zero `=_bag`/perf value**, and are hereby closed as **won't-do (with cause)**, not deferred:
  - **Wave 5 binding-lift** (`BindingLiftTest::testUnionSubstitution`) — architecturally **N/A**: sf's `Plan` is a bag-union of independent `Branch` SELECTs by design (ADR-0006), streamed and concatenated; there is no single collapsed SELECT into which to hoist a shared URI-template binding. Matching Ontop's collapsed SQL *text* would require re-architecting Union lowering for a pure text-shape change with no `=_bag` or performance benefit (the DB re-optimizes regardless). Already charter-excluded by prior team-lead directive in the M4 worklist.
  - **test10/11** (`Slice·Distinct·Union` fold) — **correct-but-unoptimized**: empirically the result *set* is already correct (verified); only the SQL-shape arm-drop pruning is absent. The worklist documents this as a genuine hard boundary (cross-arm content overlap under DISTINCT — a later arm's contribution to a LIMIT window is not a fixed row count once earlier output can dedupe it), qualitatively harder than any Wave-C rule and of no functional value.

  **Net: Tier-3 is closed. `=_bag` CORRECTNESS + FEATURE parity with Ontop 5.5.0 is complete and regression-locked across all tiers; the residual is SQL-text cosmetics that sf's architecture does not express and that carry no correctness or performance consequence.**

  **Deep re-investigation (2026-07-08, on explicit request to do Tier-3 in full).** Every remaining Tier-3 SQL-shape item was re-examined empirically and ALL reduce to the SAME root — sf's two foundational design decisions, ADR-0006 (a `Plan` is a bag-union of independent `Branch` SELECTs, streamed + concatenated) and ADR-0007 (RDF term construction is LIFTED out of SQL into Rust reconstruction), which are enforced by invariant tests:
  - *binding-lift*: for a shared-URI-template union, sf emits N separate branch SELECTs projecting RAW key columns (`SELECT t0."id" AS c0 …`), with the `?s` IRI template applied at Rust reconstruction, never in SQL. Ontop's single collapsed SELECT with the template hoisted into SQL is unreachable without reversing BOTH ADR-0006 and ADR-0007.
  - *test10/11*: the `=_bag` result is correct and now locked (`adr0025_tier3_slice_distinct_union_folds_bag_correct`) — a constant `Union` folds bottom-up (`try_fold_constant_union`) → `Distinct` dedups → `Slice` truncates. The emitted shape is sf's constant-row branches, not Ontop's single `VALUES` clause.
  - *Values representation*: sf lowers `Values{r1..rn}` to N constant-row branches ("one branch per row", an invariant asserted in `lower.rs`'s own tests). Even collapsing this to a single SQL `VALUES` clause reverses a documented, test-enforced design decision.

  So Tier-3 SQL-*text* parity is not a set of local rewrites — it is a reversal of ADR-0006 + ADR-0007, a multi-week foundational rewrite that would REGRESS the bounded-memory streaming design (ADR-0006's raison d'être) and the indexed-join term-lifting design (ADR-0007) for ZERO `=_bag`/performance gain (the DB re-optimizes emitted SQL regardless). That is a foundational architecture decision reserved to the maintainer, not an autonomous cosmetic change; if byte-level Ontop SQL-shape parity is ever required, it belongs in its own ADR superseding ADR-0006/0007, not here. **Tier-3 remains closed won't-do-with-cause; the `=_bag` content is complete and regression-locked.**

---

## TIER 1 — Real correctness bugs (HIGHEST PRIORITY)

**Two pre-existing bugs with genuinely wrong answers, but NARROW blast radius — confirmed zero impact on any of the 15 GTFS feature-class queries and zero impact on W3C RDB2RDF conformance. Both are explicit ADR-0007 violations and must be fixed with full rigor.** Found as side effects of the SubPlan-OPTIONAL hardening (ADR-0025 session notes) and deliberately deferred outside assigned scope. Caution: both touch machinery used by every query; fixing either requires adversarial review with a specific angle (see acceptance criteria).

### 1. **opts-nullability bug** — OPTIONAL-left-unbound variable incompatible-mapping join semantics

**Symptom:** an OPTIONAL-left-unbound variable, later reused as a join key by a DIFFERENT, later mandatory pattern via a DIFFERENT anchor variable, is incorrectly treated as SQL `NULL` with ordinary equi-join semantics (drops rows) instead of SPARQL compatible-mapping semantics (§18.5: an empty domain intersection is vacuously compatible — should ADD rows, not drop them).

**Confirmed via:** independent manual derivation matching oxigraph-spareval's actual output.

**Affected paths:** BOTH flat and tree, identically rooted in the same InnerJoin/BGP binding-merge logic:
- Flat: `crates/sf-sparql/src/unfold.rs` → `merge`/`join_branches` binding-merge logic (verify against current HEAD before acting — this churns fast)
- Tree: `crates/sf-sparql/src/iq/lower.rs` → `insert_or_unify` (the InnerJoin/BGP fold)

**Confirmed NARROW:** same-anchor-variable reuse (the common case) is unaffected — only the cross-anchor binding-reuse scenario mis-handles NULL compatibility.

**Acceptance criteria for fix:**
1. Re-verify file:line citations against current HEAD; file locations may have drifted.
2. RED test: write a failing unit test that reproduces the silent wrong answer (actual vs. spareval mismatch).
3. =_bag proof on the reference oracle (SQLite in-process + oxigraph-spareval, using `crates/sf-conformance/tests/differential_tree.rs`).
4. Revert-proof: fix reverted ⇒ test fails.
5. Permanent regression test added to the harness.
6. Adversarial refute-only review **with a cross-anchor NULL-compatibility angle** — both paths (`insert_or_unify` + `merge`/`join_branches`) touch core join-fold machinery every query uses; the review MUST specifically verify no silent row-drops on cross-anchor binding reuse, since that exact blind spot previously masked the related `not_exists_cond_for` anti-join-FILTER bug (see Wave 7 / Consequences below).
7. All gates hold: `cargo test --workspace`, `cargo clippy -D warnings --all-targets`, `cargo fmt --check`, W3C RDB2RDF ≥81/82 SQLite floor.

### 2. **lower_as_subplan ORDER BY + LIMIT silent drop** — derived-table lowering drops ORDER/LIMIT, produces wrong answer

**Symptom:** `iq/lower.rs`'s `lower_as_subplan` function silently drops `ORDER BY` + `LIMIT` when lowering to a derived table used as an INNER JOIN input, producing a silent wrong answer instead of a sound 501 (distinct from the OPTIONAL/`left_join_over_subplan` path, which correctly sound-501s that shape).

**Confirmed via:** pre-existing bug found earlier in the broader program, now documented.

**Affected path:** `crates/sf-sparql/src/iq/lower.rs` → `lower_as_subplan` (plain INNER-JOIN SubPlan path, not OPTIONAL)

**Root cause:** the function apparently does not guard ORDER BY+LIMIT the same way as the OPTIONAL path does (which correctly 501s this shape in commit `e7cb7e6`).

**Acceptance criteria for fix:**
1. Re-verify file:line, function signature, and surrounding context against current HEAD.
2. RED test: failing test reproducing the wrong answer (e.g. a query with ORDER BY+LIMIT inside an INNER-joined SubPlan).
3. =_bag proof on the reference oracle.
4. Revert-proof: fix reverted ⇒ test fails.
5. Permanent regression test.
6. Adversarial refute-only review **checking whether the fix makes the path sound-501 (correct behavior) or whether the fix silently rewires ORDER BY+LIMIT handling without that guard** — given that the OPTIONAL path has already proven this is non-trivial, the fix must demonstrate it doesn't just hide the bug in a different code path.
7. All gates hold as for Tier 1 item 1.

---

## TIER 2 — Sound-501 feature-completeness gaps (REAL ONTOP CAPABILITIES SF SOUNDLY DECLINES)

**Five architecturally-proven must-stay-501 items under ADR-0007 — each is a real Ontop capability that sf currently sounds 501 on (correct behavior: better to refuse than to risk a wrong answer). Implementing any requires architectural change, not just effort. Each is a milestone-sized effort, NOT a quick fix. A rushed fix that turns any of these into a possibly-wrong answer is explicitly forbidden (ADR-0007 absolute rule).**

### 1. **Property-path inner inside EXISTS/NOT EXISTS/MINUS**

**Blocker:** `SqlCond::Exists` and `SqlCond::NotExists` carry `Vec<Scan>` (base table scans only); a property path compiles to a `WITH RECURSIVE` CTE (`sf_s`/`sf_o`), which is not referenceable from `EXISTS (… FROM scans)`.

**Requires:** a new CTE-aware variant of `SqlCond` (or a generalised `SqlCond::WithTable` that wraps both base scans and CTEs). This is real future work, not "too hard."

**Effort:** M1 or M2 milestone (SQL layer generalization + Exists path hardening).

### 2. **Multi-branch (UNION) SubPlan as a join/OPTIONAL input**

**Blocker:** `lower_as_subplan` remaps the inner plan's projection against ONE inner branch's output columns. A `UNION ALL` derived table has multiple branches with potentially different term-structures and types — it needs the same cross-arm type agreement that `try_sql_group_over_union` separately proves and gates (q9 agg-over-UNION pushdown). That check is not present in the SubPlan-as-join path.

**Also affects:** the unverified boundary "`LeftJoinJoinLimit: multi-branch right-side SubPlan → 501`" — likely the same class, but this specific boundary was not independently re-verified this session. Whoever picks this up should check whether fixing item 2 already subsumes it.

**Requires:** proof of cross-arm TermSpec agreement (same column types and IRI-template structures across all union arms) before lowering a multi-branch SubPlan as a join key or OPTIONAL input. Could be integrated into `try_as_subplan` or `lower_as_subplan`.

**Effort:** M1 milestone (SubPlan machinery extension, ~500 lines of proof + guard).

### 3. **COUNT(DISTINCT \*)**

**Blocker:** counts distinct *whole solutions* (all columns together). The `AggCol` IR node targets a single column, and multi-column `COUNT(DISTINCT …)` is non-portable (SQLite rejects multi-column DISTINCT in an aggregate). Sound form: `COUNT(*)` over `SELECT DISTINCT <all cols>` — a structural emission change in the lowering path.

**Note:** `COUNT(DISTINCT ?v)` for a single-column DISTINCT already works today.

**Requires:** a new emission path in `iq/lower.rs` → `emit.rs` that wraps the aggregation's project columns in a derived-table `SELECT DISTINCT` before applying `COUNT(*)`.

**Effort:** M1 or M2 milestone (lowering + emission layer ~300 lines).

### 4. **GROUP BY over a property-path closure**

**Blocker:** a property-path branch has an empty `core` — its variables live only in the CTE output (`sf_s`/`sf_o` columns), not in the raw base columns that `group_key_columns` / `single_column_of` read. Grouping over a CTE output requires the CTE as a SubPlan (similar blocker to item 2).

**Requires:** path-as-derived-table + aggregation-over-SubPlan (real future work).

**Effort:** M2 milestone (path refactoring + SubPlan aggregation, ~800 lines + adversarial gate on GROUP-BY-over-path correctness).

### 5. **Post-GROUP-BY expression over a UNION aggregate**

**Blocker:** the multi-branch aggregation path lowers to `rust_group` (in-process grouping, Rust executor). The executor can only RENAME aggregate outputs (e.g. `SUM→total`), not COMPUTE expressions over them (e.g. `SUM / COUNT`). Single-branch SQL GROUP BY already supports post-aggregate expressions via SQL's native `SELECT` projection logic, but Rust `rust_group` cannot.

**Requires:** a new post-group expression evaluator in the Rust executor that computes expressions over the `rust_group` outputs. Existing code: `crates/sf-sparql/src/exec_core.rs` (`rust_group_execute` / `rust_group_result_rows`).

**Dossier note (2026-07-07, `docs/research/ontop-optimizer-dossier.md`):** Ontop NEVER executes aggregation in-process — it emits one SQL statement or fails outright. So the true mismatch is sf's `rust_group` in-process fallback, not a missing expression evaluator: the primary fix is to widen the gap-2 UNION-pooling proof so fewer shapes reach `rust_group` at all; a Rust-side post-agg evaluator is fallback-of-last-resort.

**Effort:** M2 or M3 milestone (executor extension ~400 lines + gate on agg-expr correctness under bag semantics).

---

## TIER 3 — Cosmetic SQL-shape parity (LOWEST VALUE)

**~27 union-structural and join-elimination rewrites that make sf's emitted SQL resemble Ontop's exact join shapes. NONE of these affect =_bag correctness — the database re-optimises the emitted SQL regardless. All are already specced in `docs/design/ADR-0023-M4-optionb-worklist.md` (§Family 2/3/4/5 / Wave 5/6/7 sections; verify against current design doc before acting). Parallelizable across sub-agents (disjoint shapes), but shared `lower.rs`/`normalize.rs`/`leftjoin.rs` commits must serial-gate per the program's collision rule.**

### Wave 5 — Group B binding-lift / Values-fold

Structurally simplest (no LeftJoin involvement); touching `iq/normalize.rs` only. Sub-items: shared-variable binding hoisting above UNION, Values-row reordering, constant-arm folding. **Effort:** ~200 lines, 1 agent, 2–3 days.

### Wave 6 — Group A Slice/Values/Distinct folding

Slice-over-Values truncation; Distinct-over-Values deduplication; interactions. **Effort:** ~250 lines, 1 agent, 2–3 days. (Note: Wave C of the M4 probe — June 2026 — already implemented most of this; verify what remains in the current codebase.)

### Wave 7 — Join-elimination SQL-shape collapse

The `left_join_*` / `not_exists_cond_for` machinery — the most sensitive tier-3 path. Rewrites left-join chains into NOT EXISTS conditions or MINUS operators, matching Ontop's SQL shapes.

**Caution:** Wave 7 touches shared lowering paths that were a blind spot for a pre-existing `not_exists_cond_for` anti-join-FILTER bug (commit `feb7336`, already fixed and merged into main). **Whoever implements Wave 7 MUST re-run the adversarial review WITH a match-removing-filter angle after any change**, specifically checking that the OPTIONAL's own inner FILTER is correctly threaded through all variants of the anti-join code path. (See commit message `feb7336` for the bug pattern.)

**Effort:** ~300 lines, 1 agent, 3–4 days + specialized review gate.

### Total Tier 3 effort: ~750 lines, parallelizable into 1–2-week milestones.

---

## Recommended Sequencing

1. **Tier 1 first (correctness risk).** Both bugs are genuine ADR-0007 violations; neither is optional. Tier 1 execution should be a dedicated session.
2. **Tier 2 — revised by the Ontop dossier (2026-07-07, `docs/research/ontop-optimizer-dossier.md`).** The dossier grounds each gap in Ontop's actual source and reshapes the plan:
   - **Gaps 2, 4, 5 collapse into ONE milestone.** All three reduce to a single missing primitive — "pool N UNION branches into one derived table with proven cross-arm compatibility" — which sf already has one narrow working instance of (`try_sql_group_over_union`). Generalizing that into `lower_as_subplan`'s multi-branch path likely closes all three at once (gap 4 grouping-over-path falls out for free once paths are SubPlan-lowerable; gap 5's real fix is fewer shapes reaching `rust_group`, not a new evaluator). Do this milestone first.
   - **Gap 3 (COUNT DISTINCT *)** next — small, self-contained lowering change; note Ontop itself has a *live bug* here (silently drops DISTINCT), so sf's `SELECT DISTINCT`+`COUNT(*)` rewrite is original and more correct than the reference.
   - **Gap 1 (path-in-EXISTS)** last, and reframed as **original work, not a port** — Ontop has no general recursive-path support (only a hard-coded `rdfs:subClassOf*` TBox closure; no `WITH RECURSIVE` anywhere), so sf is already ahead of Ontop here. Largest effort (CTE-aware `SqlCond`).
3. **Tier 3 after tier-2 correctness is stable.** Waves can run in parallel on separate worktrees (one agent per wave), but shared commits must serial-gate:
   - Agents working Waves 5/6 in parallel; open PRs simultaneously.
   - Wave 7 PR waits for Waves 5/6 to merge.
   - **Before Wave 7 merge:** mandatory adversarial review with the `not_exists_cond_for` match-removing-filter focus.

---

## Benchmark and Validation Posture

**Out-of-scope work:** The honest sf-vs-Ontop head-to-head re-race (ONTOP_HOME=~/ontop-work/ontop-cli scripts/compare/race.sh at scales 1/100/1000/10000) is a **valuable validation step** for any tier-2/3 milestone, but is not a gating criterion for this ADR itself. Note:

- Must rebuild `target/release/semantic-fabric` first (`cargo build --release -p sf-cli`); a stale binary will hide wins or mask regressions (empirically discovered 2026-07-03).
- BENCHMARKS.md's checked-in numbers may lag the current engine; re-race if claiming a win.
- Report fractions (e.g. "0.92×–1.81×") never "100%/parity" — be precise about scale and direction.

---

## Consequences and Tradeoffs

**Good:**
- Tier 1 work is scoped and gated; no surprise regressions once fixed.
- Tier 2 items each have a proven architectural blocker; no "just implement it harder" surprises.
- Tier 3 is parallelizable and cosmetic; can proceed in parallel to new features without correctness risk.

**Bad/Cost:**
- Tier 1 bugs touch hot-path machinery (every query uses InnerJoin/OPTIONAL); fixes require surgical adversarial review and carry risk if the review misses an angle.
- Tier 2 items are each M1–M3 milestones; no quick wins in feature completeness.
- Tier 3 work is 27 rewrites, not one; requires coordination across multiple agents/worktrees.

**Neutral:**
- The ADR-0007 =_bag rule is preserved throughout all tiers.
- The differential oracle (`differential_tree.rs`) provides the correctness floor for all work.

---

## Stale-Branch Warnings (Do-Not-Do)

**Branch `feat/optimizer-gaps-close` is STALE / PRUNED.** The `not_exists_cond_for` fix it re-discovered already lives on main (commit `feb7336`, "OPTIONAL anti-join must apply its own inner FILTER"). Do NOT merge `feat/optimizer-gaps-close`. The redundant fix lives upstream; re-basing would create a duplicate conflict. If you see a PR for this branch, close it with a link to commit `feb7336`.

**Commit `2015846` (flat-era, pre-tree IR) is OBSOLETE.** Do NOT cherry-pick or merge. All value has been ported to the tree IR.

**Branch `feat/ontop-parity-wave-a-b` is the superseded flat-UCQ approach.** The tree IR (ADR-0023) is the current architecture. Do NOT merge.

---

## Charter Exclusions (Out of Scope)

- **OWL 2 QL tier-2 entailment** (RHS-existential / tree-witness saturation) — excluded by ADR-0008 and held externally in ODR-0030 (`semantic-modelling` repo). Tier-2 queries stay depth-0 / `501`.
- **Protégé / `.obda`** — N/A (Java GUI, no place in a no-JVM Rust engine).
- **General CQ-containment chase** — out of charter; a separable optimization workstream.

---

## More Information

* **ADR-0007** (=_bag absolute rule, ADR-0005/0012 gates) — the correctness anchor; quoted extensively above.
* **ADR-0021** (ontop parity program umbrella) — the charter and wave structure; this ADR is the **closing document** for ADR-0021.
* **ADR-0022** (WS-G, Wave 1 oracle/test port) — prerequisite for all work.
* **ADR-0023** (operator-tree IR, M0–M8 complete) — the architecture enabling this work.
* **ADR-0024** (SqlBackend abstraction, streaming spike ADOPT-LATER) — execution layer unification; independent of this ADR.
* **docs/design/ADR-0023-M4-optionb-worklist.md** — detailed tier-3 wave specs (verify against current design doc; it churns fast).
* **crates/sf-conformance/tests/differential_tree.rs** — the oracle harness for all =_bag proofs (117/117 tests as of 2026-07-07).
* **BENCHMARKS.md** — q9/q6/q13 re-raced and updated 2026-07-03; live race data; not a gating criterion.
* **Horizon trackers:** [[adr-0023-optimizer-residue-horizon]], [[adr-0024-executor-backend-abstraction-horizon]], [[ontop-parity-horizon]] (if applicable in your session).
