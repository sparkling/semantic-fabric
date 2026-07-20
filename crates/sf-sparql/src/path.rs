//! Property-path compilation (ADR-0007 *recursive paths compile to source-dialect
//! recursive CTEs*; ADR-0008 transitive properties served live; ADR-0010 recursion
//! bounds). Translates a `?s PATH ?o` pattern into a [`PathClosure`] branch whose
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

use sf_core::ir::{LogicalSource, ObjectMap, Segment, Template, TermMap, TermSpec};
use sf_core::Term;
use spargebra::algebra::PropertyPathExpression;
use spargebra::term::{NamedNode, TermPattern};

use crate::iq::{Branch, HopExpr, HopRelation, PathClosure, PathKind, TermDef};
use crate::unfold::{bind, Unfolder};
use crate::{Error, Result};

/// ADR-0010 recursion-depth backstop for property-path closures. SPARQL path
/// reachability is set-based (the CTE body uses `UNION`), so the closure already
/// terminates on a finite hop relation — this bound is the safety net against a
/// pathological mapping, capping the longest chased chain. A simple path longer
/// than this is truncated (a documented limit, not a correctness claim).
const PATH_MAX_DEPTH: usize = 256;

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
        use PropertyPathExpression as P;
        let (kind, inner): (PathKind, &PropertyPathExpression) = match path {
            P::OneOrMore(p) => (PathKind::OneOrMore, p),
            P::ZeroOrMore(p) => (PathKind::ZeroOrMore, p),
            P::ZeroOrOne(p) => (PathKind::ZeroOrOne, p),
            // A single predicate or a bare sequence/alternative/inverse/negated
            // composite (no closure operator) is one step.
            other => (PathKind::One, other),
        };

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
                && compiled
                    .single_pred
                    .as_deref()
                    .is_some_and(|p| self.graph_is_single_predicate(p));
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
        let subj_def = TermDef::Derived {
            term_map: rewrite_single_col(&compiled.subj_map, "sf_s")?,
            alias,
        };
        let obj_def = TermDef::Derived {
            term_map: rewrite_single_col(&compiled.obj_map, "sf_o")?,
            alias,
        };

        let mut branch = Branch::empty();
        branch.path = Some(PathClosure {
            alias,
            kind,
            hop: compiled.expr,
            max_depth: PATH_MAX_DEPTH,
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

    /// Compile a (closure-free) path sub-expression into a [`CompiledHop`].
    fn compile_path(&self, path: &PropertyPathExpression) -> Result<CompiledHop> {
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

    /// Compile `!(...)` — the union of every mapped predicate EXCEPT the negated
    /// set. The R2RML predicate set is finite, so the complement is enumerable; all
    /// included predicates must share one endpoint shape pair (else the union is not
    /// reconstructible with a single term map → 501).
    fn compile_nps(&self, negated: &[NamedNode]) -> Result<CompiledHop> {
        let mut complement: Vec<String> = Vec::new();
        for tm in self.maps {
            // `rr:class` adds rdf:type triples whose object IRIs we cannot fold into
            // the raw-key complement — defer rather than under-produce.
            if !tm.subject.classes.is_empty() {
                return Err(Error::Unsupported(
                    "!p over a graph with rr:class (rdf:type) triples cannot be \
                     enumerated soundly → 501"
                        .to_owned(),
                ));
            }
            for pom in &tm.predicate_object_maps {
                // Same graph filter as `resolve_pred_hop` (R2RML §4.6 POM-overrides-
                // subject-map precedence): a POM outside the active GRAPH context
                // must not contribute to the `!p` complement enumeration either.
                let eff_graphs = if pom.graphs.is_empty() {
                    &tm.subject.graphs
                } else {
                    &pom.graphs
                };
                if !crate::unfold::graph_maps_match(self.current_graph.as_ref(), eff_graphs) {
                    continue;
                }
                for pm in &pom.predicates {
                    let q = match pm {
                        TermMap::Constant(Term::NamedNode(q)) => q.as_str().to_owned(),
                        _ => {
                            return Err(Error::Unsupported(
                                "!p over a non-constant (column/template) predicate map \
                                 cannot be enumerated → 501"
                                    .to_owned(),
                            ))
                        }
                    };
                    if negated.iter().any(|n| n.as_str() == q) || complement.contains(&q) {
                        continue;
                    }
                    complement.push(q);
                }
            }
        }

        let mut leaves: Vec<CompiledHop> = Vec::new();
        let mut shape: Option<(NodeShape, NodeShape)> = None;
        for q in &complement {
            let c = self.resolve_pred_hop(q)?;
            match &shape {
                None => shape = Some((c.subj_shape.clone(), c.obj_shape.clone())),
                Some((s, o)) => {
                    if *s != c.subj_shape || *o != c.obj_shape {
                        return Err(Error::Unsupported(
                            "!p complement predicates have differing endpoint node shapes \
                             — the union cannot be reconstructed with one term map → 501"
                                .to_owned(),
                        ));
                    }
                }
            }
            leaves.push(c);
        }

        let (subj_shape, obj_shape) = shape.ok_or_else(|| {
            Error::Unsupported(
                "!p complement is empty (no non-negated predicate is mapped) → 501".to_owned(),
            )
        })?;
        let subj_map = leaves[0].subj_map.clone();
        let obj_map = leaves[0].obj_map.clone();
        let exprs = leaves.into_iter().map(|c| c.expr).collect();
        Ok(CompiledHop {
            expr: HopExpr::Nps(exprs),
            subj_map,
            obj_map,
            subj_shape,
            obj_shape,
            single_pred: None,
        })
    }

    /// Resolve a single predicate IRI to its one-hop leaf: exactly one producing
    /// triples-map, a direct constant predicate, single-column subject and `Term`
    /// object term maps; else 501.
    ///
    /// Runs [`Self::find_pred_hop`] twice. The first, graph-scoped pass mirrors
    /// `Unfolder::pattern_branches`'s identical check for ordinary triples
    /// (`unfold.rs`): a predicate-object map whose triples live in a graph other
    /// than the active `GRAPH <g>` (or the default graph) contributes no hop.
    /// Unlike ordinary triples, though, "no graph-matching candidate" is not by
    /// itself grounds for a 501 — R2RML §7.4 graph scoping is a MAPPING-level
    /// fact (a wrong-graph POM's rows are never visible under a mismatched
    /// GRAPH, regardless of their content), so the sound answer is an EMPTY
    /// relation, not a refusal. The second, unscoped pass answers exactly the
    /// question that decides between the two: is `pred_iri` mapped ANYWHERE? If
    /// so, its real term maps/shapes are kept (sound to reuse — they describe
    /// how this predicate's terms are built when it IS visible, which composite
    /// shape-matching needs regardless of whether any row ever flows through)
    /// and only the relation itself is swapped for a statically-empty derived
    /// table ([`empty_hop`]). Only when even the unscoped pass finds nothing —
    /// or finds only an uncompilable shape (an ambiguous or refObjectMap-joined
    /// candidate is still, and always, a 501) — does `pred_iri` genuinely fail
    /// to compile.
    fn resolve_pred_hop(&self, pred_iri: &str) -> Result<CompiledHop> {
        if let Some(hop) = self.find_pred_hop(pred_iri, true)? {
            return Ok(hop);
        }
        if let Some(hop) = self.find_pred_hop(pred_iri, false)? {
            return Ok(empty_hop(hop));
        }
        Err(Error::Unsupported(format!(
            "property path predicate {pred_iri} is not mapped → 501"
        )))
    }

    /// The search loop behind [`Self::resolve_pred_hop`]. `graph_scoped = true`
    /// restricts to predicate-object maps whose effective graph matches
    /// `current_graph` (R2RML §4.6 POM-overrides-subject-map precedence) — the
    /// real-relation case. `false` searches the WHOLE mapping regardless of
    /// graph; `resolve_pred_hop` calls it that way ONLY as the empty-hop shape
    /// source once the scoped pass finds nothing, never to admit real rows from
    /// the wrong graph.
    fn find_pred_hop(&self, pred_iri: &str, graph_scoped: bool) -> Result<Option<CompiledHop>> {
        let mut found: Option<CompiledHop> = None;
        for tm in self.maps {
            for pom in &tm.predicate_object_maps {
                if graph_scoped {
                    let eff_graphs = if pom.graphs.is_empty() {
                        &tm.subject.graphs
                    } else {
                        &pom.graphs
                    };
                    if !crate::unfold::graph_maps_match(self.current_graph.as_ref(), eff_graphs) {
                        continue;
                    }
                }
                let produces = pom.predicates.iter().any(|pm| {
                    matches!(pm, TermMap::Constant(Term::NamedNode(q)) if q.as_str() == pred_iri)
                });
                if !produces {
                    continue;
                }
                for om in &pom.objects {
                    let obj_map = match om {
                        ObjectMap::Term(t) => t.clone(),
                        ObjectMap::Ref(_) => {
                            return Err(Error::Unsupported(
                                "property path over a refObjectMap-joined predicate → 501"
                                    .to_owned(),
                            ))
                        }
                    };
                    if found.is_some() {
                        return Err(Error::Unsupported(
                            "property path over a predicate produced by >1 mapping → 501"
                                .to_owned(),
                        ));
                    }
                    let subj_map = tm.subject.term.clone();
                    found = Some(CompiledHop {
                        expr: HopExpr::Pred(HopRelation {
                            source: tm.source.clone(),
                            subj_col: single_col(&subj_map)?,
                            obj_col: single_col(&obj_map)?,
                        }),
                        subj_shape: node_shape(&subj_map)?,
                        obj_shape: node_shape(&obj_map)?,
                        subj_map,
                        obj_map,
                        single_pred: Some(pred_iri.to_owned()),
                    });
                }
            }
        }
        Ok(found)
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
