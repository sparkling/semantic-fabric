# ADR-0023 M3 — Resolution / Normalize / Lower Pipeline

> **Status:** contract (the spec M3a–M3d implement against).
> **Authority:** subordinate to `docs/design/ADR-0023-design-lock.md` (LOCKED) and
> `docs/adr/ADR-0023-query-ir-architecture-flat-ucq-vs-iq-tree.md`. This document refines
> §3/§4/§5/§9 of the lock for the **tree path**; where the two disagree, the design owner
> must record the §9 amendments flagged in §9 below before M3 merges.
> **Oracle:** the flat `unfold.rs` / `emit.rs` translation is the PROVEN base translation
> and the empirical oracle. M3 is a re-expression of `unfold.rs` as a tree fold; it does
> **not** re-derive translation, and it must be `=_bag` (multiset-equivalent) to the flat
> translation, gated end-to-end by a differential (§7).
> **Harness law:** smallest change; reuse the proven flat machinery over new abstractions;
> an optimization fires only when its precondition holds, else a sound no-op; trust only
> the gated artifact.

---

## 1. Pipeline (build → resolve → normalize → lower)

Four total functions, each consuming the prior stage's output. The flat `unfold.rs`/`emit.rs`
remains the oracle for every stage.

```
spargebra::Query
  │  (1) BUILD   — M2, EXISTS                 build.rs:65  build_tree(gp, graph)
  ▼     context-free; Intensional leaves + SYMBOLIC FILTER/BIND
IqNode  (Intensional leaves, IqCond::Expr, BindDef::Expr)
  │  (2) RESOLVE — M3a                        iq/resolve.rs  resolve(node, cx)
  ▼     Intensional → Extensional+Construction(+Union/InnerJoin); var→column scope established
IqNode  (ZERO Intensional; FILTER/BIND still symbolic)
  │  (3) NORMALIZE-min — M3b                  iq/normalize.rs  normalize(node)
  ▼     subst-lifting + join-over-union (§4.16) + Empty/True pruning → leaf-CQ spine
IqNode  (Union-of-(Construction over Join/LeftJoin/Filter of leaves); each branch ONE bindings map)
  │  (4) LOWER   — M3c                        iq/lower.rs  lower(node, dialect)
  ▼     per leaf-CQ → ONE Branch; FILTER/BIND resolved HERE, per-branch; §5.1 cases → 501
Plan { branches: Vec<Branch>, distinct/limit/offset/order, rust_group }
```

**Where FILTER/BIND resolve, and why it is mandatory (not stylistic).** FILTER/ON conditions
and BIND definitions remain **variable-referencing** (raw `spargebra::Expression`) through
BUILD + RESOLVE + NORMALIZE, and are resolved to `SqlCond`/`TermDef` **only at LOWER,
per-leaf-CQ**, by reusing the proven flat `lower_filter_expr` / `bind_term_def`. A FILTER above
a `Union` has no single `ColRef` for `?x`: `var_col` reads each branch's own `bindings` — a
different alias, possibly a different source column, possibly a constructed term with no column
at all. Only after NORMALIZE splits the union into per-alternative leaf-CQs does `?x` have one
binding to resolve against. This supersedes the literal "lower FILTER/BIND inline at build"
reading of the lock's §9 Option-B note (§9 amendment below).

**Triple resolution timing vs FILTER/BIND lowering timing are distinct.** RESOLVE may resolve
triple patterns eagerly (reusing the flat `bgp`/`atom` machinery, the spirit of Option-B), but
FILTER/BIND lowering MUST be deferred to per-leaf-CQ LOWER. The lock's §9 conflated the two.

---

## 2. IR refinements (exact Rust)

Two new symbolic carriers in `crates/sf-sparql/src/iq/node.rs`. No other `node.rs`/`iq.rs` type
changes for M3. Both REUSE the existing resolved vocabulary (`SqlCond`, `TermDef`) rather than
paralleling it.

### 2.1 `IqCond::Expr` — the variable-referencing FILTER/ON leaf

```rust
// crates/sf-sparql/src/iq/node.rs — enum IqCond
pub enum IqCond {
    /// Variable-referencing FILTER/ON leaf. Symbolic through resolve+normalize;
    /// lowered to `Sql` per leaf-CQ at LOWER via flat `lower_filter_expr`.
    Expr(Box<spargebra::algebra::Expression>),
    Sql(SqlCond),               // resolved/pushed leaf (LOWER output, M4 input)
    And(Vec<IqCond>),
    Or(Vec<IqCond>),
    Not(Box<IqCond>),
    Exists(Box<IqNode>),        // unchanged
    NotExists(Box<IqNode>),     // unchanged
}
```

*Rationale.* A FILTER over a multi-branch `Union` resolves `?x` to a different `ColRef` per
branch; one `SqlCond` cannot represent it (proven by `var_col`/`unify` reading branch-local
bindings). `EXISTS`/`NOT EXISTS` already carry an unlowered `IqNode` for the same reason —
`Expr` is the leaf-condition analogue. Lowering reuses `lower_filter_expr` verbatim (it already
walks `And`/`Or`/`Not`/`Exists` and delegates leaves to `filter_cond`). `Sql(SqlCond)` stays
the resolved/pushed carrier filled by LOWER and by the §4 (M4) rewrite rules — no parallel
condition system.

### 2.2 `BindDef` — the unresolved-or-resolved Construction substitution

```rust
// crates/sf-sparql/src/iq/node.rs
pub enum BindDef {
    Resolved(TermDef),                          // const/column/template/Coalesce/Concat/Agg
    Expr(Box<spargebra::algebra::Expression>),  // symbolic until LOWER
}
// IqNode::Construction { child, subst: BTreeMap<Var, BindDef>, project }
// LOWER folds each entry:
//   Resolved(td) => bindings.insert(v, td)
//   Expr(e)      => bindings.insert(v, bind_term_def(&e, &bindings)?)
```

`Construction.subst` becomes `BTreeMap<Var, BindDef>`. This **retires the M2 context-free
non-constant-BIND 501** with no parallel term system: `Resolved(TermDef)` keeps the existing
resolved carrier intact (`Const`/`Derived`/`Coalesce`/`Concat`/`Agg`); `Expr` carries the raw
expression until LOWER, mirroring the `AggArg::Var|Expr` and `OrderKey.expr` precedents
(variable-referencing until exec). BUILD now emits `BindDef::Expr` instead of a 501.

### 2.3 No further M3 type changes (documented deferral)

`AggArg::Var|Expr(TermDef)` already suffices for aggregate arguments. General arithmetic
agg-arg / BIND beyond `TermDef`'s `Concat` vocabulary stays a **tracked sound-501** resolved at
LOWER (`Err(Unsupported("non-Concat BIND / agg-arg expr → 501"))`). If pursued post-M3 it is a
NEW `TermDef` variant holding `Box<Expression>` evaluated at reconstruction (mirroring
`OrderKey.expr`) — an `iq.rs` concern, orthogonal to resolution timing, out of M3 scope.

### 2.4 Doc-comment correction (load-bearing)

`node.rs` currently documents `IqCond`/`Construction.subst` as "resolved at build". Update to:
**"symbolic until LOWER (`Expr`) | resolved (`Sql`/`Resolved`)"**. The `Union` invariant
doc-comment (`node.rs:84-88`) must be relaxed per §3-corollary-R3 / §5 below — it must NOT
require concrete NULL-padding.

---

## 3. Resolve contract (M3a)

Intensional → Extensional **reuses the flat per-triple logic verbatim — do not re-derive.**

### 3.1 Per-Intensional algorithm

For each `IqNode::Intensional { pattern, graph }`, drive the body of `pattern_branches`
(`unfold.rs:388`) + `atom` (`unfold.rs:429`):

- Loop over all triples-maps; for each, emit class atoms (`rdf:type` / variable predicate) plus,
  per POM, every `(predicate-map × object-map)` pair.
- Each surviving atom yields ONE leaf-CQ arm:
  `Extensional(scan @ fresh-alias, bind) + Construction(subst: subject/object/predicate var →
  TermDef via def_of — Const for rr:constant else Derived{term_map, alias}) + InnerJoin cond`
  from `constrain`/`unify` for constant positions and shared-var re-occurrence.
- `rr:refObjectMap` (`unfold.rs:468`) resolves the parent map, allocates a SECOND alias, and
  emits a **2-scan InnerJoin** with `SqlCond::ColEq(child.j, parent.j)` join conds.
- `tbox.predicate_can_match` fast-rejects arms **before any alias is spent**.
- 0 surviving arms ⇒ `IqNode::Empty { vars }` (the pattern's variables).
- exactly 1 arm ⇒ that subtree directly (no `Union` wrapper).
- N ≥ 2 arms ⇒ `IqNode::Union { children, project = pattern's variables }`.
- Active graph: `graph_maps_match` against the resolved constant `graph`; a variable graph is
  already 501 at BUILD.

Filters and binds are **NOT touched** by RESOLVE — they stay `IqCond::Expr` / `BindDef::Expr`.

### 3.2 Scope establishment

The alias counter (`Unfolder::alias`, `unfold.rs:78`) hands out fresh aliases. Each
`Extensional.scan.alias` plus its `Construction.subst` entries (`var → TermDef::Derived{term_map,
alias}`) **ARE** the var→column scope that NORMALIZE's structural rules and LOWER's
`var_col`/`unify` read. After RESOLVE, every variable in every arm has a concrete binding — the
precondition for join-over-union (needs each arm's projected vars) and substitution-lifting
(needs the var→TermDef map).

### 3.3 Smallest-change implementation

Extract the per-`(tm, pom)` atom body into a **shared primitive** returning
`(Scan, bindings, conds)`, consumed by BOTH the flat `atom` (still producing `Branch`) and tree
RESOLVE (producing `Extensional` + `Construction` + `InnerJoin`). `unify` / `def_of` /
`predicate_match` / `map_by_id` are reused unchanged. `=_bag` follows because the arm set,
conds, aliases, and pruning are identical to the flat translation.

### 3.4 Corrected: a multi-arm / refObjectMap pattern under an OPTIONAL right (R1)

> **Refuted-and-restated** (verification ledger §9, item R1). The naive `=_bag` argument is
> sound for INNER joins but does NOT extend to `LeftJoin`. RESOLVE turns a multi-triples-map
> pattern into `Union{children}` (N ≥ 2) and a refObjectMap into a **2-scan InnerJoin**
> (`core.len() == 2`). When such a pattern is the RIGHT of an OPTIONAL, distributing the left
> join over the union (`A⟕(C∪D) → (A⟕C)∪(A⟕D)`) over-counts the null-padded row by one per
> arm, and feeding a multi-arm `Union` / 2-scan right into `build_left_join` is impossible
> (`build_left_join` consumes only `right.core[0]`).

**Contract:** RESOLVE produces the `Union`/2-scan-`InnerJoin` faithfully; the OPTIONAL
semantics are preserved downstream by (a) NORMALIZE **exempting `LeftJoin` from
right-distribution** (§4) and (b) LOWER routing the `LeftJoin` through the existing
`crate::leftjoin::left_join_branches` dispatcher (§5), which already implements
`P OPT R = (P ⋈ R) ∪ (P − R)` with exactly one no-match (`NOT EXISTS Ri`) branch. RESOLVE
itself introduces no special-case for OPTIONAL.

---

## 4. Normalize-minimum (M3b) — M4 optimization rules explicitly OUT

The MINIMUM to reach the lowerable leaf-CQ spine. **Three** transformations, NOTHING
cost-driven. The leaf-CQ spine is: `Union`-of-(`Construction` over a `Join`/`LeftJoin`/`Filter`
of `Extensional`/`Values`/`Path` leaves).

### (a) Substitution-lifting (§3, minimal)

Fold `Construction ∘ Construction` (compose subst, intersect project) and push `Construction`
through `InnerJoin`/`Union` so each leaf-CQ terminates in **at most ONE `Construction`** over a
join/filter of leaves — giving each branch a single `var → TermDef` bindings map that LOWER
folds into `Branch.bindings`. Variable equivalence classes from join `ColEq` propagate as subst
entries (the flat `rewrite_alias`/`merge` dance becomes "apply the substitution at the
`Construction`"). **Stop as soon as each branch has one bindings map**; do NOT chase a global
fixpoint of optimizing rewrites. **Forbidden:** any arm-merge or structural dedup that is not
proven multiplicity-preserving (§7 / ledger R5) — a `Union` arm is never collapsed into a
structurally identical sibling.

### (b) Join-over-union distribution (§4.16)

- `InnerJoin(A, Union(B1..Bn)) ⇒ Union(InnerJoin(A,B1)..InnerJoin(A,Bn))` (either operand).
- `Filter(Union(B1..Bn)) ⇒ Union(Filter(B1)..Filter(Bn))`, **CLONING the symbolic
  `IqCond::Expr` into each arm** (never a pre-lowered `SqlCond` — corollary 1).
- `LeftJoin` distributes ONLY over a `Union` on its **LEFT** (preserved) operand:
  `(A∪B)⟕C → (A⟕C)∪(B⟕C)` is sound; `A⟕(C∪D)` MUST NOT be split.
- A `LeftJoin` with a `Union`/multi-scan **RIGHT** does **not** distribute: it stays a
  `LeftJoin` node and is handed to `left_join_branches` at LOWER (§5.3).

This is the structural step that turns a multi-map/UNION query into a flat `Vec` of single-scope
leaf-CQs == the flat bag-union `Vec<Branch>`.

#### Corrected: UNDEF-padded shared join variables (R2)

> **Refuted-and-restated** (ledger §9, item R2). The abstract distributive law is `=_bag`-faithful
> only if a shared join variable that is UNBOUND in an arm is treated with SPARQL compatibility
> semantics. The flat oracle (`unfold.rs:1161` `merge`) treats a join var **absent** in one side
> as a free/cartesian dimension (rebind from the bound side, **no equality**). A NULL-rejecting
> equi-join over a padded UNDEF silently drops the entire cartesian arm.

**Contract:**

1. A `Union` arm that does not bind a projected variable marks it **UNDEF/absent** (a
   distinguished unbound marker), **never a NULL-valued column** — mirroring the locked
   `Values` rule (design-lock:563 "UNDEF ⇒ absent var").
2. When `InnerJoin`/`Filter` distributes into an arm (§4.16) and a join/shared variable is
   UNDEF-padded (absent) in that arm, unification MUST treat it as unbound: **emit NO equality
   and rebind the variable from the bound operand** (exactly `merge`'s `bindings.get(var) == None
   ⇒ insert other side`, `unfold.rs:1161`). The per-arm join degenerates to a free dimension on
   that variable, not a NULL-rejecting equi-join.
3. Only variables BOUND (provably non-UNDEF) in BOTH operands of a distributed arm may seed a
   `ColEq`/term-equality. UNDEF-padded shared vars are skipped and lifted from the bound side.
4. If the implementation cannot prove which arms bind a shared join variable, the sound fallback
   is to NOT pad-then-distribute (keep base-style different-width arms) or 501 — never silently
   drop rows.

### (c) Identity pruning

Drop `IqNode::Empty` (Union identity, InnerJoin absorbing) and `IqNode::True` (InnerJoin
identity, **condition-free only**) so neither reaches LOWER as a leaf needing a `Branch`.

### EXPLICITLY OUT (M4 — the §4 optimizer)

Selectivity-driven filter push-down; self-join / FD elimination (waves 1b/1c); DISTINCT-driven
OPTIONAL pruning (wave-D); redundant-join removal; unsatisfiable-cond detection beyond
`predicate_can_match`. These require the resolved `ColRef` form and operate after/at lowering on
`Sql(SqlCond)` — the proven wave-A..E optimizer, NOT part of reaching the lowerable spine.

---

## 5. Lower contract (M3c)

Per leaf-CQ → ONE `Branch` via bottom-up fold, reusing every existing constructor/emitter. THIS
is the single point where filters/binds resolve.

- **Extensional** → core `Scan` (`Branch::single` / push into core);
  `ColOrConst::Const` positions → `where_conds` constraints via `constrain`/`unify`.
- **InnerJoin** (natural join) → shared-var equality into `where_conds` as `SqlCond::ColEq`
  (reuse `unify`); children's cores concatenated.
- **LeftJoin** → see §5.3 (dispatcher).
- **Filter** → `IqCond::Expr` resolved HERE: `lower_filter_expr(expr, &branch) → where_conds`;
  `IqCond::Sql` passes through; `Exists`/`NotExists` → `SqlCond::Exists`/`NotExists` via
  `lower_exists`.
- **Construction** → `subst` folded entry-by-entry into `bindings`:
  `BindDef::Resolved(td) → insert td`; `BindDef::Expr(e) → insert bind_term_def(&e, &bindings)?`.
- **Path** → `Branch.path` (mutually exclusive with core).
- **Values** → core-less `Const` `Branch` (SELECT `<consts>`, no FROM); `UNDEF ⇒ absent var`.
- **Aggregation** → single-CQ child ⇒ `Branch.agg = Aggregation{keys, aggs}` (`emit_agg_branch`
  SQL `GROUP BY`); multi-branch (Union/Values) child ⇒ `Plan.rust_group = RustGroup{keys, aggs}`
  (executor groups in Rust). Same IR scope; only strategy differs by child branch count.
- **Union** → see §5.2.
- **Modifiers:** `Distinct → Plan.distinct`; `Slice → Plan.limit/offset`; `OrderBy → Plan.order`.
  `prepared_branches` copies onto `branches[0]` for the single-branch case (distinct always;
  limit/offset only when order empty; ORDER pushed to SQL only for `rr:column` terms else 501).

### 5.1 Per-branch FILTER/BIND resolution invariant (R4)

> **Refuted-and-restated** (ledger §9, item R4). "Reuse the proven fn ⇒ `=_bag`" is unsound if
> the fn is applied OUTSIDE the domain the flat path exercises, or against the wrong bindings map.

**Contract:**

1. LOWER applies each symbolic `IqCond::Expr` / `BindDef::Expr` **per resulting branch**, looping
   like `unfold.rs:136-142` — never against a single chosen bindings map over a still-multi-branch
   child. **Assert** the child has lowered to exactly ONE branch at the point an `Expr` is resolved
   (NORMALIZE §4(b) guarantees this by fully pushing FILTER/BIND through unions to leaf-CQs).
2. A variable whose per-arm binding is a NULL-pad / `Construction` pad entry (not a real
   `Derived{Column}` or genuine resolved term) is treated as **ABSENT** at LOWER, so the reused
   resolvers (`var_col`/`filter_cond`/`bind_term_def`) hit the **same `None`/`Err` path the flat
   oracle hits** — restoring exact flat parity (both 501) on the multi-width-union FILTER/BIND
   domain. A var bound to `Coalesce`/`Concat`/`Agg` is opaque to `unify`/`var_col`
   (`Unsupported`) — the filter/join on it defers to a tracked 501, never silently dropped or
   mis-resolved.

### 5.2 Union lowering — NO bound NULL-padding (R3)

> **Refuted-and-restated** (ledger §9, item R3). The `node.rs:84-88` invariant "every arm projects
> exactly `project`, NULL-padding narrower arms via a Construction" CONTRADICTS the flat oracle
> (`unfold.rs:145-150` `l.branches.extend(r.branches)` — a var absent from an arm is genuinely
> UNBOUND) and has no representation (`TermDef` has no unbound variant). Padding binds a var where
> the oracle leaves it unbound, diverging under DISTINCT, COUNT(*), BOUND(), and raw projection.

**Contract:**

1. LOWER emits **one `Branch` per arm carrying ONLY that arm's own bindings** (the
   `unfold.rs:145-150` `extend` behavior), appended to `Plan.branches` (bag union, ADR-0006;
   **NO SQL UNION node**).
2. A variable not bound by an arm stays **ABSENT** from that branch's bindings — genuinely
   unbound, never padded to a concrete `TermDef`.
3. `IqNode::Union.project` is **scope bookkeeping for parent resolution only**; it must NOT
   materialize as bound padding at lowering. The `node.rs:84-88` invariant is relaxed accordingly
   (§2.4). If an explicit unbound is ever truly needed, add a first-class `TermDef::Unbound`
   rendering as variable-absent — do not overload `Const`/`Derived`.

### 5.3 LeftJoin lowering — reuse `left_join_branches` verbatim (R1, R3)

> **Refuted-and-restated** (ledger §9, items R1, R3). DESIGN's "lower LeftJoin via
> `build_left_join`; multi-scan/Union OPTIONAL right → §5.1 SubPlan 501" is BOTH unsound (for the
> distributed case) AND a parity regression (the flat oracle DOES support multi-branch/multi-scan
> OPTIONAL right via the ISWC-2018 decomposition, `leftjoin.rs:64-86`).

**Contract:** lower a `LeftJoin` by handing `left: Vec<Branch>` and `right: Vec<Branch>` to
`crate::leftjoin::left_join_branches` (`leftjoin.rs:27`), which already routes correctly:

- right is empty ⇒ identity (`OPTIONAL {}` = noop);
- single-branch, single-scan, opt-free right ⇒ one `OptJoin` via `build_left_join`
  (NullSafeEq ON / R1, R5 inner-FILTER in `extra`, R2 `Coalesce` shared-var re-projection via
  `def_is_nullable`) — reused verbatim;
- multi-branch OR multi-scan right ⇒ `P OPT R = (P ⋈ R) ∪ (P − R)`: one inner-join branch per
  Ri plus exactly ONE no-match branch carrying the conjunction of `NOT EXISTS Ri`
  (`not_exists_cond_for`) — the no-match branch produced exactly once;
- nested OPTIONAL inside the right (`opts` non-empty) ⇒ tracked 501 (matching the flat oracle).

The symbolic `IqCond::Expr` in `LeftJoin.cond` (R5 inner FILTER) is lowered against the
**COMBINED left+right bindings**, NOT branch-local — `build_left_join`/`inner_join_one` already
do this; M3 must not change the scope.

### 5.4 §5.1 SubPlan derived table (M5/M7 scope — tracked sound-501 in M3)

The cases un-lowerable to today's `Branch`: subquery-as-join-operand;
Path-joined-with-pattern (and Agg-over-Path); nested `Aggregation`/`Distinct`/`Slice` as a join
input; multi-branch HAVING; variable graph. (Multi-scan/Union OPTIONAL right is **NOT** here — it
lowers via §5.3.) M3 emits each as `Error::Unsupported("… → 501")` **AT THE LOWER STEP** — never
a silent miscompile. They are expressible in the tree (scope known without flattening) but have
no `Branch` slot.

**Layering correction (R-SubPlan).** The lock literally specifies
`LogicalSource::SubPlan(Box<Plan>)`, which is a **CYCLIC crate dep** (`Plan ∈ sf-sparql`,
`LogicalSource ∈ sf-core`, `sf-sparql → sf-core` is one-directional). When built post-M3, the
derived-table variant goes at the **sf-sparql `Scan` level** (a `Scan`-local source enum or
`Scan::SubPlan` sibling), NOT `sf-core LogicalSource`. Render in `emit` `scan_ref` as
`( <nested SELECT> ) AS t{alias}` (mirror the existing `Query(q)` arm), splicing nested params in
text order; `alias_sources`/`branch_actuals` get a `SubPlan` no-op (columns are the nested
projection's `c{i}` labels, like the path CTE). Flag as a §5.1/§9 amendment to the design owner.

---

## 6. `=_bag` per stage

`=_bag` is structural, proven per stage, and gated end-to-end by the differential (§7).

- **RESOLVE.** Each `Intensional` → `Union`/`InnerJoin` of `Extensional`+`Construction` is
  exactly the flat `pattern_branches`/`atom` output: same arm set (per map × POM × pred×obj pair,
  plus class atoms), same refObjectMap 2-scan join, same conds, same fresh aliases (same `alias()`
  counter), same `predicate_can_match` pruning. The solution multiset is identical triple-by-triple
  because RESOLVE reuses the proven primitive verbatim, only emitting tree nodes instead of
  `Vec<Branch>`. OPTIONAL over a multi-arm right preserves `=_bag` via §5.3 (NOT distribution).
- **NORMALIZE-min.** Substitution-lifting is semantics-preserving renaming/merging (the tree form
  of `rewrite_alias`/`merge`), introducing no row and dropping none, and **never collapsing a
  `Union` arm** (R5). Join-over-union (§4.16) is the distributive law of join over BAG union
  (∪ = multiset concat, no dedup), restricted to InnerJoin (either arm) / Filter / LeftJoin-left
  (R1). The UNDEF-padded-shared-var rule (§4(b), R2) reproduces `merge`'s compatibility semantics
  so no cartesian arm is dropped. `Empty`/`True` pruning are bag identities. No DISTINCT is
  introduced (`Distinct` stays a `Plan` modifier).
- **LOWER.** Every fold step is the identity the flat path already proves: `Extensional → Scan`,
  `ColEq` via `unify`, `OptJoin`/decomposition via `left_join_branches`, and crucially
  `lower_filter_expr`/`bind_term_def` are the SAME functions the flat Filter/Extend arms call, now
  invoked per-leaf-CQ where each branch has one bindings map (§5.1), against ABSENT (not padded)
  bindings for unbound vars (§5.2, R3/R4). `Union → Vec<Branch>` is the same bag-union container
  (`Plan.branches`, ADR-0006 streaming); modifiers via `prepared_branches` are identical.

The whole M3 pipeline is a re-expression of `unfold.rs` as a tree fold; the lowered `Plan`'s
multiset equals the flat translation's multiset **by construction**, and the differential proves
it empirically — but the differential is **necessary, not sufficient** (§7).

---

## 7. Shadow differential + switch plan (M3d)

M3d builds the tree path ALONGSIDE the flat path; the flat `unfold.rs` stays the default and the
oracle.

1. Add a tree entry (`translate_tree`) producing `Plan` via build → resolve → normalize → lower,
   behind a flag. Do NOT remove or alter `unfold.rs`.
2. Add a differential test that, for every query, runs BOTH translators and asserts:
   (a) result MULTISETS are identical (sort rows, **compare with counts** — never set-dedup, which
   hides multiplicity bugs); (b) the tree path 501s on EXACTLY the queries the flat path 501s
   (same `Unsupported` set — no new silent passes, no new failures). Include modifier-interaction
   cases (LIMIT/OFFSET/ORDER/DISTINCT, single-branch SQL push vs multi-branch exec).

### Corrected: the differential is necessary-not-sufficient (R5)

> **Refuted-and-restated** (ledger §9, item R5). A green run over the finite W3C corpus SAMPLES
> `=_bag`; it does not PROVE the universal (∀ instance, ∀ multiplicity) property. The corpus is
> predominantly primary-keyed / set-like, so a multiplicity bug (overlapping triples-maps,
> duplicate-row union arms, non-unique join keys, NULL-pad over duplicates) never surfaces. A
> tree-vs-flat diff also compares code against near-identical code sharing the same primitives.

**Contract — close the coverage and independence gaps before relying on / retiring anything:**

1. **Multiplicity-stress fixtures** (force the differing multiplicity to actually appear):
   (i) duplicate-row tables feeding each union arm; (ii) overlapping/redundant triples-maps and
   multiple POMs emitting the SAME predicate over overlapping rows; (iii) non-unique join keys for
   self-join / refObjectMap join; (iv) OPTIONAL where the null-padded column carries duplicates;
   (v) aggregate-over-union and unbound-variable cases with multiplicity > 1.
2. **Property-based / randomized instance generation** over multiplicities (random duplicate rows
   + random overlapping maps), attacking the ∀-instance gap the fixed corpus cannot.
3. **Keep an INDEPENDENT oracle** in the gate (spareval, as `differential_oracle.rs` /
   `differential_paths.rs` already do) so a shared-primitive defect cannot hide from a tree-vs-flat
   diff.
4. **Forbid** any arm-merge / structural-dedup in substitution-lifting and §4.16 distribution
   unless proven multiplicity-preserving.

### Switch criterion (baked)

Switch the default to the tree path ONLY when the differential is fully green — **zero row-bag
diffs AND identical 501 sets** — across the W3C RDB2RDF + SPARQL corpus, the multiplicity-stress
fixtures (1), the property-based generator (2), validated against the independent oracle (3).
**Trust only the gated artifact; NEVER switch before green.** Keep the flat path in-tree as
fallback/oracle. Do **not** retire flat on the strength of a finite green window: either keep
flat as the permanent oracle/fallback, or gate retirement on a machine-checked `=_bag` argument
per NORMALIZE/LOWER rule (e.g. an explicit proof that no rule ever dedups a Union arm) — and only
in a separate change after at least one full release cycle green.

---

## 8. Sub-milestones M3a / M3b / M3c / M3d

| | Deliverable | Gate | First file → first compiling unit |
|---|---|---|---|
| **M3a RESOLVE** | `resolve(node, cx) -> Result<IqNode>` with ZERO `Intensional` leaves and established var→column scope, reusing the extracted `(Scan, bindings, conds)` atom primitive (§3.3). | Every `Intensional` replaced; arm set / conds / fresh aliases / `predicate_can_match` pruning identical to flat `pattern_branches` per pattern (unit diff); refObjectMap → 2-scan InnerJoin; FILTER/BIND untouched. | `crates/sf-sparql/src/iq/resolve.rs` → `pub fn resolve(node: IqNode, cx: &mut ResolveCx) -> Result<IqNode>`, the `Intensional` arm delegating to a `resolve_pattern` extracted from `unfold::atom`/`pattern_branches`. |
| **M3b NORMALIZE** | `normalize(node) -> Result<IqNode>` reaching the leaf-CQ spine (subst-lifting; §4.16 InnerJoin/Filter + LeftJoin-left only; Empty/True pruning). | Output is `Union`-of-(`Construction` over `Join`/`LeftJoin`/`Filter` of leaves); each branch has exactly ONE bindings map; NO LeftJoin-over-right-union split; UNDEF-padded shared vars handled per §4(b)/R2; no arm-merge; bag preserved. | `crates/sf-sparql/src/iq/normalize.rs` → `pub fn normalize(node: IqNode) -> Result<IqNode>`, starting with subst-composition (`Construction ∘ Construction`). |
| **M3c LOWER** | `lower(node, dialect) -> Result<Plan>`: per leaf-CQ → ONE `Branch`, reusing `emit`/`leftjoin`/`unify`. | Per-leaf-CQ → one `Branch`; LeftJoin via `left_join_branches` (§5.3); Union → `Vec<Branch>` with own-bindings, NO padding (§5.2); FILTER/BIND per-branch via `lower_filter_expr`/`bind_term_def` with absent-not-padded semantics (§5.1); §5.4 cases → `Unsupported` 501. | `crates/sf-sparql/src/iq/lower.rs` → `pub fn lower(node: IqNode, dialect: Dialect) -> Result<Plan>`, the `Extensional`/`InnerJoin` leaf-CQ fold first. |
| **M3d SHADOW** | `translate_tree` entry behind a flag + the differential harness (§7). | Green differential: zero row-bag diffs AND identical 501 set across W3C corpus + multiplicity-stress fixtures + property-based generator, validated vs the independent spareval oracle. | `crates/sf-conformance/tests/differential_tree.rs` (extend `differential_oracle.rs`) → a `#[test]` running both paths and asserting multiset + 501-set equality. |

---

## 9. Verification ledger

Five elements were submitted for `=_bag` certification; **all five were refuted as stated and are
RESTATED here — 0 survived unchanged, 5 corrected and baked into the contract above. No
deferrals.**

| # | Element | Verdict | Corrected restatement (baked) |
|---|---|---|---|
| R1 | Resolve: Intensional→Extensional + multi-TM Union, then OPTIONAL | refuted | LeftJoin is EXEMPT from join-over-union right-distribution; lower via `left_join_branches` (single-scan→`build_left_join`; multi-branch/multi-scan→ISWC-2018 `(P⋈R)∪(P−R)` + one `NOT EXISTS` no-match branch). §3.4, §4(b), §5.3. Differential: multi-TM OPTIONAL right with both sources empty ⇒ exactly ONE null-padded solution. |
| R2 | Normalize-min: subst-lifting + join-over-union | refuted | §4.16 distributive law is bag-faithful only with SPARQL compatibility: a Union arm marks an unbound projected var UNDEF/absent (never NULL column); a distributed InnerJoin/Filter treats an UNDEF-padded shared join var as unbound (NO equality, rebind from bound side — `merge` semantics, `unfold.rs:1161`); only vars bound in BOTH operands seed `ColEq`; else keep different-width arms or 501. §4(b). |
| R3 | Lower: per-leaf var→ColRef + Union→Vec\<Branch\> + Aggregation | refuted | Union lowers one `Branch` per arm with ONLY its own bindings; unbound var stays ABSENT, never padded to a concrete `TermDef`; `Union.project` is parent-scope bookkeeping, not bound padding; relax `node.rs:84-88`. Multi-branch/multi-scan OPTIONAL right lowers via the ISWC-2018 decomposition (NOT a §5.1 501). §5.2, §5.3. |
| R4 | IR refinements preserve `=_bag` (var-referencing conds/binds) | refuted | At LOWER treat any var whose per-arm binding is a NULL-pad as ABSENT, so `var_col`/`filter_cond`/`bind_term_def` hit the same `None`/`Err` (both 501) as the flat oracle on multi-width-union FILTER/BIND; apply each symbolic `Expr`/`BindDef::Expr` PER resulting branch (loop, `unfold.rs:136-142`), asserting a single-branch child; §4.16 InnerJoin-only, never LeftJoin-over-right-union. §5.1. |
| R5 | Shadow differential is the correct gate | refuted | Differential is NECESSARY-not-sufficient: add multiplicity-stress fixtures (dup-row union arms, overlapping/redundant maps + same-predicate POMs, non-unique join keys, NULL-pad over dups, agg-over-union, unbound mult>1); property-based randomized multiplicities; KEEP the independent spareval oracle; forbid unproven arm-merge/dedup; do NOT retire flat on a finite green window (permanent oracle, or machine-checked per-rule `=_bag`). §7. |

### §9 design-lock amendments to record before M3 merge

1. **`IqCond::Expr` / `BindDef` symbolic carriers (timing amendment).** The lock's §9 Option-B
   note ("resolution inline during build; lower FILTER/BIND inline via
   `lower_filter_expr`/`bind_term_def`") must be read as governing **triple resolution** and the
   **FLAT path**. The TREE defers FILTER/BIND lowering to **per-leaf-CQ LOWER** — the only sound
   timing, because a filter-over-union has no single scope at build (R2/R3/R4). The tree calls the
   IDENTICAL proven `lower_filter_expr`/`bind_term_def`, just at the point where each branch has
   exactly one bindings map. The `node.rs` doc-comments calling `IqCond`/`Construction.subst`
   "resolved at build" become "symbolic until LOWER (`Expr`) | resolved (`Sql`/`Resolved`)".
2. **`Union` invariant relaxation.** `node.rs:84-88` ("NULL-pad narrower arms via a Construction")
   is relaxed: arms keep their own bindings; an unbound projected var is ABSENT, never a concrete
   `TermDef`. `Union.project` is scope bookkeeping only (R3).
3. **§5.1 SubPlan layering.** `LogicalSource::SubPlan(Box<Plan>)` is a cyclic crate dep; the
   derived-table source belongs at the **sf-sparql `Scan` level**, rendered as
   `( <nested SELECT> ) AS t{alias}` in `emit` `scan_ref` (R-SubPlan, §5.4). Multi-scan/Union
   OPTIONAL right is removed from the §5.1 501 list (it lowers via §5.3).
