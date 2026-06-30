---
status: locked
date: 2026-06-30
locks: ADR-0023
tags: [ontop-parity, ir, iq, substitution-lifting, design-lock, bag-soundness, charter]
depends-on:
  - ADR-0004
  - ADR-0006
  - ADR-0007
  - ADR-0023
gate: "=_bag differential (PG↔SQLite) + W3C RDB2RDF floor (≥82/0) + clippy -D warnings + fmt --check"
---

# ADR-0023 Design-Lock — Native Operator-Tree IR

This is the **M1 contract** that M2–M8 build against. It is binding: a rule fires
**only** when its stated integrity-constraint precondition holds, otherwise it is a
**sound no-op**; `=_bag` (multiset-equivalence to the base translation) is the
**absolute** correctness invariant; every rewrite below carries a bag-soundness
argument that *survived* adversarial refute-only review, or has been **restated** with
the corrected precondition that closes the refutation (§8 is the ledger). The Ontop
source is **spec/oracle only** — names like `ConstructionNode` are conceptual anchors,
never transliteration targets (ADR-0004).

Authority: `docs/adr/ADR-0023-query-ir-architecture-flat-ucq-vs-iq-tree.md`,
`docs/HANDOVER-2026-06-30-ir-architecture-decision.md`.

---

## 1. Node set (the Rust enum)

One `enum IqNode`, exhaustive `match` everywhere; **no** trait objects / JVM class
hierarchy / DI. Unary children are `Box<IqNode>`, n-ary are `Vec<IqNode>`. Every node
computes a bottom-up `scope` on demand (projected vars + per-var nullability) — the
single invariant that dissolves the flat model's eager-flattening deferrals. Payloads
**reuse** the existing `iq.rs` types (`TermDef`, `SqlCond`, `AggKind`, `OrderKey`,
`Scan`, `PathClosure`) — no parallel term/condition machinery is introduced.

```rust
// crates/sf-sparql/src/iq/node.rs
pub type Var = Box<str>;            // matches Branch.bindings key domain

pub enum IqNode {
    // ---- unary substitution carrier (the heavy lifter) -------------------------
    /// Ontop ConstructionNode. `subst` REUSES TermDef as its var→term payload
    /// (ADR-0007 term-construction lifting). `project` is the declared projected set.
    Construction { child: Box<IqNode>, subst: BTreeMap<Var, TermDef>, project: Vec<Var> },

    // ---- unary boolean selection ----------------------------------------------
    /// FilterNode. Reuses SqlCond (3-valued FILTER: keep TRUE). Implicit AND.
    Filter { child: Box<IqNode>, cond: Vec<SqlCond> },

    // ---- n-ary / binary joins --------------------------------------------------
    /// InnerJoinNode: n-ary natural join on shared vars + optional joining cond.
    /// Identity = True (CONDITION-FREE only, §4.13), absorbing = Empty.
    InnerJoin { children: Vec<IqNode>, cond: Vec<SqlCond> },
    /// LeftJoinNode: binary, NON-commutative. `cond` is the OPTIONAL ON-expression.
    /// Right-only vars become nullable in scope. DESIGNATED 3-VL hotspot (§7).
    LeftJoin { left: Box<IqNode>, right: Box<IqNode>, cond: Vec<SqlCond> },

    // ---- n-ary bag union -------------------------------------------------------
    /// UnionNode (bag). INVARIANT: every child projects exactly `project`
    /// (children NULL-padded via Construction to the common signature). No dedup.
    Union { children: Vec<IqNode>, project: Vec<Var> },

    // ---- aggregation -----------------------------------------------------------
    /// AggregationNode. ONE construct for SQL-group and Rust-group paths (strategy
    /// chosen at lowering). Output scope = grouping ∪ agg vars, OWNED by the node
    /// (closes the agg-over-UNION binding-scope bug).
    Aggregation { child: Box<IqNode>, grouping: Vec<Var>, aggs: Vec<AggDef> },

    // ---- query-modifier spine --------------------------------------------------
    Distinct { child: Box<IqNode> },
    Slice    { child: Box<IqNode>, offset: usize, limit: Option<usize> },
    OrderBy  { child: Box<IqNode>, keys: Vec<OrderKey> },

    // ---- leaves ----------------------------------------------------------------
    /// ValuesNode: inline literal table (bag). None = UNDEF.
    Values { vars: Vec<Var>, rows: Vec<Vec<Option<TermDef>>> },
    /// ExtensionalDataNode: concrete mapped relation. REUSES Scan; sparse
    /// column→var/const binding + the relation's PK/UC/FK/FD constraints.
    Extensional { scan: Scan, bind: BTreeMap<Box<str>, ColOrConst> },
    /// IntensionalDataNode: unresolved triple/quad pattern. MUST NOT survive
    /// unfolding (replaced against T-mappings before normalization+lowering).
    Intensional { pattern: TriplePattern, graph: Option<GraphRef> },
    /// EmptyNode: ∅ over a declared var set. Union identity, InnerJoin absorbing.
    Empty { vars: Vec<Var> },
    /// TrueNode: one empty tuple. InnerJoin identity (condition-free only).
    True,

    // ---- specialized recursive leaf (NOT a generic node) -----------------------
    /// Property-path closure: PathClosure reused verbatim. Publishes a normal
    /// output scope so Join/Filter/Minus compose over it (kills the path 501s).
    Path { closure: PathClosure },
}

/// var := kind(arg) [DISTINCT], with §10 fixed type. `arg` is an EXPRESSION payload
/// (not bare Var) so SUM(?a+?b), GROUP_CONCAT(sep), SAMPLE are expressible (§8 gap-3).
pub struct AggDef {
    pub var: Var, pub kind: AggKind, pub arg: Option<AggArg>,
    pub distinct: bool, pub fixed_type: Option<XsdTypeCode>,
}
pub enum AggArg { Var(Var), Expr(TermDef) }     // pre-Extend may lower Expr→Var
pub enum ColOrConst { Col(Box<str>), Const(Term) }
```

**AggKind extension (M6):** add `GroupConcat { separator: Box<str> }` and `Sample`
to the existing `Count|Sum|Avg|Min|Max` (§8 gap-3). `COUNT(DISTINCT *)` rides
`AggDef.distinct`.

**NOT in the enum** (out of charter, ADR-0023 §The IR): `FlattenNode`/JSON unnest;
cost-driven translation selection; a generic `NativeNode` IR node — raw SQL exists
**only** as the Branch/emit lowering target (§5), never as an IR node. Tier-2
tree-witness rewriting stays excluded (ADR-0008).

---

## 2. spargebra-algebra → tree builder contract

`spargebra::algebra::GraphPattern → IqNode`, node-by-node, in `unfold.rs` (replacing
the eager `TransPattern`/`Vec<Branch>` flattening). Each arm builds a subtree and
publishes a bottom-up scope; **nothing** is distributed eagerly.

| `GraphPattern` | builds |
|---|---|
| `Bgp` / triple, `Path` (fixed pred) | `Intensional { pattern, graph }` leaf (resolved later by T-mapping unfold into Extensional/Construction/Union); variable-endpoint path → `Path { closure }`. |
| `Graph(NamedNode g, P)` | `build(P)` with `current_graph=g` pushed onto Intensional leaves' `graph` (constant graph only; **variable graph 501 at build**, §8 gap-5). |
| `Join(L,R)` | `InnerJoin { children:[build L, build R], cond:[] }`. NO cartesian over branch lists; union distribution deferred to normalization (§4.16). |
| `LeftJoin(L,R,expr)` | `LeftJoin { left:build L, right:build R, cond:lower(expr) }`. ONE node regardless of R shape (kills multi-scan/nested-OPTIONAL 501). |
| `Filter(expr,P)` | `Filter { child:build P, cond:lower(expr) }`. `EXISTS`/`NOT EXISTS` → `SqlCond::Exists`/`::NotExists` carrying the built inner subtree (semi/anti join). The normalizer MUST recurse into that payload (§3, §8 gap-2). |
| `Union(L,R)` | `Union { children:[build L, build R], project: ∪ child scopes }` (children Construction-padded to the common signature). |
| `Minus(L,R)` | `Filter { child:build L, cond:[NotExists(build R correlated on shared BOUND vars)] }` (disjoint-domain pair = no-op). |
| `Group(P,keys,aggs)` | `Aggregation { child:build P, grouping, aggs }`. ONE rule for single- and multi-branch inner; the node owns its scope (deletes `Plan.rust_group` side-channel + single-vs-multi fork). Aggregate **arguments that are expressions** are pre-bound by an inner Construction so `AggArg` resolves (§8 gap-3). |
| `Extend(P,var,expr)` | Construction over `build(P)` adding `var→TermDef(lower expr)`. Agg out-vars resolve from the Aggregation node's published scope (fixes agg-over-UNION BIND-unbound). |
| `Project(P,vars)` | `Construction { child:build P, project:vars }`. |
| `Distinct`/`Reduced(P)` | `Distinct { child:build P }` (Reduced = same, sound). |
| `Slice` / `OrderBy` | `Slice` / `OrderBy` (OrderKey reused). |
| `Values(vars,rows)` | `Values` leaf. |

**HAVING (§8 gap-4):** spargebra emits `Filter(Group(...))`; it builds via the uniform
`Filter` arm and the normal-form spine reserves a **post-Aggregation Filter slot**
(§3). SubQuery: a Project/Distinct/Slice/OrderBy/Aggregation chain is a real subtree,
so an inner modifier cannot leak when composed under Join/Union (fixes the flat
scope-leak).

---

## 3. Normalization contract (substitution-lifting to fixpoint)

**Goal — the normal form.** Drive any `IqNode` to *union-of-CQs-with-term-constructors*:
a root **modifier spine** over a Union of leaf CQs, each CQ a Construction over an
InnerJoin/LeftJoin/Filter of Extensional/Values/Path leaves — exactly what one `Branch`
lowers from. The locked spine, **outermost→innermost**, is:

```
Slice? / OrderBy? / Distinct? / Filter[HAVING]? / Aggregation? / Construction[project] / Union{ leaf-CQ … }
```

The post-Aggregation `Filter` (HAVING) slot is part of the locked spine (§8 gap-4).

**Engine = substitution-lifting to a fixpoint.** A Construction's `TermDef`
substitution composes INTO its child and lifts toward the root:
Construction∘Construction folds to one (compose substitutions, intersect projections);
a Construction lifts through InnerJoin/Filter/Union by composition (σ over A∪B = σA∪σB;
σ commutes with join/filter on its raw-key substrate). Variable equivalence classes
from `ColEq`/join equalities propagate as substitution entries — the flat
`rewrite_alias`/`rename_col_in_term_map` dance becomes "apply the variable
substitution at the Construction," and the load-bearing "generated IRI stays
byte-identical" argument reduces to "equal columns share a variable" **subject to the
term-coherence precondition of §4.6**.

**Structural drivers run interleaved with lifting:** push Union toward the root
(distribute Join/LeftJoin/Filter over Union arms — §4.16, the lazy replacement for
`unfold.rs` eager `join_branches`), propagate Empty/True via their monoid identities
(§4.13), and fire every constraint-gated rewrite (§4) bottom-up.

**Recursion into semi/anti-join payloads (§8 gap-2, BINDING).** The normalizer MUST
descend into the subtree carried by `SqlCond::Exists`/`NotExists` (and the
`Minus`-derived `NotExists`) and normalize it as a first-class IqNode, so self-join /
FK / empty-prune / constraint rules apply inside it. Each rewrite there carries the
same `=_bag` argument as in a positive position **plus** the §4.2 positional caveat
(Empty inside a NotExists ⇒ the NotExists is TRUE, never "drop the arm").

**Pass-ordering / termination.** Bottom-up traversal, re-firing on any changed subtree,
to a global fixpoint — subsuming the flat cascade's hand-ordered sweep, its twice-run
pass-1, and every `while let Some(..)` loop. The five flat order-constraints become
automatic: (1) path/agg/anti-join *bypass* → node-kind dispatch (rules don't match
those nodes); (2) "prune empties before self-join, re-run after merge" → re-firing
after each rewrite; (3) the 2a→2e sub-order → one self-join-elimination rule family
parameterized by constraint kind (§4.3); (4) FD inference → cached bottom-up
constraint propagation (§4.7) the join-elim rules query; (5) projection shrink →
automatic, Project is the outermost Construction (§4.12). DISTINCT-context becomes an
**ancestor-node property read off the tree** — and specifically a *distinct-bounded,
aggregation-free* predicate, not a bare boolean (§4.15, §8 gap-13).

**Termination measure.** Substitution composition is confluent toward a canonical
form; every structural rule strictly reduces a well-founded measure (node count, or
union width for empty-branch drop, or DISTINCT/redundant-leaf count) or is idempotent
(filter placement). The fixpoint is reached in finite steps. Each §4 rule states its
measure; no rule may both create and consume its own match pattern without a strict
decrease (the §7 termination risk).

---

## 4. Rule catalogue

Every rule: **transform / precondition / `=_bag` argument / ported-from**. Constraint
sets come from §4.7. On empty/missing schema every gated rule degrades to a **sound
no-op** (the inert path is a gate, ADR-0023 §Confirmation).

### 4.1 tier0-refobjectmap-inline (REFUTED → RESTATED)

* **Transform:** at the Intensional→Extensional unfold, a *join-free* `refObjectMap`
  inlines the parent IRI as a Construction substitution built from the **CHILD** alias
  (no parent Scan emitted). The `parent==child` PK self-join, when a real join exists,
  is handled by §4.3.
* **Precondition:** R2RML join-free referencing object map (`r.joins.is_empty()`),
  whose semantics mandate *child query == parent query*. The unfold MUST special-case
  `r.joins.is_empty()` and inline from the child alias — **OR** synthesize a
  `NullSafeEq` self-equality on the shared key so §4.3 collapses the parent scan. One
  of these must hold; without it the rule does NOT apply (no empty no-op stub).
* **`=_bag`:** with child-alias inlining, the joint query *is* the child query (R2RML),
  so the diagonal `{(ex/i,p,ex/i)}` is produced, not the `|T|²` cross product. An
  equality substitution emitting byte-identical terms changes no rows/multiplicities.
* **Ported-from:** Pass 0 `tier0_eliminate` (cascade/mod.rs:147) — DISSOLVES into
  unfold + §4.3. **Refutation:** the old "subsumed" no-op stub was unsound — unfold
  allocated a fresh parent alias with **zero** equalities (`r.joins` empty ⇒
  unconstrained cross product, |T|²≠|T|). Fix baked in above.

### 4.2 unsat-equality-prune → Empty (REFUTED → RESTATED)

* **Transform:** when a var's equivalence class (seeded from `Filter Cmp(=,const)`,
  propagated through InnerJoin/`ColEq`) is forced to two distinct constants, rewrite
  that CQ subtree to `Empty{vars}`; then propagate Empty per §4.13.
* **Precondition (POSITIONAL — corrected):** Empty absorbs **upward** (drops the
  Union arm / empties the parent) **only when the contradictory subtree is in a
  POSITIVE position** — a Union arm or an InnerJoin operand. Otherwise Empty is reduced
  by the operator-correct law, NOT dropped: `LeftJoin(L,Empty) ≡ L` with right-only
  vars NULL-bound; `Minus(L,Empty) ≡ L`; `FILTER NOT EXISTS{Empty} ≡ TRUE`. Seeding
  must not cross into the right child of a LeftJoin/Minus/NotExists as if positive.
  Schema-free.
* **`=_bag`:** a contradictory equality set ⇒ the arm's stream is empty; in a positive
  position removing it removes exactly zero tuples (bag union/join unchanged). In a
  non-preserving position the operator-correct law is the exact identity.
* **Ported-from:** Pass 1 `prune_iri_template_mismatch` (mod.rs:163, run twice) — the
  twice-run becomes re-firing. **Refutation:** the generalized "Empty propagation drops
  the Union arm" over-reached through a LeftJoin right side (2 rows → 0). Positional
  precondition baked in.

### 4.3 self-join-elimination (unified: unique-key | nullable-unique | composite-PK | FD) (REFUTED → RESTATED)

* **Transform:** two `Extensional` children of the SAME relation under an InnerJoin
  agreeing on a key column-set fuse into one Extensional (union of column maps); the
  licensing equalities vanish (substitution rebinds the dropped leaf's vars). If the
  determinant is **nullable**, attach `Filter IS NOT NULL(key)` above the merged leaf.
* **Precondition:** the equated column-set is a declared unique key (PK/single-col
  UC/composite-UC) **OR**, under an ancestor Distinct, a non-unique FD determinant
  `C→dep` with every dropped-leaf-reading var confined to `{C}∪dep`. **A nullable
  determinant — whether UNIQUE key OR FD determinant — requires `C` declared NOT NULL
  or a synthesized `IS NOT NULL(C)` above the merged leaf** (corrected: the IS-NOT-NULL
  synthesis applies to the FD branch too, not only nullable-unique). Else sound no-op.
* **`=_bag`:** non-null unique key ⇒ exactly one matching row ⇒ join multiplies
  nothing, only adds columns of an already-read row. Nullable case: the equi-join's
  `NULL=NULL⇒UNKNOWN` excludes NULLs; the synthesized `IS NOT NULL` re-imposes exactly
  that exclusion after merge. FD/DISTINCT: per non-null `C` the FD forces identical
  `dep`, so n copies dedup to the same set; NULL-`C` rows stay excluded by the guard.
* **Ported-from:** Passes 2a, 2a-ext (nullable), 2c (sameterm-under-DISTINCT), 2e/1c
  (FD) — six flat variants unified. **Refutation:** the FD branch lacked the
  nullable-determinant guard (∅ vs `{(NULL,1),(NULL,2)}`). Guard extended to FD branch
  (matches shipped wave-1c). 

### 4.4 self-leftjoin-elimination (unified: non-null-UC | FD-under-DISTINCT | lj-contradiction) (REFUTED → RESTATED)

* **Transform:** a LeftJoin whose right is an `Extensional` of the SAME relation as a
  left `Extensional`, joined on a shared non-null unique key (or FD determinant under
  Distinct), collapses to the left with right-only vars rebound. lj-contradiction: a
  right-side constant conflicting with a left-side constant on the same PK cell ⇒ right
  → Empty ⇒ LeftJoin normalizes to left + NULL-pad.
* **Precondition:** right is a single same-relation Extensional; ON is exactly the
  shared key (`NullSafeEq`/`ColEq`); that key is a non-null single-col UC (or FD
  determinant + ancestor Distinct + binding confinement to `{C}∪dep` + empty
  right-FILTER, FD-vacuity ⇒ NOT-NULL required). **The lone-`IsNotNull(c)`-extra
  exception fires ONLY when the OPTIONAL binds no column other than the filtered key
  column `c`** (corrected): if the opt scan binds any other column, the OptJoin+filter
  must NOT be dropped (the filter failure must NULL-pad the *entire* variable group).
  Nullable determinant refused. Sound no-op otherwise.
* **`=_bag`:** non-null UC ⇒ the optional row IS the left row ⇒ LEFT JOIN matches once,
  adds columns of an already-read row (null-safe IS-NULL disjuncts dead for NOT-NULL).
  lj-contradiction: PK equality + conflicting constants ⇒ never matches ⇒ right vars
  always NULL ⇒ drop + NULL-pad exact.
* **Ported-from:** Pass 2b + `lj_contradiction_elim` (mod.rs:326), 1b FD self-LEFT-join
  (mod.rs:617). **Refutation:** the IsNotNull exception leaked a sibling non-null
  column's real value `(1,NULL,'x')` where the base NULL-pads the whole group
  `(1,NULL,NULL)`. Single-bound-variable gate baked in.

### 4.5 distinct-prune-unused-leftjoin (REFUTED → RESTATED)

* **Transform:** under an ancestor Distinct, a LeftJoin whose right subtree contributes
  no variable observed above it is rewritten to its left child.
* **Precondition (strengthened):** the right subtree's scope is **globally dead above
  the LeftJoin** — every right var is referenced NOWHERE outside the right subtree: not
  in any intervening FILTER, BIND/Extend, ORDER BY, GROUP BY/aggregate, or outer join
  condition between the LeftJoin and the dominating Construction, **and** not projected.
  Equivalently: the right output feeds only the dominating Construction's bindings (the
  structural invariant flat pass 2d enjoyed). Ancestor Distinct required. Schema-free.
* **`=_bag`:** under DISTINCT, per left row k right matches give k identical projected
  tuples (⇒1) and 0 matches gives 1 NULL-extended tuple (⇒1), so
  `DISTINCT∘(L⟕R) ≡ DISTINCT∘L` on projected columns; a LEFT JOIN never drops a left
  row. Global-deadness ensures unboundedness of right vars is unobservable.
* **Ported-from:** Pass 2d `distinct_prune_unused_opts` (mod.rs:584). **Refutation:**
  "disjoint from projected set" alone let `FILTER(!BOUND(?y))` observe the dropped var
  (∅ vs `{x=1}`). Strengthened to global-deadness.

### 4.6 fk-pk-join-elimination (single + composite) (REFUTED → RESTATED)

* **Transform:** an InnerJoin dropping a PARENT `Extensional` reached only via a
  NOT-NULL FK to the parent's unique key: remove the parent leaf, substitute every
  parent-PK var := child-FK var at the Construction.
* **Precondition:** UNIQUENESS (parent join column is a key — cached constraint AND
  catalog `is_unique_key` agree) AND MATCH-GUARANTEE (child FK NOT NULL + declared RI)
  AND parent referenced only via the join/PK column(s) (composite adds positional FK
  alignment + completeness). **PLUS TERM-COHERENCE (new):** the FK and PK columns must
  produce byte-identical RDF lexical forms on matching rows — equal declared SQL type
  with a **deterministic/binary collation** for strings, no `CHAR(n)` blank-padding
  divergence (both `CHAR(n)` of equal n, or both `VARCHAR`/text), identical `NUMERIC`
  precision/scale (per column for composite). If the catalog cannot prove
  term-coherence, **sound no-op** (keep the parent leaf).
* **`=_bag`:** deterministic-collation + matching type/length ⇒ join-equal values are
  lexically identical ⇒ rewritten term map is byte-identical; uniqueness ⇒ no
  multiplication; NOT-NULL FK + RI ⇒ exactly one parent per child ⇒ no rows dropped.
* **Ported-from:** Pass 4 `fk_pk_join_elimination` + `find_multi_fk_pk_join`
  (joinelim.rs:92); pass 3 FD → §4.7. **Refutation:** SQL join-equality ≠ lexical
  equality under case-insensitive collation/CHAR-padding/NUMERIC scale
  (`:country/GB` vs `:country/gb`, row count identical). Term-coherence precondition
  baked in.

### 4.7 constraint-propagation (analysis, not a rewrite)

* **Transform:** compute per-node, bottom-up and **cached**: `VariableNullability`,
  unique/superkey sets, FD closure (Armstrong: equality copy across `ColEq`-equated
  vars + transitivity). Decorates the tree; the uniqueness/nullability oracle all
  join-elim and DISTINCT-removal rules consult. **Cache invalidation is mandatory:** any
  rewrite that changes a subtree invalidates the cached constraints of that node and its
  ancestors before they are re-read (the §7 stale-cache risk).
* **Precondition:** a catalog of PK/UC/FK/FD; empty schema ⇒ empty sets ⇒ all gated
  rules become sound no-ops.
* **`=_bag`:** produces no rewrite; soundness rests on `ColEq` ⇒ equal values on every
  surviving row (equality rule) + standard FD transitivity.
* **Ported-from:** Pass 3 `infer_functional_dependencies` (fd.rs); per-branch fixpoint
  dissolves into incremental per-node propagation.

### 4.8 filter-pushdown-and-conjunct-split (REFUTED → RESTATED)

* **Transform:** flatten nested AND so each conjunct is independent; attach each
  conjunct directly above the node whose variables it constrains (pushed toward leaves).
* **Precondition:** structural, idempotent. **UNION (corrected):** a conjunct crosses a
  Union node ONLY as a per-branch distribution `σ_p(⊎Bᵢ) = ⊎σ_p(Bᵢ)` — push a COPY onto
  **every** branch; **never attach a conjunct to a proper subset of a Union's
  branches.** A conjunct may be sunk below a single branch/leaf only if every variable
  it references is provably bound in every branch the Union (and any enclosing OPTIONAL
  right side) can produce. **LeftJoin:** NEVER push a predicate below a LeftJoin right
  side unless it references only provably-non-null vars there (3-VL UNKNOWN-vs-FALSE).
* **`=_bag`:** AND is commutative/associative; selection distributes over union
  *as a per-branch copy* (`mult` of t in `σ(A∪B)` = `[p(t)]·(mult_A+mult_B)`). R5
  (OPTIONAL preservation) is structural — the LeftJoin holds its own ON; an outer
  filter is never pushed onto the preserved side.
* **Ported-from:** Pass 5 `selection_pushdown` (mod.rs:841). **Refutation:** sinking a
  conjunct into the one binding branch of a live Union dropped the error-removal on the
  non-binding branch (`?x>5` over a UNION: 1 row → 2 rows). Per-branch-distribution
  precondition baked in.

### 4.9 filter-lift-across-boundaries (NEW) (REFUTED → SPLIT)

* **Transform (split):** **(a) Union lift** — lift a predicate out of all Union arms
  when `σ(A∪B)=σA∪σB` holds (predicate references only attributes bound with identical
  null-behavior in every arm). **(b) LeftJoin: do NOT move filters across the join.**
  Only (b1) a *preserved-side-only* predicate may move freely across the LeftJoin
  (both directions); (b2) downgrade is recognized **in place** on a predicate already
  positioned ABOVE the LeftJoin (§4.10) — no predicate is lifted from below the join.
* **Precondition:** (a) exact union distribution; (b1) predicate references only
  preserved (left) attributes; (b2) the predicate is already above the join. The
  incoherent "lift a right-only predicate to enable the downgrade" clause is **removed**.
  FK-join-transfer requires the FK integrity constraint explicitly (else no-op).
* **`=_bag`:** selection distributes over union exactly; preserved-side movement is
  multiplicity-neutral. Moving a right-(null-)rejecting predicate from below to above
  the NULL-introduction boundary is **forbidden** (it strips the NULL-padded
  preservation row: `{(1,NULL)}` → `∅`).
* **Ported-from:** NEW (Ontop FilterLifter). **Refutation:** the only predicates a
  LeftJoin lift can move are single-input — left-only (useless for downgrade) or
  right-only (unsound). Rule split into the sound (a)/(b1)/(b2) forms.

### 4.10 leftjoin-to-innerjoin-downgrade (NEW, generalizes 2b-pre) (REFUTED → RESTATED)

* **Transform:** rewrite `LeftJoin → InnerJoin` when (disjunct-1) a null-rejecting
  ancestor Filter on a right-only variable dominates the join, OR (disjunct-2) a
  NOT-NULL FK guarantees the right side is always matched.
* **Precondition:** **Disjunct-2 (FK): the LeftJoin ON must be EXACTLY the single
  FK=PK (Null)SafeEq with NO in-OPTIONAL FILTER** — i.e. `opt.extra.is_empty()` /
  no additional ON conjunct (corrected; mirrors `find_fd_self_left_join` precond-1). An
  in-OPTIONAL filter makes the match conditional, so RI guarantees the parent *row*
  exists, not a *full-condition* match. **Disjunct-1: "right-only variable" means a
  variable provided EXCLUSIVELY by the right operand** (never COALESCE-shared with the
  left), AND no cardinality/null-sensitive operator (aggregation, DISTINCT consuming
  the padded row, a further LeftJoin) sits between the LeftJoin and the dominating
  filter. Else sound no-op.
* **`=_bag`:** if every NULL-padded row is eliminated by a null-rejecting ancestor (or
  no padded row is produced because the match is FK-guaranteed with unconditional ON),
  then `L⟕R` and `L⋈R` have identical bags.
* **Ported-from:** Pass 2b-pre `lj_to_ij_fk_downgrade` (joinelim.rs:11) generalized.
  **Refutation:** the FK path hoisted `opt.on.chain(opt.extra)` without checking
  `extra`, turning a conditional in-OPTIONAL filter into an inner WHERE (1 → 0). Plus
  "right-only" conflated appears-in-right with exclusively-right (COALESCE). Both guards
  baked in. **This is the designated 3-VL hotspot (§7).**

### 4.11 disjunction-intersection-simplify (REFUTED → RESTATED)

* **Transform:** two conjunct Filters that are same-column equality disjunctions
  (`Or[Cmp(col,=,v),…]`) on the same column are replaced by their value-set
  intersection (singleton ⇒ bare `Cmp`; empty ⇒ Empty).
* **Precondition (new):** all constants across both disjunctions on `col` are of one
  statically-known datatype/domain AND normalized to that domain's **canonical lexical
  form** (canonical numeric / canonical IRI/typed-literal, column collation applied) —
  intersect on canonicalized values, NOT raw `String`s. Equivalently restrict to
  columns whose value-equality is provably lexical-identity (same datatype, canonical
  forms, binary/no-coercion collation, no numeric cross-type promotion). Else **leave
  the two Filters unchanged** (sound no-op).
* **`=_bag`:** with membership and intersection sharing one equality,
  `(a∈S)∧(a∈T) ≡ (a∈S∩T)` holds; singleton/Empty reductions become bag-sound.
* **Ported-from:** Pass 5b `disjunction_intersection_simplify` (mod.rs:1082).
  **Refutation:** Rust `String` intersection is finer than engine value-equality —
  `"1"` vs `"1.0"` on a numeric column intersected to ∅ and pruned the branch (1 row →
  0). Canonicalization precondition baked in.

### 4.12 distinct-removal (REFUTED → RESTATED)

* **Transform:** delete a Distinct when the projected-variable set is provably a
  superkey of its child given cached constraints.
* **Precondition:** the projected tuple is duplicate-free — a NOT-NULL UC/PK/FD covers
  the key over the projected (injective) term definitions; no OPTIONAL hides non-key
  projections; nullable UNIQUE refused. **Injectivity oracle (corrected): `TermDef::
  Concat` and `TermDef::Coalesce` count as injective ONLY when they read at most the
  single key column**; otherwise non-injective ⇒ they cannot discharge a key.
  Equivalently the key-covering binding must read EXACTLY the key column(s)
  (Const/Column/injective-Template/single-operand-Concat over just the key).
* **`=_bag`:** an injective function of a unique NOT-NULL key yields distinct tuples per
  source row, so DISTINCT is a genuine no-op. NOT-NULL is load-bearing (nullable unique
  would let removal ADD rows).
* **Ported-from:** Pass 6 `distinct_removal` + `binding_is_injective` (mod.rs:888).
  **Refutation:** `binding_is_injective` whitelisted multi-column `Concat`/`Coalesce`
  (CONCAT is many-to-one): `{x="abc"}×1` → `×2`. Concat/Coalesce gated to ≤1 key column.

### 4.13 empty-true-propagation (NEW) (REFUTED → RESTATED)

* **Transform:** InnerJoin with an Empty child ⇒ Empty over the **union of all
  children's vars** (absorbing); Union drops Empty arms (keeping their var signature);
  **LeftJoin with Empty right ⇒ left + NULL-substitution.** True handling is
  conditional (below).
* **Precondition (corrected — True clauses are gated):** "all-True join ⇒ True" and
  "drop True children" apply **only to a condition-free InnerJoin** (cond ≡ TRUE).
  First split any `InnerJoin.cond θ` into an explicit `Filter[θ]` above a condition-free
  join and constant-fold θ (θ≡false ⇒ Empty; θ≡true ⇒ drop); only then propagate. The
  Empty/absorbing and Union-arm-drop and LeftJoin-Empty-right clauses are unconditional
  monoid identities.
* **`=_bag`:** `∅⋈R=∅`, `∅∪R=R`, `1⋈R=R` are exact multiset identities; an absorbing
  InnerJoin must carry the union var signature for a well-formed projection. A
  *conditional* all-True join is `Filter[θ](True)` = True iff θ true, Empty iff θ
  false/NULL — NOT unconditionally True.
* **Ported-from:** NEW (Ontop Empty/True propagation); implicit in flat pass-1
  branch-drop. **Refutation:** "all-True ⇒ True" discarded a non-tautological joining
  condition `(c1=c2)` (0 rows → 1). Condition-free gating + signature caveat baked in.

### 4.14 aggregation-through-union (NEW — closes the agg-over-UNION bug) (REFUTED → RESTATED)

* **Transform:** `Aggregation(Union(b1..bn))` is the legal normal form; output scope
  (grouping ∪ agg vars) is OWNED by the Aggregation node; an outer Extend/Project
  resolves agg vars from it. **Optional push-down:** partial per-branch aggregates below
  the Union with a merge above.
* **Precondition (scope fix):** none (base composition the flat model lacked).
  **Push-down (corrected):** branches disjoint on the grouping key (distinct IRI
  templates) **AND the grouping key is provably bound (non-nullable / IS NOT NULL) in
  every branch, or at most one branch can yield an unbound grouping key** — otherwise
  the template-less shared NULL group spans branches and a non-mergeable aggregate
  (MAX/MIN/AVG/SAMPLE) double-counts; OR an algebraically-mergeable aggregate
  (SUM-of-COUNTs) that re-aggregates above. Else keep the single Aggregation over the
  Union.
* **`=_bag`:** the Union child is bag-faithful (concatenation), so the full multiset
  feeds the group; aggregation soundness rests on the unchanged term-lifting
  injectivity precondition (raw-key grouping ≡ constructed-term grouping, ADR-0007). The
  non-nullable-key clause prevents miscounting the cross-branch NULL group.
* **Ported-from:** NEW (Ontop AggregationSplitter); deletes `Plan.rust_group`
  side-channel + the `rust_group_plan ..t` path (unfold.rs:304,830). **Refutation:**
  "distinct templates ⇒ disjoint groups" ignored the template-less unbound group
  (`MAX` over two unbound-key branches: 1 row → 2). Non-nullable-key clause baked in.

### 4.15 union-branch-merging-and-binding-lift (NEW) (REFUTED → RESTATED)

* **Transform:** lift bindings common to ALL Union arms into a shared Construction
  above the union; fold arms differing only in a constant/template into a `Values`
  node/shared Construction **keeping all rows**; deduplicate exactly-identical arms only
  in a duplicate-insensitive context.
* **Precondition:** lifting a common binding: always sound (identical, deterministic,
  single-output substitution present in every arm). Folding: must preserve every row
  (`Values` keeps both). **Arm-dedup (corrected): fires ONLY when the path from the
  Union up to the nearest dominating Distinct contains ONLY duplicate-insensitive
  operators** — no Aggregation/GROUP BY/COUNT, no bag-cardinality consumer. The context
  predicate is *distinct-bounded AND aggregation-free*, not a bare `distinct: bool`.
* **`=_bag`:** `σ(A∪B)=σA∪σB` (lift exact); folding into Values preserves multiplicity;
  dedup is set-sound only and thus restricted to positions where bag multiplicity is
  unobserved before the dominating Distinct.
* **Ported-from:** NEW (Ontop UnionAndBindingLift / UnionBasedQueryMerger).
  **Refutation:** a bare "set context" let dedup fire with a GROUP BY/COUNT between the
  union and the DISTINCT (`COUNT(*)` over duplicated arms: `(a,4)` → `(a,2)`).
  Aggregation-free path precondition baked in.

### 4.16 join-over-union-distribution (normal-form driver) (REFUTED → RESTATED)

* **Transform:** distribute over Union arms LAZILY during normalization, after
  empty-branch pruning and constraint-gated reductions.
* **Precondition (corrected — per operator/arm):** **InnerJoin and Filter** may
  distribute over a Union in **either** operand (bag-exact). **LeftJoin may distribute
  ONLY over a Union in its LEFT (preserved) operand**: `(A∪B)⟕C → (A⟕C)∪(B⟕C)` sound;
  `A⟕(C∪D)` must **NOT** be split. Ordered AFTER unsat-prune/empty-propagation so the
  mᵗ blowup is bounded by surviving arms (T-mapping offline pruning narrows union width
  first — a soft/cost ordering, §7).
* **`=_bag`:** `⋈` distributes over bag union exactly; a LeftJoin over a **right** union
  does **not** (`A⟕(C∪D)` fabricates a spurious `(x=1,y=NULL)` padded row: 1 → 2).
* **Ported-from:** replaces eager `unfold.rs join_branches` (unfold.rs:115,1147).
  **Refutation:** the generic 4-way `(A∪B)⋈(C∪D)` schematic applied to LeftJoin over
  the non-preserved side is unsound. Restricted to InnerJoin (either arm) and LeftJoin
  (left arm only).

---

## 5. Lowering contract (tree → Branch/emit)

Normalized IR → SQL **reuses** the existing `Branch`/`emit` path (iq.rs:463); the tree
is the optimizer model, `Branch` is the lowering target. No re-implementation.

* A normalized **leaf CQ** — Construction over an InnerJoin/LeftJoin/Filter of
  Extensional/Values/Path leaves — lowers to ONE `Branch`:
  * InnerJoin Extensional leaves → `Branch.core` (`Vec<Scan>`); join equalities →
    `SqlCond::ColEq` in `Branch.where_conds`.
  * each LeftJoin → one `Branch.opts` `OptJoin` (single-scan right in v1). ON →
    `OptJoin.on` (**`NullSafeEq`, R1**); an inner FILTER → `OptJoin.extra` (**R5**);
    R2 shared-var → `TermDef::Coalesce` in bindings.
  * Filter conjuncts → `Branch.where_conds`; EXISTS/NOT EXISTS → `SqlCond::Exists`/
    `NotExists` (semi/anti join).
  * the Construction substitution lowers ENTRY-BY-ENTRY into existing `TermDef`
    variants in `Branch.bindings` (Const/Derived/Coalesce/Concat/Agg) — NOT a new term
    builder. Preserves ADR-0007 (raw-ColRef joins/filters, RDF terms materialized only
    at the outer projection via sf-core), ADR-0006 streaming, ADR-0010 bound-parameter
    discipline. `TermDef::columns()`/`Branch::projection()` stay the projection oracle.
  * a `Path` leaf → `Branch.path` (recursive-CTE FROM, empty core).
* `Union` → existing `Vec<Branch>` bag union (SQL `UNION ALL` / multi-branch stream).
* `Aggregation` → `Branch.agg` (single-CQ child ⇒ SQL `GROUP BY`) OR the executor's
  Rust group path (multi-branch Union child); the IR shape/scope are identical, the
  choice is a lowering-strategy detail. `GROUP_CONCAT`/`SAMPLE` lower via the Rust group
  path (§8 gap-3).
* HAVING (post-Aggregation Filter, §3) → SQL `HAVING` for the SQL-group path, else via
  the derived-table wrapper (below). `=_bag`: selection over grouped output.
* `Distinct`/`Slice`/`OrderBy` → today's `Plan.distinct` / `Plan.limit+offset` /
  `Plan.order`, honoring the existing single-vs-multi split.
* `Values` → core-less `Branch`(es) (`TermDef::Const` cells; UNDEF ⇒ absent var).

### 5.1 Derived-table lowering target (BLOCKER resolution — §8 gap-1)

The IR makes subquery-as-join-operand, multi-scan & Union OPTIONAL right sides,
Path-joined-with-a-pattern, and Aggregation/Distinct/Slice subqueries-as-join-input
**expressible**. `Branch` cannot represent any of these (`core` is `Vec<Scan>` over
`LogicalSource = Table|Query`; `path` is mutually exclusive with `core`, iq.rs:417;
`opts` is single-scan). The contract therefore **adds an explicit derived-table
relation**: extend the lowering target with `LogicalSource::SubPlan(Box<Plan>)` (a
`Scan` whose source is a nested `Plan`), so a normalized non-leaf subtree
(Aggregation/Distinct/Slice/Union/Path used as a join input or OPTIONAL right) lowers
to a SQL **derived table** (`( <nested SELECT> ) AS alias`) joined/left-joined in the
parent. `=_bag` argument: a derived table is bag-faithful (the nested `Plan` already
preserves `=_bag`; the outer join over it is the ordinary InnerJoin/LeftJoin bag
identity, §4). **Until `SubPlan` is built (target M7), each such shape lowers as a
tracked sound-501**, never a silent miscompile — enumerated in §5.2.

### 5.2 Expressible-but-not-yet-lowerable boundary (tracked sound-501s)

Each is a named sound-501 with a test until its lowering is built; none may fall
through to a wrong answer (the no-deferrals mandate is satisfied by the explicit-501
*plus* the §5.1 mechanism that retires them):

1. multi-scan / Union OPTIONAL right side → `SubPlan` LEFT JOIN.
2. SPARQL SubQuery used as a join operand → `SubPlan` JOIN.
3. Path joined with a pattern / Aggregation-over-Path → `SubPlan` (path keeps its CTE).
4. nested Aggregation → `SubPlan`.
5. HAVING when the SQL-group path is unavailable (multi-branch) → `SubPlan` + outer Filter.
6. variable-graph `GRAPH ?g {…}` → 501 at build (§8 gap-5; out-of-charter unless quad
   querying is declared in scope — recorded as a deliberate delta, not a silent 501).

---

## 6. Offline T-mapping stage placement

An **offline, startup-time** mapping-preprocessing stage that runs BEFORE per-query IR
construction and is **separable** from the IR core (ADR-0023 §Offline stage; Ontop
§2.1). It is **NOT an IqNode** and does not gate the IR/normalizer/lowering. Two jobs at
startup: (1) fold the class/property hierarchy (subclass/subproperty, domain/range —
sf's tier-1 entailment, today per-query in `saturate.rs`) into the mapping set so a
query over a class need not consult the ontology at runtime; (2) prune redundant
mapping/union branches using integrity constraints (PK/FK/UC), narrowing per-query
union width before unfolding. Its product is a consolidated T-mapping set consumed when
`Intensional` leaves are unfolded into Extensional/Construction/Union subtrees (every
`Intensional` MUST be resolved before normalization + SQL). Because it is a mapping-set
transform rather than a tree rewrite, it is built ALONGSIDE the tree on its own track;
an empty/absent T-mapping stage just means per-query normalization redoes the same
union pruning (correct, slower) — it **amortises** work but is **not load-bearing for
correctness**. Tier-2 tree-witness entailment stays excluded (ADR-0008; Ontop ships it
off by default).

---

## 7. Risks and the 3-VL LeftJoin hotspot

1. **LeftJoin 3-valued-logic substitution composition is the #1 regression hotspot**
   (Ontop's own history): Coalesce-of-shared-vars + `NullSafeEq` ON + right-side
   nullability, and §4.10's null-rejection proof, can silently drop/admit NULL-padded
   rows. **Mitigation (binding):** lower `NullSafeEq` (never `ColEq`) for OPTIONAL,
   `COALESCE(left,right)` for shared vars, keep inner FILTERs in the ON (R5)
   structurally; gate every LeftJoin rule (§4.4, §4.5, §4.9b, §4.10, §4.16-left) with
   the SPARQL OPTIONAL conformance + `=_bag` PG↔SQLite differential as refute-only
   adversarial review.
2. **Pass-ordering / termination of the unified fixpoint.** The five flat order
   constraints are re-expressed as re-fire-to-fixpoint (§3); each §4 rule must state a
   strictly-decreasing well-founded measure and no rule may both create and consume its
   own match without a decrease; substitution composition must be confluent.
3. **`=_bag` re-proof surface.** ~13 ported passes + 6 new rules each need an
   independent multiset argument (all stated in §4); the constraint-gated rules must be
   sound NO-OPS on empty/missing schema (re-verified by the inert-path gate); the
   DISTINCT-gated rules (§4.3 FD, §4.5, §4.12, §4.15) are the most precondition-sensitive.
4. **Constraint-propagation caching.** Stale/under-propagated cache could license an
   unsound rewrite (false uniqueness) or block a sound one. §4.7 mandates invalidation
   of a rewritten node and its ancestors before re-read.
5. **Lazy join-over-union distribution blowup.** mᵗ if empty-branch pruning + T-mapping
   narrowing under-fire before §4.16; a soft (cost) ordering the fixpoint does not
   inherently enforce — ordering pruning-before-distribution is a performance, not
   correctness, constraint.
6. **Expressible ≠ lowerable.** Multi-scan/Union OPTIONAL, SubQuery, Path-join, nested
   Aggregation, HAVING compose in the IR but need §5.1 `SubPlan`; until built they are
   tracked sound-501s (§5.2), never silent miscompiles.

---

## 8. Verification ledger

Every rule whose `=_bag` argument was **REFUTED** in adversarial review, with the
required restatement/precondition baked into §4 above (per the no-deferrals mandate the
corrected precondition is preferred over deferral). Plus every BLOCKER/important gap
from the completeness critic with its resolution.

### 8.A Refuted rules (13/17 verdicts refuted) — all restated, none deferred

| # | Rule (§) | Defect (counterexample) | Corrected precondition baked in |
|---|---|---|---|
| 1 | tier0-refobjectmap-inline (§4.1) | join-free refObjectMap → unconstrained parent cross product (\|T\|²≠\|T\|) | inline from CHILD alias on `r.joins.is_empty()` (R2RML child==parent), or synthesize NullSafeEq self-key; no empty stub |
| 2 | unsat-equality-prune (§4.2) | Empty dropped through LeftJoin right side (2→0) | positional: Empty absorbs only in POSITIVE position; else operator-correct law `LeftJoin(L,Empty)≡L` NULL-pad / `NotExists{Empty}≡TRUE` |
| 3 | self-join-elimination FD (§4.3) | nullable FD determinant re-admits NULL rows (∅→{(NULL,1),(NULL,2)}) | IS-NOT-NULL synthesis extended to FD branch, not only nullable-unique |
| 4 | self-leftjoin-elimination (§4.4) | lone-IsNotNull exception leaks sibling non-null column (`(1,NULL,NULL)`→`(1,NULL,'x')`) | exception fires only when OPTIONAL binds no column besides the filtered key |
| 5 | distinct-prune-unused-leftjoin (§4.5) | `FILTER(!BOUND(?y))` observes dropped var (∅→{x=1}) | right scope must be GLOBALLY DEAD above the LeftJoin, not merely unprojected |
| 6 | fk-pk-join-elimination (§4.6) | collation/CHAR-pad/scale: join-equal ≠ lexical-equal (`:country/GB`≠`/gb`) | TERM-COHERENCE precondition (deterministic collation, matching type/length/scale) per column |
| 7 | filter-pushdown (§4.8) | conjunct sunk into one Union branch loses error-removal (1→2) | cross Union only as per-branch distribution to EVERY arm; never a proper subset |
| 8 | filter-lift (§4.9) | only movable LeftJoin predicates are left-only (useless) or right-only (unsound, `{(1,NULL)}`→∅) | split: Union-lift + preserved-side move + in-place downgrade recognition; remove "lift right-only to enable downgrade" |
| 9 | leftjoin→innerjoin-downgrade (§4.10) | FK path hoisted in-OPTIONAL filter to WHERE (1→0); "right-only" conflated with COALESCE-shared | FK disjunct requires `opt.extra.is_empty()`; right-only = exclusively-right-provided + no intervening card/null op |
| 10 | disjunction-intersection-simplify (§4.11) | `String` intersection finer than value-eq (`"1"`vs`"1.0"`→∅ prune, 1→0) | canonicalize to the column domain's lexical form / binary collation before intersecting; else no-op |
| 11 | distinct-removal (§4.12) | multi-col `Concat` whitelisted as injective (`{x="abc"}×1`→×2) | Concat/Coalesce injective only when reading ≤1 (the key) column |
| 12 | empty-true-propagation (§4.13) | "all-True ⇒ True" discards joining cond `(c1=c2)` (0→1) | True clauses gated to condition-free InnerJoin; split θ into Filter first; Empty carries union var signature |
| 13 | aggregation-through-union push-down (§4.14) | template-less unbound group spans branches, MAX double-counts (1→2) | push-down needs grouping key provably bound in every branch (or ≤1 unbound), else single Aggregation |
| 14 | union-branch-merging dedup (§4.15) | dedup under GROUP BY/COUNT between union and DISTINCT (`(a,4)`→`(a,2)`) | arm-dedup only when path to dominating Distinct is aggregation-free (distinct-bounded AND agg-free) |
| 15 | join-over-union-distribution (§4.16) | LeftJoin over RIGHT union fabricates `(x=1,y=NULL)` (1→2) | InnerJoin/Filter either arm; LeftJoin LEFT (preserved) arm only |

(15 refutation entries cover the 13 distinct catalogue rules refuted; §4.9 and §4.10
each absorbed two related sub-defects. The 4 non-refuted verdicts —
constraint-propagation §4.7, filter-pushdown's LeftJoin caveat, the scope-fix half of
§4.14, and the lift/fold halves of §4.15 — carry their original `=_bag` arguments
unchanged.)

**Survivor count:** of 17 review verdicts, **4 `=_bag` arguments survived refutation
unchanged**; **13 required restatement** with a corrected precondition (all baked into
§4, zero deferrals).

### 8.B Completeness-critic gaps — resolutions baked into the contract

| Sev | Gap | Resolution (where) |
|---|---|---|
| **BLOCKER** | No derived-table / sub-Plan lowering target — subquery/multi-scan-OPTIONAL/path-join/agg-as-input expressible in IR but unlowerable to `Branch` | **§5.1**: add `LogicalSource::SubPlan(Box<Plan>)` derived-table target with its `=_bag` argument; **§5.2** enumerates each as a tracked sound-501 until `SubPlan` ships (M7). The ADR-0023 value-prop is realized at SQL level, not just IR. |
| important | No normalization inside EXISTS/NOT EXISTS/MINUS payloads (flat cascade bypassed them) | **§3 recursion clause** (BINDING): normalizer descends into `SqlCond::Exists`/`NotExists` subtrees; rewrites carry the §4.2 positional caveat. |
| important | Aggregate surface (GROUP_CONCAT/SAMPLE/COUNT(DISTINCT *)/SUM(expr)) unexpressible | **§1**: `AggKind += GroupConcat{sep}, Sample`; `AggDef.arg: Option<AggArg>` (Var or Expr); `COUNT(DISTINCT *)` via `AggDef.distinct`; **§2 Group arm** pre-Extends expression args; **§5** lowers GROUP_CONCAT/SAMPLE via Rust group path. |
| important | HAVING / post-aggregation Filter has no spine slot or lowering | **§3 spine** adds a post-Aggregation `Filter[HAVING]` slot; **§5** lowers to SQL `HAVING` or §5.1 `SubPlan`+outer Filter. |
| important | Variable-graph / quad querying still 501 | **§2 Graph arm** + **§5.2 item 6**: recorded as a **deliberate delta** (out-of-charter unless quad querying is declared in scope), not an implicit silent 501. |
| minor | Incomplete enumeration of expressible-but-deferred shapes | **§5.2** enumerates all six as named sound-501s with tests. |
| minor | Out-of-charter deltas (FlattenNode, tier-2 tree-witness) listed for completeness | **§1** records both as deliberate charter deltas; no action. |

---

---

## 9. Implementation amendments

Refinements discovered during implementation, recorded so the locked contract stays
accurate. Each preserves the §1–§6 intent; none relaxes a `=_bag` precondition.

* **M2 — `IqCond` (condition representation).** §1/§2 specified `Filter.cond:
  Vec<SqlCond>` and "`EXISTS`/`NOT EXISTS` → `SqlCond::Exists` carrying the built inner
  subtree." The flat `SqlCond::Exists { scans, conds }` cannot carry an unlowered
  `IqNode` subtree, so the three condition-bearing nodes (`Filter`, `InnerJoin`,
  `LeftJoin`) carry `Vec<IqCond>`, where `IqCond = Sql(SqlCond) | And | Or | Not |
  Exists(Box<IqNode>) | NotExists(Box<IqNode>)`. This **keeps** the "reuse `SqlCond`"
  intent (the pushable vocabulary is `IqCond::Sql`) and realizes the design's "the
  normalizer recurses into the EXISTS payload as a first-class `IqNode`" (§3) and "MINUS
  → `Filter[NotExists(build R)]`" (§2). At lowering a normalized `Exists`/`NotExists`
  subtree collapses to the flat `SqlCond::Exists`/`NotExists` (§5), so no new SQL path is
  added. `=_bag`: unchanged — `IqCond` is a representational carrier, not a rewrite.

* **M2/M3 — resolution architecture: inline resolution during build (Option B), not a
  deferred Intensional pass.** §2 was internally inconsistent on resolution *timing*:
  `Bgp → Intensional` ("resolved later") yet `Extend`/`Filter` "lower expr at build" —
  impossible together, because a FILTER/BIND over a triple's variable needs that
  triple's resolved columns. **Decision:** `build_tree` resolves triple patterns
  **inline** against the (offline-consolidated) T-mappings — reusing the proven flat
  `bgp()` machinery — producing `Extensional`+`Construction` (and a `Union` for a
  predicate with several triples-maps), establishing the variable→column scope, then
  lowering FILTER/BIND leaves inline via the existing `lower_filter_expr` /
  `bind_term_def` (columns now known). The **operator structure**
  (`InnerJoin`/`LeftJoin`/`Union`/`Aggregation`/`Distinct`/…) is preserved as tree nodes
  — **no eager flattening**, the ADR-0023 goal — and normalization (§3/§4) + lowering
  (§5) run on the resolved tree. This delivers every ADR-0023 guarantee (operator tree
  for logical rewrites, no eager flattening, offline T-mappings, `=_bag`) while
  **maximising reuse** of the proven flat machinery (harness "reuse over new") and
  avoiding a parallel unresolved-expression type system + a re-implemented resolve pass.
  `IqNode::Intensional` stays in the enum as the conceptual unresolved-pattern leaf (and
  for any future lazy-resolution / SERVICE path). The **M2** structural builder
  (`build.rs`, context-free) emits `Intensional` + tracked sound-501s for the
  column-dependent leaves (FILTER leaf / non-constant BIND); **M3** retires those by
  giving `build_tree` the resolution context (mappings/schema/dialect). `=_bag`: inline
  resolution *is* the flat model's proven resolution, unchanged.

---

*Locked. M2–M8 build against §1–§6 (as amended by §9); §4 preconditions and §5.1/§5.2
boundary are non-negotiable; every commit holds the gate in the front-matter.*
