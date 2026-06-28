//! Unfold — the SPARQL algebra → IQ base translation (ADR-0007 step 3, the
//! ISWC-2018 ground truth). Each triple pattern becomes the relational
//! sub-expressions of the matching mapping-IR entries; shared variables unify to
//! raw-column equalities ([`crate::unify`]); OPTIONAL becomes a NULL-safe LEFT
//! JOIN obeying R1–R5. This is the **unoptimized** tree the [`crate::cascade`]
//! then rewrites.

use sf_core::ir::{ObjectMap, TermMap, TriplesMap};
use sf_core::{NamedNode, Term};
use spargebra::algebra::GraphPattern;
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern, TriplePattern};

use crate::iq::{Branch, Scan, SqlCond, TermDef};
use crate::leftjoin::left_join_branches;
use crate::saturate::Tbox;
use crate::unify::{filter_cond, unify, Unify};
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
    pub(crate) maps: &'a [TriplesMap],
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
            // Deferred → 501 (documented, never silent): MINUS, GRAPH, ORDER BY,
            // aggregates, LATERAL, SERVICE (ADR-0007 §v1 SPARQL coverage; ADR-0008
            // tier-2).
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

enum PredMatch {
    Yes(TermDef),
    No,
}

/// A VALUES inline ground term → an RDF [`Term`]. A quoted triple term
/// (SPARQL 1.2 `sparql-12`) is deferred → 501 (never silent).
fn ground_term_to_term(gt: &GroundTerm) -> Result<Term> {
    match gt {
        GroundTerm::NamedNode(n) => Ok(Term::NamedNode(n.clone())),
        GroundTerm::Literal(l) => Ok(Term::Literal(l.clone())),
        other => Err(Error::Unsupported(format!(
            "VALUES ground term not supported in v1 → 501: {other:?}"
        ))),
    }
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
