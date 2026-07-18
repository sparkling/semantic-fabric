//! ADR-0031 differential: the RDF-star quoted-triple-pattern query rewrite
//! (`sf_sparql::star`). Mirrors `differential_paths.rs`'s structure
//! (engine-vs-expected helpers + a fixture-per-shape layout) and reuses
//! `differential_tree.rs`'s tree/flat parity idea (both translators must agree
//! on row-bag AND on the 501 set) via this file's own `diff()`/`assert_locked_501`
//! helpers — a separate integration-test binary cannot import another one's
//! private items, so the pattern is replicated rather than shared code.
//! Deliberately NOT `differential_tree.rs` itself, which a parallel work stream
//! is appending to (ADR-0023 M8: the tree path, [`translate_with`], is the
//! production default; the flat path, [`translate_with_flat`], is the `=_bag`
//! oracle/fallback — both routed here through the SAME `star::rewrite_query`
//! pre-pass, so this harness proves they stay byte-identical on every case).
//!
//! **Oracle strategy** (ADR-0031 §Test plan): `spareval` evaluates SPARQL-star
//! *natively* (reifies + triple terms), so it cannot evaluate the *original*
//! query over a graph materializing the basic encoding — that graph has no
//! `rdf:reifies`/triple-term statements for spareval to match. Every case below
//! therefore hand-asserts expected bindings (never spareval) and relies on the
//! tree/flat differential for cross-engine agreement. Bindings that carry a
//! `rr:column`-sourced literal (`?age`) are cross-checked against a baseline
//! non-star query run through the SAME engine ([`baseline_ages`]) rather than
//! hand-encoding the exact XSD lexical form R2RML's natural-type mapping
//! produces — the CORRELATION (which row's `?age` matches which `?p`) is what
//! this suite verifies, not `rr:column`'s datatype inference (covered
//! elsewhere).

use rusqlite::Connection;
use sf_conformance::oracle;
use sf_conformance::sqlite;
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, PlanForm, Tbox};
use sf_sql::Dialect;
use spargebra::SparqlParser;
use std::collections::BTreeMap;

use oxrdf::{NamedNode, Term};

// ============================================================================
// Fixture — `census_row` (mirrors `sf-mapping`'s `STAR_ASSERTED_FIXTURE` shape:
// a `#PersonAge` quoted triples map keyed by `person_id`/`age`), extended with
// `friend_id` for the property-path-endpoint case and an object-position
// StarMap (`#Quote`) reusing the SAME quoted map.
// ============================================================================

const CENSUS_SQL: &str = r#"
CREATE TABLE census_row (
    person_id INTEGER PRIMARY KEY,
    age INTEGER NOT NULL,
    friend_id INTEGER
);
INSERT INTO census_row VALUES (1, 30, 2);
INSERT INTO census_row VALUES (2, 40, NULL);
INSERT INTO census_row VALUES (3, 30, 1);
"#;

const CENSUS_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#PersonAgeAssertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#PersonAge> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:CensusRecord2026 ]
    ] .

<#Knows>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:knows ;
        rr:objectMap [ rr:template "http://ex.org/person/{friend_id}" ]
    ] .
"#;

/// Object-position StarMap fixture (test 3 only) — kept SEPARATE from
/// `CENSUS_R2RML` deliberately: the ADR-0029 synthetic-id template is a pure
/// function of the QUOTED triple's own shape (predicate + column names), not
/// of which outer map references it, so a subject-position StarMap and an
/// object-position StarMap that quote the SAME `(ex:hasAge, {person_id}, age)`
/// shape compile to the IDENTICAL template — correct per ADR-0029 (quoting the
/// identical statement twice legitimately denotes the same proposition-form
/// identity), but it turns every one of this suite's 4-basic-encoding-pattern
/// queries into a 2-carrier bag-multiplicity fixture (2^4 = 16x per row) if
/// `#PersonAgeAssertion` and `#Quote` coexist in one `parse_r2rml` call. That
/// multiplicity is real engine behavior (the same "overlapping triples-maps"
/// class `differential_tree.rs` deliberately stress-tests elsewhere), just not
/// what THIS suite's object-position case is testing, so the two StarMaps are
/// split across fixtures instead.
const CENSUS_R2RML_OBJECT: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#Quote>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/quote/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasQuote ;
        rr:objectMap [
            rml:starMap [ rml:quotedTriplesMap <#PersonAge> ]
        ]
    ] .
"#;

/// Same shape, but `#PersonAge` is ALSO marked `rml:nonAssertedTriplesMap` on
/// its one StarMap reference (test 5) — it must be suppressed as an
/// independently-matchable plain triples map, while the star pattern (which
/// goes through `#PersonAgeAssertion`'s injected basic encoding, not the
/// suppressed plain map) still matches.
const CENSUS_R2RML_NON_ASSERTED: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#PersonAgeAssertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [
        rml:starMap [
            rml:quotedTriplesMap <#PersonAge> ;
            rml:nonAssertedTriplesMap <#PersonAge>
        ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:CensusRecord2026 ]
    ] .
"#;

const EX: &str = "PREFIX ex: <http://example.com/> ";

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s))
}

fn row3(k1: &str, v1: Term, k2: &str, v2: Term, k3: &str, v3: Term) -> BTreeMap<String, Term> {
    BTreeMap::from([
        (k1.to_owned(), v1),
        (k2.to_owned(), v2),
        (k3.to_owned(), v3),
    ])
}

/// A plan's executed answer, comparable as a bag (SELECT only — this fixture
/// has no ASK/CONSTRUCT case).
fn run_select(plan: &Plan, conn: &Connection) -> Vec<BTreeMap<String, Term>> {
    let PlanForm::Select { .. } = &plan.form else {
        panic!(
            "differential_star fixtures are SELECT-only, got {:?}",
            plan.form
        );
    };
    oracle::engine_bag(&exec::select(plan, conn).expect("select exec"))
}

/// The tree/flat differential over one fixture + query (mirrors
/// `differential_tree::diff`, replicated here — see the module doc for why):
/// both translators must either both succeed with the SAME row bag, or both
/// return `Unsupported`. Returns the tree engine's rows (empty on a shared
/// 501) for the caller's hand-computed-expectation assertion.
fn diff(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");

    let flat = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    let tree = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);

    match (&flat, &tree) {
        (Ok(fp), Ok(tp)) => {
            let fa = run_select(fp, &conn);
            let ta = run_select(tp, &conn);
            assert!(
                oracle::solutions_bag_eq(&fa, &ta),
                "flat vs tree row-bag divergence on `{query}`:\n flat={fa:#?}\n tree={ta:#?}"
            );
            ta
        }
        (Err(Error::Unsupported(_)), Err(Error::Unsupported(_))) => Vec::new(),
        _ => panic!(
            "501-set mismatch on `{query}` (flat and tree must agree on Unsupported):\n \
             flat={flat:?}\n tree={tree:?}"
        ),
    }
}

/// A locked v1 boundary (rules 5/7/8, or a pre-existing shared restriction the
/// rewrite's output structurally reaches — see `star_pattern_at_property_path_
/// endpoint_is_a_locked_501` below): both engines must 501 identically.
fn assert_locked_501(r2rml: &str, query: &str) {
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let flat = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    let tree = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    assert!(
        matches!(flat, Err(Error::Unsupported(_))),
        "expected 501 on the flat path for `{query}`, got {flat:?}"
    );
    assert!(
        matches!(tree, Err(Error::Unsupported(_))),
        "expected 501 on the tree path for `{query}`, got {tree:?}"
    );
}

/// The baseline (non-star) `?p ex:hasAge ?age` bindings, keyed by person IRI —
/// run through the SAME engine so star-pattern assertions don't need to
/// hand-encode `rr:column`'s exact XSD literal form; only the row
/// set/correlation the REWRITE must reproduce is hand-computed (see module doc).
fn baseline_ages(create: &str, r2rml: &str) -> BTreeMap<NamedNode, Term> {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(&format!("{EX}SELECT ?p ?age WHERE {{ ?p ex:hasAge ?age }}"))
        .expect("query parses");
    let plan = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("baseline plain pattern must translate");
    run_select(&plan, &conn)
        .into_iter()
        .map(|mut r| {
            let p = match r.remove("p").expect("?p bound") {
                Term::NamedNode(n) => n,
                other => panic!("?p must be an IRI, got {other:?}"),
            };
            let age = r.remove("age").expect("?age bound");
            (p, age)
        })
        .collect()
}

/// The 3 `(?p, ?age, ?src)` rows every ADR-0031-rewritten reifies-elision
/// query must return — one per `census_row` row, `?src` the fixed
/// `ex:assertedBy` constant.
fn expected_asserted_by_rows(ages: &BTreeMap<NamedNode, Term>) -> Vec<BTreeMap<String, Term>> {
    ages.iter()
        .map(|(p, age)| {
            row3(
                "p",
                Term::NamedNode(p.clone()),
                "age",
                age.clone(),
                "src",
                iri("http://example.com/CensusRecord2026"),
            )
        })
        .collect()
}

// ============================================================================
// 1/2 — bare vs parenthesized subject-position syntax must yield identical
// bindings (bare syntax's reifies elision, rule 2, is the load-bearing case).
// ============================================================================

#[test]
fn bare_syntax_reifies_elision_matches_hand_computed_bindings() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<?p ex:hasAge ?age>> ex:assertedBy ?src }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    let expected = expected_asserted_by_rows(&baseline_ages(CENSUS_SQL, CENSUS_R2RML));
    assert!(
        oracle::solutions_bag_eq(&got, &expected),
        "got={got:#?}\nexpected={expected:#?}"
    );
    assert_eq!(got.len(), 3);
}

#[test]
fn parenthesized_subject_position_matches_bare_syntax_bindings() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<( ?p ex:hasAge ?age )>> ex:assertedBy ?src }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    let expected = expected_asserted_by_rows(&baseline_ages(CENSUS_SQL, CENSUS_R2RML));
    assert!(
        oracle::solutions_bag_eq(&got, &expected),
        "got={got:#?}\nexpected={expected:#?}"
    );
}

// ============================================================================
// 3 — object position (a non-reifies predicate quoting a triple as its
// object), via `#Quote`'s object-position StarMap.
// ============================================================================

#[test]
fn object_position_star_pattern_matches_hand_computed_bindings() {
    let query =
        format!("{EX}SELECT ?q ?p ?age WHERE {{ ?q ex:hasQuote <<( ?p ex:hasAge ?age )>> }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML_OBJECT, &query);
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML_OBJECT);
    let expected: Vec<_> = ages
        .iter()
        .map(|(p, age)| {
            let id = p.as_str().rsplit('/').next().unwrap();
            row3(
                "q",
                iri(&format!("http://ex.org/quote/{id}")),
                "p",
                Term::NamedNode(p.clone()),
                "age",
                age.clone(),
            )
        })
        .collect();
    assert!(
        oracle::solutions_bag_eq(&got, &expected),
        "got={got:#?}\nexpected={expected:#?}"
    );
    assert_eq!(got.len(), 3);
}

// ============================================================================
// 4 — explicit reifier variable projection: binds the synthetic
// `urn:sf-star:...` IRI, deterministically across independent runs.
// ============================================================================

#[test]
fn explicit_reifier_variable_binds_synthetic_iri_deterministically() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r ?p ?age WHERE {{ ?r rdf:reifies <<( ?p ex:hasAge ?age )>> }}"
    );
    let run1 = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    let run2 = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(run1.len(), 3);
    assert!(
        oracle::solutions_bag_eq(&run1, &run2),
        "same row must yield the same synthetic id across two independent runs:\n \
         run1={run1:#?}\n run2={run2:#?}"
    );
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    for row in &run1 {
        let r = match &row["r"] {
            Term::NamedNode(n) => n,
            other => panic!("?r must bind an IRI, got {other:?}"),
        };
        assert!(
            r.as_str().starts_with("urn:sf-star:"),
            "?r must bind the synthetic proposition-form IRI (ADR-0031 R5): {r}"
        );
        let p = match &row["p"] {
            Term::NamedNode(n) => n.clone(),
            other => panic!("?p must be an IRI, got {other:?}"),
        };
        assert_eq!(
            row.get("age"),
            ages.get(&p),
            "?age must match the baseline age for ?p"
        );
    }
}

// ============================================================================
// 5 — a `nonAssertedTriplesMap` quoted map is invisible to a PLAIN pattern but
// still reachable through the star pattern (which never queries the
// suppressed plain map — it goes through the injected basic encoding).
// ============================================================================

#[test]
fn non_asserted_triples_map_hides_plain_pattern_but_star_pattern_still_matches() {
    let plain = format!("{EX}SELECT ?p ?age WHERE {{ ?p ex:hasAge ?age }}");
    let starred =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<?p ex:hasAge ?age>> ex:assertedBy ?src }}");
    let plain_rows = diff(CENSUS_SQL, CENSUS_R2RML_NON_ASSERTED, &plain);
    assert!(
        plain_rows.is_empty(),
        "the non-asserted quoted map must not be independently matchable: {plain_rows:#?}"
    );
    let starred_rows = diff(CENSUS_SQL, CENSUS_R2RML_NON_ASSERTED, &starred);
    assert_eq!(
        starred_rows.len(),
        3,
        "the star pattern must still match via the asserted wrapper: {starred_rows:#?}"
    );
}

// ============================================================================
// 6/7/8 — locked v1 boundaries (ADR-0031 rules 4/5/7/8): both engines 501
// identically, unchanged by this rewrite.
// ============================================================================

#[test]
fn construct_template_quoting_a_triple_is_a_locked_501() {
    // Rule 9's OTHER v1 boundary (not the WHERE-pattern rewrite): today, pre-
    // ADR-0031, `exec_core::instantiate`'s `TermPattern → Term` closure silently
    // returns `None` for `Triple` (its wildcard arm) — that SILENTLY DROPS the
    // templated triple from CONSTRUCT output rather than erring. The translate-time
    // check added in `lib.rs` (both engines) turns that into an honest 501 instead.
    let query = format!(
        "{EX}CONSTRUCT {{ <<( ?p ex:hasAge ?age )>> ex:assertedBy ?src }} \
         WHERE {{ ?p ex:hasAge ?age . BIND(ex:CensusRecord2026 AS ?src) }}"
    );
    assert_locked_501(CENSUS_R2RML, &query);
}

#[test]
fn nested_quoted_triple_pattern_is_a_locked_501() {
    let query = format!(
        "{EX}SELECT * WHERE {{ <<( <<( ?a ex:hasAge ?b )>> ex:assertedBy ?c )>> ex:assertedBy ?d }}"
    );
    assert_locked_501(CENSUS_R2RML, &query);
}

#[test]
fn values_ground_quoted_triple_stays_a_locked_501() {
    // Pre-existing (`unfold::ground_term_to_term`'s wildcard) — rule 7: Values
    // is untouched by this rewrite, so this behavior is unchanged by ADR-0031.
    let query = format!("{EX}SELECT ?t WHERE {{ VALUES ?t {{ <<( ex:a ex:hasAge ex:b )>> }} }}");
    assert_locked_501(CENSUS_R2RML, &query);
}

#[test]
fn is_triple_function_stays_a_locked_501() {
    // Pre-existing (`unify::filter_cond`'s `str_match` wildcard, shared by both
    // engines via `iq/lower.rs`'s reuse of the SAME function) — rule 8: v1
    // never fabricates a native triple term, so `isTRIPLE`/`TRIPLE`/`SUBJECT`/
    // `PREDICATE`/`OBJECT` are untouched by this rewrite.
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:hasAge ?age . FILTER isTRIPLE(?age) }}");
    assert_locked_501(CENSUS_R2RML, &query);
}

// ============================================================================
// 9 — a star pattern inside FILTER EXISTS (rule 6: the walker recurses into
// `Expression::Exists` bodies).
// ============================================================================

#[test]
fn star_pattern_inside_filter_exists_matches_hand_computed_bindings() {
    let query = format!(
        "{EX}SELECT ?p WHERE {{ \
           ?p ex:hasAge ?age . \
           FILTER EXISTS {{ <<?p ex:hasAge ?age>> ex:assertedBy ?src }} \
         }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    // Every row is asserted (CENSUS_R2RML), so EXISTS holds for every ?p.
    let expected: Vec<_> = ages
        .keys()
        .map(|p| BTreeMap::from([("p".to_owned(), Term::NamedNode(p.clone()))]))
        .collect();
    assert!(
        oracle::solutions_bag_eq(&got, &expected),
        "got={got:#?}\nexpected={expected:#?}"
    );
    assert_eq!(got.len(), 3);
}

// ============================================================================
// 10 — a star pattern at a property-path endpoint (rule 6b: the identity's 4
// patterns joined alongside the Path node). This structurally reaches a
// PRE-EXISTING, unrelated v1 boundary: `unfold::merge` (shared by both engines
// — `iq/lower.rs`'s `InnerJoin` arm calls the SAME `join_branches`) already
// refuses to join ANY `Branch` carrying a property-path closure with anything
// else ("v1 = a standalone ?s P+ ?o"). The rewrite's Join-wrapped-Bgp+Path
// injection is architecturally correct per the ADR (mirrors the DESCRIBE→CBD
// precedent) and forward-compatible if that restriction is ever lifted — but
// TODAY it necessarily lands on that boundary for EVERY star-at-path-endpoint
// query, so this is a locked 501, not a bindings differential. Lifting the
// join-with-path restriction is a separate, substantial path-composition
// feature, out of scope for ADR-0031 (which is the query REWRITE, not a
// change to `unfold.rs`'s join semantics) — reported to the team lead.
// ============================================================================

#[test]
fn star_pattern_at_property_path_endpoint_is_a_locked_501() {
    let query = format!("{EX}SELECT ?age ?x WHERE {{ <<( ?p ex:hasAge ?age )>> ex:knows+ ?x }}");
    assert_locked_501(CENSUS_R2RML, &query);
}
