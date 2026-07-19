//! Unfold — the SPARQL algebra → IQ base translation (ADR-0007 step 3, the
//! ISWC-2018 ground truth). Each triple pattern becomes the relational
//! sub-expressions of the matching mapping-IR entries; shared variables unify to
//! raw-column equalities ([`crate::unify`]); OPTIONAL becomes a NULL-safe LEFT
//! JOIN obeying R1–R5. This is the **unoptimized** tree the [`crate::cascade`]
//! then rewrites.

use sf_core::datatype::XsdTypeCode;
use sf_core::ir::{ObjectMap, TermMap, TriplesMap};
use sf_core::{NamedNode, Term};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, GraphPattern, OrderExpression,
    PropertyPathExpression,
};
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern, Variable};

use crate::iq::lower::convert_path_branches;
use crate::iq::{
    AggCol, AggKind, Aggregation, Branch, ColRef, GroupKey, OrderKey, RustAgg, RustGroup, Scan,
    SqlCond, TermDef,
};
use crate::leftjoin::{def_is_nullable, left_join_branches, null_safe};
use crate::saturate::Tbox;
use crate::unify::{filter_cond, templates_provably_disjoint, unify, Unify};
use crate::{Error, Result};

pub(crate) const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The translation of one graph pattern: a bag union of [`Branch`]es plus the
/// solution modifiers peeled from the algebra.
pub struct TransPattern {
    pub branches: Vec<Branch>,
    pub project: Option<Vec<String>>,
    pub distinct: bool,
    pub limit: Option<usize>,
    pub offset: usize,
    pub order: Vec<OrderKey>,
    /// Rust-level GROUP BY descriptor for a multi-branch inner (set by [`Unfolder::group`]
    /// when the inner pattern produced more than one branch).
    pub rust_group: Option<RustGroup>,
}

impl TransPattern {
    fn plain(branches: Vec<Branch>) -> Self {
        Self {
            branches,
            project: None,
            distinct: false,
            limit: None,
            offset: 0,
            order: Vec::new(),
            rust_group: None,
        }
    }
}

/// Walks the mappings + T-Box, allocating fresh scan aliases.
pub struct Unfolder<'a> {
    pub(crate) maps: &'a [TriplesMap],
    tbox: &'a Tbox,
    dialect: sf_sql::Dialect,
    next_alias: usize,
    /// The named graph currently active inside a `GRAPH <g> { ... }` clause, or
    /// `None` when translating the default graph (no GRAPH wrapper). Set/restored
    /// by the `GraphPattern::Graph` arm; all other arms inherit the current value.
    pub(crate) current_graph: Option<NamedNode>,
}

impl<'a> Unfolder<'a> {
    pub fn new(maps: &'a [TriplesMap], tbox: &'a Tbox, dialect: sf_sql::Dialect) -> Self {
        Self {
            maps,
            tbox,
            dialect,
            next_alias: 0,
            current_graph: None,
        }
    }

    pub(crate) fn alias(&mut self) -> usize {
        let a = self.next_alias;
        self.next_alias += 1;
        a
    }

    fn map_by_id(&self, id: &str) -> Option<&'a TriplesMap> {
        self.maps.iter().find(|m| m.id == id)
    }

    /// Translate a graph pattern, peeling Project/Distinct/Reduced/Slice and
    /// dispatching the operators (ADR-0007 v1 coverage; the rest return 501).
    pub fn translate_pattern(&mut self, gp: &GraphPattern) -> Result<TransPattern> {
        match gp {
            GraphPattern::Project { inner, variables } => {
                let mut t = self.translate_pattern(inner)?;
                t.project = Some(variables.iter().map(|v| v.as_str().to_owned()).collect());
                Ok(t)
            }
            GraphPattern::Distinct { inner } => {
                let mut t = self.translate_pattern(inner)?;
                t.distinct = true;
                Ok(t)
            }
            // REDUCED permits but does not require dedup → safe no-op (ADR-0007).
            GraphPattern::Reduced { inner } => self.translate_pattern(inner),
            GraphPattern::Slice {
                inner,
                start,
                length,
            } => {
                let mut t = self.translate_pattern(inner)?;
                t.offset = *start;
                t.limit = *length;
                Ok(t)
            }
            GraphPattern::Bgp { patterns } => Ok(TransPattern::plain(self.bgp(patterns)?)),
            GraphPattern::Join { left, right } => {
                let mut l = self.translate_pattern(left)?;
                let mut r = self.translate_pattern(right)?;
                reject_dropped_slice(&l)?;
                reject_dropped_slice(&r)?;
                // ADR-0033 mirror: convert any path-carrying branch to an ordinary
                // derived-table Scan BEFORE `join_branches` (`merge`'s path-join guard)
                // ever sees it — the identical conversion the tree `IqNode::InnerJoin`
                // arm already applies (`iq/lower.rs`), reused verbatim so both engines
                // share one conversion, not two. A path-free branch passes through
                // untouched (the common case).
                convert_path_branches(&mut l.branches, self.dialect, &mut self.next_alias)?;
                convert_path_branches(&mut r.branches, self.dialect, &mut self.next_alias)?;
                Ok(TransPattern::plain(join_branches(l.branches, r.branches)?))
            }
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                let l = self.translate_pattern(left)?;
                let r = self.translate_pattern(right)?;
                reject_dropped_slice(&l)?;
                reject_dropped_slice(&r)?;
                Ok(TransPattern::plain(left_join_branches(
                    l.branches,
                    r.branches,
                    expression.as_ref(),
                    self.dialect,
                )?))
            }
            GraphPattern::Filter { expr, inner } => {
                let mut t = self.translate_pattern(inner)?;
                for b in &mut t.branches {
                    // Pass the full branch so lower_filter_expr can detect
                    // OPTIONAL-derived outer variables in EXISTS/NOT EXISTS
                    // correlation (opt_aliases fix — see lower_exists).
                    let cond = self.lower_filter_expr(expr, b)?;
                    b.where_conds.push(cond);
                }
                Ok(t)
            }
            GraphPattern::Union { left, right } => {
                let mut l = self.translate_pattern(left)?;
                let r = self.translate_pattern(right)?;
                reject_dropped_slice(&l)?;
                reject_dropped_slice(&r)?;
                l.branches.extend(r.branches);
                Ok(TransPattern::plain(l.branches))
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            } => Ok(TransPattern::plain(vec![
                self.path_branch(subject, path, object)?
            ])),
            // BIND(expr AS ?v) — translate the inner pattern, then add ?v computed
            // from `expr` to every branch's output bindings. BIND adds an output
            // column only; it never changes row multiplicity (=_bag preserved). An
            // expression outside the supported subset defers the whole query → 501
            // ([`crate::unify::bind_term_def`]; ADR-0007 term-construction lifting).
            GraphPattern::Extend {
                inner,
                variable,
                expression,
            } => {
                let mut t = self.translate_pattern(inner)?;
                for b in &mut t.branches {
                    let def = crate::unify::bind_term_def(expression, &b.bindings)
                        .map_err(Error::Unsupported)?;
                    bind(b, variable.as_str(), def)?;
                }
                Ok(t)
            }
            // VALUES — an inline constant solution sequence: a bag union of
            // core-less branches, one per binding row. A bound cell becomes a
            // `Const` binding; an UNDEF cell leaves that variable unbound (absent).
            // It composes with the surrounding pattern through the existing
            // shared-variable `join_branches` unification (a Join wrapping VALUES).
            // Each row contributes exactly one solution (=_bag preserved).
            GraphPattern::Values {
                variables,
                bindings,
            } => {
                let mut branches = Vec::with_capacity(bindings.len());
                for row in bindings {
                    let mut b = Branch::empty();
                    for (var, cell) in variables.iter().zip(row.iter()) {
                        if let Some(gt) = cell {
                            let def = TermDef::Const(ground_term_to_term(gt)?);
                            bind(&mut b, var.as_str(), def)?;
                        }
                        // None (UNDEF) ⇒ leave the variable unbound (absent).
                    }
                    branches.push(b);
                }
                Ok(TransPattern::plain(branches))
            }
            // ORDER BY (SPARQL §15.1) — order over the value space. v1 supported
            // subset: each key is a *bound variable*, Asc or Desc, possibly several.
            // The keys are peeled onto `TransPattern` here; the actual sort is pinned
            // later (single-branch → SQL `ORDER BY … NULLS FIRST/LAST` in
            // [`crate::emit`]; multi-branch bag-union → the global stable sort in
            // [`crate::exec`], which per-branch SQL cannot give). Variable keys are
            // lowered to `OrderKey { expr: None }`; expression keys (`STRLEN(?n)` etc.)
            // store the SPARQL expression and a synthetic var name so the exec layer
            // evaluates and injects the sort value per solution before sorting.
            GraphPattern::OrderBy { inner, expression } => {
                let mut t = self.translate_pattern(inner)?;
                let mut keys = Vec::with_capacity(expression.len());
                for oe in expression {
                    let (expr, descending) = match oe {
                        OrderExpression::Asc(e) => (e, false),
                        OrderExpression::Desc(e) => (e, true),
                    };
                    match expr {
                        Expression::Variable(v) => {
                            keys.push(OrderKey {
                                var: v.as_str().to_owned(),
                                descending,
                                expr: None,
                            });
                        }
                        other => {
                            // Non-variable: store the expression; exec evaluates it.
                            let syn = format!("__sf_ord_{}", keys.len());
                            keys.push(OrderKey {
                                var: syn,
                                descending,
                                expr: Some(Box::new(other.clone())),
                            });
                        }
                    }
                }
                t.order = keys;
                Ok(t)
            }
            // MINUS (SPARQL §8.3) — translate `left` and `right`, then exclude each
            // left solution that is compatible with a right solution it shares a
            // bound variable with (a correlated anti-join). When a left/right pair
            // shares no bound variable the pair never removes the left row, so a
            // variable-disjoint MINUS is a NO-OP returning `left` unchanged (NOT
            // empty) — the canonical §8.3 gotcha. See [`minus_branches`].
            GraphPattern::Minus { left, right } => {
                let l = self.translate_pattern(left)?;
                let r = self.translate_pattern(right)?;
                reject_dropped_slice(&l)?;
                reject_dropped_slice(&r)?;
                Ok(TransPattern::plain(minus_branches(l.branches, r.branches)?))
            }
            // GROUP BY + aggregates (SPARQL §11). v1 groups a SINGLE-branch inner:
            // emit `GROUP BY <raw key cols>` + COUNT/SUM/AVG/MIN/MAX, the keys
            // lowered to their raw key columns (term-construction lifting — the term
            // is rebuilt at projection). Implicit grouping (empty `variables`) is one
            // group over all inner rows, yielding one row even when the inner is
            // empty (COUNT(*)=0). A multi-branch inner (UNION/VALUES), GROUP_CONCAT /
            // SAMPLE, or a key/aggregate over a constructed/non-column expression is
            // deferred → 501 (never silently wrong). See [`Self::group`].
            GraphPattern::Group {
                inner,
                variables,
                aggregates,
            } => self.group(inner, variables, aggregates),
            // GRAPH <g> { P } — translate P restricted to triples in named graph g.
            // v1: constant graph IRI only (`NamedNodePattern::NamedNode`); a variable
            // graph name requires runtime IRI lookup → 501 (never silently wrong).
            // The `current_graph` context is saved, set to g, translated, restored so
            // nested GRAPH clauses work correctly.
            GraphPattern::Graph { name, inner } => match name {
                NamedNodePattern::NamedNode(g) => {
                    let saved = self.current_graph.take();
                    self.current_graph = Some(g.clone());
                    let result = self.translate_pattern(inner);
                    self.current_graph = saved;
                    result
                }
                NamedNodePattern::Variable(_) => Err(Error::Unsupported(
                    "GRAPH ?var (variable graph name) is deferred → 501 (v1)".to_owned(),
                )),
            },
            // Deferred → 501 (documented, never silent): LATERAL, SERVICE
            // (ADR-0007 §v1 SPARQL coverage; ADR-0008 tier-2).
            other => Err(Error::Unsupported(format!(
                "graph pattern not supported in v1 → 501: {other:?}"
            ))),
        }
    }

    /// GROUP BY + aggregates (SPARQL §11) over a single-branch inner. Builds one
    /// [`Branch`] carrying the inner FROM/WHERE plus an [`Aggregation`]: the
    /// grouping keys lowered to raw key columns, and each aggregate output column.
    /// The output bindings are the grouping variables (their original term defs,
    /// rebuilt from the grouped raw columns) plus the aggregate result variables.
    fn group(
        &mut self,
        inner: &GraphPattern,
        variables: &[Variable],
        aggregates: &[(Variable, AggregateExpression)],
    ) -> Result<TransPattern> {
        let t = self.translate_pattern(inner)?;
        // v1: a single-branch inner is grouped in SQL (one GROUP BY per SELECT).
        // A multi-branch (UNION/VALUES) inner cannot be grouped per SQL arm because
        // groups would span arms; instead, buffer all solutions in Rust and aggregate
        // there via `Plan::rust_group` (Rust-level GROUP BY path).
        if t.branches.len() != 1 {
            return rust_group_plan(t, variables, aggregates);
        }
        let mut branch = t.branches.into_iter().next().unwrap();
        if branch.path.is_some() {
            return Err(Error::Unsupported(
                "GROUP BY over a property-path closure is deferred → 501".to_owned(),
            ));
        }
        if branch.agg.is_some() {
            return Err(Error::Unsupported(
                "nested GROUP BY (aggregate over an aggregate) is deferred → 501".to_owned(),
            ));
        }

        // The grouping keys, lowered to their raw key columns.
        let mut keys = Vec::with_capacity(variables.len());
        let mut out_bindings = std::collections::BTreeMap::new();
        for v in variables {
            let def = branch.bindings.get(v.as_str()).ok_or_else(|| {
                Error::Unsupported(format!(
                    "GROUP BY ?{} is not a bound variable in the group's inner → 501",
                    v.as_str()
                ))
            })?;
            let cols = group_key_columns(def, v.as_str())?;
            // The grouping variable stays in scope after grouping, rebuilt from its
            // (now grouped) raw columns by its original term def.
            out_bindings.insert(v.as_str().to_owned(), def.clone());
            keys.push(GroupKey {
                var: v.as_str().to_owned(),
                cols,
            });
        }

        // The aggregate result columns share one reserved synthetic alias (they are
        // computed in SQL, never read from a base scan).
        let agg_alias = self.alias();
        let mut aggs = Vec::with_capacity(aggregates.len());
        for (out_var, expr) in aggregates {
            let (kind, arg, distinct, fixed_type) = lower_aggregate(expr, &branch.bindings)?;
            let out = ColRef::new(agg_alias, out_var.as_str());
            out_bindings.insert(
                out_var.as_str().to_owned(),
                TermDef::Agg {
                    col: out.clone(),
                    kind,
                    operand: arg.clone(),
                    fixed_type,
                },
            );
            aggs.push(AggCol {
                var: out_var.as_str().to_owned(),
                kind,
                arg,
                distinct,
                out,
                fixed_type,
            });
        }

        // After grouping, ONLY the grouping vars + aggregate results are in scope
        // (the inner pattern's other variables are not projected by the group).
        branch.bindings = out_bindings;
        branch.agg = Some(Aggregation { keys, aggs });
        Ok(TransPattern::plain(vec![branch]))
    }

    /// Translate a BGP: each pattern → its alternative branches, then the
    /// patterns are joined (product + shared-variable unification).
    fn bgp(&mut self, patterns: &[TriplePattern]) -> Result<Vec<Branch>> {
        let mut acc: Vec<Branch> = vec![Branch::empty()];
        for tp in patterns {
            let alts = self.pattern_branches(tp)?;
            acc = join_branches(acc, alts)?;
            if acc.is_empty() {
                break; // an empty product stays empty (all pruned)
            }
        }
        Ok(acc)
    }

    /// Resolve one triple pattern **at a given active graph** to its flat atom
    /// alternatives (ADR-0023 M3a). This is the entry point the tree-path
    /// [`crate::iq::resolve`] drives per [`crate::iq::node::IqNode::Intensional`]:
    /// it pins the `current_graph` to the leaf's resolved constant graph (so
    /// [`graph_maps_match`] filters exactly as the flat `GRAPH <g>` path does),
    /// delegates to the **unchanged** [`Self::pattern_branches`] oracle, then
    /// restores the previous graph context. Behaviour (arm set, conds, fresh
    /// aliases, `predicate_can_match` pruning) is byte-identical to the flat
    /// translation — that is the `=_bag` argument (M3 design §3, §6).
    pub(crate) fn resolve_pattern(
        &mut self,
        tp: &TriplePattern,
        graph: Option<&NamedNode>,
    ) -> Result<Vec<Branch>> {
        let saved = self.current_graph.take();
        self.current_graph = graph.cloned();
        let out = self.pattern_branches(tp);
        self.current_graph = saved;
        out
    }

    /// Resolve one property-path pattern `?s PATH ?o` **at a given active graph** to its
    /// flat [`PathClosure`](crate::iq::PathClosure) branch (ADR-0023 M5 Wave 1). This is
    /// the entry point the tree-path [`crate::iq::resolve`] drives per
    /// [`crate::iq::node::IqNode::UnresolvedPath`]: it pins the `current_graph` exactly as
    /// the flat `GRAPH <g> { ?s PATH ?o }` path does (saved/restored around the call) and
    /// delegates to the **unchanged** [`Self::path_branch`] oracle — so the compiled hop
    /// relation, recursion bound, node-shape soundness checks, and reflexive-enumeration
    /// 501s are byte-identical to the flat translation (the `=_bag` argument). The
    /// resulting [`Branch`] carries `path = Some(closure)` + the subject/object bindings
    /// (+ a `?s PATH ?s` self-unify `ColEq` in `where_conds`), bridged to an
    /// [`IqNode::Path`] by [`crate::iq::resolve::bridge_branch`].
    pub(crate) fn resolve_path(
        &mut self,
        subject: &TermPattern,
        path: &PropertyPathExpression,
        object: &TermPattern,
        graph: Option<&NamedNode>,
    ) -> Result<Branch> {
        let saved = self.current_graph.take();
        self.current_graph = graph.cloned();
        let out = self.path_branch(subject, path, object);
        self.current_graph = saved;
        out
    }

    /// All atom alternatives for one triple pattern (a bag union over the
    /// matching triples-maps / predicate-object maps / `rr:class` entries).
    pub(crate) fn pattern_branches(&mut self, tp: &TriplePattern) -> Result<Vec<Branch>> {
        let mut out = Vec::new();
        // Predicate match set (direct + sub-properties + inverse/symmetric).
        let pred_iri = match &tp.predicate {
            NamedNodePattern::NamedNode(p) => Some(p.as_str().to_owned()),
            NamedNodePattern::Variable(_) => None,
        };
        let want_type = pred_iri.as_deref() == Some(RDF_TYPE);

        for tm in self.maps {
            // rr:class → rdf:type atoms (when predicate is rdf:type or a variable).
            // rr:class triples inherit the subject map's graph.
            if want_type || pred_iri.is_none() {
                // Skip if the subject map's graph doesn't match the active GRAPH clause.
                if graph_maps_match(self.current_graph.as_ref(), &tm.subject.graphs) {
                    self.class_atoms(tp, tm, &mut out)?;
                }
            }
            for pom in &tm.predicate_object_maps {
                // Effective graph: POM overrides subject map (R2RML §4.6).
                let eff_graphs = if pom.graphs.is_empty() {
                    &tm.subject.graphs
                } else {
                    &pom.graphs
                };
                if !graph_maps_match(self.current_graph.as_ref(), eff_graphs) {
                    continue;
                }
                for pm in &pom.predicates {
                    for om in &pom.objects {
                        if let Some(b) = self.atom(tp, tm, pm, om, pred_iri.as_deref())? {
                            out.push(b);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Build one predicate-object atom branch, or `None` if it cannot match.
    fn atom(
        &mut self,
        tp: &TriplePattern,
        tm: &TriplesMap,
        pm: &TermMap,
        om: &ObjectMap,
        pred_iri: Option<&str>,
    ) -> Result<Option<Branch>> {
        // Fast-reject path (ADR-0013 Path-B): for a concrete query predicate against
        // a constant-predicate POM, check matchability *before* allocating an alias or
        // Branch. Most POMs do not match most query predicates; skipping the Branch
        // allocation here eliminates (N-1)/N of the per-triple-pattern allocations for
        // a mapping with N POMs. Only the constant-predicate case is hoisted; column/
        // template predicates and variable-predicate queries fall through as before.
        if let (Some(p), TermMap::Constant(sf_core::Term::NamedNode(q))) = (pred_iri, pm) {
            if !self.tbox.predicate_can_match(q.as_str(), p) {
                return Ok(None);
            }
        }

        let alias = self.alias();
        let mut branch = Branch::single(Scan {
            alias,
            source: tm.source.clone(),
        });

        // Predicate position.
        let (pred_def, swap) = self.predicate_match(tm, pm, alias, pred_iri)?;
        let pred_def = match pred_def {
            PredMatch::No => return Ok(None),
            PredMatch::Yes(d) => d,
        };

        // Subject + object definitions from the mapping (swap for inverse preds).
        let subj_def = def_of(&tm.subject.term, alias);
        let obj_def = match om {
            ObjectMap::Term(otm) => def_of(otm, alias),
            ObjectMap::Ref(r) => {
                let parent = self
                    .map_by_id(&r.parent_triples_map)
                    .ok_or_else(|| {
                        Error::Mapping(format!("unknown parent map {}", r.parent_triples_map))
                    })?
                    .clone();
                let palias = self.alias();
                branch.core.push(Scan {
                    alias: palias,
                    source: parent.source.clone(),
                });
                for j in &r.joins {
                    branch.where_conds.push(SqlCond::ColEq(
                        crate::iq::ColRef::new(alias, j.child.clone()),
                        crate::iq::ColRef::new(palias, j.parent.clone()),
                    ));
                }
                def_of(&parent.subject.term, palias)
            }
        };
        // R2RML §11: a column/template object term map produces NO RDF term (hence no
        // triple) when any referenced column is NULL. Capture those columns now, before
        // `obj_def` is moved, so we can guard them below. The join already excludes NULL
        // child columns for a `Ref` (parentTriplesMap) object, so only a plain column/
        // template object map needs the explicit guard.
        let obj_null_guard: Vec<crate::iq::ColRef> = match om {
            ObjectMap::Term(_) => obj_def.columns(),
            ObjectMap::Ref(_) => Vec::new(),
        };

        let (q_subj, q_obj) = if swap {
            (obj_def, subj_def)
        } else {
            (subj_def, obj_def)
        };

        // Bind/constrain the three query positions.
        if let NamedNodePattern::Variable(pv) = &tp.predicate {
            bind(&mut branch, pv.as_str(), pred_def)?;
        }
        self.bind_position(&mut branch, &tp.subject, q_subj)?;
        if !self.bind_position(&mut branch, &tp.object, q_obj)? {
            return Ok(None);
        }
        // Enforce the R2RML §11 NULL rule inside SQL (not only at reconstruct time): a
        // NULL data column drops the row. Without this, a NULL object would still emit a
        // solution (object UNBOUND), so an anti-join (SPARQL MINUS / NOT EXISTS), whose
        // correlation is the clone of these `where_conds`, would correlate on the subject
        // alone and wrongly remove every left row.
        for col in obj_null_guard {
            if !branch
                .where_conds
                .iter()
                .any(|c| matches!(c, SqlCond::IsNotNull(r) if r == &col))
            {
                branch.where_conds.push(SqlCond::IsNotNull(col));
            }
        }
        Ok(Some(branch))
    }

    /// `rr:class` → `rdf:type` atoms (subject a `:C`), with class-query
    /// saturation: a query for `:C` matches mapped classes in `saturate_class`.
    fn class_atoms(
        &mut self,
        tp: &TriplePattern,
        tm: &TriplesMap,
        out: &mut Vec<Branch>,
    ) -> Result<()> {
        // The object position selects which classes match.
        let wanted: Option<Vec<String>> = match &tp.object {
            TermPattern::NamedNode(c) => Some(self.tbox.saturate_class(c.as_str())),
            TermPattern::Variable(_) => None,
            _ => return Ok(()), // class object can only be an IRI or a variable
        };
        for class in &tm.subject.classes {
            if let Some(w) = &wanted {
                if !w.iter().any(|c| c == class.as_str()) {
                    continue;
                }
            }
            let alias = self.alias();
            let mut branch = Branch::single(Scan {
                alias,
                source: tm.source.clone(),
            });
            let subj_def = def_of(&tm.subject.term, alias);
            // predicate is rdf:type (matched); bind object var to the class IRI.
            if let TermPattern::Variable(ov) = &tp.object {
                bind(
                    &mut branch,
                    ov.as_str(),
                    TermDef::Const(Term::NamedNode(class.clone())),
                )?;
            }
            if let NamedNodePattern::Variable(pv) = &tp.predicate {
                bind(
                    &mut branch,
                    pv.as_str(),
                    TermDef::Const(Term::NamedNode(NamedNode::new_unchecked(RDF_TYPE))),
                )?;
            }
            if self.bind_position(&mut branch, &tp.subject, subj_def)? {
                out.push(branch);
            }
        }
        Ok(())
    }

    /// Decide whether the mapping predicate term map satisfies the query
    /// predicate, returning the predicate's [`TermDef`] (for a variable) and the
    /// inverse-swap flag.
    fn predicate_match(
        &self,
        _tm: &TriplesMap,
        pm: &TermMap,
        alias: usize,
        pred_iri: Option<&str>,
    ) -> Result<(PredMatch, bool)> {
        match pred_iri {
            None => Ok((PredMatch::Yes(def_of(pm, alias)), false)), // variable predicate
            Some(p) => {
                let direct = self.tbox.saturate_predicate(p);
                let inverse = self.tbox.inverse_predicates(p);
                match pm {
                    TermMap::Constant(Term::NamedNode(q)) => {
                        if direct.iter().any(|i| i == q.as_str()) {
                            Ok((
                                PredMatch::Yes(TermDef::Const(Term::NamedNode(q.clone()))),
                                false,
                            ))
                        } else if inverse.iter().any(|i| i == q.as_str()) {
                            Ok((
                                PredMatch::Yes(TermDef::Const(Term::NamedNode(q.clone()))),
                                true,
                            ))
                        } else {
                            Ok((PredMatch::No, false))
                        }
                    }
                    // A column/template predicate map could produce p — constrain it.
                    TermMap::Column(..) | TermMap::Template(..) => {
                        Ok((PredMatch::Yes(def_of(pm, alias)), false))
                    }
                    TermMap::Constant(_) => Ok((PredMatch::No, false)),
                }
            }
        }
    }

    /// Bind a query term position (subject/object) to a mapping def: a variable
    /// records the binding (unifying on re-occurrence within the atom); a constant
    /// adds a unification condition. Returns `false` if the atom is pruned.
    fn bind_position(&self, branch: &mut Branch, pat: &TermPattern, def: TermDef) -> Result<bool> {
        match pat {
            TermPattern::Variable(v) => bind(branch, v.as_str(), def),
            TermPattern::NamedNode(n) => {
                self.constrain(branch, TermDef::Const(Term::NamedNode(n.clone())), def)
            }
            TermPattern::Literal(l) => {
                self.constrain(branch, TermDef::Const(Term::Literal(l.clone())), def)
            }
            // A blank node in a graph pattern is a NON-DISTINGUISHED join variable
            // (SPARQL 1.1 §4.1.4 / §18.2.1), not a constant. spargebra desugars bare
            // sequence/inverse paths (`p/q`, `^(...)`) into a BGP joined by a fresh
            // middle blank node, so binding it as a constant would prune the join and
            // collapse the plan to Empty. Bind it as a synthetic join variable keyed by
            // its stable spargebra id, namespaced (`__bnode_`) so it can never collide
            // with a real SPARQL variable name; the outer Construction projection is
            // driven by SELECT scope, so it is projected away.
            TermPattern::BlankNode(b) => bind(branch, &format!("__bnode_{}", b.as_str()), def),
            other => Err(Error::Unsupported(format!(
                "term pattern not supported in v1 → 501: {other:?}"
            ))),
        }
    }

    fn constrain(&self, branch: &mut Branch, c: TermDef, def: TermDef) -> Result<bool> {
        match unify(&c, &def) {
            Unify::Sat(conds) => {
                branch.where_conds.extend(conds);
                Ok(true)
            }
            Unify::Empty => Ok(false),
            Unify::Unsupported(why) => Err(Error::Unsupported(why)),
        }
    }

    /// Lower a SPARQL FILTER expression to a [`SqlCond`], handling `EXISTS` and
    /// `NOT EXISTS` subqueries by translating the inner [`GraphPattern`] through
    /// the full unfolding pipeline (which requires `&mut self` for alias counters).
    /// Non-EXISTS expressions are delegated to [`filter_cond`] from `unify`.
    ///
    /// `FILTER NOT EXISTS { P }` and `FILTER EXISTS { P }` are the only SPARQL
    /// constructs that embed a pattern inside a FILTER; everything else is a
    /// pure expression over bindings.
    fn lower_filter_expr(&mut self, expr: &Expression, outer: &Branch) -> Result<SqlCond> {
        match expr {
            Expression::Exists(p) => self.lower_exists(p, outer, /*negated=*/ false),
            Expression::Not(inner) => {
                if let Expression::Exists(p) = inner.as_ref() {
                    self.lower_exists(p, outer, /*negated=*/ true)
                } else {
                    Ok(SqlCond::Not(Box::new(
                        self.lower_filter_expr(inner, outer)?,
                    )))
                }
            }
            Expression::And(a, b) => Ok(SqlCond::And(vec![
                self.lower_filter_expr(a, outer)?,
                self.lower_filter_expr(b, outer)?,
            ])),
            Expression::Or(a, b) => Ok(SqlCond::Or(vec![
                self.lower_filter_expr(a, outer)?,
                self.lower_filter_expr(b, outer)?,
            ])),
            other => filter_cond(other, &outer.bindings, self.dialect).map_err(Error::Unsupported),
        }
    }

    /// Translate `EXISTS { P }` or `NOT EXISTS { P }` to a [`SqlCond`].
    ///
    /// P is unfolded through the full mapping pipeline, producing one branch per
    /// matching (TriplesMap, POM) pair (a bag-union). For `NOT EXISTS` every
    /// branch that can match must be absent, so each becomes a `SqlCond::NotExists`
    /// and they are AND'd. For `EXISTS` at least one branch must match, so each
    /// becomes a `SqlCond::Exists` and they are OR'd (`=_bag`: the multiplicity of
    /// matching right rows is irrelevant — only existence is tested).
    ///
    /// Correlation: shared variables between the outer binding and an inner branch
    /// are correlated via raw-key equality (ADR-0007 term-construction lifting).
    /// v1 restriction: if a shared variable may be UNBOUND on the outer side
    /// (reads an OPTIONAL scan alias) we defer → 501 rather than emit a wrong
    /// NULL = value equality.
    fn lower_exists(&mut self, p: &GraphPattern, outer: &Branch, negated: bool) -> Result<SqlCond> {
        let inner = self.translate_pattern(p)?;
        if inner.branches.is_empty() {
            // P produces no branches at all (unmapped): EXISTS → always false, NOT EXISTS → always true.
            return if negated {
                Ok(SqlCond::And(vec![])) // vacuously true
            } else {
                Ok(SqlCond::Or(vec![])) // vacuously false — rendered as 1=0
            };
        }
        // Outer OPTIONAL scan aliases: a shared variable whose TermDef reads one of
        // these aliases may be UNBOUND (the OPTIONAL arm did not fire), so SQL
        // `outer_col = inner_col` would be NULL ≠ value — wrong. Defer → 501 to
        // avoid silent data corruption (mirrors minus_branches, line ~980).
        let outer_opt_aliases: Vec<usize> = outer.opts.iter().map(|o| o.scan.alias).collect();
        let mut sub_conds = Vec::with_capacity(inner.branches.len());
        for r in &inner.branches {
            if r.path.is_some() {
                return Err(Error::Unsupported(
                    "EXISTS with a property-path inner is deferred → 501 (v1)".to_owned(),
                ));
            }
            // Build the inner subquery's conditions: right branch's own conds +
            // correlation equalities for every shared variable.
            let mut corr = r.where_conds.clone();
            let mut never_compatible = false;
            for (v, ldef) in &outer.bindings {
                let Some(rdef) = r.bindings.get(v) else {
                    continue; // not shared
                };
                if def_reads_opt_alias(ldef, &outer_opt_aliases) {
                    return Err(Error::Unsupported(format!(
                        "EXISTS shared variable ?{v} may be UNBOUND on the outer side (OPTIONAL) → 501 \
                         (v1 supports non-OPTIONAL shared variables)"
                    )));
                }
                match unify(ldef, rdef) {
                    Unify::Sat(conds) => corr.extend(conds),
                    Unify::Empty => {
                        never_compatible = true;
                        break;
                    }
                    Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                }
            }
            if never_compatible {
                // This branch can never match the outer row.
                // NOT EXISTS: vacuously satisfied (never removes left row). EXISTS: no OR arm.
                continue;
            }
            if negated {
                sub_conds.push(SqlCond::NotExists {
                    scans: r.core.clone(),
                    conds: corr,
                });
            } else {
                sub_conds.push(SqlCond::Exists {
                    scans: r.core.clone(),
                    conds: corr,
                });
            }
        }
        Ok(if negated {
            SqlCond::And(sub_conds) // AND of NOT EXISTS: all branches must fail to match
        } else {
            SqlCond::Or(sub_conds) // OR of EXISTS: at least one branch must match
        })
    }

    /// `true` iff `pred_iri` is the ONLY predicate the whole mapping produces —
    /// no other `rr:predicate` and no `rr:class` (which would add `rdf:type`
    /// triples and class-IRI object nodes). In that case the hop relation's node
    /// set (subjects ∪ objects of `pred_iri`) equals the active graph's node set,
    /// making `P*`/`p?`'s reflexive ZeroLengthPath provably complete (under the
    /// same-domain raw-key assumption that already underpins `P+`).
    pub(crate) fn graph_is_single_predicate(&self, pred_iri: &str) -> bool {
        for tm in self.maps {
            if !tm.subject.classes.is_empty() {
                return false;
            }
            for pom in &tm.predicate_object_maps {
                let only_this_pred = pom.predicates.iter().all(|pm| {
                    matches!(pm, TermMap::Constant(Term::NamedNode(q)) if q.as_str() == pred_iri)
                });
                if !only_this_pred {
                    return false;
                }
            }
        }
        true
    }
}

/// Whether the effective graph maps of a triple are compatible with the active
/// `GRAPH <g>` clause (or the absence of one).
///
/// Whether a TriplesMap/POM with the given effective `graphs` matches the active
/// GRAPH context:
///
/// * `active = None` (no GRAPH clause, default-graph context):
///   accept only triples that belong to the **default graph** — i.e. whose
///   `graphs` is empty (no `rr:graphName` declared). A non-empty `graphs` with
///   a constant named-node entry means those triples live in a named graph, not
///   the default graph, and must **not** appear in default-graph queries
///   (R2RML §7.4 / SPARQL §13.1). Column/template graph maps are unknowable at
///   translation time and are conservatively included (never wrong on the "missing
///   rows" side; the alternative would silently drop valid triples).
/// * `active = Some(g)` — GRAPH <g> clause:
///   accept only triples where a constant graph map equals `g`. Column/template
///   graph maps are treated as non-matching (conservative — never admits wrong rows).
pub(crate) fn graph_maps_match(
    active: Option<&NamedNode>,
    graphs: &[sf_core::ir::TermMap],
) -> bool {
    // R2RML §6.1: `rr:defaultGraph` is a legal constant graph map that explicitly
    // places triples in the default graph.  It is stored in the IR as a NamedNode
    // with this IRI; treat it the same as an absent graph map (i.e. default-graph).
    const RR_DEFAULT_GRAPH: &str = "http://www.w3.org/ns/r2rml#defaultGraph";

    match active {
        None => {
            // Default-graph query: include triples that have no rr:graph declaration
            // (empty) OR whose rr:graph map includes rr:defaultGraph.  R2RML §7.4
            // allows simultaneous rr:defaultGraph + named-graph declarations on the
            // same predicate-object map; that triple appears in BOTH graphs, so it
            // must be visible in the default-graph view too.
            // Triples declared exclusively in named graphs are excluded (=_bag fix).
            graphs.is_empty()
                || graphs.iter().any(|gm| {
                    matches!(gm, sf_core::ir::TermMap::Constant(sf_core::Term::NamedNode(n))
                        if n.as_str() == RR_DEFAULT_GRAPH)
                })
        }
        Some(g) => {
            // GRAPH <g>: at least one constant graph map must equal g.
            // rr:defaultGraph is never equal to any user-specified named-graph IRI.
            graphs.iter().any(|gm| {
                matches!(gm, sf_core::ir::TermMap::Constant(sf_core::Term::NamedNode(n)) if n == g)
            })
        }
    }
}

enum PredMatch {
    Yes(TermDef),
    No,
}

/// A VALUES inline ground term → an RDF [`Term`]. A quoted triple term
/// (SPARQL 1.2 `sparql-12`) is deferred → 501 (never silent).
pub(crate) fn ground_term_to_term(gt: &GroundTerm) -> Result<Term> {
    match gt {
        GroundTerm::NamedNode(n) => Ok(Term::NamedNode(n.clone())),
        GroundTerm::Literal(l) => Ok(Term::Literal(l.clone())),
        other => Err(Error::Unsupported(format!(
            "VALUES ground term not supported in v1 → 501: {other:?}"
        ))),
    }
}

/// Build a [`TransPattern`] with a [`RustGroup`] descriptor for a multi-branch
/// GROUP BY inner (called when the inner produced more than one branch).
fn rust_group_plan(
    t: TransPattern,
    variables: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
) -> Result<TransPattern> {
    let keys: Vec<String> = variables.iter().map(|v| v.as_str().to_owned()).collect();
    let mut aggs = Vec::with_capacity(aggregates.len());
    for (out_var, expr) in aggregates {
        let (kind, arg_var, distinct, fixed_type) = parse_rust_agg(expr)?;
        aggs.push(RustAgg {
            out_var: out_var.as_str().to_owned(),
            kind,
            arg_var,
            distinct,
            fixed_type,
        });
    }
    Ok(TransPattern {
        rust_group: Some(RustGroup {
            keys,
            aggs,
            post_exprs: Vec::new(),
        }),
        ..t
    })
}

/// Parse one [`AggregateExpression`] into `(kind, arg_var, distinct, fixed_type)`.
/// Does not require branch bindings — the arg is returned as a variable name
/// rather than a [`ColRef`], for use in the Rust-level GROUP BY path.
fn parse_rust_agg(
    expr: &AggregateExpression,
) -> Result<(AggKind, Option<String>, bool, Option<XsdTypeCode>)> {
    match expr {
        AggregateExpression::CountSolutions { distinct } => {
            // Flat's Rust-group path cannot bind an aggregate-over-UNION result var (the
            // documented agg-var limitation), so COUNT(DISTINCT *) stays a sound 501 here —
            // the TREE path handles it (ADR-0025 Tier-2 gap 3; tree exceeds flat).
            if *distinct {
                return Err(Error::Unsupported(
                    "COUNT(DISTINCT *) is deferred → 501 (v1 supports COUNT(*))".to_owned(),
                ));
            }
            Ok((AggKind::Count, None, false, Some(XsdTypeCode::Integer)))
        }
        AggregateExpression::FunctionCall {
            name,
            expr: inner,
            distinct,
        } => {
            let var = match inner {
                Expression::Variable(v) => Some(v.as_str().to_owned()),
                _ => {
                    return Err(Error::Unsupported(
                        "aggregate over a non-variable expression is deferred → 501 \
                         (v1 aggregates a single column-backed variable)"
                            .to_owned(),
                    ))
                }
            };
            let (kind, fixed) = match name {
                AggregateFunction::Count => (AggKind::Count, Some(XsdTypeCode::Integer)),
                AggregateFunction::Sum => (AggKind::Sum, None),
                AggregateFunction::Avg => (AggKind::Avg, None),
                AggregateFunction::Min => (AggKind::Min, None),
                AggregateFunction::Max => (AggKind::Max, None),
                AggregateFunction::GroupConcat { .. } => {
                    return Err(Error::Unsupported(
                        "GROUP_CONCAT is deferred → 501 (string-join semantics)".to_owned(),
                    ))
                }
                AggregateFunction::Sample => {
                    return Err(Error::Unsupported(
                        "SAMPLE is deferred → 501 (non-deterministic pick)".to_owned(),
                    ))
                }
                AggregateFunction::Custom(_) => {
                    return Err(Error::Unsupported(
                        "custom aggregate function is deferred → 501".to_owned(),
                    ))
                }
            };
            Ok((kind, var, *distinct, fixed))
        }
    }
}

/// The raw key columns a GROUP BY key lowers to: a column/template term map's
/// columns (grouping by the raw key ≡ grouping by the constructed term, the
/// term-lifting injectivity assumption, ADR-0007) — **gated on
/// [`crate::cascade::binding_is_injective`]**, the same soundness condition
/// DISTINCT-removal already requires: a `TermMap::Column` is trivially
/// injective, but a non-injective `TermMap::Template` (adjacent column slots
/// with no separator, or 2+ column slots on a non-IRI/non-percent-encoded
/// term type) can map two DISTINCT raw-column tuples to the SAME constructed
/// term — grouping by the raw columns would then split one SPARQL group into
/// two, silently under-counting. A constant / COALESCE / CONCAT key has no
/// single raw key to group by (a constant doesn't partition; a constructed
/// multi-source term can't be reduced to a GROUP BY column soundly) → deferred
/// 501 (never silently wrong).
pub(crate) fn group_key_columns(def: &TermDef, var: &str) -> Result<Vec<ColRef>> {
    match def {
        TermDef::Derived { .. } if !crate::cascade::binding_is_injective(def) => {
            Err(Error::Unsupported(format!(
                "GROUP BY ?{var} is a non-injective template key (two distinct raw-column \
                 tuples could construct the same term) → 501"
            )))
        }
        TermDef::Derived { .. } => {
            let cols = def.columns();
            if cols.is_empty() {
                Err(Error::Unsupported(format!(
                    "GROUP BY ?{var} reduces to no raw column → 501"
                )))
            } else {
                Ok(cols)
            }
        }
        _ => Err(Error::Unsupported(format!(
            "GROUP BY ?{var} is a constructed/constant key (not reducible to a raw \
             GROUP BY column) → 501"
        ))),
    }
}

/// Lower one [`AggregateExpression`] to `(kind, arg col, distinct, fixed result
/// type)`. `COUNT(*)` has no argument column; every other function aggregates a
/// single bound variable reducible to ONE raw column. An aggregate over a complex
/// expression / multi-column term, GROUP_CONCAT, SAMPLE, custom, or `COUNT(DISTINCT
/// *)` is deferred → 501 (never silently wrong).
fn lower_aggregate(
    expr: &AggregateExpression,
    bindings: &std::collections::BTreeMap<String, TermDef>,
) -> Result<(AggKind, Option<ColRef>, bool, Option<XsdTypeCode>)> {
    match expr {
        // COUNT(*) — counts solutions in the group; result xsd:integer.
        AggregateExpression::CountSolutions { distinct } => {
            if *distinct {
                return Err(Error::Unsupported(
                    "COUNT(DISTINCT *) is deferred → 501 (v1 supports COUNT(*))".to_owned(),
                ));
            }
            Ok((AggKind::Count, None, false, Some(XsdTypeCode::Integer)))
        }
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct,
        } => {
            // v1: the aggregated expression must be a single bound variable that
            // lowers to ONE raw column (term-construction lifting — never a
            // constructed/computed expression).
            let Expression::Variable(v) = expr else {
                return Err(Error::Unsupported(
                    "aggregate over a non-variable expression is deferred → 501 \
                     (v1 aggregates a single column-backed variable)"
                        .to_owned(),
                ));
            };
            let def = bindings.get(v.as_str()).ok_or_else(|| {
                Error::Unsupported(format!(
                    "aggregate variable ?{} is not bound in the group's inner → 501",
                    v.as_str()
                ))
            })?;
            let col = single_column_of(def, v.as_str())?;
            let (kind, fixed) = match name {
                // COUNT(?v) — counts solutions where ?v is BOUND (non-NULL col);
                // COUNT(DISTINCT ?v) — distinct bound values. Result xsd:integer.
                AggregateFunction::Count => (AggKind::Count, Some(XsdTypeCode::Integer)),
                // SUM/MIN/MAX keep the source numeric type (decltype/storage at
                // reconstruction). AVG (§11.4: SUM/COUNT under XPath numeric
                // promotion) follows the OPERAND numeric type, resolved from the
                // operand's §10 type at reconstruction — never pinned (SQLite's AVG
                // always yields REAL, so a fixed decimal is wrong for a double
                // operand).
                AggregateFunction::Sum => (AggKind::Sum, None),
                AggregateFunction::Avg => (AggKind::Avg, None),
                AggregateFunction::Min => (AggKind::Min, None),
                AggregateFunction::Max => (AggKind::Max, None),
                AggregateFunction::GroupConcat { .. } => {
                    return Err(Error::Unsupported(
                        "GROUP_CONCAT is deferred → 501 (string-join semantics)".to_owned(),
                    ))
                }
                AggregateFunction::Sample => {
                    return Err(Error::Unsupported(
                        "SAMPLE is deferred → 501 (non-deterministic pick)".to_owned(),
                    ))
                }
                AggregateFunction::Custom(_) => {
                    return Err(Error::Unsupported(
                        "custom aggregate function is deferred → 501".to_owned(),
                    ))
                }
            };
            Ok((kind, Some(col), *distinct, fixed))
        }
    }
}

/// The single raw column a column-backed variable reads (for SUM/AVG/MIN/MAX/COUNT
/// over a column). A constant / multi-column template / COALESCE / CONCAT binding
/// has no single aggregation column → deferred 501.
pub(crate) fn single_column_of(def: &TermDef, var: &str) -> Result<ColRef> {
    if let TermDef::Derived { .. } = def {
        if let [col] = def.columns().as_slice() {
            return Ok(col.clone());
        }
    }
    Err(Error::Unsupported(format!(
        "aggregate over ?{var} requires a single column-backed value (not a \
         constructed/multi-column term) → 501"
    )))
}

/// A mapping term map → a [`TermDef`] at `alias` (constants need no alias).
fn def_of(tm: &TermMap, alias: usize) -> TermDef {
    match tm {
        TermMap::Constant(t) => TermDef::Const(t.clone()),
        other => TermDef::Derived {
            term_map: other.clone(),
            alias,
        },
    }
}

/// Bind `var` in `branch`, unifying with any existing binding. `Ok(false)` ⇒ the
/// branch is pruned (disjoint self-unification).
pub(crate) fn bind(branch: &mut Branch, var: &str, def: TermDef) -> Result<bool> {
    if let Some(existing) = branch.bindings.get(var) {
        match unify(existing, &def) {
            Unify::Sat(conds) => {
                branch.where_conds.extend(conds);
                Ok(true)
            }
            Unify::Empty => Ok(false),
            Unify::Unsupported(why) => Err(Error::Unsupported(why)),
        }
    } else {
        branch.bindings.insert(var.to_owned(), def);
        Ok(true)
    }
}

/// SPARQL MINUS (§8.3) as a correlated anti-join. The result is the LEFT
/// solutions minus every left solution that is COMPATIBLE with some right solution
/// **with which it shares at least one bound variable**.
///
/// * **Disjoint-domain rule.** When a left/right branch pair shares no bound
///   variable, that pair can never remove the left row (the domains don't
///   intersect), so a globally variable-disjoint MINUS is a NO-OP returning `left`
///   unchanged — NOT empty (the §8.3 difference from `NOT EXISTS`). This falls out
///   per-pair: an empty shared set contributes no `NotExists`.
/// * **Compatibility** is raw-key equality on every shared variable (term lifting,
///   ADR-0007) — an unbound variable does not constrain. Each kept pair becomes a
///   `NOT EXISTS` over the right branch correlated on those equalities.
/// * **Bag semantics.** The `NotExists` is a pure WHERE filter, so a surviving left
///   solution keeps its LEFT multiplicity and the right multiplicities neither
///   multiply nor dedup the left rows.
///
/// v1 supports shared variables statically bound (non-OPTIONAL) on both sides. An
/// OPTIONAL / property-path right side, a property-path left side, or a shared
/// variable that may be UNBOUND (a COALESCE'd / CONCAT'd binding, or one reading an
/// OPTIONAL scan alias) is deferred → 501 (never a silently wrong answer).
fn minus_branches(left: Vec<Branch>, right: Vec<Branch>) -> Result<Vec<Branch>> {
    for r in &right {
        if !r.opts.is_empty() || r.path.is_some() {
            return Err(Error::Unsupported(
                "MINUS with an OPTIONAL / property-path right side is deferred → 501".to_owned(),
            ));
        }
    }
    let mut out = Vec::with_capacity(left.len());
    for mut l in left {
        if l.path.is_some() {
            return Err(Error::Unsupported(
                "MINUS over a property-path left side is deferred → 501".to_owned(),
            ));
        }
        let l_opt_aliases: Vec<usize> = l.opts.iter().map(|o| o.scan.alias).collect();
        let mut anti: Vec<SqlCond> = Vec::new();
        for r in &right {
            // The variables bound in BOTH this left branch and this right branch.
            let shared: Vec<&str> = r
                .bindings
                .keys()
                .filter(|v| l.bindings.contains_key(*v))
                .map(String::as_str)
                .collect();
            if shared.is_empty() {
                continue; // disjoint domains for this pair → never removes the left row
            }
            let mut corr = r.where_conds.clone();
            let mut never_compatible = false;
            for v in &shared {
                let ldef = &l.bindings[*v];
                let rdef = &r.bindings[*v];
                // v1: a shared variable that may be UNBOUND on the left (it reads an
                // OPTIONAL scan) would need unbound-does-not-constrain handling → 501.
                if def_reads_opt_alias(ldef, &l_opt_aliases) {
                    return Err(Error::Unsupported(format!(
                        "MINUS shared variable ?{v} may be UNBOUND on the left (OPTIONAL) → 501 \
                         (v1 supports non-OPTIONAL shared variables)"
                    )));
                }
                match unify(ldef, rdef) {
                    Unify::Sat(conds) => corr.extend(conds),
                    // Provably never equal on a shared variable ⇒ never compatible ⇒
                    // this right branch can never remove the left row.
                    Unify::Empty => {
                        never_compatible = true;
                        break;
                    }
                    Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                }
            }
            if never_compatible {
                continue;
            }
            anti.push(SqlCond::NotExists {
                scans: r.core.clone(),
                conds: corr,
            });
        }
        l.where_conds.extend(anti);
        out.push(l);
    }
    Ok(out)
}

/// Whether a term def reads any of the given OPTIONAL scan aliases — i.e. its
/// value may be UNBOUND (the trigger to defer a MINUS shared variable → 501).
fn def_reads_opt_alias(def: &TermDef, opt_aliases: &[usize]) -> bool {
    def.columns().iter().any(|c| opt_aliases.contains(&c.alias))
}

/// Join two bag-unions (the product), unifying shared variables in each pair.
/// ADR-0025 Tier-1 bug #2 (flat mirror): a nested sub-SELECT with a SLICE (LIMIT/OFFSET) as
/// a Join/LeftJoin/Union/Minus operand would have its slice SILENTLY DROPPED — the
/// branch-combining operators consume only `.branches`, discarding the operand's own
/// `limit`/`offset`. It cannot be emitted soundly either (the surviving subset depends on
/// the SPARQL ORDER BY, which sf applies in the executor, not in SQL — SQL collation ≠
/// SPARQL order). Sound 501 (ADR-0007), mirroring the tree `lower_as_subplan` boundary. An
/// ORDER BY with NO slice is a no-op for a bag-valued operand and is allowed through.
fn reject_dropped_slice(t: &TransPattern) -> Result<()> {
    if t.limit.is_some() || t.offset > 0 {
        return Err(Error::Unsupported(
            "SubPlan with LIMIT/OFFSET as a join/union/minus operand is not yet supported → \
             501 (the slice would be silently dropped; its surviving subset depends on the \
             SPARQL ORDER BY, applied in the executor not SQL — ADR-0025 Tier-1)"
                .to_owned(),
        ));
    }
    Ok(())
}

pub fn join_branches(left: Vec<Branch>, right: Vec<Branch>) -> Result<Vec<Branch>> {
    let mut out = Vec::new();
    for l in &left {
        for r in &right {
            if let Some(b) = merge(l.clone(), r)? {
                out.push(b);
            }
        }
    }
    Ok(out)
}

/// Merge a right branch into a left branch (inner join). `None` ⇒ pruned.
fn merge(mut left: Branch, right: &Branch) -> Result<Option<Branch>> {
    if left.path.is_some() || right.path.is_some() {
        // ADR-0032 D6 flat/tree 501-parity: before surfacing the unconditional
        // "no join onto any path branch" 501 below, check whether this join is
        // PROVABLY DISJOINT via the same leading-literal-prefix mechanism
        // `unify::align_templates` applies during ordinary unification on the tree
        // path. The tree path has no preemptive path check, so it reaches
        // `align_templates` and proves an empty join BEFORE its own (otherwise
        // identical) path restriction would ever fire; the flat path used to 501
        // unconditionally first, never getting that far — a genuine, narrow
        // tree-exceeds-flat divergence (differential_star.rs's
        // `star_pattern_at_property_path_endpoint_*` pin). Only the disjointness
        // proof is checked here, never the full `unify()` — a Sat/Unsupported
        // verdict on a path-carrying branch is not safe to act on before the path
        // restriction it would otherwise bypass. Everything else keeps 501ing
        // exactly as before (e.g. differential_paths.rs's
        // `closure_joined_with_class_pattern_hits_the_identical_general_boundary_
        // on_both_engines` pin: its templates are IDENTICAL, not disjoint, so no
        // escape applies and both engines still 501).
        for (var, rdef) in &right.bindings {
            if let Some(ldef) = left.bindings.get(var) {
                if templates_provably_disjoint(ldef, rdef) {
                    return Ok(None);
                }
            }
        }
        return Err(Error::Unsupported(
            "joining a property-path closure with another pattern deferred → 501 \
             (v1 = a standalone ?s P+ ?o)"
                .to_owned(),
        ));
    }
    // ADR-0025 Tier-1 (opts-nullability), flat mirror of the tree `insert_or_unify` fix:
    // a shared var whose LEFT def reads a nullable (prior-OPTIONAL) alias may be UNBOUND;
    // plain equality then drops the row, but SPARQL compatible-merge (§18.5) keeps it and
    // binds from the mandatory side. `nullable_aliases` here reflects `left`'s accumulated
    // OPTIONAL scans (the bug shape: OPTIONAL on the left, mandatory re-join on the right).
    // Union BOTH branches' nullable aliases: the OPTIONAL-bearing group can be the RIGHT
    // operand (its OPTIONAL scans live in `right`), so a set from `left` alone would miss it
    // and silently fall through to plain equality — an order-dependent row drop (the tree
    // path is immune because its fold has already merged both operands' opts into one
    // accumulator by the time the shared-var equality runs).
    let opt_aliases = &left.nullable_aliases() | &right.nullable_aliases();
    for (var, rdef) in &right.bindings {
        match left.bindings.get(var) {
            Some(ldef) => {
                let ldef = ldef.clone();
                let l_nullable = def_is_nullable(&ldef, &opt_aliases);
                let r_nullable = def_is_nullable(rdef, &opt_aliases);
                match unify(&ldef, rdef) {
                    Unify::Sat(conds) => {
                        if (l_nullable || r_nullable) && !conds.is_empty() {
                            // BOTH sides nullable ⇒ non-injective COALESCE needed, which
                            // SQL DISTINCT/dedup cannot collapse → sound 501 (mirror of the
                            // tree `insert_or_unify` fix; ADR-0025 both-nullable residual).
                            if l_nullable && r_nullable {
                                return Err(Error::Unsupported(
                                    "INNER JOIN correlating on a variable bound by TWO OPTIONALs \
                                     (both sides nullable) is not yet supported → 501 (the \
                                     compatible-merge value needs a non-injective COALESCE that \
                                     SQL-level DISTINCT/dedup cannot collapse — ADR-0025)"
                                        .to_owned(),
                                ));
                            }
                            // R1 null-safe equality; R2 mandatory-side raw value (no COALESCE).
                            for c in conds {
                                left.where_conds.push(null_safe(c, true));
                            }
                            let merged = if l_nullable { rdef.clone() } else { ldef };
                            left.bindings.insert(var.clone(), merged);
                        } else {
                            left.where_conds.extend(conds);
                        }
                    }
                    // A nullable side that is provably disjoint on VALUES can still be UNBOUND
                    // (compatible) — pruning the whole branch would lose those rows; the
                    // plain-equality fold cannot express "keep only the unbound-compatible
                    // rows" → sound 501, mirroring the tree path.
                    Unify::Empty if l_nullable || r_nullable => {
                        return Err(Error::Unsupported(
                            "INNER JOIN correlating on an OPTIONAL-bound (nullable) variable \
                             whose definitions are provably disjoint is not yet supported → \
                             501 (compatible-merge must keep the unbound-compatible rows)"
                                .to_owned(),
                        ));
                    }
                    Unify::Empty => return Ok(None),
                    Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                }
            }
            None => {
                left.bindings.insert(var.clone(), rdef.clone());
            }
        }
    }
    left.core.extend(right.core.iter().cloned());
    left.opts.extend(right.opts.iter().cloned());
    left.where_conds.extend(right.where_conds.iter().cloned());
    // SubPlan joins from the right branch are appended to the merged result so
    // that a SubPlan-as-join-operand (ADR-0023 M5 Wave 2) survives the merge.
    // The flat path never sets `subplan_joins`, so this is a no-op on the flat path.
    left.subplan_joins
        .extend(right.subplan_joins.iter().cloned());
    Ok(Some(left))
}
