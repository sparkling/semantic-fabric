//! The intermediate-query (IQ) relational model — the rewrite target the SPARQL
//! algebra is unfolded into (ADR-0007 step 3, the ISWC-2018 base translation).
//!
//! **Term-construction lifting (ADR-0007).** Joins and FILTERs in this model are
//! expressed over **raw key columns** ([`ColRef`] / [`SqlCond`]); RDF terms are
//! never built inside a join/filter predicate. Each output position carries a
//! [`TermDef`] — a recipe (the `sf-core` term map + the scan alias it reads) that
//! is materialised into an `oxrdf` term **only** during result reconstruction
//! (the outermost projection), via the single `sf-core` term-gen path. This keeps
//! equi-joins on indexed columns and keeps the source's row estimates accurate.
//!
//! A query compiles to one or more [`Branch`]es. A single branch is one SQL
//! `SELECT`; multiple branches are a **bag union** (SPARQL `UNION` / the
//! per-triples-map alternatives of a triple pattern), streamed and concatenated
//! (the bounded-memory equivalent of a SQL `UNION ALL` over heterogeneous
//! per-branch projections — ADR-0006).

use std::collections::{BTreeMap, HashSet};

use sf_core::datatype::XsdTypeCode;
use sf_core::ir::{LogicalSource, Segment, TermMap, TermType};
use sf_core::Term;
use spargebra::algebra::Expression;

/// The operator-tree IR node set (ADR-0023 M2, design-lock §1). The tree is the
/// optimizer model; the [`Branch`] below is its SQL-lowering target.
pub mod node;

/// RESOLVE (ADR-0023 M3a): replace every [`node::IqNode::Intensional`] leaf with its
/// resolved `Extensional`/`Construction`(/`Union`/`InnerJoin`) subtree, reusing the
/// proven flat [`crate::unfold`] atom oracle verbatim. The flat path is untouched.
pub mod resolve;

/// NORMALIZE-min (ADR-0023 M3b): substitution-lift + join-over-union distribution +
/// Empty/True pruning to drive a RESOLVED tree to the lowerable leaf-CQ spine. The
/// flat path is untouched (not wired into the live engine).
pub mod normalize;

/// LOWER (ADR-0023 M3c): fold a NORMALIZED leaf-CQ spine into a [`crate::Plan`]
/// (bag-union of [`Branch`]es), reusing the proven flat `emit`/`leftjoin`/`unify`
/// machinery. FILTER/BIND resolve HERE, per leaf-CQ. The flat path is untouched
/// (not wired into the live engine).
pub mod lower;

/// A reference to a raw source column of a specific scan alias. Aliases are
/// small integers; emission renders them `t{alias}` (ADR-0007 lifting: joins and
/// filters are over these, never over constructed term strings).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColRef {
    pub alias: usize,
    pub column: Box<str>,
}

impl ColRef {
    pub fn new(alias: usize, column: impl Into<Box<str>>) -> Self {
        Self {
            alias,
            column: column.into(),
        }
    }
}

/// How an output position (a projected SPARQL variable, or a CONSTRUCT-template
/// term slot) is built. Term construction is **lifted**: a `Derived` def names
/// the `sf-core` term map plus the scan alias whose columns feed it, and the term
/// is generated only at reconstruction time (never in SQL).
#[derive(Debug, Clone)]
pub enum TermDef {
    /// A fixed RDF term (an `rr:constant` term map, or a bound query constant).
    Const(Term),
    /// A column/template term map evaluated against the columns of `alias`.
    Derived { term_map: TermMap, alias: usize },
    /// **R2 (ADR-0007).** A shared variable is one SPARQL variable but two SQL
    /// representations after a LEFT JOIN — project it as `COALESCE(left, right)`.
    /// Reconstruction tries the preserved (`left`) side first and falls back to
    /// `right` when `left`'s source columns are NULL (the optional did not match).
    Coalesce(Box<TermDef>, Box<TermDef>),
    /// A `BIND(CONCAT(…) AS ?v)` computed value: the operand defs are reconstructed
    /// and their lexical values concatenated into a plain literal at the outer
    /// projection (term-construction lifting — built in Rust at reconstruction,
    /// never in SQL). An unbound / non-literal operand makes the CONCAT an error, so
    /// the BIND variable is left unbound (SPARQL §17.4.x / §10 ASSIGN).
    Concat(Vec<TermDef>),
    /// **An aggregate result (SPARQL §11).** The value is computed in SQL
    /// (`COUNT`/`SUM`/`AVG`/`MIN`/`MAX`) and projected at `col` — a synthetic
    /// column on the [`Aggregation`]'s reserved alias, not a base scan column. The
    /// `kind` drives both the empty-group value (SPARQL §11: SUM/COUNT over an
    /// empty multiset ⇒ `"0"^^xsd:integer`; AVG/MIN/MAX ⇒ UNBOUND) and the result
    /// datatype. `fixed_type` pins the type when the function fixes it (COUNT ⇒
    /// `xsd:integer`); otherwise the type is the column's resolved §10 type
    /// (decltype/storage-class — MIN/MAX/SUM keep the source value's numeric type).
    /// AVG (§11.4) follows the **operand** numeric type under XPath promotion
    /// (`xsd:integer`/`xsd:decimal` ⇒ `xsd:decimal`; `xsd:double` kept) — resolved
    /// from `operand`'s §10 type, not the SQL aggregate column's (SQLite's `AVG`
    /// always yields a `REAL`).
    Agg {
        col: ColRef,
        kind: AggKind,
        /// The single aggregated argument column (`None` for `COUNT(*)`), used to
        /// resolve AVG's §10 operand type at reconstruction.
        operand: Option<ColRef>,
        fixed_type: Option<XsdTypeCode>,
    },
    /// **ADR-0032 D2** — a native RDF 1.2 triple term (`<<( s p o )>>`), realized
    /// at reconstruction by recursively building `subject`/`predicate`/`object`
    /// and composing them via `oxrdf::Triple::from_terms` (`exec_core::build_term`).
    /// This is the ONLY route by which this engine ever produces a `Term::Triple`
    /// — deliberately bypassing `sf_core::term::generate` (its `GenTerm` has no
    /// triple arm by design, ADR-0006 zero-alloc). `object` recurses for object-side
    /// nesting (arbitrary depth) for free. Every downstream consumer that matches
    /// exhaustively on `TermDef` (unify/leftjoin/cascade) treats this the same way
    /// it treats `Coalesce`/`Concat` — a multi-source constructed term, not
    /// reducible to a single raw column.
    ComposedTriple {
        subject: Box<TermDef>,
        predicate: Box<TermDef>,
        object: Box<TermDef>,
    },
}

impl TermDef {
    /// The raw columns this def reads (for projection collection). Constants read
    /// nothing; a `Coalesce` reads both sides' columns.
    pub fn columns(&self) -> Vec<ColRef> {
        match self {
            TermDef::Const(_) => Vec::new(),
            TermDef::Derived { term_map, alias } => term_map_columns(term_map)
                .into_iter()
                .map(|c| ColRef::new(*alias, c))
                .collect(),
            TermDef::Coalesce(l, r) => {
                let mut cols = l.columns();
                for c in r.columns() {
                    cols.push(c);
                }
                cols
            }
            TermDef::Concat(parts) => {
                let mut cols = Vec::new();
                for p in parts {
                    cols.extend(p.columns());
                }
                cols
            }
            TermDef::Agg { col, .. } => vec![col.clone()],
            TermDef::ComposedTriple {
                subject,
                predicate,
                object,
            } => {
                let mut cols = subject.columns();
                cols.extend(predicate.columns());
                cols.extend(object.columns());
                cols
            }
        }
    }
}

/// Rust-level GROUP BY descriptor for a multi-branch (UNION/VALUES) inner
/// pattern (SPARQL §11). When [`Plan::rust_group`][crate::Plan::rust_group] is
/// set, the executor buffers every solution from the inner branches, groups them
/// by `keys`, computes `aggs` in Rust, and streams one result row per group.
#[derive(Debug, Clone)]
pub struct RustGroup {
    /// Grouping variable names (empty ⇒ implicit grouping: one group over all rows).
    pub keys: Vec<String>,
    /// Aggregate output descriptors, in projection order.
    pub aggs: Vec<RustAgg>,
    /// Post-GROUP-BY expressions `(out_var, expr)` computed OVER the aggregate outputs
    /// (ADR-0025 Tier-2 gap 5) — e.g. `(?c := COUNT(?x) * 2)`. `expr` references the
    /// aggregate's internal output var (bound in the result row) and is evaluated by
    /// `eval_expr` in `rust_group_result_rows` after every aggregate is materialised. Empty
    /// for the common bare-rename case (handled by renaming the aggregate's own `out_var`).
    pub post_exprs: Vec<(String, spargebra::algebra::Expression)>,
}

/// One aggregate column in a [`RustGroup`].
#[derive(Debug, Clone)]
pub struct RustAgg {
    /// Output variable name.
    pub out_var: String,
    pub kind: AggKind,
    /// Input variable (`None` for `COUNT(*)`).
    pub arg_var: Option<String>,
    pub distinct: bool,
    pub fixed_type: Option<XsdTypeCode>,
}

/// A GROUP BY + aggregates carrier on a [`Branch`] (SPARQL §11). The branch's
/// `core`/`opts`/`where_conds` are the inner pattern's single-branch FROM/WHERE;
/// this records the grouping keys (lowered to their **raw key columns** — term
/// construction is rebuilt at projection, ADR-0007) and each aggregate output
/// column. v1 carries a single inner branch only (a multi-branch UNION/VALUES
/// inner cannot GROUP BY per arm in SQL → deferred 501 in [`crate::unfold`]).
#[derive(Debug, Clone)]
pub struct Aggregation {
    /// The grouping keys (empty ⇒ *implicit* grouping: one group over all rows,
    /// yielding one row even when the inner is empty).
    pub keys: Vec<GroupKey>,
    /// The aggregate output columns.
    pub aggs: Vec<AggCol>,
}

/// One GROUP BY key: an output variable lowered to the raw key columns it groups
/// by (the term map's columns). Grouping by the raw key ≡ grouping by the
/// constructed term (the term-construction-lifting injectivity assumption that
/// already underpins joins, ADR-0007); the term is rebuilt at projection.
#[derive(Debug, Clone)]
pub struct GroupKey {
    pub var: String,
    pub cols: Vec<ColRef>,
}

/// One aggregate output column: its result variable, the set function, the single
/// raw argument column (`None` for `COUNT(*)`), DISTINCT, and the synthetic output
/// column the SQL result is projected as (read back at reconstruction).
#[derive(Debug, Clone)]
pub struct AggCol {
    pub var: String,
    pub kind: AggKind,
    pub arg: Option<ColRef>,
    pub distinct: bool,
    pub out: ColRef,
    /// The fixed §10 result type, when the function pins it (COUNT ⇒ integer,
    /// AVG ⇒ decimal); `None` ⇒ take the value's resolved decltype/storage type
    /// (SUM/MIN/MAX keep the source numeric type).
    pub fixed_type: Option<XsdTypeCode>,
}

/// The set functions wired in v1 (SPARQL §11.4). GROUP_CONCAT / SAMPLE / custom
/// are deferred → 501 in [`crate::unfold`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

/// The columns referenced by a (non-constant) term map, in order.
pub fn term_map_columns(term_map: &TermMap) -> Vec<Box<str>> {
    match term_map {
        TermMap::Constant(_) => Vec::new(),
        TermMap::Column(c, _) => vec![c.clone()],
        TermMap::Template(t, _) => t
            .segments()
            .iter()
            .filter_map(|s| match s {
                sf_core::ir::Segment::Column(c) => Some(c.clone()),
                sf_core::ir::Segment::Literal(_) => None,
            })
            .collect(),
    }
}

/// The RDF term type a (non-constant) term map produces, if statically known.
pub fn term_map_type(term_map: &TermMap) -> Option<TermType> {
    match term_map {
        TermMap::Constant(_) => None,
        TermMap::Column(_, spec) | TermMap::Template(_, spec) => Some(spec.term_type),
    }
}

/// A SQL boolean condition over **raw columns** + bound-parameter constants
/// (ADR-0010 R1: values are bound parameters only, never inlined). Rendered by
/// [`crate::emit`]; the `String` payloads are bound at execution time.
#[derive(Debug, Clone)]
pub enum SqlCond {
    /// `l = r` — an inner-join key equality (raw columns).
    ColEq(ColRef, ColRef),
    /// `(l = r OR l IS NULL OR r IS NULL)` — the OPTIONAL shared-variable
    /// compatibility condition (ADR-0007 R1); **never** a plain `l = r`.
    NullSafeEq(ColRef, ColRef),
    /// `col <op> ?` — a comparison against a bound constant (its lexical form).
    Cmp(ColRef, CmpOp, String),
    /// A source-side string-match pushdown — the near-free FTS baseline
    /// (ADR-0020 §2): a SPARQL string FILTER lowered so the source index/scan does
    /// the work. `param` is the match operand and is a **bound parameter only**
    /// (ADR-0010 R1) — for a `LIKE`, `param` is the already-escaped,
    /// wildcard-anchored pattern (never concatenated into the SQL text); for a
    /// PostgreSQL regex it is the raw pattern. See [`StrMatchOp`].
    StrMatch {
        col: ColRef,
        op: StrMatchOp,
        param: String,
    },
    /// `col IS NOT NULL` — `BOUND(?v)`.
    IsNotNull(ColRef),
    /// `col IS NULL` — the NULL half of an OPTIONAL shared-variable compatibility
    /// guard when one side is a constant (ADR-0007 R1: an unbound variable is
    /// compatible with any value, so a nullable column must be admitted).
    IsNull(ColRef),
    Not(Box<SqlCond>),
    And(Vec<SqlCond>),
    Or(Vec<SqlCond>),
    /// `NOT EXISTS (SELECT 1 FROM <scans> WHERE <conds>)` — the correlated
    /// anti-join backing SPARQL MINUS (§8.3). `scans` is the right (minuend)
    /// pattern's `FROM`; `conds` are that pattern's own conditions plus the
    /// shared-variable correlation equalities, which reference the OUTER (left)
    /// scan aliases (term-construction lifting: raw-key equality stands in for RDF
    /// compatibility, ADR-0007). A left row survives iff no right row satisfies
    /// `conds`, so it is a pure filter — the left bag multiplicity is preserved and
    /// the right multiplicities never fan it out.
    NotExists {
        scans: Vec<Scan>,
        conds: Vec<SqlCond>,
    },
    /// `EXISTS (SELECT 1 FROM <scans> WHERE <conds>)` — the correlated semi-join
    /// backing SPARQL `FILTER EXISTS { P }` (§8.4). Semantics mirror `NotExists`
    /// but the sense is reversed: a left row survives iff at least one right row
    /// satisfies `conds`. The correlation equalities follow the same raw-key
    /// equality convention as `NotExists`.
    Exists {
        scans: Vec<Scan>,
        conds: Vec<SqlCond>,
    },
    /// `[NOT] EXISTS (<path WITH prelude> SELECT 1 FROM t{pc.alias} WHERE <conds>)` — a
    /// correlated semi/anti-join whose inner is a property-path CLOSURE (ADR-0025 Tier-2
    /// gap 1). Unlike `Exists`/`NotExists` (base-table `scans`), the inner relation is the
    /// recursive-CTE distinct-pairs table `t{pc.alias}(sf_s, sf_o)`; `conds` correlate its
    /// `sf_s`/`sf_o` columns with the outer bound columns. `negated` ⇒ NOT EXISTS / MINUS.
    PathExists {
        pc: PathClosure,
        conds: Vec<SqlCond>,
        negated: bool,
    },
    /// `render(t1) = render(t2)` — Run 4 Wave B3, the general fallback for two
    /// template-bound term definitions [`crate::unify::align_templates`] can
    /// neither align column-by-column (same segment kind-sequence) nor prove
    /// disjoint (a leading-literal-prefix conflict). Each side is the
    /// template's own segment list paired with its source alias (the SAME
    /// `Segment`s `align_templates` reads); [`crate::emit`] renders each side
    /// as a dialect-appropriate SQL string concatenation and compares the two
    /// with `=`. `align_templates` only ever builds this for the term classes
    /// where SQL lexical/string equality on the rendered form IS RDF term
    /// equality (IRIs; plain, untagged, untyped-or-`xsd:string` literals) —
    /// see its own doc comment for the restriction and the NULL-propagation
    /// soundness argument.
    ///
    /// The trailing `bool` is Run 4 B-repair FIX 2: whether BOTH sides are an
    /// IRI-kind template (`true`) — `align_templates`'s caller only ever
    /// builds this when `spec1.term_type == spec2.term_type`, so one flag
    /// covers both sides. R2RML/RFC 3987 template expansion percent-encodes
    /// a column's value for an IRI term type only (`sf_core::ir::Template::
    /// expand`'s `encode_iri` flag); a plain-literal template never encodes.
    /// [`crate::emit::render_template_concat`] must mirror that PER SEGMENT,
    /// or two differently-shaped templates whose column values disagree only
    /// by an encodable character compare wrong in both directions (see its
    /// doc comment).
    TemplateEq(Vec<Segment>, usize, Vec<Segment>, usize, bool),
}

/// How a [`SqlCond::StrMatch`] matches (the near-free FTS baseline, ADR-0020 §2).
///
/// **Dialect split.** `Like` is SQL `LIKE`, used to back SPARQL
/// `CONTAINS`/`STRSTARTS`/`STRENDS` — whose semantics are case-sensitive. Because
/// only PostgreSQL's `LIKE` is genuinely case-sensitive (SQLite `LIKE` is
/// ASCII-case-insensitive by default; MySQL's default collations are
/// case-insensitive), this op is emitted **only for PostgreSQL**; on the other
/// dialects the lowering reports the FILTER unsupported rather than a wrong
/// case-folding match (never silently wrong). `RegexMatch`/`RegexMatchI` are
/// PostgreSQL's POSIX-regex operators (`~` / `~*`), used only for `REGEX` on
/// PostgreSQL; SQLite has no built-in `REGEXP` operator, so `REGEX` there stays
/// unsupported (the FILTER is not rewritten — never silently dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrMatchOp {
    /// `col LIKE ? ESCAPE '\'` — substring/prefix/suffix match. Emitted only on
    /// PostgreSQL, whose `LIKE` is genuinely case-sensitive (SPARQL semantics).
    Like,
    /// PostgreSQL `col ~ ?` — case-sensitive POSIX regex.
    RegexMatch,
    /// PostgreSQL `col ~* ?` — case-insensitive POSIX regex (`REGEX(?,?, "i")`).
    RegexMatchI,
}

/// One ORDER BY key (SPARQL §15.1): a sort direction plus either a plain bound
/// variable (`expr = None`) or a complex SPARQL expression (`expr = Some`). The
/// ordering is over the SPARQL value space — execution pins it so an unbound
/// (NULL) key sorts FIRST for ASC and LAST for DESC (never the dialect default),
/// and bound terms order blank-node < IRI < literal.
///
/// For expression keys the exec layer evaluates the expression against each
/// solution's binding map and stores the result under `var` (a synthetic name
/// `__sf_ord_{n}`) before sorting, so `order_cmp` can look up the key by name.
#[derive(Debug, Clone)]
pub struct OrderKey {
    pub var: String,
    pub descending: bool,
    /// Set for non-variable ORDER BY expressions (e.g. `STRLEN(?x)`). The exec
    /// layer evaluates this and injects the result into the solution before sorting.
    pub expr: Option<Box<Expression>>,
}

/// Comparison operators supported by pushed-down FILTERs (ADR-0007 v1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn as_sql(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

/// One `FROM` relation: a base table (`rr:tableName`) or an R2RML view
/// (`rr:sqlQuery`), bound to a scan alias.
#[derive(Debug, Clone)]
pub struct Scan {
    pub alias: usize,
    pub source: LogicalSource,
}

/// A single OPTIONAL right side rendered as a SQL `LEFT JOIN` (ADR-0007 R1–R5).
///
/// v1 supports a **single-scan** right side (the common `OPTIONAL { ?s :p ?o }`).
/// Multi-scan OPTIONAL right sides are deferred (a derived-table LEFT JOIN; TODO,
/// ADR-0007).
#[derive(Debug, Clone)]
pub struct OptJoin {
    pub scan: Scan,
    /// Shared-variable compatibility conditions — emitted NULL-safe (R1).
    pub on: Vec<SqlCond>,
    /// A FILTER **inside** the OPTIONAL — goes in the `ON`, never the outer WHERE
    /// (R5).
    pub extra: Vec<SqlCond>,
}

/// How a [`PathClosure`]'s [`HopExpr`] is iterated (SPARQL 1.1 §9 path semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// Exactly the hop relation, distinct node pairs — one step, no closure:
    /// `^p`, `p/q`, `p|q`, `!p`, or any sequence/alternative/inverse composite.
    One,
    /// `p?` — the hop ∪ the reflexive `(x, x)` pairs over the active graph's
    /// nodes (the ZeroLengthPath of §9.3). Only emitted over a single-predicate
    /// bare-leaf hop (so the hop's node set equals the graph's); else 501.
    ZeroOrOne,
    /// `P+` — the transitive closure (1+ hops), no reflexive pairs.
    OneOrMore,
    /// `P*` — the transitive closure ∪ the reflexive `(x, x)` pairs (same
    /// single-predicate-graph restriction as `ZeroOrOne`).
    ZeroOrMore,
}

/// The one-hop relation a [`PathClosure`] iterates, built from the mapping over
/// **raw key columns** (term-construction lifting, ADR-0007). Each node carries a
/// known [`HopRelation`] leaf; the composite operators (`Inverse`/`Seq`/`Alt`/
/// `Nps`) compile to nested subqueries each yielding the canonical `(sf_s, sf_o)`
/// pair. Raw-key equality stands in for RDF-term equality at every junction;
/// `unfold`'s `path::compile` only builds a node when the meeting endpoints share a
/// *node shape* (so equal raw key ⟺ equal RDF term), else it defers to 501.
#[derive(Debug, Clone)]
pub enum HopExpr {
    /// A single mapped predicate's `(subject-key, object-key)` base relation.
    Pred(HopRelation),
    /// `^p` — swap the inner hop's subject and object.
    Inverse(Box<HopExpr>),
    /// `p/q` — join the two inner hops on the shared middle node (`a.sf_o = b.sf_s`).
    Seq(Box<HopExpr>, Box<HopExpr>),
    /// `p|q|…` — the **set** union of the inner hops' pairs (the `spareval` oracle
    /// evaluates an alternative with set semantics; emission dedups via `UNION`).
    Alt(Vec<HopExpr>),
    /// `!(p1|…)` — a negated property set: the union of the *complement* predicate
    /// hops. Unlike `Alt`, the oracle gives this **bag** semantics (one solution per
    /// matching triple — §18.2.2 length-one path), so emission is a `UNION ALL` over
    /// the per-predicate distinct pairs and the `PathKind::One` wrapper omits its
    /// outer `DISTINCT`. A pair connected by two complement predicates therefore
    /// yields two solutions, matching the oracle (never silently undercounted).
    Nps(Vec<HopExpr>),
}

impl HopExpr {
    /// The single base relation iff this hop is a bare predicate leaf — the only
    /// shape over which `P*`/`p?` reflexive `(x, x)` enumeration is emitted.
    pub fn as_pred(&self) -> Option<&HopRelation> {
        match self {
            HopExpr::Pred(r) => Some(r),
            _ => None,
        }
    }
}

/// A property-path closure (ADR-0007 *recursive paths compile to source-dialect
/// recursive CTEs*, ADR-0008 `owl:TransitiveProperty` served live, ADR-0010
/// *recursion bounds*).
///
/// A branch carrying a `PathClosure` has an **empty `core`**: its `FROM` is the
/// (possibly recursive) CTE, not a base scan. The CTE runs over the **raw key
/// columns** of the one-hop relation (term-construction lifting, ADR-0007): it
/// projects two canonical key columns (`sf_s`, `sf_o`) and (for the recursive
/// kinds) a depth counter; the branch's `bindings` build the RDF subject/object
/// terms from those keys at the outer projection only. Connectivity is
/// **set-based** over node pairs (SPARQL `P+`/`P*` semantics): emission wraps the
/// relation in a `SELECT DISTINCT sf_s, sf_o` so a pair reached at several depths
/// or around a cycle yields one solution. The recursion (for `OneOrMore`/
/// `ZeroOrMore`) terminates **only** via `sf_d < max_depth` (the depth key keeps
/// cyclic revisits distinct in the recursive member; the SQLite `CYCLE` clause is
/// the later MB-4 wave). `max_depth` is the ADR-0010 bound.
#[derive(Debug, Clone)]
pub struct PathClosure {
    /// The scan alias the CTE is bound to; the outer projection reads
    /// `t{alias}.sf_s` / `t{alias}.sf_o`.
    pub alias: usize,
    /// Which path operator this closure realises.
    pub kind: PathKind,
    /// The one-hop relation the closure iterates (a predicate leaf or a
    /// sequence/alternative/inverse composite).
    pub hop: HopExpr,
    /// ADR-0010 recursion-depth backstop (the `WHERE sf_d < max_depth` guard).
    pub max_depth: usize,
}

/// The one-hop `(subject-key, object-key)` relation a [`PathClosure`] closes
/// over: a single base source plus the two **raw key columns** the predicate's
/// subject / object term maps read. Raw-column equality stands in for RDF-term
/// equality here (the v1 simple-predicate case: subject and object term maps key
/// the same node domain); the bindings rebuild the terms at projection time.
#[derive(Debug, Clone)]
pub struct HopRelation {
    pub source: LogicalSource,
    /// The raw column producing the hop's subject key.
    pub subj_col: Box<str>,
    /// The raw column producing the hop's object key.
    pub obj_col: Box<str>,
}

/// A derived-table (SubPlan) join: a sub-`Plan` lowered to `(SELECT …) AS t{alias}`.
///
/// The parent branch joins the derived table on `on` conditions (column equalities
/// over the shared SPARQL variables). `left = true` emits a `LEFT JOIN` (the
/// LeftJoinJoinLimit case, ADR-0023 M5 Wave 2); `left = false` emits an
/// `INNER JOIN … ON` (a modifier-bearing subquery as a join/filter input).
///
/// **Flat branches NEVER set this field** — it is always an empty `Vec` for
/// branches produced by the flat [`crate::unfold`] path. That invariant keeps the
/// flat SQL output byte-identical after the M5 Wave 2 changes (ADR-0023 §5.4).
#[derive(Debug, Clone)]
pub struct SubPlanJoin {
    pub alias: usize,
    pub plan: Box<crate::Plan>,
    pub on: Vec<SqlCond>,
    pub left: bool,
}

/// One compiled SQL `SELECT`. The conjunctive core (inner joins) is a set of
/// scans plus key equalities applied in `WHERE` (CROSS JOIN + WHERE-eq ≡ inner
/// join — emission renders it that way); OPTIONALs are `LEFT JOIN`s layered on
/// top (R5: an outer FILTER is in `where_conds`, never pushed onto the preserved
/// side).
#[derive(Debug, Clone)]
pub struct Branch {
    pub core: Vec<Scan>,
    pub opts: Vec<OptJoin>,
    /// Variable → its term definition (the output binding environment).
    pub bindings: BTreeMap<String, TermDef>,
    /// `WHERE` conditions: core key equalities, constant-position filters, and
    /// post-OPTIONAL FILTERs (R5).
    pub where_conds: Vec<SqlCond>,
    pub distinct: bool,
    pub limit: Option<usize>,
    pub offset: usize,
    /// ORDER BY keys, pushed into this branch's SQL (the single-branch case;
    /// [`crate::Plan::prepared_branches`]). A multi-branch bag-union cannot order
    /// in per-branch SQL — that global sort happens in [`crate::exec`].
    pub order: Vec<OrderKey>,
    /// When set, this branch is a recursive property-path closure: its `FROM` is
    /// the CTE (empty `core`), not a base scan (ADR-0007 recursive paths).
    pub path: Option<PathClosure>,
    /// When set, this branch is a GROUP BY + aggregates over its inner FROM/WHERE
    /// (SPARQL §11): emission renders `GROUP BY <key cols>` + the aggregate SQL,
    /// and reconstruction reads the grouping keys + aggregate result columns.
    pub agg: Option<Aggregation>,
    /// Derived-table (SubPlan) joins — nested Plans emitted as `(SELECT …) AS t{alias}`.
    /// Always `Vec::new()` for flat-path branches; only populated by the tree path's
    /// `lower_as_subplan` (ADR-0023 M5 Wave 2: closes §5.4 nested-modifier 501 sites).
    pub subplan_joins: Vec<SubPlanJoin>,
    /// `true` once this branch has absorbed an NPS (negated property set, `!p`)
    /// closure's own `UNION ALL` bag — set by `iq::lower::convert_path_branches`
    /// when `path.hop` is `HopExpr::Nps`, and OR-preserved through every later
    /// `unfold::merge` (never cleared). NPS is the ONE path kind with its own
    /// documented, LEGITIMATE bag multiplicity that survives `=_bag` on purpose
    /// (`iq.rs`'s own `HopExpr::Nps` doc: "a triple connected by two complement
    /// predicates therefore yields two solutions") — every OTHER path kind
    /// already self-dedups via its closure's own `SELECT DISTINCT sf_s, sf_o`,
    /// so this flag is never needed there. ADR-0034's D1 sets `Branch::distinct`
    /// per PATTERN, before that pattern is known to end up merged alongside an
    /// unrelated NPS closure — `merge` ORs `distinct` forward so a genuinely
    /// unkeyed sibling table's own duplicate-row need survives the join
    /// (`cross_source_with_duplicate_bag_multiplicity_diverges_from_oracle`),
    /// but blindly OR-ing `distinct` INTO an `nps` branch would apply that
    /// `SELECT DISTINCT` over NPS's own protected columns too, silently
    /// collapsing two DIFFERENT underlying triples (different predicates, both
    /// outside the negated set) that legitimately produce the same output
    /// values — found via `differential_paths.rs`'s `negated_property_set_
    /// multiplicity_joined_engine_matches_oracle_bag_counts` once `merge`
    /// started OR-ing at all. `merge` checks this flag and skips the OR (in
    /// EITHER direction) whenever it is set on either side.
    pub nps: bool,
}

impl Branch {
    /// An empty branch (no relations yet) — the identity for BGP joining.
    pub fn empty() -> Self {
        Self {
            core: Vec::new(),
            opts: Vec::new(),
            bindings: BTreeMap::new(),
            where_conds: Vec::new(),
            distinct: false,
            limit: None,
            offset: 0,
            order: Vec::new(),
            path: None,
            agg: None,
            subplan_joins: Vec::new(),
            nps: false,
        }
    }

    /// A branch over a single scan with no conditions.
    pub fn single(scan: Scan) -> Self {
        let mut b = Self::empty();
        b.core.push(scan);
        b
    }

    /// Each scan alias paired with the source it reads (core scans, OPTIONAL right
    /// sides, and a property-path CTE's one-hop source). Used to look up a source's
    /// actual column names for dialect identifier resolution at emission.
    pub fn alias_sources(&self) -> Vec<(usize, &LogicalSource)> {
        let mut out: Vec<(usize, &LogicalSource)> = Vec::new();
        for s in &self.core {
            out.push((s.alias, &s.source));
        }
        for o in &self.opts {
            out.push((o.scan.alias, &o.scan.source));
        }
        // A MINUS anti-join carries the right (minuend) pattern's scans inside a
        // `NotExists` WHERE condition; surface them so the executor probes their
        // column catalog (SQL:2008 identifier folding) like any other scan.
        for cond in &self.where_conds {
            collect_not_exists_scans(cond, &mut out);
        }
        // A property-path closure's leaf sources are resolved directly against the
        // column catalog at emission (its CTE alias projects the canonical `sf_s` /
        // `sf_o` keys, not raw base columns), so it contributes no alias→source
        // entry here.
        out
    }

    /// Scan/derived-table aliases whose columns can be NULL because the row came
    /// from the no-match side of a LEFT JOIN — the trigger for the R1 null-safe ON,
    /// the R2 `COALESCE` projection, and the EXISTS-substitution unbound-variable
    /// guard. This is the union of:
    ///
    /// * every prior-OPTIONAL scan alias (`opts` — the `OptJoin` right sides), and
    /// * every LEFT-JOINed SubPlan derived-table alias (`subplan_joins` with
    ///   `left == true` — a modifier sub-SELECT attached as an OPTIONAL's right
    ///   operand, ADR-0023 Item 1d): when that LEFT JOIN finds no match the
    ///   subplan's output variables are UNBOUND, exactly like an unmatched OPTIONAL.
    ///
    /// Every downstream detector that decides "might this variable be unbound?"
    /// ([`crate::leftjoin::def_is_nullable`], the tree's `def_reads_opt_alias`) MUST
    /// consult THIS set, not `opts` alone — a variable bound by a LEFT-JOINed SubPlan
    /// is just as nullable as one bound by an unmatched OPTIONAL, and treating it as
    /// mandatory silently corrupts a correlating EXISTS / NOT EXISTS / MINUS /
    /// second-OPTIONAL (ADR-0007). Flat-path branches never set `subplan_joins`, so
    /// this degrades to exactly the old `opts`-only set there (no flat SQL change).
    pub fn nullable_aliases(&self) -> HashSet<usize> {
        let mut aliases: HashSet<usize> = self.opts.iter().map(|o| o.scan.alias).collect();
        for sp in &self.subplan_joins {
            if sp.left {
                aliases.insert(sp.alias);
            }
        }
        aliases
    }

    /// All raw columns the branch must project: every binding's columns plus
    /// every column mentioned in a condition. De-duplicated, deterministic order.
    pub fn projection(&self) -> Vec<ColRef> {
        let mut cols: Vec<ColRef> = Vec::new();
        let push = |c: ColRef, cols: &mut Vec<ColRef>| {
            if !cols.contains(&c) {
                cols.push(c);
            }
        };
        for def in self.bindings.values() {
            for c in def.columns() {
                push(c, &mut cols);
            }
        }
        // When this branch carries a single-branch DISTINCT pushed into the SQL
        // (`SELECT DISTINCT …`), the SELECT list MUST be exactly the output-determining
        // set the executor reconstructs — i.e. the binding columns only. The condition
        // columns below are enforced in WHERE / JOIN-ON and are never read back by
        // `exec::reconstruct` (it iterates `bindings`), so including them in a DISTINCT
        // projection over-widens the dedup key and collapses nothing (e.g. q15: a join
        // key that varies per joined row would defeat `DISTINCT ?route`). Skipping them
        // under DISTINCT makes the dedup run over the projected key alone; for every
        // non-DISTINCT branch the loops still run, keeping its SELECT list byte-identical.
        if !self.distinct {
            for cond in &self.where_conds {
                collect_cond_cols(cond, &mut |c| push(c.clone(), &mut cols));
            }
            for opt in &self.opts {
                for cond in opt.on.iter().chain(opt.extra.iter()) {
                    collect_cond_cols(cond, &mut |c| push(c.clone(), &mut cols));
                }
            }
            for sp in &self.subplan_joins {
                for cond in &sp.on {
                    collect_cond_cols(cond, &mut |c| push(c.clone(), &mut cols));
                }
            }
        }
        cols
    }
}

/// Walk every [`ColRef`] mentioned by a condition.
pub fn collect_cond_cols(cond: &SqlCond, f: &mut impl FnMut(&ColRef)) {
    match cond {
        SqlCond::ColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            f(a);
            f(b);
        }
        SqlCond::Cmp(a, _, _) | SqlCond::IsNotNull(a) | SqlCond::IsNull(a) => f(a),
        SqlCond::StrMatch { col, .. } => f(col),
        SqlCond::Not(c) => collect_cond_cols(c, f),
        SqlCond::And(cs) | SqlCond::Or(cs) => {
            for c in cs {
                collect_cond_cols(c, f);
            }
        }
        // `NotExists` and `Exists` (MINUS / FILTER EXISTS) are opaque to outer
        // column collection: their inner conditions reference the subquery's own
        // scans, and the outer correlation columns are already projected via the
        // outer bindings — nothing is added here.
        SqlCond::NotExists { .. } | SqlCond::Exists { .. } | SqlCond::PathExists { .. } => {}
        // Run 4 Wave B3: every `Segment::Column` on EITHER side is a real column
        // reference against that side's alias — the FK/PK elimination safety
        // checks (`joinelim.rs`'s `parent_referenced_only_via[_set]`) that consume
        // this walk MUST see them, or a parent scan a `TemplateEq` still needs
        // could be eliminated out from under it, leaving a dangling alias.
        SqlCond::TemplateEq(sx, a1, sy, a2, _) => {
            for seg in sx {
                if let Segment::Column(c) = seg {
                    f(&ColRef::new(*a1, c.clone()));
                }
            }
            for seg in sy {
                if let Segment::Column(c) = seg {
                    f(&ColRef::new(*a2, c.clone()));
                }
            }
        }
    }
}

/// Walk every scan a `NotExists` or `Exists` subquery carries in `cond`
/// (recursing through boolean combinators), pairing each with its source for
/// catalog lookup.
fn collect_not_exists_scans<'a>(cond: &'a SqlCond, out: &mut Vec<(usize, &'a LogicalSource)>) {
    match cond {
        SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
            for s in scans {
                out.push((s.alias, &s.source));
            }
            for c in conds {
                collect_not_exists_scans(c, out);
            }
        }
        SqlCond::Not(c) => collect_not_exists_scans(c, out),
        SqlCond::And(cs) | SqlCond::Or(cs) => {
            for c in cs {
                collect_not_exists_scans(c, out);
            }
        }
        _ => {}
    }
}
