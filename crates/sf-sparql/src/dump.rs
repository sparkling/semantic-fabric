//! Mapping-IR-driven **quad** dump (ADR-0005 named-graph conformance).
//!
//! The `?s ?p ?o` CONSTRUCT dump ([`crate::exec::construct`]) reconstructs the
//! virtual graph as triples in the *default* graph — it has no place to carry the
//! `rr:graphMap` graph term. This module walks the R2RML mapping IR directly,
//! emitting one [`Branch`] per (atom × graph-target) so the existing executor
//! ([`crate::exec::dump_quads`]) materialises **quads**: the subject/predicate/
//! object via the single `sf-core` term-gen path (datatype §10 included, ADR-0015),
//! and the graph via that **same** path — no second term path, no datatype drift.
//!
//! R2RML graph semantics (§6.1): a generated triple is placed in the union of the
//! subject map's graphs and the predicate-object map's graphs; an empty union ⇒
//! the default graph; an `rr:defaultGraph` member ⇒ the default graph. This is a
//! mapping-IR walk, distinct from the SPARQL → SQL CONSTRUCT path (no spargebra,
//! no BGP); it reuses the IQ [`Branch`] model so SQL emission, ref-object joins,
//! and bounded-memory streaming all come for free.

use sf_core::ir::{ObjectMap, TermMap, TriplesMap};
use sf_core::{NamedNode, Term};

use crate::iq::{Branch, ColRef, Scan, SqlCond, TermDef};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// The `rr:defaultGraph` IRI — a graph map naming it means the default graph.
const RR_DEFAULT_GRAPH: &str = "http://www.w3.org/ns/r2rml#defaultGraph";

/// The fixed variable names the dump branches bind. The executor reads these back
/// when assembling quads ([`crate::exec::dump_quads`]); `G` is present in a
/// branch's `bindings` iff that branch targets a *named* graph.
pub const VAR_S: &str = "s";
pub const VAR_P: &str = "p";
pub const VAR_O: &str = "o";
pub const VAR_G: &str = "g";

/// The child scan alias (the triples-map's own logical table) and the parent scan
/// alias (a referencing object map's parent table). Aliases need only be unique
/// *within* a branch, so every branch reuses this fixed pair.
const CHILD: usize = 0;
const PARENT: usize = 1;

/// Build the bag-union of branches that dumps the whole mapping as quads. Each
/// branch produces `s`/`p`/`o` (and, for a named-graph target, `g`) per source
/// row; the default-graph target omits `g`.
pub fn build_branches(maps: &[TriplesMap]) -> Vec<Branch> {
    let mut out = Vec::new();
    for tm in maps {
        class_atoms(tm, &mut out);
        for pom in &tm.predicate_object_maps {
            // §6.1 graph union: the subject map's graphs ∪ this POM's graphs.
            let combined: Vec<&TermMap> =
                tm.subject.graphs.iter().chain(pom.graphs.iter()).collect();
            let targets = graph_targets(&combined);
            for pm in &pom.predicates {
                for om in &pom.objects {
                    for &gt in &targets {
                        if let Some(b) = pom_branch(maps, tm, pm, om, gt) {
                            out.push(b);
                        }
                    }
                }
            }
        }
    }
    out
}

/// `rr:class` → `rdf:type` atoms, placed in the subject map's graphs (§6.1).
fn class_atoms(tm: &TriplesMap, out: &mut Vec<Branch>) {
    if tm.subject.classes.is_empty() {
        return;
    }
    let sg: Vec<&TermMap> = tm.subject.graphs.iter().collect();
    let targets = graph_targets(&sg);
    for class in &tm.subject.classes {
        for &gt in &targets {
            let mut b = Branch::single(Scan {
                alias: CHILD,
                source: tm.source.clone(),
            });
            b.bindings.insert(VAR_S.to_owned(), def_of(&tm.subject.term, CHILD));
            b.bindings.insert(
                VAR_P.to_owned(),
                TermDef::Const(Term::NamedNode(NamedNode::new_unchecked(RDF_TYPE))),
            );
            b.bindings
                .insert(VAR_O.to_owned(), TermDef::Const(Term::NamedNode(class.clone())));
            bind_graph(&mut b, gt);
            out.push(b);
        }
    }
}

/// One predicate-object atom branch for a single graph target, or `None` if a
/// referencing object map names an unknown parent triples-map.
fn pom_branch(
    maps: &[TriplesMap],
    tm: &TriplesMap,
    pm: &TermMap,
    om: &ObjectMap,
    gt: Option<&TermMap>,
) -> Option<Branch> {
    let mut b = Branch::single(Scan {
        alias: CHILD,
        source: tm.source.clone(),
    });
    b.bindings.insert(VAR_S.to_owned(), def_of(&tm.subject.term, CHILD));
    b.bindings.insert(VAR_P.to_owned(), def_of(pm, CHILD));
    let obj_def = match om {
        ObjectMap::Term(otm) => def_of(otm, CHILD),
        ObjectMap::Ref(r) => {
            let parent = maps.iter().find(|m| m.id == r.parent_triples_map)?;
            b.core.push(Scan {
                alias: PARENT,
                source: parent.source.clone(),
            });
            for j in &r.joins {
                b.where_conds.push(SqlCond::ColEq(
                    ColRef::new(CHILD, j.child.clone()),
                    ColRef::new(PARENT, j.parent.clone()),
                ));
            }
            def_of(&parent.subject.term, PARENT)
        }
    };
    b.bindings.insert(VAR_O.to_owned(), obj_def);
    // A graph map is evaluated against the subject's (child) logical table (§6.1).
    bind_graph(&mut b, gt);
    Some(b)
}

/// Bind the `g` variable for a named-graph target; the default-graph target
/// (`None`) leaves `g` unbound, which the executor reads as the default graph.
fn bind_graph(b: &mut Branch, gt: Option<&TermMap>) {
    if let Some(gm) = gt {
        b.bindings.insert(VAR_G.to_owned(), def_of(gm, CHILD));
    }
}

/// The distinct graph destinations for a triple given its applicable graph maps:
/// `Some(gm)` = a named graph, `None` = the default graph. No graph maps ⇒ a
/// single default-graph target; an `rr:defaultGraph` member ⇒ a default-graph
/// target (§6.1). Duplicate destinations are harmless — the comparison dataset is
/// a *set* of quads — so no per-row dedup is needed here.
fn graph_targets<'a>(graphs: &[&'a TermMap]) -> Vec<Option<&'a TermMap>> {
    if graphs.is_empty() {
        return vec![None];
    }
    graphs
        .iter()
        .map(|&gm| if is_default_graph(gm) { None } else { Some(gm) })
        .collect()
}

/// Is this graph map the `rr:defaultGraph` constant?
fn is_default_graph(gm: &TermMap) -> bool {
    matches!(gm, TermMap::Constant(Term::NamedNode(n)) if n.as_str() == RR_DEFAULT_GRAPH)
}

/// A mapping term map → a [`TermDef`] at `alias` (constants need no alias). Mirrors
/// `unfold::def_of`: a constant is emitted by reference, everything else is a
/// `Derived` recipe materialised at reconstruction via the single term-gen path.
fn def_of(tm: &TermMap, alias: usize) -> TermDef {
    match tm {
        TermMap::Constant(t) => TermDef::Const(t.clone()),
        other => TermDef::Derived {
            term_map: other.clone(),
            alias,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::ir::{LogicalSource, PredicateObjectMap, SubjectMap, Template, TermSpec};

    fn tmpl(t: &str) -> TermMap {
        TermMap::Template(Template::parse(t).unwrap(), TermSpec::iri())
    }
    fn const_iri(iri: &str) -> TermMap {
        TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked(iri)))
    }

    /// A subject-map graph map names every generated triple's graph: the POM atom
    /// gets a `g` binding (named-graph target), no extra default-graph branch.
    #[test]
    fn subject_graph_map_yields_named_graph_branch() {
        let tm = TriplesMap {
            id: "M1".into(),
            source: LogicalSource::Table("Student".into()),
            subject: SubjectMap {
                term: tmpl("http://ex/s/{ID}"),
                classes: vec![],
                graphs: vec![const_iri("http://ex/g")],
            },
            predicate_object_maps: vec![PredicateObjectMap {
                predicates: vec![const_iri("http://ex/p")],
                objects: vec![ObjectMap::Term(TermMap::Column(
                    "Name".into(),
                    TermSpec::plain_literal(),
                ))],
                graphs: vec![],
            }],
        };
        let branches = build_branches(&[tm]);
        assert_eq!(branches.len(), 1);
        assert!(branches[0].bindings.contains_key(VAR_G), "named-graph target binds g");
    }

    /// No graph maps anywhere ⇒ the default graph: the atom branch omits `g`.
    #[test]
    fn no_graph_maps_yield_default_graph_branch() {
        let tm = TriplesMap {
            id: "M1".into(),
            source: LogicalSource::Table("Student".into()),
            subject: SubjectMap {
                term: tmpl("http://ex/s/{ID}"),
                classes: vec![],
                graphs: vec![],
            },
            predicate_object_maps: vec![PredicateObjectMap {
                predicates: vec![const_iri("http://ex/p")],
                objects: vec![ObjectMap::Term(TermMap::Column(
                    "Name".into(),
                    TermSpec::plain_literal(),
                ))],
                graphs: vec![],
            }],
        };
        let branches = build_branches(&[tm]);
        assert_eq!(branches.len(), 1);
        assert!(!branches[0].bindings.contains_key(VAR_G), "default-graph target omits g");
    }

    /// Subject graph ∪ POM graph (distinct) ⇒ two branches for the one atom, so the
    /// triple lands in *both* graphs (the D009b `practises` shape).
    #[test]
    fn subject_and_pom_graphs_union_to_two_branches() {
        let tm = TriplesMap {
            id: "M1".into(),
            source: LogicalSource::Table("Student".into()),
            subject: SubjectMap {
                term: tmpl("http://ex/s/{ID}"),
                classes: vec![],
                graphs: vec![const_iri("http://ex/students")],
            },
            predicate_object_maps: vec![PredicateObjectMap {
                predicates: vec![const_iri("http://ex/practises")],
                objects: vec![ObjectMap::Term(const_iri("http://ex/o"))],
                graphs: vec![const_iri("http://ex/practise")],
            }],
        };
        let branches = build_branches(&[tm]);
        assert_eq!(branches.len(), 2, "one atom into two distinct graphs ⇒ two branches");
        assert!(branches.iter().all(|b| b.bindings.contains_key(VAR_G)));
    }

    /// `rr:class` emits an `rdf:type` atom into the subject map's graph.
    #[test]
    fn class_atom_uses_subject_graph() {
        let tm = TriplesMap {
            id: "M1".into(),
            source: LogicalSource::Table("Student".into()),
            subject: SubjectMap {
                term: tmpl("http://ex/s/{ID}"),
                classes: vec![NamedNode::new_unchecked("http://ex/Student")],
                graphs: vec![const_iri("http://ex/g")],
            },
            predicate_object_maps: vec![],
        };
        let branches = build_branches(&[tm]);
        assert_eq!(branches.len(), 1);
        let b = &branches[0];
        assert!(b.bindings.contains_key(VAR_G));
        assert!(matches!(b.bindings.get(VAR_P), Some(TermDef::Const(Term::NamedNode(n))) if n.as_str() == RDF_TYPE));
    }
}
