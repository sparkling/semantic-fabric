//! Request-owned mapping-wide path checks. Raw callers retain exact semantics.
use super::Unfolder;
use crate::graph_map::RR_DEFAULT_GRAPH;
use crate::{CompilerWorkMode, Result};
use sf_core::ir::TermMap;
use sf_core::{NamedNode, Term};

impl Unfolder<'_> {
    /// A reflexive path is complete only when the whole mapping uses this one
    /// predicate, with no rr:class. This deliberately remains mapping-wide.
    pub(crate) fn graph_is_single_predicate(&self, pred_iri: &str) -> Result<bool> {
        self.work_checkpoint()?;
        self.reserve_product(&[self.maps.len()])?;
        for tm in self.maps {
            self.work_checkpoint()?;
            if !tm.subject.classes.is_empty() {
                return Ok(false);
            }
            self.reserve_product(&[tm.predicate_object_maps.len()])?;
            for pom in &tm.predicate_object_maps {
                self.work_checkpoint()?;
                self.reserve_product(&[pom.predicates.len()])?;
                for pm in &pom.predicates {
                    self.work_checkpoint()?;
                    if !matches!(pm, TermMap::Constant(Term::NamedNode(q)) if q.as_str() == pred_iri)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        self.work_checkpoint()?;
        Ok(true)
    }

    /// ADR-0035: any dynamic graph anywhere makes constant-only enumeration
    /// incomplete, even when its predicate differs from the requested path.
    pub(crate) fn has_non_constant_graph_map(&self) -> Result<bool> {
        self.work_checkpoint()?;
        self.reserve_product(&[self.maps.len()])?;
        for tm in self.maps {
            self.work_checkpoint()?;
            if self.dynamic_graphs(&tm.subject.graphs)? {
                return Ok(true);
            }
            self.reserve_product(&[tm.predicate_object_maps.len()])?;
            for pom in &tm.predicate_object_maps {
                self.work_checkpoint()?;
                if self.dynamic_graphs(&pom.graphs)? {
                    return Ok(true);
                }
            }
        }
        self.work_checkpoint()?;
        Ok(false)
    }

    fn dynamic_graphs(&self, graphs: &[TermMap]) -> Result<bool> {
        self.reserve_product(&[graphs.len()])?;
        for graph in graphs {
            self.work_checkpoint()?;
            if !matches!(graph, TermMap::Constant(_)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Distinct constant named graphs in first-declaration order, excluding
    /// rr:defaultGraph. Called only after the mapping-wide dynamic-graph check.
    /// Charge visits, prospective duplicate comparisons and each actual copy.
    pub(crate) fn declared_constant_graphs(&self) -> Result<Vec<NamedNode>> {
        self.work_checkpoint()?;
        self.reserve_product(&[self.maps.len()])?;
        let mut out = Vec::new();
        for tm in self.maps {
            self.work_checkpoint()?;
            self.collect_graphs(&tm.subject.graphs, &mut out)?;
            self.reserve_product(&[tm.predicate_object_maps.len()])?;
            for pom in &tm.predicate_object_maps {
                self.work_checkpoint()?;
                self.collect_graphs(&pom.graphs, &mut out)?;
            }
        }
        self.work_checkpoint()?;
        Ok(out)
    }

    fn collect_graphs(&self, graphs: &[TermMap], out: &mut Vec<NamedNode>) -> Result<()> {
        self.reserve_product(&[graphs.len()])?;
        for graph in graphs {
            self.work_checkpoint()?;
            if let TermMap::Constant(Term::NamedNode(n)) = graph {
                if n.as_str() == RR_DEFAULT_GRAPH {
                    continue;
                }
                self.reserve_product(&[out.len()])?;
                if !out.contains(n) {
                    out.push(self.copy_graph_name(n)?);
                }
            }
        }
        Ok(())
    }

    /// One scalar owner plus its UTF-8 payload, reserved before the exact clone.
    pub(crate) fn copy_graph_name(&self, graph: &NamedNode) -> Result<NamedNode> {
        self.work_checkpoint()?;
        if let CompilerWorkMode::Metered(context) = self.work_mode() {
            context.reserve_checked_product(&[graph.as_str().len()])?;
            context.reserve_checked_sum(&[1])?;
        }
        self.work_checkpoint()?;
        Ok(graph.clone())
    }
}

#[cfg(test)]
#[path = "graph_inventory_tests.rs"]
mod tests;
