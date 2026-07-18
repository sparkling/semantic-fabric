//! ADR-0032 D2/R6 — the conformance-side **decoder**: turns an `rml:StarMap`
//! materialization (the SQL-backed "basic encoding" — synthetic
//! `rdf:PropositionForm` nodes, ADR-0029 §B.2 / ADR-0032 D1) back into the
//! NATIVE RDF 1.2 graph it stands for (triple terms in object position,
//! `rdf:reifies` reifiers), per the RDF 1.2 Interoperability note's basic
//! encoding. This is the second half of D0's equivalence frame: the engine's
//! own SQL-rewritten answer over the ENCODING must equal `spareval`'s answer
//! over this DECODED graph running the ORIGINAL (un-rewritten) query.
//!
//! Decode conditions mirror the interop note's own wording: a
//! `rdf:PropositionForm` node must carry EXACTLY ONE of
//! `rdf:propositionFormSubject` / `...Predicate` / `...Object` each — a
//! missing or duplicated component is an error ("report an error if it can
//! not unambiguously determine s, p, or o"), never silently ignored or
//! guessed at. `propositionFormObject` nesting (object-side only — ADR-0032
//! D1 item 5; subject-side nesting is a load-time mapping error upstream and
//! structurally impossible in a `Quad`, whose `subject` field is
//! `NamedOrBlankNode`, never `Term`) resolves bottom-up, recursively, with
//! defensive cycle detection: D1 emission is acyclic by construction, but
//! this decoder does not trust that invariant blindly. A `PropositionForm`
//! node surfacing in SUBJECT position of a non-description triple, or in
//! PREDICATE/GRAPH position anywhere, is a real finding (should never occur
//! under D1 emission) and is reported as an error, never silently dropped.
//!
//! The vocabulary constants below MUST match `sf-mapping/src/r2rml.rs`'s and
//! `sf-sparql/src/star.rs`'s own copies of the same names exactly — a third
//! crate, so not shared by import (the same hand-declared-per-crate
//! convention those two already use).

use std::collections::{HashMap, HashSet};

use oxrdf::{Dataset, GraphName, NamedNode, NamedOrBlankNode, Quad, Term, Triple};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_PROPOSITION_FORM: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#PropositionForm";
const RDF_PROPOSITION_FORM_SUBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormSubject";
const RDF_PROPOSITION_FORM_PREDICATE: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormPredicate";
const RDF_PROPOSITION_FORM_OBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormObject";

/// One `PropositionForm` node's raw (not-yet-recursively-resolved) shape —
/// the 3 component values read directly off its own description quads.
struct RawShape {
    s: Term,
    p: Term,
    o: Term,
}

/// Decode every `rdf:PropositionForm` node in `dataset` into a native
/// `Term::Triple`, substituting it wherever it appears in OBJECT position
/// (ADR-0032 D2) — including as the object of an `rdf:reifies` triple (which
/// thereby becomes a genuine native reification) and as the direct object of
/// any other predicate (a genuine native triple-term object). Returns `Err`
/// on any decode ambiguity or spec-impossible position described in the
/// module doc — never a silent drop or guess.
pub fn decode_proposition_forms(dataset: &Dataset) -> Result<Dataset, String> {
    let quads: Vec<Quad> = dataset.iter().map(oxrdf::QuadRef::into_owned).collect();

    let mut by_subject: HashMap<NamedOrBlankNode, Vec<usize>> = HashMap::new();
    for (i, q) in quads.iter().enumerate() {
        by_subject.entry(q.subject.clone()).or_default().push(i);
    }

    // Pass 1a: identify every PropositionForm node (anything typed
    // `rdf:PropositionForm`, in any graph).
    let prop_form_class = Term::NamedNode(NamedNode::new_unchecked(RDF_PROPOSITION_FORM));
    let mut prop_nodes: HashSet<NamedOrBlankNode> = HashSet::new();
    for q in &quads {
        if q.predicate.as_str() == RDF_TYPE && q.object == prop_form_class {
            prop_nodes.insert(q.subject.clone());
        }
    }

    // Pass 1b: for each, extract its exactly-one s/p/o component, rejecting a
    // missing/duplicated component or a predicate beyond the 4 recognized
    // description predicates (the "stray subject-position use" finding).
    let mut raw: HashMap<NamedOrBlankNode, RawShape> = HashMap::new();
    for n in &prop_nodes {
        let mut s_vals = Vec::new();
        let mut p_vals = Vec::new();
        let mut o_vals = Vec::new();
        let mut extraneous = Vec::new();
        for &i in by_subject.get(n).map(Vec::as_slice).unwrap_or(&[]) {
            let q = &quads[i];
            match q.predicate.as_str() {
                RDF_TYPE => {}
                RDF_PROPOSITION_FORM_SUBJECT => s_vals.push(q.object.clone()),
                RDF_PROPOSITION_FORM_PREDICATE => p_vals.push(q.object.clone()),
                RDF_PROPOSITION_FORM_OBJECT => o_vals.push(q.object.clone()),
                other => extraneous.push(other.to_owned()),
            }
        }
        if !extraneous.is_empty() {
            return Err(format!(
                "PropositionForm node {n} carries {} additional predicate(s) beyond the 4 \
                 basic-encoding description predicates ({extraneous:?}) — a PropositionForm \
                 node must appear in SUBJECT position only in its own description quads \
                 (ADR-0032 D2)",
                extraneous.len()
            ));
        }
        raw.insert(
            n.clone(),
            RawShape {
                s: one_component(n, "rdf:propositionFormSubject", s_vals)?,
                p: one_component(n, "rdf:propositionFormPredicate", p_vals)?,
                o: one_component(n, "rdf:propositionFormObject", o_vals)?,
            },
        );
    }

    // Pass 1c: build each node's Term::Triple bottom-up (recursing only
    // through propositionFormObject nesting), memoized, with cycle detection.
    let mut built: HashMap<NamedOrBlankNode, Term> = HashMap::new();
    let mut visiting: HashSet<NamedOrBlankNode> = HashSet::new();
    let nodes: Vec<NamedOrBlankNode> = raw.keys().cloned().collect();
    for n in &nodes {
        build_triple_term(n, &raw, &mut built, &mut visiting)?;
    }

    // Pass 2: rewrite the dataset — drop the description quads, substitute a
    // PropositionForm object with its built triple term, and reject a
    // PropositionForm node surfacing in predicate/graph position.
    let mut out = Vec::with_capacity(quads.len());
    for q in &quads {
        if raw.contains_key(&q.subject) {
            continue; // one of the validated description quads — dropped
        }
        if raw.contains_key(&NamedOrBlankNode::NamedNode(q.predicate.clone())) {
            return Err(format!(
                "PropositionForm node {} appears in PREDICATE position ({} {} {}) — \
                 spec-impossible under D1 emission (RDF 1.2 Concepts §3.1: only object \
                 position may hold a triple term)",
                q.predicate, q.subject, q.predicate, q.object
            ));
        }
        if let Some(gn) = graph_name_node(&q.graph_name) {
            if raw.contains_key(&gn) {
                return Err(format!(
                    "PropositionForm node {gn} appears in GRAPH position — spec-impossible \
                     under D1 emission"
                ));
            }
        }
        let object = match node_of(&q.object) {
            Some(n) if raw.contains_key(&n) => built
                .get(&n)
                .cloned()
                .expect("every PropositionForm node was built in pass 1c"),
            _ => q.object.clone(),
        };
        out.push(Quad::new(
            q.subject.clone(),
            q.predicate.clone(),
            object,
            q.graph_name.clone(),
        ));
    }
    Ok(Dataset::from_iter(out))
}

/// Validate exactly-one cardinality for one description component (the
/// interop note's own decode condition: "report an error if it can not
/// unambiguously determine s, p, or o").
fn one_component(n: &NamedOrBlankNode, label: &str, mut vals: Vec<Term>) -> Result<Term, String> {
    match vals.len() {
        1 => Ok(vals.remove(0)),
        0 => Err(format!(
            "PropositionForm node {n} is missing its {label} component (interop note decode \
             condition: unambiguous s/p/o required)"
        )),
        found => Err(format!(
            "PropositionForm node {n} has {found} {label} components, expected exactly one \
             (interop note decode condition: unambiguous s/p/o required)"
        )),
    }
}

/// Recursively build `n`'s `Term::Triple`, memoized in `built`. Only the
/// OBJECT component ever recurses (propositionFormObject nesting, ADR-0032 D1
/// item 5) — subject/predicate are used as plain terms, matching the
/// mapping-side emission, which rejects subject-side nesting at load time and
/// only ever puts a constant IRI in predicate position.
fn build_triple_term(
    n: &NamedOrBlankNode,
    raw: &HashMap<NamedOrBlankNode, RawShape>,
    built: &mut HashMap<NamedOrBlankNode, Term>,
    visiting: &mut HashSet<NamedOrBlankNode>,
) -> Result<Term, String> {
    if let Some(t) = built.get(n) {
        return Ok(t.clone());
    }
    if !visiting.insert(n.clone()) {
        return Err(format!(
            "cycle detected while decoding PropositionForm node {n} (propositionFormObject \
             nesting must be acyclic — ADR-0032 D1 emission is acyclic by construction; an \
             actual cycle is a real finding, not a legitimate shape)"
        ));
    }
    let shape = raw.get(n).expect("n is a key of raw by construction");
    let subject = match &shape.s {
        Term::NamedNode(nn) => NamedOrBlankNode::NamedNode(nn.clone()),
        Term::BlankNode(bn) => NamedOrBlankNode::BlankNode(bn.clone()),
        other => {
            return Err(format!(
                "PropositionForm node {n}'s propositionFormSubject is not an IRI or blank \
                 node ({other}) — cannot build a well-formed triple"
            ))
        }
    };
    let predicate = match &shape.p {
        Term::NamedNode(nn) => nn.clone(),
        other => {
            return Err(format!(
                "PropositionForm node {n}'s propositionFormPredicate is not an IRI ({other}) \
                 — cannot build a well-formed triple"
            ))
        }
    };
    let object = match node_of(&shape.o) {
        Some(inner) if raw.contains_key(&inner) => build_triple_term(&inner, raw, built, visiting)?,
        _ => shape.o.clone(),
    };
    visiting.remove(n);
    let triple = Term::Triple(Box::new(Triple::new(subject, predicate, object)));
    built.insert(n.clone(), triple.clone());
    Ok(triple)
}

/// `term` as a [`NamedOrBlankNode`] key, if it is one — a literal or an
/// already-native triple term never identifies a PropositionForm node (those
/// ids are always IRIs, or in principle blank nodes, at the R2RML term-map
/// level, never literals or triple terms).
fn node_of(term: &Term) -> Option<NamedOrBlankNode> {
    match term {
        Term::NamedNode(nn) => Some(NamedOrBlankNode::NamedNode(nn.clone())),
        Term::BlankNode(bn) => Some(NamedOrBlankNode::BlankNode(bn.clone())),
        _ => None,
    }
}

/// `graph_name` as a [`NamedOrBlankNode`] key, if it names a graph at all
/// (the default graph never does).
fn graph_name_node(graph_name: &GraphName) -> Option<NamedOrBlankNode> {
    match graph_name {
        GraphName::NamedNode(nn) => Some(NamedOrBlankNode::NamedNode(nn.clone())),
        GraphName::BlankNode(bn) => Some(NamedOrBlankNode::BlankNode(bn.clone())),
        GraphName::DefaultGraph => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::Literal;

    const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";
    const EX_S: &str = "http://ex/s";
    const EX_P: &str = "http://ex/p";
    const EX_R: &str = "http://ex/r";

    fn nn(s: &str) -> NamedNode {
        NamedNode::new_unchecked(s)
    }

    fn quad(s: &str, p: &str, o: Term) -> Quad {
        Quad::new(nn(s), nn(p), o, GraphName::DefaultGraph)
    }

    /// One PropositionForm node's 4 description quads (`rdf:type` + the 3
    /// components), keyed at `pf_iri`.
    fn description(pf_iri: &str, s: Term, p: Term, o: Term) -> Vec<Quad> {
        vec![
            quad(pf_iri, RDF_TYPE, prop_form_term()),
            quad(pf_iri, RDF_PROPOSITION_FORM_SUBJECT, s),
            quad(pf_iri, RDF_PROPOSITION_FORM_PREDICATE, p),
            quad(pf_iri, RDF_PROPOSITION_FORM_OBJECT, o),
        ]
    }

    fn prop_form_term() -> Term {
        Term::NamedNode(nn(RDF_PROPOSITION_FORM))
    }

    fn iri(s: &str) -> Term {
        Term::NamedNode(nn(s))
    }

    // ------------------------------------------------------------------
    // Simple: one reifier reifying one non-nested proposition.
    // ------------------------------------------------------------------

    #[test]
    fn decodes_simple_subject_position_reification() {
        let mut quads = description("urn:pf1", iri(EX_S), iri(EX_P), Literal::from(42).into());
        quads.push(quad(EX_R, RDF_REIFIES, iri("urn:pf1")));
        let encoded = Dataset::from_iter(quads);

        let decoded = decode_proposition_forms(&encoded).expect("decode succeeds");

        let expected_triple = Term::Triple(Box::new(Triple::new(
            nn(EX_S),
            nn(EX_P),
            Term::Literal(Literal::from(42)),
        )));
        let expected = Dataset::from_iter([quad(EX_R, RDF_REIFIES, expected_triple)]);
        assert_eq!(decoded, expected, "decoded={decoded:?}");
    }

    #[test]
    fn decodes_object_position_direct_quote() {
        // No reifier at all — the PropositionForm id is the direct object of
        // an ordinary predicate (ADR-0029/0032 object-position shape).
        let mut quads = description("urn:pf1", iri(EX_S), iri(EX_P), iri("urn:o"));
        quads.push(quad("urn:q", "http://ex/hasQuote", iri("urn:pf1")));
        let encoded = Dataset::from_iter(quads);

        let decoded = decode_proposition_forms(&encoded).expect("decode succeeds");

        let expected_triple = Term::Triple(Box::new(Triple::new(nn(EX_S), nn(EX_P), iri("urn:o"))));
        let expected = Dataset::from_iter([quad("urn:q", "http://ex/hasQuote", expected_triple)]);
        assert_eq!(decoded, expected, "decoded={decoded:?}");
    }

    // ------------------------------------------------------------------
    // Nested depth-2 / depth-3 (propositionFormObject pointing at another
    // PropositionForm node) — resolved bottom-up.
    // ------------------------------------------------------------------

    #[test]
    fn decodes_nested_depth_2() {
        let mut quads = description("urn:mid", iri("urn:s2"), iri("urn:p2"), iri("urn:leafval"));
        quads.extend(description(
            "urn:outer",
            iri("urn:s1"),
            iri("urn:p1"),
            iri("urn:mid"), // nests: outer's object IS the mid PropositionForm node
        ));
        quads.push(quad(EX_R, RDF_REIFIES, iri("urn:outer")));
        let encoded = Dataset::from_iter(quads);

        let decoded = decode_proposition_forms(&encoded).expect("decode succeeds");

        let inner = Triple::new(nn("urn:s2"), nn("urn:p2"), iri("urn:leafval"));
        let outer = Triple::new(nn("urn:s1"), nn("urn:p1"), Term::Triple(Box::new(inner)));
        let expected = Dataset::from_iter([quad(EX_R, RDF_REIFIES, Term::Triple(Box::new(outer)))]);
        assert_eq!(decoded, expected, "decoded={decoded:?}");
    }

    #[test]
    fn decodes_nested_depth_3() {
        let mut quads = description("urn:leaf", iri("urn:s3"), iri("urn:p3"), iri("urn:leafval"));
        quads.extend(description(
            "urn:mid",
            iri("urn:s2"),
            iri("urn:p2"),
            iri("urn:leaf"),
        ));
        quads.extend(description(
            "urn:outer",
            iri("urn:s1"),
            iri("urn:p1"),
            iri("urn:mid"),
        ));
        quads.push(quad(EX_R, RDF_REIFIES, iri("urn:outer")));
        let encoded = Dataset::from_iter(quads);

        let decoded = decode_proposition_forms(&encoded).expect("decode succeeds");

        let leaf = Triple::new(nn("urn:s3"), nn("urn:p3"), iri("urn:leafval"));
        let mid = Triple::new(nn("urn:s2"), nn("urn:p2"), Term::Triple(Box::new(leaf)));
        let outer = Triple::new(nn("urn:s1"), nn("urn:p1"), Term::Triple(Box::new(mid)));
        let expected = Dataset::from_iter([quad(EX_R, RDF_REIFIES, Term::Triple(Box::new(outer)))]);
        assert_eq!(decoded, expected, "decoded={decoded:?}");
    }

    // ------------------------------------------------------------------
    // Multi-reifier: two DISTINCT reifiers reifying the SAME (deduplicated)
    // proposition node — both must decode to the SAME Term::Triple value.
    // ------------------------------------------------------------------

    #[test]
    fn decodes_multi_reifier_sharing_one_proposition() {
        let mut quads = description("urn:pf1", iri(EX_S), iri(EX_P), iri("urn:o"));
        quads.push(quad("urn:r1", RDF_REIFIES, iri("urn:pf1")));
        quads.push(quad("urn:r1", "http://ex/assertedBy", iri("urn:srcA")));
        quads.push(quad("urn:r2", RDF_REIFIES, iri("urn:pf1")));
        quads.push(quad("urn:r2", "http://ex/assertedBy", iri("urn:srcB")));
        let encoded = Dataset::from_iter(quads);

        let decoded = decode_proposition_forms(&encoded).expect("decode succeeds");

        let tt = Term::Triple(Box::new(Triple::new(nn(EX_S), nn(EX_P), iri("urn:o"))));
        let expected = Dataset::from_iter([
            quad("urn:r1", RDF_REIFIES, tt.clone()),
            quad("urn:r1", "http://ex/assertedBy", iri("urn:srcA")),
            quad("urn:r2", RDF_REIFIES, tt),
            quad("urn:r2", "http://ex/assertedBy", iri("urn:srcB")),
        ]);
        assert_eq!(decoded, expected, "decoded={decoded:?}");
    }

    // ------------------------------------------------------------------
    // Error cases: missing component, duplicated component, stray
    // subject-position use, and a defensively-detected cycle.
    // ------------------------------------------------------------------

    #[test]
    fn errs_on_missing_component() {
        // Only subject + predicate — no propositionFormObject at all.
        let quads = vec![
            quad("urn:pf1", RDF_TYPE, prop_form_term()),
            quad("urn:pf1", RDF_PROPOSITION_FORM_SUBJECT, iri(EX_S)),
            quad("urn:pf1", RDF_PROPOSITION_FORM_PREDICATE, iri(EX_P)),
        ];
        let encoded = Dataset::from_iter(quads);
        let err = decode_proposition_forms(&encoded).expect_err("must reject a missing object");
        assert!(
            err.contains("propositionFormObject") && err.contains("missing"),
            "err={err}"
        );
    }

    #[test]
    fn errs_on_duplicated_component() {
        let mut quads = description("urn:pf1", iri(EX_S), iri(EX_P), iri("urn:o1"));
        quads.push(quad("urn:pf1", RDF_PROPOSITION_FORM_OBJECT, iri("urn:o2")));
        let encoded = Dataset::from_iter(quads);
        let err = decode_proposition_forms(&encoded).expect_err("must reject a duplicated object");
        assert!(
            err.contains("propositionFormObject") && err.contains('2'),
            "err={err}"
        );
    }

    #[test]
    fn errs_on_stray_subject_position_use() {
        // The PropositionForm node ALSO carries an unrelated predicate as
        // subject — a real finding, never silently dropped.
        let mut quads = description("urn:pf1", iri(EX_S), iri(EX_P), iri("urn:o"));
        quads.push(quad(
            "urn:pf1",
            "http://ex/extra",
            Literal::new_simple_literal("oops").into(),
        ));
        let encoded = Dataset::from_iter(quads);
        let err = decode_proposition_forms(&encoded)
            .expect_err("must reject a stray predicate on a PropositionForm node");
        assert!(
            err.contains("additional predicate") && err.contains("http://ex/extra"),
            "err={err}"
        );
    }

    #[test]
    fn errs_on_defensively_detected_cycle() {
        // urn:a's object IS urn:b, and urn:b's object IS urn:a — impossible
        // under real D1 emission (object-position-only nesting is acyclic by
        // construction), but the decoder must not infinitely recurse or
        // panic on a hand-crafted adversarial input.
        let mut quads = description("urn:a", iri(EX_S), iri(EX_P), iri("urn:b"));
        quads.extend(description("urn:b", iri(EX_S), iri(EX_P), iri("urn:a")));
        let encoded = Dataset::from_iter(quads);
        let err = decode_proposition_forms(&encoded).expect_err("must reject a cycle");
        assert!(err.contains("cycle"), "err={err}");
    }

    #[test]
    fn errs_on_proposition_form_node_in_predicate_position() {
        // A PropositionForm node's own IRI reused as a PREDICATE elsewhere —
        // spec-impossible (RDF 1.2 Concepts §3.1), a real finding.
        let mut quads = description("urn:pf1", iri(EX_S), iri(EX_P), iri("urn:o"));
        quads.push(Quad::new(
            nn("urn:x"),
            nn("urn:pf1"),
            iri("urn:y"),
            GraphName::DefaultGraph,
        ));
        let encoded = Dataset::from_iter(quads);
        let err = decode_proposition_forms(&encoded)
            .expect_err("must reject a PropositionForm node used as a predicate");
        assert!(err.contains("PREDICATE position"), "err={err}");
    }
}
