//! Prospective request work for path mapping searches, not source-side recursion.

use sf_core::ir::{ObjectMap, TermMap};
use sf_core::Term;
use spargebra::term::NamedNode;

use super::{empty_hop, node_shape, single_col, CompiledHop};
use crate::iq::{HopExpr, HopRelation};
use crate::unfold::Unfolder;
use crate::{CompilerWorkMode, Error, Result};

impl Unfolder<'_> {
    fn copy_path_map(&self, map: &TermMap) -> Result<TermMap> {
        match self.work_mode() {
            CompilerWorkMode::Uncontrolled => Ok(map.clone()),
            CompilerWorkMode::Metered(context) => context.clone_term_map(map),
        }
    }

    fn path_graph_matches(&self, subject: &[TermMap], pom: &[TermMap]) -> Result<bool> {
        let graphs = self.graph_union(subject, pom)?;
        self.reserve_product(&[graphs.len()])?;
        crate::graph_map::path_scope_matches(self.current_graph.as_ref(), &graphs)
    }

    /// Finite, ordered complement. Borrow IRIs and move the first hop's endpoint
    /// maps/shapes; retaining Nps (even for one leaf) preserves bag semantics.
    pub(super) fn compile_nps(&self, negated: &[NamedNode]) -> Result<CompiledHop> {
        self.work_checkpoint()?;
        self.reserve_product(&[self.maps.len()])?;
        let mut complement: Vec<&str> = Vec::new();
        for tm in self.maps {
            self.work_checkpoint()?;
            // This rejection is global, before graph filtering, as in raw paths.
            if !tm.subject.classes.is_empty() {
                return Err(Error::Unsupported(
                    "!p over a graph with rr:class (rdf:type) triples cannot be enumerated soundly → 501".to_owned(),
                ));
            }
            self.reserve_product(&[tm.predicate_object_maps.len()])?;
            for pom in &tm.predicate_object_maps {
                self.work_checkpoint()?;
                if !self.path_graph_matches(&tm.subject.graphs, &pom.graphs)? {
                    continue;
                }
                self.reserve_product(&[pom.predicates.len()])?;
                for pm in &pom.predicates {
                    self.work_checkpoint()?;
                    let q = match pm {
                        TermMap::Constant(Term::NamedNode(q)) => q.as_str(),
                        _ => return Err(Error::Unsupported(
                            "!p over a non-constant (column/template) predicate map cannot be enumerated → 501".to_owned(),
                        )),
                    };
                    self.reserve_product(&[negated.len()])?;
                    if negated.iter().any(|n| n.as_str() == q) {
                        continue;
                    }
                    self.reserve_product(&[complement.len()])?;
                    if !complement.contains(&q) {
                        complement.push(q);
                    }
                }
            }
        }
        self.reserve_product(&[complement.len()])?;
        let mut predicates = complement.into_iter();
        let first = predicates.next().ok_or_else(|| {
            Error::Unsupported(
                "!p complement is empty (no non-negated predicate is mapped) → 501".to_owned(),
            )
        })?;
        self.work_checkpoint()?;
        let first = self.resolve_pred_hop(first)?;
        let mut exprs = vec![first.expr];
        for q in predicates {
            self.work_checkpoint()?;
            let hop = self.resolve_pred_hop(q)?;
            if first.subj_shape != hop.subj_shape || first.obj_shape != hop.obj_shape {
                return Err(Error::Unsupported(
                    "!p complement predicates have differing endpoint node shapes — the union cannot be reconstructed with one term map → 501".to_owned(),
                ));
            }
            exprs.push(hop.expr);
        }
        self.work_checkpoint()?;
        Ok(CompiledHop {
            expr: HopExpr::Nps(exprs),
            single_pred: None,
            ..first
        })
    }

    /// First search in the active graph; only an absent scoped hop permits a
    /// second, separately charged unscoped search. That pass supplies endpoint
    /// shapes for a provably empty relation, never wrong-graph source rows.
    pub(super) fn resolve_pred_hop(&self, pred_iri: &str) -> Result<CompiledHop> {
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

    fn find_pred_hop(&self, pred_iri: &str, graph_scoped: bool) -> Result<Option<CompiledHop>> {
        self.work_checkpoint()?;
        self.reserve_product(&[self.maps.len()])?;
        let mut found: Option<CompiledHop> = None;
        for tm in self.maps {
            self.work_checkpoint()?;
            self.reserve_product(&[tm.predicate_object_maps.len()])?;
            for pom in &tm.predicate_object_maps {
                self.work_checkpoint()?;
                if graph_scoped && !self.path_graph_matches(&tm.subject.graphs, &pom.graphs)? {
                    continue;
                }
                self.reserve_product(&[pom.predicates.len()])?;
                // Duplicate predicate declarations in ONE POM remain one producer.
                let produces = pom.predicates.iter().any(|pm| {
                    matches!(pm, TermMap::Constant(Term::NamedNode(q)) if q.as_str() == pred_iri)
                });
                if !produces {
                    continue;
                }
                self.reserve_product(&[pom.objects.len()])?;
                for om in &pom.objects {
                    self.work_checkpoint()?;
                    let obj_map = match om {
                        ObjectMap::Term(t) => self.copy_path_map(t)?,
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
                    let subj_map = self.copy_path_map(&tm.subject.term)?;
                    found = Some(CompiledHop {
                        expr: HopExpr::Pred(HopRelation {
                            source: self.copy_source(&tm.source)?,
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
        self.work_checkpoint()?;
        Ok(found)
    }
}

#[cfg(test)]
#[path = "mapping_work_tests.rs"]
mod tests;
