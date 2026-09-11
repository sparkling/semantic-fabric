//! Request-owned atom expansion; raw callers retain the same mapping semantics.

use sf_core::ir::{LogicalSource, PredicateObjectMap, TermMap, TriplesMap};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use super::{AtomGraph, Unfolder, RDF_TYPE};
use crate::build::control::{BuildVec, BuildWork};
use crate::graph_map::is_default_graph;
use crate::{iq::Branch, CompilerWorkMode, Result};

pub(crate) fn all_pairwise_disjoint_with_work(
    arms: &[Branch],
    work: BuildWork<'_>,
) -> Result<bool> {
    work.checkpoint()?;
    for i in 0..arms.len() {
        work.charge(1)?;
        for j in i + 1..arms.len() {
            if !arms_disjoint(&arms[i], &arms[j], work)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(crate) fn disjoint_groups_with_work(
    arms: &[Branch],
    work: BuildWork<'_>,
) -> Result<Vec<Vec<usize>>> {
    let mut parent = work.vector(arms.len())?;
    parent.extend(0..arms.len());
    fn find(parent: &mut [usize], index: usize, work: BuildWork<'_>) -> Result<usize> {
        let mut root = index;
        loop {
            work.charge(1)?;
            if parent[root] == root {
                break;
            }
            root = parent[root];
        }
        let mut cursor = index;
        while cursor != root {
            work.charge(1)?;
            let next = parent[cursor];
            parent[cursor] = root;
            cursor = next;
        }
        Ok(root)
    }
    for i in 0..arms.len() {
        work.charge(1)?;
        for j in i + 1..arms.len() {
            if !arms_disjoint(&arms[i], &arms[j], work)? {
                let left = find(&mut parent, i, work)?;
                let right = find(&mut parent, j, work)?;
                work.charge(1)?;
                parent[left.max(right)] = left.min(right);
            }
        }
    }
    let mut groups = work.vector(arms.len())?;
    groups.extend((0..arms.len()).map(|_| BuildVec::new(Vec::new())));
    for i in 0..arms.len() {
        let root = find(&mut parent, i, work)?;
        work.push(&mut groups[root], i)?;
    }
    let mut out = BuildVec::new(Vec::new());
    for group in groups {
        work.charge(1)?;
        if !group.values.is_empty() {
            work.push(&mut out, group.into_inner())?;
        }
    }
    Ok(out.into_inner())
}

fn arms_disjoint(a: &Branch, b: &Branch, work: BuildWork<'_>) -> Result<bool> {
    use crate::iq::TermDef;
    work.charge(1)?;
    for (name, left) in &a.bindings {
        work.charge(1)?;
        // Every possible ordered-map comparison is prepaid before lookup.
        for other in b.bindings.keys() {
            work.charge(1)?;
            work.charge(name.len().min(other.len()))?;
        }
        let Some(right) = b.bindings.get(name) else {
            continue;
        };
        if let (TermDef::Const(a), TermDef::Const(b)) = (left, right) {
            let equal = match work.mode {
                CompilerWorkMode::Uncontrolled => a == b,
                CompilerWorkMode::Metered(cx) => cx.constant_terms_equal(left, right)?,
            };
            if !equal {
                return Ok(true);
            }
            continue;
        }
        if template_prefix_conflict(left, right, work)? {
            return Ok(true);
        }
        let specs = super::mapping_term_map(left)
            .and_then(super::term_map_spec)
            .zip(super::mapping_term_map(right).and_then(super::term_map_spec));
        if let Some((a, b)) = specs {
            work.charge(1)?;
            for text in [
                a.language.as_deref(),
                b.language.as_deref(),
                Some(super::normalized_datatype(a)),
                Some(super::normalized_datatype(b)),
            ]
            .into_iter()
            .flatten()
            {
                work.charge(text.len())?;
            }
        }
        if super::term_specs_disjoint(left, right) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn template_prefix_conflict(
    left: &crate::iq::TermDef,
    right: &crate::iq::TermDef,
    work: BuildWork<'_>,
) -> Result<bool> {
    use sf_core::ir::Segment;
    let template = |def| match super::mapping_term_map(def) {
        Some(TermMap::Template(template, spec)) if spec.base.is_none() => Some(template.segments()),
        _ => None,
    };
    let (Some(left), Some(right)) = (template(left), template(right)) else {
        return Ok(false);
    };
    // Borrow each leading literal byte; no temporary prefix String is built.
    fn next<'a>(
        segments: &mut std::slice::Iter<'a, Segment>,
        bytes: &mut std::str::Bytes<'a>,
        work: BuildWork<'_>,
    ) -> Result<Option<u8>> {
        loop {
            work.charge(1)?;
            if let Some(byte) = bytes.next() {
                return Ok(Some(byte));
            }
            match segments.next() {
                Some(Segment::Literal(text)) => *bytes = text.bytes(),
                _ => return Ok(None),
            }
        }
    }
    let (mut left, mut right) = (left.iter(), right.iter());
    let (mut a, mut b) = ("".bytes(), "".bytes());
    loop {
        let Some(a) = next(&mut left, &mut a, work)? else {
            return Ok(false);
        };
        let Some(b) = next(&mut right, &mut b, work)? else {
            return Ok(false);
        };
        if a != b {
            return Ok(true);
        }
    }
}

impl<'a> Unfolder<'a> {
    pub(crate) fn with_work_mode(mut self, work: CompilerWorkMode<'a>) -> Self {
        self.work_mode = work;
        self
    }

    pub(crate) fn work_mode(&self) -> CompilerWorkMode<'a> {
        self.work_mode
    }

    pub(crate) fn work_checkpoint(&self) -> Result<()> {
        match self.work_mode {
            CompilerWorkMode::Uncontrolled => Ok(()),
            CompilerWorkMode::Metered(context) => context.checkpoint(),
        }
    }

    pub(crate) fn reserve_product(&self, factors: &[usize]) -> Result<()> {
        if let CompilerWorkMode::Metered(context) = self.work_mode {
            context.reserve_checked_product(factors)?;
        }
        Ok(())
    }

    pub(crate) fn copy_source(&self, source: &LogicalSource) -> Result<LogicalSource> {
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
    pub(crate) fn graph_union<'g>(
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
