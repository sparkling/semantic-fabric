//! Unfold — the SPARQL algebra → IQ base translation (ADR-0007 step 3, the
//! ISWC-2018 ground truth). Each triple pattern becomes the relational
//! sub-expressions of the matching mapping-IR entries; shared variables unify to
//! raw-column equalities ([`crate::unify`]); OPTIONAL becomes a NULL-safe LEFT
//! JOIN obeying R1–R5. This is the **unoptimized** tree the [`crate::cascade`]
//! then rewrites.

use sf_core::ir::{ObjectMap, Segment, TermMap, Template, TriplesMap};
use sf_core::{NamedNode, Term};
use spargebra::algebra::{GraphPattern, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::iq::{Branch, HopRelation, PathClosure, Scan, SqlCond, TermDef};
use crate::leftjoin::left_join_branches;
use crate::saturate::Tbox;
use crate::unify::{filter_cond, unify, Unify};
use crate::{Error, Result};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// ADR-0010 recursion-depth backstop for property-path closures. SPARQL path
/// reachability is set-based (the CTE body uses `UNION`), so the closure already
/// terminates on a finite hop relation — this bound is the safety net against a
/// pathological mapping, capping the longest chased chain. A simple path longer
/// than this is truncated (a documented limit, not a correctness claim — deeper
/// chains stay future work, like the rest of the path surface deferred to 501).
const PATH_MAX_DEPTH: usize = 256;

/// The translation of one graph pattern: a bag union of [`Branch`]es plus the
/// solution modifiers peeled from the algebra.
pub struct TransPattern {
    pub branches: Vec<Branch>,
    pub project: Option<Vec<String>>,
    pub distinct: bool,
    pub limit: Option<usize>,
    pub offset: usize,
}

impl TransPattern {
    fn plain(branches: Vec<Branch>) -> Self {
        Self {
            branches,
            project: None,
            distinct: false,
            limit: None,
            offset: 0,
        }
    }
}

/// Walks the mappings + T-Box, allocating fresh scan aliases.
pub struct Unfolder<'a> {
    maps: &'a [TriplesMap],
    tbox: &'a Tbox,
    dialect: sf_sql::Dialect,
    next_alias: usize,
}

impl<'a> Unfolder<'a> {
    pub fn new(maps: &'a [TriplesMap], tbox: &'a Tbox, dialect: sf_sql::Dialect) -> Self {
        Self {
            maps,
            tbox,
            dialect,
            next_alias: 0,
        }
    }

    fn alias(&mut self) -> usize {
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
                let l = self.translate_pattern(left)?;
                let r = self.translate_pattern(right)?;
                Ok(TransPattern::plain(join_branches(l.branches, r.branches)?))
            }
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                let l = self.translate_pattern(left)?;
                let r = self.translate_pattern(right)?;
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
                    let cond =
                        filter_cond(expr, &b.bindings, self.dialect).map_err(Error::Unsupported)?;
                    b.where_conds.push(cond);
                }
                Ok(t)
            }
            GraphPattern::Union { left, right } => {
                let mut l = self.translate_pattern(left)?;
                let r = self.translate_pattern(right)?;
                l.branches.extend(r.branches);
                Ok(TransPattern::plain(l.branches))
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            } => Ok(TransPattern::plain(vec![self.path_branch(subject, path, object)?])),
            // Deferred → 501 (documented, never silent): property paths, MINUS,
            // GRAPH, BIND, VALUES, ORDER BY, aggregates, LATERAL, SERVICE
            // (ADR-0007 §v1 SPARQL coverage; ADR-0008 tier-2).
            other => Err(Error::Unsupported(format!(
                "graph pattern not supported in v1 → 501: {other:?}"
            ))),
        }
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

    /// All atom alternatives for one triple pattern (a bag union over the
    /// matching triples-maps / predicate-object maps / `rr:class` entries).
    fn pattern_branches(&mut self, tp: &TriplePattern) -> Result<Vec<Branch>> {
        let mut out = Vec::new();
        // Predicate match set (direct + sub-properties + inverse/symmetric).
        let pred_iri = match &tp.predicate {
            NamedNodePattern::NamedNode(p) => Some(p.as_str().to_owned()),
            NamedNodePattern::Variable(_) => None,
        };
        let want_type = pred_iri.as_deref() == Some(RDF_TYPE);

        for tm in self.maps {
            // rr:class → rdf:type atoms (when predicate is rdf:type or a variable).
            if want_type || pred_iri.is_none() {
                self.class_atoms(tp, tm, &mut out)?;
            }
            for pom in &tm.predicate_object_maps {
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
                    .ok_or_else(|| Error::Mapping(format!("unknown parent map {}", r.parent_triples_map)))?
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
        let (q_subj, q_obj) = if swap { (obj_def, subj_def) } else { (subj_def, obj_def) };

        // Bind/constrain the three query positions.
        if let NamedNodePattern::Variable(pv) = &tp.predicate {
            bind(&mut branch, pv.as_str(), pred_def)?;
        }
        self.bind_position(&mut branch, &tp.subject, q_subj)?;
        match self.bind_position(&mut branch, &tp.object, q_obj)? {
            true => Ok(Some(branch)),
            false => Ok(None),
        }
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
                bind(&mut branch, ov.as_str(), TermDef::Const(Term::NamedNode(class.clone())))?;
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
                            Ok((PredMatch::Yes(TermDef::Const(Term::NamedNode(q.clone()))), false))
                        } else if inverse.iter().any(|i| i == q.as_str()) {
                            Ok((PredMatch::Yes(TermDef::Const(Term::NamedNode(q.clone()))), true))
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
            TermPattern::BlankNode(b) => {
                self.constrain(branch, TermDef::Const(Term::BlankNode(b.clone())), def)
            }
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

    /// Translate a property-path pattern `?s P+ ?o` / `?s P* ?o` into a recursive
    /// closure branch (ADR-0007). v1 covers `OneOrMore` / `ZeroOrMore` over a
    /// **single predicate IRI** with both endpoints variables; everything else
    /// (the `?` operator, `!`/NPS, sequence / alternative / inverse combinations,
    /// a bound endpoint, a predicate served by a `refObjectMap` join or by more
    /// than one triples-map, a multi-column key) stays deferred → 501.
    fn path_branch(
        &mut self,
        subject: &TermPattern,
        path: &PropertyPathExpression,
        object: &TermPattern,
    ) -> Result<Branch> {
        let (pred, reflexive) = match path {
            PropertyPathExpression::OneOrMore(p) => (p.as_ref(), false),
            PropertyPathExpression::ZeroOrMore(p) => (p.as_ref(), true),
            other => {
                return Err(Error::Unsupported(format!(
                    "property path operator deferred → 501 (v1 = P+/P* only): {other:?}"
                )))
            }
        };
        let pred_iri = match pred {
            PropertyPathExpression::NamedNode(p) => p.as_str(),
            other => {
                return Err(Error::Unsupported(format!(
                    "property path over a non-predicate sub-path deferred → 501: {other:?}"
                )))
            }
        };
        let (subj_var, obj_var) = match (subject, object) {
            (TermPattern::Variable(s), TermPattern::Variable(o)) => (s.as_str(), o.as_str()),
            _ => {
                return Err(Error::Unsupported(
                    "property path with a bound endpoint deferred → 501 (v1 = ?s P+ ?o)".to_owned(),
                ))
            }
        };

        // `P*` (ZeroOrMore) must bind the reflexive `(x, x)` ZeroLengthPath for
        // EVERY node of the active graph (SPARQL 1.1 §9.3), not just nodes of the
        // path predicate. The raw-key single-hop model can only seed reflexive
        // pairs over the hop's own node set, which equals the graph's node set
        // ONLY when `pred_iri` is the sole predicate the mapping produces. For a
        // multi-predicate virtual graph it would silently miss nodes appearing
        // only under another predicate, so defer rather than ship a wrong answer.
        if reflexive && !self.graph_is_single_predicate(pred_iri) {
            return Err(Error::Unsupported(
                "P* (ZeroOrMore) over a multi-predicate virtual graph deferred → 501: \
                 the reflexive ZeroLengthPath must bind (x,x) for every node of the \
                 active graph, which the single-predicate raw-key hop model cannot \
                 enumerate provably across heterogeneous term maps"
                    .to_owned(),
            ));
        }

        // The one-hop relation: the (subject, object) raw-key pairs the mapping
        // produces for `pred_iri`. v1 requires exactly one producing triples-map
        // with a direct constant predicate and a single-column term object.
        let hop = self.hop_relation(pred_iri)?;

        // Rebuild the subject / object term maps to read the CTE's canonical key
        // columns (`sf_s` / `sf_o`); terms are materialised at projection only.
        let subj_def = TermDef::Derived {
            term_map: rewrite_single_col(&hop.subj_map, "sf_s")?,
            alias: hop.alias,
        };
        let obj_def = TermDef::Derived {
            term_map: rewrite_single_col(&hop.obj_map, "sf_o")?,
            alias: hop.alias,
        };

        let mut branch = Branch::empty();
        branch.path = Some(PathClosure {
            alias: hop.alias,
            reflexive,
            hop: HopRelation {
                source: hop.source,
                subj_col: hop.subj_col,
                obj_col: hop.obj_col,
            },
            max_depth: PATH_MAX_DEPTH,
        });
        // Bind via the shared helper so `?s P+ ?s` self-unifies (ColEq sf_s,sf_o).
        bind(&mut branch, subj_var, subj_def)?;
        match bind(&mut branch, obj_var, obj_def)? {
            true => Ok(branch),
            false => Err(Error::Unsupported(
                "property-path endpoints unify to empty → 501".to_owned(),
            )),
        }
    }

    /// `true` iff `pred_iri` is the ONLY predicate the whole mapping produces —
    /// no other `rr:predicate` and no `rr:class` (which would add `rdf:type`
    /// triples and class-IRI object nodes). In that case the hop relation's node
    /// set (subjects ∪ objects of `pred_iri`) equals the active graph's node set,
    /// making `P*`'s reflexive ZeroLengthPath provably complete (under the v1
    /// same-domain raw-key assumption that already underpins `P+`).
    fn graph_is_single_predicate(&self, pred_iri: &str) -> bool {
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

    /// Resolve the single one-hop relation for `pred_iri` (v1: exactly one
    /// producing triples-map, direct constant predicate, single-column subject and
    /// `Term` object term maps; else 501).
    fn hop_relation(&mut self, pred_iri: &str) -> Result<ResolvedHop> {
        let mut found: Option<ResolvedHop> = None;
        for tm in self.maps {
            for pom in &tm.predicate_object_maps {
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
                                "property path over a refObjectMap-joined predicate deferred → 501"
                                    .to_owned(),
                            ))
                        }
                    };
                    if found.is_some() {
                        return Err(Error::Unsupported(
                            "property path over a predicate from >1 mapping deferred → 501".to_owned(),
                        ));
                    }
                    found = Some(ResolvedHop {
                        alias: self.alias(),
                        source: tm.source.clone(),
                        subj_col: single_col(&tm.subject.term)?,
                        obj_col: single_col(&obj_map)?,
                        subj_map: tm.subject.term.clone(),
                        obj_map,
                    });
                }
            }
        }
        found.ok_or_else(|| {
            Error::Unsupported(format!("property path predicate {pred_iri} is not mapped → 501"))
        })
    }
}

/// A resolved one-hop relation plus the term maps that rebuild its endpoints.
struct ResolvedHop {
    alias: usize,
    source: sf_core::ir::LogicalSource,
    subj_col: Box<str>,
    obj_col: Box<str>,
    subj_map: TermMap,
    obj_map: TermMap,
}

/// The single source column a term map reads, or 501 if it is not single-column
/// (a constant, or a multi-column template — deferred for v1 paths).
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

enum PredMatch {
    Yes(TermDef),
    No,
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
fn bind(branch: &mut Branch, var: &str, def: TermDef) -> Result<bool> {
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

/// Join two bag-unions (the product), unifying shared variables in each pair.
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
        return Err(Error::Unsupported(
            "joining a property-path closure with another pattern deferred → 501 \
             (v1 = a standalone ?s P+ ?o)"
                .to_owned(),
        ));
    }
    for (var, rdef) in &right.bindings {
        match left.bindings.get(var) {
            Some(ldef) => match unify(ldef, rdef) {
                Unify::Sat(conds) => left.where_conds.extend(conds),
                Unify::Empty => return Ok(None),
                Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
            },
            None => {
                left.bindings.insert(var.clone(), rdef.clone());
            }
        }
    }
    left.core.extend(right.core.iter().cloned());
    left.opts.extend(right.opts.iter().cloned());
    left.where_conds.extend(right.where_conds.iter().cloned());
    Ok(Some(left))
}
