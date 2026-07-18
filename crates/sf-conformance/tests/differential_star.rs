//! ADR-0032 D3 differential: the RDF-star quoted-triple-pattern query rewrite
//! (`sf_sparql::star`), superseding ADR-0031's rules R2/R5 in place. Mirrors
//! `differential_paths.rs`'s structure (engine-vs-expected helpers + a
//! fixture-per-shape layout) and reuses `differential_tree.rs`'s tree/flat
//! parity idea (both translators must agree on row-bag AND on the 501 set)
//! via this file's own `diff()`/`assert_locked_501` helpers — a separate
//! integration-test binary cannot import another one's private items, so the
//! pattern is replicated rather than shared code. Deliberately NOT
//! `differential_tree.rs` itself, which a parallel work stream is appending
//! to (ADR-0023 M8: the tree path, [`translate_with`], is the production
//! default; the flat path, [`translate_with_flat`], is the `=_bag`
//! oracle/fallback — both routed here through the SAME `star::rewrite_query`
//! pre-pass, so this harness proves they stay byte-identical on every case).
//!
//! **Oracle strategy** (ADR-0032 §Test plan): `spareval` evaluates SPARQL-star
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

/// Object-position StarMap fixture (test 3 and the bare-syntax-in-object-
/// position matrix cell) — kept SEPARATE from `CENSUS_R2RML` deliberately:
/// the ADR-0029 synthetic-id template is a pure function of the QUOTED
/// triple's own shape (predicate + column names), not of which outer map
/// references it, so a subject-position StarMap and an object-position
/// StarMap that quote the SAME `(ex:hasAge, {person_id}, age)` shape compile
/// to the IDENTICAL template — correct per ADR-0029 (quoting the identical
/// statement twice legitimately denotes the same proposition-form identity),
/// but it turns every one of this suite's 4-basic-encoding-pattern queries
/// into a 2-carrier bag-multiplicity fixture (2^4 = 16x per row) if
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
/// its one StarMap reference (test 5, and the annotation-sugar
/// plain-triple-requirement matrix cell) — it must be suppressed as an
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

/// TWO subject-position StarMaps (`#AssertionA`, `#AssertionB`) quoting the
/// SAME `#PersonAge` shape — ADR-0032 D1's reifier-multiplicity fixture
/// (mirrors `sf-mapping`'s `STAR_TWO_ASSERTIONS_SAME_SHAPE_FIXTURE`): each
/// declaring map mints its OWN reifier, both `rdf:reifies` the very same
/// (deduplicated) proposition.
const CENSUS_R2RML_TWO_ASSERTIONS: &str = r#"
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

<#AssertionA>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAge> ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:SourceA ]
    ] .

<#AssertionB>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAge> ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:SourceB ]
    ] .
"#;

/// Object-side nesting, depth 2 (ADR-0032 D1 item 5 / D3 rule R4): `#Outer`
/// (subject position) reifies `#Mid`'s shape; `#Mid`'s own OBJECT is a
/// nested StarMap quoting `#Leaf`. Mirrors `sf-mapping`'s
/// `STAR_NESTED_DEPTH2_FIXTURE`, reusing the `age` column as `#Leaf`'s
/// object (a second, unrelated predicate over the same column keeps the
/// fixture small — the VALUE is irrelevant, only shape/correlation matters).
const STAR_NESTED_DEPTH2_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#Leaf>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/leaf/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasScore ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#Mid>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rml:starMap [ rml:quotedTriplesMap <#Leaf> ] ]
    ] .

<#Outer>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#Mid> ] ] ;
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

/// A locked boundary (unrelated to the star rewrite itself, or a still-
/// out-of-scope Wave-2b construct — see each call site's own comment): both
/// engines must 501 identically.
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

/// The 3 `(?p, ?age, ?src)` rows every no-elision-rewritten reifies query
/// must return — one per `census_row` row, `?src` the fixed `ex:assertedBy`
/// constant.
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
// 1/2 — bare reifies-sugar vs parenthesized subject-position TripleTerm MUST
// diverge (ADR-0032 Breaking #2/#3, the matching matrix's core law): bare
// `<<...>>` desugars to a reifies pattern (rule R2, no elision) and matches
// every genuinely reified statement; parenthesized `<<(...)>>` in SUBJECT
// position is a TripleTerm pattern in its own right and can never match
// (SPARQL 1.2 §18.1.3, rule R1) — v1 wrongly conflated the two.
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
fn parenthesized_subject_position_triple_term_is_statically_empty() {
    // SPARQL 1.2 §18.1.3, verbatim: "A triple pattern that has another triple
    // pattern in its subject position will fail to match on any RDF graph."
    // v1 wrongly matched this identically to the bare-syntax case above
    // (ADR-0032 Breaking #3) — legal to write, guaranteed zero solutions,
    // never an error.
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<( ?p ex:hasAge ?age )>> ex:assertedBy ?src }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(got.is_empty(), "got={got:#?}");
}

// ============================================================================
// 3 — object position (a non-reifies predicate quoting a triple as its
// object), via `#Quote`'s object-position StarMap, plus the matching
// matrix's complementary cell: BARE syntax in the SAME object position must
// NOT match (no reifies rows exist for an unreified triple term).
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

#[test]
fn bare_syntax_in_object_position_does_not_match_an_unreified_triple_term() {
    // `?q ex:hasQuote << ?p ex:hasAge ?age >>` — bare syntax desugars (at ANY
    // syntactic position) to `?q ex:hasQuote _:b . _:b rdf:reifies <<(...)>>`.
    // `#Quote` (`CENSUS_R2RML_OBJECT`) is an OBJECT-position StarMap: Wave 1
    // mints NO reifier / `rdf:reifies` triple for it at all (nothing there to
    // reify — `sf-mapping`'s `expand_star_map_object` rejects even an
    // explicit `rml:reifierMap`). So the reifies conjunct can never match,
    // and the WHOLE query is empty — the cell proving bare-sugar no longer
    // over-matches an unreified triple term the way v1 did (ADR-0032
    // Breaking #2). Contrast with `object_position_star_pattern_matches_
    // hand_computed_bindings` immediately above: the SAME fixture DOES match
    // when queried with parenthesized `<<(...)>>` (the TripleTerm cell).
    let query = format!("{EX}SELECT ?q ?p ?age WHERE {{ ?q ex:hasQuote << ?p ex:hasAge ?age >> }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML_OBJECT, &query);
    assert!(got.is_empty(), "got={got:#?}");
}

// ============================================================================
// 4 — explicit reifier variable projection: binds the reifier-family
// `urn:sf-star:r:...` IRI (ADR-0032 D1's role split — the SUBJECT of the
// no-elision reifies pattern, never the proposition id), deterministically
// across independent runs. This is also the matching matrix's "explicit
// `?r rdf:reifies <<(...)>>` over a SUBJECT-position mapping" cell.
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
            r.as_str().starts_with("urn:sf-star:r:"),
            "?r must bind the REIFIER-family synthetic IRI specifically (ADR-0032 D1: \
             `X rdf:reifies TT` binds the reifier at X, never the proposition id): {r}"
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
// 4b — reifier multiplicity (ADR-0032 D1 / Concepts §1.5: "There can be
// multiple, distinct reifiers related to the same abstract proposition"):
// two DIFFERENT star maps quoting the SAME shape must mint two DISTINCT
// reifiers, both reifying the one (deduplicated) proposition.
// ============================================================================

#[test]
fn reifier_multiplicity_two_star_maps_same_shape_yield_distinct_reifiers() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?r ?src WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
           ?r ex:assertedBy ?src \
         }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML_TWO_ASSERTIONS, &query);
    assert_eq!(
        got.len(),
        6,
        "3 people x 2 declaring star maps = 6 rows: got={got:#?}"
    );

    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML_TWO_ASSERTIONS);
    let mut by_person: BTreeMap<NamedNode, Vec<&BTreeMap<String, Term>>> = BTreeMap::new();
    for row in &got {
        let p = match &row["p"] {
            Term::NamedNode(n) => n.clone(),
            other => panic!("?p must be an IRI, got {other:?}"),
        };
        assert_eq!(
            row.get("age"),
            ages.get(&p),
            "?age must match the baseline for ?p"
        );
        by_person.entry(p).or_default().push(row);
    }
    assert_eq!(by_person.len(), 3, "one group per person: {by_person:#?}");

    let src_a = iri("http://example.com/SourceA");
    let src_b = iri("http://example.com/SourceB");
    for (p, rows) in &by_person {
        assert_eq!(
            rows.len(),
            2,
            "person {p} must have exactly 2 reifiers: {rows:#?}"
        );
        assert_ne!(
            rows[0]["r"], rows[1]["r"],
            "the two reifiers sharing person {p}'s proposition must be distinct IRIs"
        );
        let has_a = rows.iter().any(|r| r["src"] == src_a);
        let has_b = rows.iter().any(|r| r["src"] == src_b);
        assert!(
            has_a && has_b,
            "person {p} must have exactly one reifier from EACH declaring map \
             (distinguishable by its OWN assertedBy source): {rows:#?}"
        );
    }
}

// ============================================================================
// 4c — annotation sugar `s p o {| ... |}` (probed directly against
// spargebra 0.4.6+sparql-12): desugars to THREE patterns — the PLAIN triple
// `s p o` (asserted), a fresh blank node reifying that SAME triple, and the
// annotation's own predicate-object pattern(s) on that blank node. Annotation
// sugar therefore both ASSERTS and REIFIES (ADR-0032 D6: "the engine follows
// the parser's algebra" — locked here as the CONFIRMED shape).
// ============================================================================

#[test]
fn annotation_sugar_asserts_and_reifies_matches_same_rows_as_bare_sugar() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ ?p ex:hasAge ?age {{| ex:assertedBy ?src |}} }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    let expected = expected_asserted_by_rows(&baseline_ages(CENSUS_SQL, CENSUS_R2RML));
    assert!(
        oracle::solutions_bag_eq(&got, &expected),
        "got={got:#?}\nexpected={expected:#?}"
    );
    assert_eq!(got.len(), 3);
}

#[test]
fn annotation_sugar_also_requires_the_plain_triple_unlike_bare_sugar() {
    // The SAME annotation query, over the NON-ASSERTED fixture (`#PersonAge`
    // suppressed as an independently-matchable plain triple), must return
    // EMPTY — unlike bare `<<?p ex:hasAge ?age>> ex:assertedBy ?src` (see
    // `non_asserted_triples_map_hides_plain_pattern_but_star_pattern_still_
    // matches`, below), which matches via the reifies chain ALONE. This is
    // the observable proof that annotation sugar's parser-algebra
    // plain-triple conjunct is a REAL, load-bearing extra requirement, not a
    // no-op — the concrete answer to D6's "does `{| |}` also match the inner
    // triple as asserted" open question, for THIS parser.
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ ?p ex:hasAge ?age {{| ex:assertedBy ?src |}} }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML_NON_ASSERTED, &query);
    assert!(got.is_empty(), "got={got:#?}");
}

// ============================================================================
// 4d — explicit-reifier-variable sugar `<< s p o ~ ?r >>` (probed: desugars
// to `?r rdf:reifies <<(s p o)>>` with `?r` substituted directly at the
// reifier position, no fresh blank node) end to end, joined against a second
// pattern on `?r` — the actual SURFACE syntax ADR-0032 D3 names, as opposed
// to `explicit_reifier_variable_binds_synthetic_iri_deterministically`'s
// already-unsugared `?r rdf:reifies <<(...)>>` form.
// ============================================================================

#[test]
fn explicit_reifier_sugar_e2e_matches_same_rows_as_manual_reifies_pattern() {
    let query = format!(
        "{EX}SELECT ?p ?age ?r ?src WHERE {{ << ?p ex:hasAge ?age ~ ?r >> . ?r ex:assertedBy ?src }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(got.len(), 3, "got={got:#?}");
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    for row in &got {
        let r = match &row["r"] {
            Term::NamedNode(n) => n,
            other => panic!("?r must bind an IRI, got {other:?}"),
        };
        assert!(
            r.as_str().starts_with("urn:sf-star:r:"),
            "?r must bind the reifier-family synthetic IRI: {r}"
        );
        let p = match &row["p"] {
            Term::NamedNode(n) => n.clone(),
            other => panic!("?p must be an IRI, got {other:?}"),
        };
        assert_eq!(row.get("age"), ages.get(&p));
        assert_eq!(row["src"], iri("http://example.com/CensusRecord2026"));
    }
}

// ============================================================================
// 4e — object-side nesting, depth 2, end to end (ADR-0032 D3 rule R4):
// `#Outer` reifies `#Mid`'s proposition, whose OWN object is `#Leaf`'s
// nested proposition. The REWRITE itself is verified correct — inspected
// directly (`star::rewrite_query`): `__sf_star_1` (Leaf's identity) carries
// its 4 patterns, `__sf_star_0` (Mid's identity) carries its 4 with
// `propositionFormObject` pointing at `__sf_star_1`, and `?r rdf:reifies
// __sf_star_0` closes the chain — exactly D3's rule R4. But EXECUTING it
// structurally reaches a PRE-EXISTING, unrelated boundary: `desc(#Leaf)` and
// `desc(#Mid)` are TWO DISTINCT quoted shapes' description maps, and EVERY
// description map shares the identical 4-predicate vocabulary (rdf:type,
// propositionForm{Subject,Predicate,Object}) by construction (ADR-0032 D1) —
// so `__sf_star_0`'s own 4-pattern group is candidate-ambiguous per pattern
// (each COULD source from either map) until the CONSTANT-valued
// propositionFormPredicate pattern narrows it. `unify.rs::align_templates`
// explores candidate pairings before that narrowing completes and hits a
// `desc(#Leaf)`-vs-`desc(#Mid)` SUBJECT-template pairing (5 segments vs 8) —
// a pairing that IS provably disjoint (different literal prefixes) but
// `align_templates` bails `Unsupported("template length mismatch")` on the
// length check alone, by conservative design ("a kind/length mismatch is
// conservatively unsupported (never an unsound prune)", `unify.rs` doc
// comment), rather than proving disjointness via the prefix text. This is
// NOT specific to nesting: it reproduces for ANY query touching 2+ DISTINCT
// quoted shapes' description maps in one BGP with a shared variable (nesting
// just guarantees ≥2 distinct shapes by construction) — reported to the
// team lead as a new, out-of-file-scope finding (`unify.rs`/`unfold.rs`
// candidate narrowing, not `star.rs`'s rewrite).
// ============================================================================

#[test]
fn object_side_nesting_depth_2_hits_a_pre_existing_multi_shape_unify_boundary() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r ?p ?leaf ?score WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge <<( ?leaf ex:hasScore ?score )>> )>> \
         }}"
    );
    assert_locked_501(STAR_NESTED_DEPTH2_R2RML, &query);
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
// 6/7/8 — locked boundaries, unrelated to this rewrite or still out of scope
// (Wave-2b territory): both engines 501 identically.
// ============================================================================

#[test]
fn construct_template_quoting_a_triple_is_a_locked_501() {
    // The CONSTRUCT-template guard (not the WHERE-pattern rewrite): today,
    // `exec_core::instantiate`'s `TermPattern → Term` closure silently
    // returns `None` for `Triple` (its wildcard arm) — that SILENTLY DROPS the
    // templated triple from CONSTRUCT output rather than erring. The translate-time
    // check in `lib.rs` (both engines, via `star::construct_template_has_quoted_triple`)
    // turns that into an honest 501 instead. ADR-0032 D2 replaces this with
    // real instantiation + spec-defined dropping of illegal output, but that
    // is Wave-2b's `exec_core` work — untouched here.
    let query = format!(
        "{EX}CONSTRUCT {{ <<( ?p ex:hasAge ?age )>> ex:assertedBy ?src }} \
         WHERE {{ ?p ex:hasAge ?age . BIND(ex:CensusRecord2026 AS ?src) }}"
    );
    assert_locked_501(CENSUS_R2RML, &query);
}

#[test]
fn subject_side_nested_quoted_triple_is_statically_empty() {
    // The OUTER quoted triple's own SUBJECT is ANOTHER quoted triple
    // (`<<( ?a ex:hasAge ?b )>>` sits where the outer quote's `s` belongs) —
    // subject-side nesting, spec-impossible at any depth (RDF 1.2 Concepts
    // §3.1: triple terms are object-position-only). v1 rejected ALL nesting
    // as Unsupported (a scope choice); ADR-0032 D3 rule R1 makes subject-side
    // nesting specifically a statically-empty match, never an error, exactly
    // like the non-nested subject-position case above (Breaking #3).
    // Object-side nesting (rule R4) is instead now SUPPORTED — see
    // `object_side_nesting_depth_2_e2e_matches_hand_computed_bindings` above.
    let query = format!(
        "{EX}SELECT * WHERE {{ <<( <<( ?a ex:hasAge ?b )>> ex:assertedBy ?c )>> ex:assertedBy ?d }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(got.is_empty(), "got={got:#?}");
}

#[test]
fn values_ground_quoted_triple_stays_a_locked_501() {
    // Pre-existing (`unfold::ground_term_to_term`'s wildcard) — rule R6:
    // Values is untouched by this rewrite, so this behavior is unchanged.
    let query = format!("{EX}SELECT ?t WHERE {{ VALUES ?t {{ <<( ex:a ex:hasAge ex:b )>> }} }}");
    assert_locked_501(CENSUS_R2RML, &query);
}

#[test]
fn is_triple_function_stays_a_locked_501() {
    // Pre-existing (`unify::filter_cond`'s `str_match` wildcard, shared by both
    // engines via `iq/lower.rs`'s reuse of the SAME function) — this rewrite
    // never fabricates a native triple term, so `isTRIPLE`/`TRIPLE`/`SUBJECT`/
    // `PREDICATE`/`OBJECT` are untouched (Wave-2b: TT-valued variables flowing
    // into expressions).
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:hasAge ?age . FILTER isTRIPLE(?age) }}");
    assert_locked_501(CENSUS_R2RML, &query);
}

// ============================================================================
// 9 — a star pattern inside FILTER EXISTS (rule R5a: the walker recurses into
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
// 10 — a star pattern at a property-path endpoint (rule R5b: the identity's 4
// patterns joined alongside the Path node). This structurally reaches a
// PRE-EXISTING, unrelated boundary: `unfold::merge` (shared by both engines
// — `iq/lower.rs`'s `InnerJoin` arm calls the SAME `join_branches`) already
// refuses to join ANY `Branch` carrying a property-path closure with anything
// else ("v1 = a standalone ?s P+ ?o"). The rewrite's Join-wrapped-Bgp+Path
// injection is architecturally correct per the ADR (mirrors the DESCRIBE→CBD
// precedent) and forward-compatible if that restriction is ever lifted — but
// TODAY it necessarily lands on that boundary for EVERY star-at-path-endpoint
// query, so this is a locked 501, not a bindings differential. Lifting the
// join-with-path restriction is a separate, substantial path-composition
// feature, out of scope for this rewrite (D6) — reported to the team lead.
// ============================================================================

#[test]
fn star_pattern_at_property_path_endpoint_is_a_locked_501() {
    let query = format!("{EX}SELECT ?age ?x WHERE {{ <<( ?p ex:hasAge ?age )>> ex:knows+ ?x }}");
    assert_locked_501(CENSUS_R2RML, &query);
}
