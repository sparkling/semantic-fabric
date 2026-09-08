//! Property-path compilation (ADR-0007 *recursive paths compile to source-dialect
//! recursive CTEs*; ADR-0008 transitive properties served live; ADR-0049 cycle
//! governance). Translates a `?s PATH ?o` pattern into a [`PathClosure`] branch whose
//! one-hop relation ([`HopExpr`]) is a predicate leaf or a sequence/alternative/
//! inverse/negated composite over **raw key columns** (term-construction lifting):
//! the closure iterates the keys and the RDF terms are built only at the outer
//! projection ([`crate::exec`]).
//!
//! # Soundness — raw-key equality stands in for RDF-term equality
//!
//! The model joins on *raw source keys* at every junction (the recursive
//! `c.sf_o = h.sf_s` step, a `p/q` middle node, a `p|q` / `!p` union). That is
//! sound **only** when the two term maps meeting at the junction produce equal RDF
//! terms for equal raw keys — i.e. they share a *node shape* ([`node_shape`]: the
//! term type + datatype/language/base + the template's literal skeleton). Every
//! composite checks the relevant shapes and **defers to 501** when they differ,
//! rather than emit a wrong join across heterogeneous IRI templates.
//!
//! # Supported vs deferred (SPARQL 1.1 §9)
//!
//! * `^p` (inverse), `p/q` (sequence, matching middle shape), `p|q` (alternative,
//!   matching endpoint shapes), `!p` / `!(p1|…)` (negated property set, enumerated
//!   over the finite R2RML predicate set), and `P+`/`(composite)+` (transitive
//!   closure, closable when subject/object shapes match) — all supported.
//! * `P*` and `p?` add the reflexive ZeroLengthPath `(x, x)` over **every** node of
//!   the active graph (§9.3). The raw-key model can enumerate that only over a
//!   single-predicate graph whose one predicate is this hop's bare leaf (so the
//!   hop's node set equals the graph's); otherwise → 501.
//! * A bound endpoint, a nested closure inside a composite, a `refObjectMap`-joined
//!   or multi-mapping or multi-column predicate, and a non-constant/`rr:class`
//!   predicate under `!p` — all stay explicit 501s (never silently wrong).

use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};
use sf_core::Term;
use spargebra::algebra::PropertyPathExpression;
use spargebra::term::TermPattern;

use crate::iq::{
    mapping_term_def, Branch, HopExpr, HopRelation, PathClosure, PathKind, R2rmlGraphScope, TermDef,
};
use crate::unfold::{bind, Unfolder};
use crate::{Error, Result};

mod mapping_work;

/// A compiled one-hop relation plus the term maps and node shapes its endpoints
/// reconstruct from / are checked against.
struct CompiledHop {
    /// The relation, ready for emission.
    expr: HopExpr,
    /// The subject endpoint's term map (rebuilt to read `sf_s` at projection).
    subj_map: TermMap,
    /// The object endpoint's term map (rebuilt to read `sf_o` at projection).
    obj_map: TermMap,
    /// The subject endpoint's node shape (for closability / composition checks).
    subj_shape: NodeShape,
    /// The object endpoint's node shape.
    obj_shape: NodeShape,
    /// `Some(iri)` iff this hop is a bare predicate leaf — the only shape over
    /// which reflexive (`P*`/`p?`) enumeration is permitted.
    single_pred: Option<String>,
}

/// A canonical signature of how a single-column term map constructs an RDF term
/// from its raw key. Two term maps with equal shapes produce equal terms iff their
/// raw key values are equal — the soundness predicate for raw-key joins.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeShape(String);

impl<'a> Unfolder<'a> {
    /// Translate a property-path pattern `?s PATH ?o` into a [`PathClosure`] branch
    /// (see the module docs for the supported surface).
    pub(crate) fn path_branch(
        &mut self,
        subject: &TermPattern,
        path: &PropertyPathExpression,
        object: &TermPattern,
    ) -> Result<Branch> {
        self.work_checkpoint()?;
        use PropertyPathExpression as P;
        let (kind, inner): (PathKind, &PropertyPathExpression) = match path {
            P::OneOrMore(p) => (PathKind::OneOrMore, p),
            P::ZeroOrMore(p) => (PathKind::ZeroOrMore, p),
            P::ZeroOrOne(p) => (PathKind::ZeroOrOne, p),
            // A single predicate or a bare sequence/alternative/inverse/negated
            // composite (no closure operator) is one step.
            other => (PathKind::One, other),
        };

        // Recursive syntax and duplicate-elimination semantics are proven only
        // on the three currently evidenced relational emission targets. This is
        // a compiler capability boundary, not production backend admission. In
        // particular, SQL Server does not accept the generic `WITH RECURSIVE`
        // form. Reject during
        // translation rather than emit unproven SQL or risk a non-terminating /
        // incomplete closure on a scaffolded backend (ADR-0049 R4).
        if matches!(kind, PathKind::OneOrMore | PathKind::ZeroOrMore)
            && !self.dialect.supports_recursive_paths()
        {
            return Err(Error::Unsupported(
                "recursive P+/P* requires a proven finite-pair fixed point; this SQL dialect \
                 is not admitted for recursive property paths → 501"
                    .to_owned(),
            ));
        }

        let (subj_var, obj_var) = match (subject, object) {
            (TermPattern::Variable(s), TermPattern::Variable(o)) => (s.as_str(), o.as_str()),
            _ => {
                return Err(Error::Unsupported(
                    "property path with a bound endpoint deferred → 501 (v1 = ?s PATH ?o)"
                        .to_owned(),
                ))
            }
        };

        let compiled = self.compile_path(inner)?;

        // A transitive closure walks within ONE node domain, so the hop's subject
        // and object endpoints must share a node shape (equal raw key ⟺ equal term
        // around the `c.sf_o = h.sf_s` step).
        if matches!(kind, PathKind::OneOrMore | PathKind::ZeroOrMore)
            && compiled.subj_shape != compiled.obj_shape
        {
            return Err(Error::Unsupported(
                "P+/P* over a composite whose subject and object node shapes differ \
                 cannot be closed soundly (the recursive raw-key join would cross \
                 heterogeneous term domains) → 501"
                    .to_owned(),
            ));
        }

        // `P*`/`p?` bind the reflexive `(x, x)` for EVERY node of the active graph
        // (SPARQL §9.3). Enumerating every node of the active graph requires a union
        // over ALL tables in the mapping (every subject and object column), which is
        // architecturally constrained (ADR-0007): the raw-key CTE uses a single term
        // map for reconstruction; mixing nodes from tables with different term maps in
        // the same CTE would reconstruct wrong RDF terms.
        //
        // The one sound case: a bare single-predicate hop over a graph that uses
        // ONLY that predicate — all graph nodes are the hop's own rows, so the CTE
        // node universe equals the active graph.
        //
        // ADR-0007 decision: any other shape → 501. This is NOT a deferral; it is a
        // documented architectural bound. Lifting it requires either materialising the
        // full node universe in SQL (one column per term map, outer UNION) or
        // switching to a term-first CTE model — both are ADR-0020 / MB-5 scope.
        if matches!(kind, PathKind::ZeroOrMore | PathKind::ZeroOrOne) {
            let reflexive_ok = compiled.expr.as_pred().is_some()
                && match compiled.single_pred.as_deref() {
                    Some(p) => self.graph_is_single_predicate(p)?,
                    None => false,
                };
            if !reflexive_ok {
                return Err(Error::Unsupported(
                    "P*/p? reflexive ZeroLengthPath: graph node enumeration is ADR-0007 \
                     bounded — supported only over a single-predicate, single-table mapping \
                     (any multi-predicate or composite hop would reconstruct from a \
                     mixed-term-map CTE → silent data corruption) → 501"
                        .to_owned(),
                ));
            }
        }

        let alias = self.alias();
        let graph_scope = match self.current_graph.as_ref() {
            None => R2rmlGraphScope::Default,
            Some(graph) => R2rmlGraphScope::Mapped {
                term_map: TermMap::Constant(Term::NamedNode(graph.clone())),
                alias,
            },
        };
        let subj_def = mapping_term_def(
            &rewrite_single_col(&compiled.subj_map, "sf_s")?,
            alias,
            graph_scope.clone(),
        );
        let obj_def = mapping_term_def(
            &rewrite_single_col(&compiled.obj_map, "sf_o")?,
            alias,
            graph_scope,
        );

        let mut branch = Branch::empty();
        branch.path = Some(PathClosure {
            alias,
            kind,
            hop: compiled.expr,
        });
        // Bind via the shared helper so `?s PATH ?s` self-unifies (ColEq sf_s,sf_o).
        bind(&mut branch, subj_var, subj_def)?;
        match bind(&mut branch, obj_var, obj_def)? {
            true => Ok(branch),
            false => Err(Error::Unsupported(
                "property-path endpoints unify to empty → 501".to_owned(),
            )),
        }
    }

    /// ADR-0035 item 3: `GRAPH ?v { …PATH… }` — a [`PathClosure`] needs ONE pinned
    /// graph per compiled CTE, so a variable graph cannot fan out per-branch the way
    /// an ordinary triple pattern does ([`Unfolder::pattern_branches`]/[`Unfolder::
    /// atom`]). Instead this compiles as a union over [`crate::unfold::declared_
    /// constant_graphs`]: one [`Self::path_branch`] call per declared constant graph
    /// `g_i` (`current_graph = Some(g_i)`, reusing the SAME single-graph closure
    /// compiler verbatim — "each already works post-Run-4"), with `v` bound to
    /// `Const(g_i)` on the resulting branch. A `g_i` this path's own predicate(s)
    /// never actually use still compiles fine — [`Self::resolve_pred_hop`]'s own
    /// graph-scoped/graceful-empty search just yields a provably empty relation for
    /// it — so over-enumerating every declared constant graph is sound (never
    /// silently drops a valid graph) even though some arms turn out empty.
    ///
    /// Boundary (ADR-0035, "paths under non-constant graph maps → 501"): when
    /// [`Self::has_non_constant_graph_map`] finds a template/column
    /// `rr:graphMap` ANYWHERE in the mapping, the true named-graph set for some
    /// predicate is row-dependent (not statically enumerable), so this refuses
    /// outright rather than answering over the constant subset alone (unsound —
    /// could silently omit rows for that predicate). Otherwise `declared_constant_
    /// graphs` IS the complete enumeration, and an empty result from it is a sound
    /// empty answer (a mapping with no named graphs at all), never a 501.
    pub(crate) fn path_branches_for_graph_var(
        &mut self,
        subject: &TermPattern,
        path: &PropertyPathExpression,
        object: &TermPattern,
        var: &str,
    ) -> Result<Vec<Branch>> {
        self.work_checkpoint()?;
        if self.has_non_constant_graph_map()? {
            return Err(Error::Unsupported(
                "GRAPH ?v { …PATH… } over a mapping declaring a template/column \
                 rr:graphMap is deferred → 501 (a PathClosure needs one pinned graph per \
                 compiled CTE; that graph set is row-dependent, not statically enumerable \
                 — ADR-0035)"
                    .to_owned(),
            ));
        }
        let graphs = self.declared_constant_graphs()?;
        self.reserve_product(&[graphs.len()])?;
        let mut out = Vec::with_capacity(graphs.len());
        for g in graphs {
            let pinned = self.copy_graph_name(&g)?;
            let saved_g = self.current_graph.take();
            let saved_v = self.current_graph_var.take();
            self.current_graph = Some(pinned);
            let branch = self.path_branch(subject, path, object);
            self.current_graph = saved_g;
            self.current_graph_var = saved_v;
            let mut branch = branch?;
            if bind(&mut branch, var, TermDef::Const(Term::NamedNode(g)))? {
                out.push(branch);
            }
        }
        self.work_checkpoint()?;
        Ok(out)
    }

    /// Compile a (closure-free) path sub-expression into a [`CompiledHop`].
    fn compile_path(&self, path: &PropertyPathExpression) -> Result<CompiledHop> {
        self.work_checkpoint()?;
        use PropertyPathExpression as P;
        match path {
            P::NamedNode(p) => self.resolve_pred_hop(p.as_str()),
            P::Reverse(inner) => {
                let c = self.compile_path(inner)?;
                reject_nested_nps(&c.expr)?;
                Ok(CompiledHop {
                    expr: HopExpr::Inverse(Box::new(c.expr)),
                    subj_map: c.obj_map,
                    obj_map: c.subj_map,
                    subj_shape: c.obj_shape,
                    obj_shape: c.subj_shape,
                    single_pred: None,
                })
            }
            P::Sequence(a, b) => {
                let ca = self.compile_path(a)?;
                let cb = self.compile_path(b)?;
                reject_nested_nps(&ca.expr)?;
                reject_nested_nps(&cb.expr)?;
                if ca.obj_shape != cb.subj_shape {
                    return Err(Error::Unsupported(
                        "p/q sequence: the left object and right subject term maps have \
                         different node shapes — a raw-key join on the middle node would \
                         be unsound → 501"
                            .to_owned(),
                    ));
                }
                Ok(CompiledHop {
                    expr: HopExpr::Seq(Box::new(ca.expr), Box::new(cb.expr)),
                    subj_map: ca.subj_map,
                    obj_map: cb.obj_map,
                    subj_shape: ca.subj_shape,
                    obj_shape: cb.obj_shape,
                    single_pred: None,
                })
            }
            P::Alternative(a, b) => {
                let ca = self.compile_path(a)?;
                let cb = self.compile_path(b)?;
                reject_nested_nps(&ca.expr)?;
                reject_nested_nps(&cb.expr)?;
                if ca.subj_shape != cb.subj_shape || ca.obj_shape != cb.obj_shape {
                    return Err(Error::Unsupported(
                        "p|q alternative: the two branches have different endpoint node \
                         shapes — the union cannot be reconstructed with one term map → 501"
                            .to_owned(),
                    ));
                }
                let mut parts = flatten_alt(ca.expr);
                parts.extend(flatten_alt(cb.expr));
                Ok(CompiledHop {
                    expr: HopExpr::Alt(parts),
                    subj_map: ca.subj_map,
                    obj_map: ca.obj_map,
                    subj_shape: ca.subj_shape,
                    obj_shape: ca.obj_shape,
                    single_pred: None,
                })
            }
            P::NegatedPropertySet(negated) => self.compile_nps(negated),
            // A nested closure operator as a hop sub-relation (e.g. `(p+)/q`) is
            // not supported; composite hops cover sequence/alternative/inverse/NPS
            // of predicates only.
            P::ZeroOrMore(_) | P::OneOrMore(_) | P::ZeroOrOne(_) => Err(Error::Unsupported(
                "nested closure operator inside a composite path → 501 \
                 (composite hops cover sequence/alternative/inverse/NPS of predicates)"
                    .to_owned(),
            )),
        }
    }
}

/// Convert a real, well-typed [`CompiledHop`] into one that provably yields
/// ZERO rows — reusing its term maps/shapes verbatim (still-sound reconstruction
/// specs; composite shape-matching needs them regardless of whether any row
/// ever flows through this hop) and swapping only the relation for a synthetic
/// empty derived table, the same statically-empty-derived-table idiom
/// `emit::emit_subplan_sql` uses for an empty SubPlan (`SELECT 1 AS __sf_empty
/// WHERE 1 = 0`).
fn empty_hop(hop: CompiledHop) -> CompiledHop {
    CompiledHop {
        expr: HopExpr::Pred(HopRelation {
            source: LogicalSource::Query("SELECT 1 AS s, 1 AS o WHERE 1 = 0".to_owned()),
            subj_col: "s".into(),
            obj_col: "o".into(),
        }),
        ..hop
    }
}

/// Flatten a top-level `Alt` so `(p|q)|r` yields one `Alt(vec![p, q, r])`.
fn flatten_alt(expr: HopExpr) -> Vec<HopExpr> {
    match expr {
        HopExpr::Alt(v) => v,
        other => vec![other],
    }
}

/// Whether a hop sub-tree contains a negated property set anywhere.
fn contains_nps(expr: &HopExpr) -> bool {
    match expr {
        HopExpr::Nps(_) => true,
        HopExpr::Pred(_) => false,
        HopExpr::Inverse(i) => contains_nps(i),
        HopExpr::Seq(a, b) => contains_nps(a) || contains_nps(b),
        HopExpr::Alt(v) => v.iter().any(contains_nps),
    }
}

/// A negated property set carries **bag** semantics (one solution per matching
/// triple) that the engine preserves only when the NPS is the whole hop at
/// `PathKind::One` (no outer `DISTINCT`) or sits under a transitive closure (the
/// closure's own `DISTINCT` makes the result set-valued regardless). Nested inside
/// a set-semantics composite (`^`, `/`, `|`) at length one, the bag would be either
/// collapsed by the surrounding `DISTINCT` (undercount) or multiplied by a join —
/// neither provably matches the oracle, so defer rather than emit a wrong answer.
fn reject_nested_nps(expr: &HopExpr) -> Result<()> {
    if contains_nps(expr) {
        return Err(Error::Unsupported(
            "negated property set nested inside an inverse/sequence/alternative \
             composite carries bag semantics the set-valued composite cannot \
             preserve soundly → 501"
                .to_owned(),
        ));
    }
    Ok(())
}

/// The canonical node shape of a single-column term map (the soundness key for
/// raw-key joins; see the module docs). A constant term map has no key column and
/// cannot be a path endpoint → 501.
fn node_shape(tm: &TermMap) -> Result<NodeShape> {
    let s = match tm {
        TermMap::Column(_, spec) => format!("col\u{1}{}", spec_tag(spec)),
        TermMap::Template(t, spec) => {
            let mut parts = format!("tmpl\u{1}{}", spec_tag(spec));
            for seg in t.segments() {
                match seg {
                    Segment::Literal(l) => {
                        parts.push('\u{1}');
                        parts.push('L');
                        parts.push_str(l);
                    }
                    Segment::Column(_) => {
                        parts.push('\u{1}');
                        parts.push('C');
                    }
                }
            }
            parts
        }
        TermMap::Constant(_) => {
            return Err(Error::Unsupported(
                "property path endpoint is a constant term map → 501".to_owned(),
            ))
        }
    };
    Ok(NodeShape(s))
}

/// A term map's term-type + literal modifiers, canonicalised for shape equality.
fn spec_tag(spec: &TermSpec) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}",
        spec.term_type, spec.datatype, spec.language, spec.base
    )
}

/// The single source column a term map reads, or 501 if it is not single-column
/// (a constant, or a multi-column template — deferred for paths).
fn single_col(tm: &TermMap) -> Result<Box<str>> {
    let cols = crate::iq::term_map_columns(tm);
    match cols.len() {
        1 => Ok(cols.into_iter().next().unwrap()),
        _ => Err(Error::Unsupported(
            "property path endpoint term map is not single-column → 501".to_owned(),
        )),
    }
}

/// Clone a single-column term map, redirecting its one column reference to
/// `new_col` (the CTE's canonical key column). Constants / multi-column maps 501.
fn rewrite_single_col(tm: &TermMap, new_col: &str) -> Result<TermMap> {
    match tm {
        TermMap::Column(_, spec) => Ok(TermMap::Column(new_col.into(), spec.clone())),
        TermMap::Template(t, spec) => {
            let mut seen = 0;
            let segments = t
                .segments()
                .iter()
                .map(|s| match s {
                    Segment::Literal(l) => Segment::Literal(l.clone()),
                    Segment::Column(_) => {
                        seen += 1;
                        Segment::Column(new_col.into())
                    }
                })
                .collect::<Vec<_>>();
            if seen != 1 {
                return Err(Error::Unsupported(
                    "property path endpoint template is not single-column → 501".to_owned(),
                ));
            }
            Ok(TermMap::Template(
                Template::from_segments(segments).map_err(|e| Error::Mapping(e.to_string()))?,
                spec.clone(),
            ))
        }
        TermMap::Constant(_) => Err(Error::Unsupported(
            "property path endpoint is a constant term map → 501".to_owned(),
        )),
    }
}
