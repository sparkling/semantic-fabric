//! The operator-tree intermediate query (IQ) node set — ADR-0023's native-Rust
//! query IR (design-lock `docs/design/ADR-0023-design-lock.md` §1).
//!
//! **The tree is the optimizer model; [`Branch`](super::Branch) is its SQL-lowering
//! target.** A query is built into an [`IqNode`] tree (replacing the eager
//! `Vec<Branch>` flattening), normalized by substitution-lifting to a fixpoint
//! (ADR-0023 §Normalization), then a normalized *leaf CQ* lowers to today's
//! `Branch`/`emit` path — preserving ADR-0006 streaming, ADR-0007 term-construction
//! lifting, and ADR-0010 bound-parameter discipline. Nothing here re-implements term
//! or condition machinery: payloads **reuse** the existing [`TermDef`], [`SqlCond`],
//! [`AggKind`], [`OrderKey`], [`Scan`], and [`PathClosure`] types.
//!
//! Modelled as one Rust `enum` with exhaustive `match` — **no** trait objects, JVM
//! class hierarchy, or DI (ADR-0004). Unary children are `Box<IqNode>`; n-ary
//! children are `Vec<IqNode>`. **Out of charter, not modelled:** `FlattenNode` / JSON
//! unnest, cost-driven translation selection, and any generic `NativeNode` — raw SQL
//! exists only as the `Branch`/`emit` lowering target, never as an IR node.

use std::collections::BTreeMap;

use sf_core::datatype::XsdTypeCode;
use sf_core::{NamedNode, Term};
use spargebra::algebra::{Expression, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use super::{AggKind, OrderKey, PathClosure, Scan, SqlCond, TermDef};

/// A SPARQL variable name. `Box<str>` (not `String`) keeps nodes compact; it converts
/// freely to the `String` key domain of [`Branch::bindings`](super::Branch::bindings)
/// at lowering.
pub type Var = Box<str>;

/// One node of the operator-tree IR (ADR-0023 design-lock §1). Fifteen variants: the
/// in-charter IQ node set plus the specialized recursive [`IqNode::Path`] leaf.
///
/// Every node has a bottom-up *scope* (its projected variables + per-variable
/// nullability), computed on demand during normalization — the single property that
/// dissolves the flat model's eager-flattening deferrals (a subtree composes under
/// `Join`/`LeftJoin`/`Union` uniformly because its scope is known without flattening).
#[derive(Debug, Clone)]
pub enum IqNode {
    // ---- unary substitution carrier (the heavy lifter) ---------------------------
    /// Ontop `ConstructionNode`. `subst` maps each variable to a [`BindDef`] — a resolved
    /// [`TermDef`] (ADR-0007 term-construction lifting) or a **symbolic** `spargebra`
    /// expression carried until per-leaf-CQ lowering (a `BIND(?v := expr)` over a variable
    /// has no column until its triple resolves; M3 design §2.2). `project` is the declared
    /// projected variable set. Construction∘Construction folds to one during normalization
    /// (compose substitutions, intersect projections — ADR-0023 §Normalization).
    Construction {
        child: Box<IqNode>,
        subst: BTreeMap<Var, BindDef>,
        project: Vec<Var>,
    },

    // ---- unary boolean selection -------------------------------------------------
    /// `FilterNode`. `cond` is an implicit conjunction of [`IqCond`]s (3-valued SPARQL
    /// FILTER: a solution is kept only when the condition is TRUE). [`IqCond`] reuses
    /// the pushable [`SqlCond`] vocabulary and adds `EXISTS`/`NOT EXISTS` over a built
    /// `IqNode` subtree — which the flat `SqlCond::Exists` cannot carry before lowering.
    Filter {
        child: Box<IqNode>,
        cond: Vec<IqCond>,
    },

    // ---- n-ary / binary joins ----------------------------------------------------
    /// `InnerJoinNode`: an n-ary natural join over shared variables plus an optional
    /// joining condition (a conjunction of [`IqCond`]s, populated by filter push-down
    /// during normalization). Identity is [`IqNode::True`] (**condition-free only**,
    /// design §4.13); absorbing element is [`IqNode::Empty`].
    InnerJoin {
        children: Vec<IqNode>,
        cond: Vec<IqCond>,
    },
    /// `LeftJoinNode`: binary and **non-commutative**; `cond` is the OPTIONAL
    /// ON-expression (a conjunction of [`IqCond`]s). Variables provided only by `right`
    /// become nullable in the output scope. This is the designated 3-valued-logic
    /// regression hotspot (design §7).
    LeftJoin {
        left: Box<IqNode>,
        right: Box<IqNode>,
        cond: Vec<IqCond>,
    },

    // ---- n-ary bag union ---------------------------------------------------------
    /// `UnionNode` (bag semantics — no dedup). `project` is the union of the arms' scopes,
    /// kept as **scope bookkeeping for parent resolution only** (M3 design §5.2, R3). Arms
    /// are NOT padded to a common signature: each arm keeps its own bindings, and a
    /// variable an arm does not bind stays genuinely **unbound/absent** at lowering
    /// (matching the flat `Vec<Branch>` bag union — never a concrete NULL-valued term).
    Union {
        children: Vec<IqNode>,
        project: Vec<Var>,
    },

    // ---- aggregation -------------------------------------------------------------
    /// `AggregationNode` (SPARQL §11). ONE construct for both the SQL-`GROUP BY` and
    /// the Rust-group lowering paths (the strategy is chosen at lowering, not modelled
    /// here). The output scope (`grouping` ∪ each [`AggDef::var`]) is **owned by this
    /// node** — which is what lets an outer `Extend`/`Project` resolve aggregate
    /// variables and closes the agg-over-UNION binding-scope bug (design §4.14).
    Aggregation {
        child: Box<IqNode>,
        grouping: Vec<Var>,
        aggs: Vec<AggDef>,
    },

    // ---- query-modifier spine ----------------------------------------------------
    /// `DistinctNode`: multiset → set on the child's projected tuples.
    Distinct { child: Box<IqNode> },
    /// `SliceNode`: SPARQL OFFSET/LIMIT (`limit == None` ⇒ no upper bound).
    Slice {
        child: Box<IqNode>,
        offset: usize,
        limit: Option<usize>,
    },
    /// `OrderByNode`: SPARQL §15.1 ordering. [`OrderKey`] reused verbatim.
    OrderBy {
        child: Box<IqNode>,
        keys: Vec<OrderKey>,
    },

    // ---- leaves ------------------------------------------------------------------
    /// `ValuesNode`: an inline literal table (bag). A `None` cell is SPARQL `UNDEF`
    /// (the variable is unbound in that row).
    Values {
        vars: Vec<Var>,
        rows: Vec<Vec<Option<TermDef>>>,
    },
    /// `ExtensionalDataNode`: a concrete mapped relation. **Reuses** [`Scan`]; `bind`
    /// is the sparse column→variable/constant binding the pattern reads. The relation's
    /// PK/UC/FK/FD constraints are looked up from the catalog by the constraint-driven
    /// rules (design §4.7), not stored here.
    Extensional {
        scan: Scan,
        bind: BTreeMap<Box<str>, ColOrConst>,
    },
    /// `IntensionalDataNode`: an unresolved triple/quad pattern. **MUST NOT survive
    /// unfolding** — it is replaced against the T-mappings (into
    /// `Extensional`/`Construction`/`Union` subtrees) before normalization and
    /// lowering. `graph` is the resolved *constant* active graph (a variable graph is a
    /// 501 at build time — design §2 `Graph` arm / §5.2 item 6).
    Intensional {
        pattern: TriplePattern,
        graph: Option<NamedNode>,
    },
    /// `UnresolvedPathNode`: a SPARQL property-path pattern `?s PATH ?o` (any closure
    /// `P+`/`P*`/`p?`, sequence `p/q`, alternative `p|q`, inverse `^p`, or negated
    /// property set `!p`) carried **verbatim** from BUILD until RESOLVE compiles it.
    /// Like [`IqNode::Intensional`] it is a **transient leaf that MUST NOT survive
    /// RESOLVE** (M5 Wave 1): the closure relation is built from the T-mappings (the
    /// hop relation reads the triples-maps), so it cannot be compiled context-free at
    /// BUILD — RESOLVE reuses the flat [`Unfolder::path_branch`](crate::unfold) VERBATIM
    /// to turn it into an [`IqNode::Path`] closure (design §5.2 item 3). `graph` is the
    /// resolved *constant* active graph (a variable graph is already a build-time 501);
    /// the length-1 fixed-predicate path (≡ one triple) stays an `Intensional`, not this.
    UnresolvedPath {
        subject: TermPattern,
        path: PropertyPathExpression,
        object: TermPattern,
        graph: Option<NamedNode>,
    },
    /// `EmptyNode`: ∅ over a declared variable set. Union identity, InnerJoin absorbing
    /// (design §4.13). Carries `vars` so a parent projection stays well-formed when the
    /// node is absorbed.
    Empty { vars: Vec<Var> },
    /// `TrueNode`: the single empty tuple. InnerJoin identity — **for a condition-free
    /// join only** (design §4.13).
    True,

    // ---- specialized recursive leaf (not a generic node) -------------------------
    /// A property-path closure ([`PathClosure`] reused verbatim). It publishes an
    /// ordinary output scope, so `InnerJoin`/`Filter`/`LeftJoin`/`Minus` compose over
    /// it — which is what retires the flat model's path-join 501s.
    Path { closure: PathClosure },
}

/// One aggregate output: `var := kind(arg) [DISTINCT]` with its §10 fixed result type.
///
/// `arg` is an **expression** payload (not a bare [`Var`]) so `SUM(?a + ?b)`,
/// `GROUP_CONCAT(… ; SEPARATOR=…)`, and `SAMPLE` are expressible (design §8 gap-3); an
/// expression argument may be pre-bound by an inner [`IqNode::Construction`] (an
/// `Extend`) that lowers it to a plain variable. `COUNT(DISTINCT *)` rides
/// [`AggDef::distinct`] with `arg == None`.
#[derive(Debug, Clone)]
pub struct AggDef {
    pub var: Var,
    pub kind: AggKind,
    /// The aggregated argument (`None` for `COUNT(*)`).
    pub arg: Option<AggArg>,
    pub distinct: bool,
    /// The fixed §10 result type when the set function pins it (COUNT ⇒ integer,
    /// AVG ⇒ decimal); `None` ⇒ take the value's resolved decltype (SUM/MIN/MAX keep
    /// the source numeric type).
    pub fixed_type: Option<XsdTypeCode>,
}

/// An aggregate's argument: a bound variable, or a SPARQL expression (lowered via a
/// [`TermDef`]). A pre-Extend may rewrite [`AggArg::Expr`] into [`AggArg::Var`] before
/// lowering so the SQL/Rust group path sees a single column.
#[derive(Debug, Clone)]
pub enum AggArg {
    Var(Var),
    Expr(TermDef),
}

/// What an [`IqNode::Extensional`] column position binds to: a raw source column (read
/// into a variable / used in a join key) or a fixed RDF term (a constant-position
/// pattern term, e.g. the predicate of a triple pattern).
#[derive(Debug, Clone)]
pub enum ColOrConst {
    Col(Box<str>),
    Const(Term),
}

/// A tree-level boolean condition for [`IqNode::Filter`] / `InnerJoin` / `LeftJoin`.
///
/// It **reuses** the pushable [`SqlCond`] vocabulary (comparisons, `IS [NOT] NULL`,
/// column equality, string match) and adds the two cases the flat `SqlCond` cannot hold
/// before lowering: `EXISTS` / `NOT EXISTS` over a **built `IqNode` subtree** (design §2
/// `Filter`/`Minus` arms). The normalizer descends into those subtrees as first-class
/// `IqNode`s (design §3 recursion clause); at **lowering**, a normalized subtree
/// collapses to the flat [`SqlCond::Exists`](super::SqlCond::Exists) /
/// [`SqlCond::NotExists`](super::SqlCond::NotExists) (scans + correlation conds), so no
/// new SQL-emission path is introduced.
#[derive(Debug, Clone)]
pub enum IqCond {
    /// A variable-referencing FILTER / ON leaf carried as a raw `spargebra` expression.
    /// Kept symbolic through resolve + normalize (a FILTER above a `Union` has no single
    /// column for a variable — each arm binds it to a different alias/column, or to a
    /// constructed term with no column), then lowered to `Sql` **per leaf-CQ** at LOWER
    /// via the flat `lower_filter_expr` (M3 design §2.1).
    Expr(Box<Expression>),
    /// A resolved/pushed leaf over raw columns + bound constants — the flat vocabulary,
    /// filled at LOWER and by the §4 (M4) rewrite rules.
    Sql(SqlCond),
    And(Vec<IqCond>),
    Or(Vec<IqCond>),
    Not(Box<IqCond>),
    /// `FILTER EXISTS { P }` — a correlated semi-join over the built subtree (correlated
    /// on the variables shared with the enclosing scope, resolved at lowering).
    Exists(Box<IqNode>),
    /// `FILTER NOT EXISTS { P }` (SPARQL §11.4.7 — a pure correlated existence test,
    /// true/false regardless of whether `P` shares a variable with the enclosing
    /// scope) and `MINUS` (SPARQL §8.3.2 — a DIFFERENT semantics: a documented
    /// no-op when the outer and inner variable domains are disjoint, since a
    /// disjoint-domain right side can never remove a left solution). Both lower
    /// through the same correlated-anti-join machinery (`lower_iq_exists`) and
    /// differ in exactly this one precondition; `is_minus` says which SPARQL
    /// construct built this node so that machinery can apply the right one.
    NotExists {
        inner: Box<IqNode>,
        is_minus: bool,
    },
}

/// A [`IqNode::Construction`] substitution entry: either a resolved [`TermDef`] (the
/// existing term-construction-lifting carrier — `Const`/`Derived`/`Coalesce`/`Concat`/
/// `Agg`), or a **symbolic** `spargebra` expression carried until per-leaf-CQ lowering
/// (a `BIND(?v := expr)` over a variable has no column until its triple resolves; M3
/// design §2.2). LOWER folds `Resolved(td)` straight into the branch bindings and
/// resolves `Expr(e)` via the flat `bind_term_def` against the now-known per-branch
/// bindings (mirroring the `AggArg::Var|Expr` and `OrderKey.expr` precedents).
#[derive(Debug, Clone)]
pub enum BindDef {
    Resolved(TermDef),
    Expr(Box<Expression>),
}

impl IqNode {
    /// The bottom-up *variable scope* this node publishes (design-lock §1: "every
    /// node computes a bottom-up `scope` on demand" — the single invariant that
    /// dissolves the flat model's eager-flattening deferrals). The list is
    /// deterministic, de-duplicated, and in a stable bottom-up order, so a parent
    /// `InnerJoin`/`LeftJoin`/`Union` composes over a child's scope **without
    /// flattening it** — the property the M2 builder relies on to populate
    /// `Union.project` and an `Extend`'s `Construction.project` (design-lock §2).
    ///
    /// Per-variant scope (design-lock §1 / §2):
    /// * `Construction`/`Union` — the declared `project` (the node owns its scope).
    /// * `Filter`/`Distinct`/`Slice`/`OrderBy` — the child's scope (a query modifier
    ///   never adds or drops a variable).
    /// * `InnerJoin` — the de-duplicated union of every child's scope.
    /// * `LeftJoin` — left scope ++ right scope (right-only vars are nullable in the
    ///   output, but still in scope).
    /// * `Aggregation` — the grouping keys ++ each aggregate's output variable (the
    ///   node owns its scope, closing the agg-over-UNION binding bug, design §4.14).
    /// * `Values`/`Empty` — the declared `vars`; `True` — none (the empty tuple).
    /// * `Intensional` — the variables in the triple pattern's subject/predicate/
    ///   object positions (the `graph` here is a resolved *constant*, never a var).
    /// * `UnresolvedPath` — the variables in the path pattern's subject/object
    ///   positions (a property path has no predicate variable; `graph` is a constant).
    /// * `Extensional` — its `bind` keys (the variables the resolved relation reads).
    /// * `Path` — empty: a [`PathClosure`](super::PathClosure) is keyed by the
    ///   canonical `sf_s`/`sf_o` raw columns and carries **no** SPARQL variable
    ///   names, so none are recoverable here. (The M2 builder never emits `Path` — it
    ///   defers a property-path closure to resolution, design §5.2 item 3.)
    pub fn output_vars(&self) -> Vec<Var> {
        match self {
            IqNode::Construction { project, .. } | IqNode::Union { project, .. } => project.clone(),
            IqNode::Filter { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. } => child.output_vars(),
            IqNode::InnerJoin { children, .. } => {
                let mut out = Vec::new();
                for c in children {
                    push_unique_all(&mut out, c.output_vars());
                }
                out
            }
            IqNode::LeftJoin { left, right, .. } => {
                let mut out = left.output_vars();
                push_unique_all(&mut out, right.output_vars());
                out
            }
            IqNode::Aggregation { grouping, aggs, .. } => {
                let mut out = Vec::new();
                push_unique_all(&mut out, grouping.clone());
                for a in aggs {
                    push_unique(&mut out, a.var.clone());
                }
                out
            }
            IqNode::Values { vars, .. } | IqNode::Empty { vars } => vars.clone(),
            IqNode::Extensional { bind, .. } => bind.keys().cloned().collect(),
            IqNode::Intensional { pattern, .. } => triple_pattern_vars(pattern),
            IqNode::UnresolvedPath {
                subject, object, ..
            } => {
                let mut out = Vec::new();
                if let TermPattern::Variable(v) = subject {
                    push_unique(&mut out, v.as_str().into());
                }
                if let TermPattern::Variable(v) = object {
                    push_unique(&mut out, v.as_str().into());
                }
                out
            }
            IqNode::True | IqNode::Path { .. } => Vec::new(),
        }
    }
}

/// Push `v` onto `out` iff absent — a small stable-order de-duplicator for scopes.
fn push_unique(out: &mut Vec<Var>, v: Var) {
    if !out.contains(&v) {
        out.push(v);
    }
}

/// Push every element of `vs` onto `out`, de-duplicating in stable order.
fn push_unique_all(out: &mut Vec<Var>, vs: Vec<Var>) {
    for v in vs {
        push_unique(out, v);
    }
}

/// The variables a triple pattern binds, in subject→predicate→object order
/// (de-duplicated, so a repeated variable such as `?x ?x ?x` is listed once).
pub(crate) fn triple_pattern_vars(tp: &TriplePattern) -> Vec<Var> {
    let mut out = Vec::new();
    if let TermPattern::Variable(v) = &tp.subject {
        push_unique(&mut out, v.as_str().into());
    }
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        push_unique(&mut out, v.as_str().into());
    }
    if let TermPattern::Variable(v) = &tp.object {
        push_unique(&mut out, v.as_str().into());
    }
    out
}
