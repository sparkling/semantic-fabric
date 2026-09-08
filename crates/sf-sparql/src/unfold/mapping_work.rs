//! Request-owned atom expansion; raw callers retain the same mapping semantics.

use sf_core::ir::{LogicalSource, PredicateObjectMap, TermMap, TriplesMap};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use super::{AtomGraph, Unfolder, RDF_TYPE};
use crate::graph_map::is_default_graph;
use crate::{iq::Branch, CompilerWorkMode, Result};

impl<'a> Unfolder<'a> {
    pub(crate) fn with_work_mode(mut self, work: CompilerWorkMode<'a>) -> Self {
        self.work_mode = work;
        self
    }

    pub(super) fn work_checkpoint(&self) -> Result<()> {
        match self.work_mode {
            CompilerWorkMode::Uncontrolled => Ok(()),
            CompilerWorkMode::Metered(context) => context.checkpoint(),
        }
    }

    pub(super) fn reserve_product(&self, factors: &[usize]) -> Result<()> {
        if let CompilerWorkMode::Metered(context) = self.work_mode {
            context.reserve_checked_product(factors)?;
        }
        Ok(())
    }

    pub(super) fn copy_source(&self, source: &LogicalSource) -> Result<LogicalSource> {
        match self.work_mode {
            CompilerWorkMode::Uncontrolled => Ok(source.clone()),
            CompilerWorkMode::Metered(context) => context.clone_logical_source(source),
        }
    }

    pub(super) fn filter_graphs(
        &self,
        branch: &mut Branch,
        graphs: &[&TermMap],
        alias: usize,
    ) -> Result<bool> {
        self.reserve_product(&[graphs.len()])?;
        crate::graph_map::apply_filter(branch, self.current_graph.as_ref(), graphs, alias)
    }

    /// Preserve canonical first-declaration order and exact graph-map equality.
    /// Charge input visits before iteration and prospective comparisons before
    /// each duplicate check; no graph-attempt vector or owned map copy is built.
    fn graph_union<'g>(
        &self,
        subject: &'g [TermMap],
        pom: &'g [TermMap],
    ) -> Result<Vec<&'g TermMap>> {
        if matches!(self.work_mode, CompilerWorkMode::Uncontrolled) {
            return Ok(sf_core::graph_map::union(subject, pom));
        }
        self.reserve_product(&[subject.len()])?;
        self.reserve_product(&[pom.len()])?;
        let mut graphs = Vec::new();
        for graph in subject.iter().chain(pom) {
            self.work_checkpoint()?;
            self.reserve_product(&[graphs.len()])?;
            if !graphs.contains(&graph) {
                graphs.push(graph);
            }
        }
        Ok(graphs)
    }

    fn graph_attempt_count(&self, graphs: &[&TermMap], variable: bool) -> Result<usize> {
        // Named-graph mode scans once to count and once to execute. In filter
        // mode atom/class filtering scans the set itself; it still has one attempt.
        if variable {
            self.reserve_product(&[graphs.len(), 2])?;
            Ok(graphs.iter().filter(|gm| !is_default_graph(gm)).count())
        } else {
            Ok(1)
        }
    }

    /// All alternatives for one triple pattern. Fixed/default graphs filter
    /// exactly one attempt; GRAPH ?g enumerates only distinct named graphs.
    /// Classes inherit subject graphs only. Candidate charges include later
    /// rejection; raw alias, order, entailment and binding behavior are retained.
    pub(crate) fn pattern_branches(&mut self, tp: &TriplePattern) -> Result<Vec<Branch>> {
        self.work_checkpoint()?;
        self.reserve_product(&[self.maps.len()])?;
        let mut out = Vec::new();
        let pred_iri = match &tp.predicate {
            NamedNodePattern::NamedNode(p) => Some(p.as_str()),
            NamedNodePattern::Variable(_) => None,
        };
        let graph_var = self.current_graph_var.clone();
        for tm in self.maps {
            self.work_checkpoint()?;
            if pred_iri == Some(RDF_TYPE) || pred_iri.is_none() {
                let graphs = self.graph_union(&tm.subject.graphs, &[])?;
                let attempts = self.graph_attempt_count(&graphs, graph_var.is_some())?;
                // Invalid class-object forms return before visiting any class.
                if matches!(
                    tp.object,
                    TermPattern::NamedNode(_) | TermPattern::Variable(_)
                ) {
                    self.reserve_product(&[attempts, tm.subject.classes.len()])?;
                }
                match &graph_var {
                    Some(v) => {
                        for &gm in &graphs {
                            self.work_checkpoint()?;
                            if !is_default_graph(gm) {
                                self.class_atoms(tp, tm, &graphs, Some((v, gm)), &mut out)?;
                            }
                        }
                    }
                    None => self.class_atoms(tp, tm, &graphs, None, &mut out)?,
                }
            }
            self.reserve_product(&[tm.predicate_object_maps.len()])?;
            for pom in &tm.predicate_object_maps {
                self.work_checkpoint()?;
                let graphs = self.graph_union(&tm.subject.graphs, &pom.graphs)?;
                let attempts = self.graph_attempt_count(&graphs, graph_var.is_some())?;
                self.reserve_product(&[attempts, pom.predicates.len(), pom.objects.len()])?;
                match &graph_var {
                    Some(v) => {
                        for &gm in &graphs {
                            self.work_checkpoint()?;
                            if !is_default_graph(gm) {
                                self.pom_atoms(
                                    tp,
                                    tm,
                                    pom,
                                    pred_iri,
                                    AtomGraph::Bind(v, gm),
                                    &mut out,
                                )?;
                            }
                        }
                    }
                    None => {
                        self.pom_atoms(tp, tm, pom, pred_iri, AtomGraph::Filter(&graphs), &mut out)?
                    }
                }
            }
        }
        self.work_checkpoint()?;
        Ok(out)
    }

    fn pom_atoms(
        &mut self,
        tp: &TriplePattern,
        tm: &TriplesMap,
        pom: &PredicateObjectMap,
        predicate: Option<&str>,
        graph: AtomGraph<'_>,
        out: &mut Vec<Branch>,
    ) -> Result<()> {
        // A zero product must not walk an arbitrarily long predicate vector.
        if pom.objects.is_empty() {
            return Ok(());
        }
        for pm in &pom.predicates {
            for om in &pom.objects {
                self.work_checkpoint()?;
                if let Some(branch) = self.atom(tp, tm, pm, om, predicate, graph)? {
                    out.push(branch);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "mapping_work_tests.rs"]
mod tests;
