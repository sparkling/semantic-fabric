//! Canonical R2RML target-graph set construction.
//!
//! A predicate-object triple is placed in the set union of graph maps on its
//! subject map and predicate-object map (R2RML §11). Keeping this normalization
//! in `sf-core` lets mapping expansion, query unfolding, paths, and materialized
//! quad dumping share one definition.

use crate::ir::TermMap;

/// Return the distinct subject-map/POM graph-map union in deterministic,
/// first-declaration order. An empty result denotes the implicit default graph;
/// an explicit `rr:defaultGraph` remains a normal member for downstream target
/// selection.
pub fn union<'a>(subject: &'a [TermMap], pom: &'a [TermMap]) -> Vec<&'a TermMap> {
    let mut graphs = Vec::with_capacity(subject.len() + pom.len());
    for graph in subject.iter().chain(pom) {
        if !graphs.contains(&graph) {
            graphs.push(graph);
        }
    }
    graphs
}

/// Owned form for IR construction sites that must retain the normalized set.
pub fn union_owned(subject: &[TermMap], pom: &[TermMap]) -> Vec<TermMap> {
    union(subject, pom).into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{TermSpec, TermType};
    use crate::{NamedNode, Term};

    fn iri(value: &str) -> TermMap {
        TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked(value)))
    }

    #[test]
    fn union_is_distinct_and_subject_first_for_static_and_dynamic_maps() {
        let dynamic = TermMap::Column(
            "g".into(),
            TermSpec {
                term_type: TermType::Iri,
                datatype: None,
                language: None,
                base: None,
            },
        );
        let subject = vec![iri("http://ex/g1"), dynamic.clone()];
        let pom = vec![iri("http://ex/g1"), iri("http://ex/g2"), dynamic];

        let got = union_owned(&subject, &pom);
        assert_eq!(
            got,
            vec![subject[0].clone(), subject[1].clone(), pom[1].clone()]
        );
    }

    #[test]
    fn empty_union_preserves_the_implicit_default_graph_marker() {
        assert!(union(&[], &[]).is_empty());
    }
}
