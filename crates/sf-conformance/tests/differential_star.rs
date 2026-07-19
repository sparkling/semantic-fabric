//! ADR-0032 D3 differential: the RDF-star quoted-triple-pattern query rewrite
//! (`sf_sparql::star`), superseding ADR-0031's rules R2/R5 in place. Mirrors
//! `differential_paths.rs`'s structure (engine-vs-expected helpers + a
//! fixture-per-shape layout) and reuses `differential_tree.rs`'s tree/flat
//! parity idea (both translators must agree on row-bag AND on the 501 set)
//! via this file's own `diff()` helper — a separate
//! integration-test binary cannot import another one's private items, so the
//! pattern is replicated rather than shared code. Deliberately NOT
//! `differential_tree.rs` itself, which a parallel work stream is appending
//! to (ADR-0023 M8: the tree path, [`translate_with`], is the production
//! default; the flat path, [`translate_with_flat`], is the `=_bag`
//! oracle/fallback — both routed here through the SAME `star::rewrite_query`
//! pre-pass, so this harness proves they stay byte-identical on every case).
//!
//! **Oracle strategy** (ADR-0032 §Test plan, realized by Wave 3): `spareval`
//! evaluates SPARQL-star *natively*, and since Wave 3 it DOES run the
//! *original* query — the materialized encoding is first passed through
//! [`sf_conformance::star_decode::decode_proposition_forms`], yielding the
//! native RDF 1.2 graph (real `rdf:reifies` statements + `Term::Triple`
//! objects), and the `_oracle_agrees` companions below assert answer
//! equivalence per ADR-0032 R6 (zero disagreements as of 2026-07-18). The
//! hand-asserted bindings remain as belt-and-braces alongside the tree/flat
//! differential. Bindings that carry a
//! `rr:column`-sourced literal (`?age`) are cross-checked against a baseline
//! non-star query run through the SAME engine ([`baseline_ages`]) rather than
//! hand-encoding the exact XSD lexical form R2RML's natural-type mapping
//! produces — the CORRELATION (which row's `?age` matches which `?p`) is what
//! this suite verifies, not `rr:column`'s datatype inference (covered
//! elsewhere).

use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::star_decode::decode_proposition_forms;
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, PlanForm, Tbox};
use sf_sql::Dialect;
use spargebra::SparqlParser;
use std::collections::{BTreeMap, HashMap};

use oxrdf::{Dataset, NamedNode, Term, Triple};

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

/// [`diff`]'s CONSTRUCT counterpart (ADR-0032 D2): both translators must
/// either both succeed with the SAME triple bag, or both return
/// `Unsupported`. Returns the tree engine's triples, sorted (a `Vec`, not a
/// `HashSet`, so a duplicate triple's MULTIPLICITY is preserved for the
/// caller's own assertion, though this suite's cases don't happen to produce
/// duplicates).
fn diff_construct(create: &str, r2rml: &str, query: &str) -> Vec<Triple> {
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
            let mut ft = exec::construct_triples(fp, &conn).expect("construct exec");
            let mut tt = exec::construct_triples(tp, &conn).expect("construct exec");
            ft.sort_by_key(ToString::to_string);
            tt.sort_by_key(ToString::to_string);
            assert_eq!(ft, tt, "flat vs tree triple-bag divergence on `{query}`");
            tt
        }
        (Err(Error::Unsupported(_)), Err(Error::Unsupported(_))) => Vec::new(),
        _ => panic!(
            "501-set mismatch on `{query}` (flat and tree must agree on Unsupported):\n \
             flat={flat:?}\n tree={tree:?}"
        ),
    }
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
// 4e — object-side nesting, depth 2, end to end (ADR-0032 D3 rule R4) —
// FLIPPED (Wave 2b, item 5's align_templates literal-prefix lift) from a
// locked 501 into the bindings differential it was designed to be. The
// REWRITE itself was already verified correct at the AST level (W2a):
// `__sf_star_1` (Leaf's identity) carries its 4 patterns, `__sf_star_0`
// (Mid's identity) carries its 4 with `propositionFormObject` pointing at
// `__sf_star_1`, and `?r rdf:reifies __sf_star_0` closes the chain — rule
// R4. EXECUTING it used to hit a PRE-EXISTING, unrelated boundary:
// `desc(#Leaf)` and `desc(#Mid)` are TWO DISTINCT quoted shapes' description
// maps sharing the identical 4-predicate vocabulary (ADR-0032 D1), so
// `__sf_star_0`'s own 4-pattern group was candidate-ambiguous per pattern
// until the CONSTANT-valued propositionFormPredicate narrowed it —
// `unify.rs::align_templates` explored a `desc(#Leaf)`-vs-`desc(#Mid)`
// SUBJECT-template pairing (5 segments vs 8, provably disjoint by differing
// literal prefixes) and used to bail `Unsupported` on the length mismatch
// alone. Item 5's leading-literal-prefix disjointness check now proves that
// EXACT pairing disjoint instead, pruning the bad candidate soundly.
// ============================================================================

#[test]
fn object_side_nesting_depth_2_e2e_matches_hand_computed_bindings() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r ?p ?leaf ?score WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge <<( ?leaf ex:hasScore ?score )>> )>> \
         }}"
    );
    let got = diff(CENSUS_SQL, STAR_NESTED_DEPTH2_R2RML, &query);
    assert_eq!(got.len(), 3, "got={got:#?}");

    // `#Leaf`'s own hasScore is a PLAIN (non-star) pattern — a sound baseline
    // for `?score`'s correlation, cross-checked through the SAME engine
    // (this file's established philosophy — never a hand-typed XSD form).
    let leaf_scores = diff(
        CENSUS_SQL,
        STAR_NESTED_DEPTH2_R2RML,
        &format!("{EX}SELECT ?leaf ?score WHERE {{ ?leaf ex:hasScore ?score }}"),
    );
    let scores_by_leaf: HashMap<Term, Term> = leaf_scores
        .into_iter()
        .map(|mut r| (r.remove("leaf").unwrap(), r.remove("score").unwrap()))
        .collect();

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
            Term::NamedNode(n) => n.as_str(),
            other => panic!("?p must be an IRI, got {other:?}"),
        };
        let leaf = match &row["leaf"] {
            Term::NamedNode(n) => n.as_str(),
            other => panic!("?leaf must be an IRI, got {other:?}"),
        };
        // Same person_id correlates ?p (http://ex.org/person/{id}) and ?leaf
        // (http://ex.org/leaf/{id}) — both templates key off the SAME column.
        let p_id = p.rsplit('/').next().unwrap();
        let leaf_id = leaf.rsplit('/').next().unwrap();
        assert_eq!(
            p_id, leaf_id,
            "?p and ?leaf must share the same person_id: {row:#?}"
        );
        assert_eq!(
            row.get("score"),
            scores_by_leaf.get(&row["leaf"]),
            "?score must match the baseline for ?leaf: {row:#?}"
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
// 6/7/8 — locked boundaries, unrelated to this rewrite or still out of scope
// (Wave-2b territory): both engines 501 identically.
// ============================================================================

#[test]
fn construct_template_quoting_a_triple_in_object_position_produces_real_triples() {
    // ADR-0032 D2: the old 501 guard (`star::construct_template_has_quoted_triple`)
    // is gone — `star::substitute_construct_template` + `exec_core::instantiate`'s
    // recursive `TermPattern::Triple` arm now REALLY instantiate a native
    // `Term::Triple` object (RDF 1.2 §3.1: object position is legal).
    let query = format!(
        "{EX}CONSTRUCT {{ ?p ex:hasQuote <<( ?p ex:hasAge ?age )>> }} \
         WHERE {{ ?p ex:hasAge ?age }}"
    );
    let got = diff_construct(CENSUS_SQL, CENSUS_R2RML, &query);
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    let mut expected: Vec<Triple> = ages
        .iter()
        .map(|(p, age)| {
            let inner = Triple::new(
                p.clone(),
                NamedNode::new_unchecked("http://example.com/hasAge"),
                age.clone(),
            );
            Triple::new(
                p.clone(),
                NamedNode::new_unchecked("http://example.com/hasQuote"),
                Term::Triple(Box::new(inner)),
            )
        })
        .collect();
    expected.sort_by_key(ToString::to_string);
    assert_eq!(got, expected, "got={got:#?}\nexpected={expected:#?}");
    assert_eq!(got.len(), 3);
    // The produced object is a genuine native Term::Triple (not, say, some
    // string-encoded stand-in) — the acceptance bar D2 sets.
    assert!(matches!(got[0].object, Term::Triple(_)));
}

#[test]
fn construct_template_quoting_a_triple_in_illegal_subject_position_drops_silently() {
    // RDF 1.2 §3.1: only the OBJECT position may hold a triple term — a
    // template quoting one in SUBJECT position (`<<(...)>> ex:assertedBy
    // ?src`) is legal to WRITE but every instantiation is ill-formed. §16.2:
    // an ill-formed instantiation is silently DROPPED from the CONSTRUCT
    // output, never an error — `Triple::from_terms` naturally rejects it
    // (`Term::Triple` has no `TryInto<NamedOrBlankNode>`), so
    // `exec_core::instantiate` returns `None` for every row and the
    // translated plan runs successfully to an EMPTY graph (not a 501 — the
    // old guard covered this shape too, indiscriminately; now it is
    // distinguished from the object-position "production" cell above).
    let query = format!(
        "{EX}CONSTRUCT {{ <<( ?p ex:hasAge ?age )>> ex:assertedBy ?src }} \
         WHERE {{ ?p ex:hasAge ?age . BIND(ex:CensusRecord2026 AS ?src) }}"
    );
    let got = diff_construct(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(got.is_empty(), "got={got:#?}");
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
fn values_projected_only_ground_triple_decomposes_and_reprojects_natively() {
    // ADR-0032 D3 item 2, R6: a VALUES ground triple term, referenced
    // NOWHERE else in the query (pure pass-through) — `star::rewrite_values`
    // decomposes ?t's column into 3 fresh component columns carrying the
    // ground s/p/o as CONSTANT rows, and the projection seam
    // (`star::apply_composed_bindings`) reassembles them into the SAME
    // native `Term::Triple` at output. No DB correlation needed — the
    // decomposed value IS the whole answer.
    let query = format!("{EX}SELECT ?t WHERE {{ VALUES ?t {{ <<( ex:a ex:hasAge ex:b )>> }} }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(got.len(), 1, "got={got:#?}");
    let expected = Term::Triple(Box::new(Triple::new(
        NamedNode::new_unchecked("http://example.com/a"),
        NamedNode::new_unchecked("http://example.com/hasAge"),
        iri("http://example.com/b"),
    )));
    assert_eq!(got[0]["t"], expected, "got={got:#?}");
}

#[test]
fn values_multi_row_ground_triples_decompose_and_reproject_per_row() {
    // Multiple VALUES rows, each carrying a DIFFERENT ground triple —
    // row count and per-row correlation must both be preserved through the
    // column-major decompose / transpose-back / re-project round trip.
    let query = format!(
        "{EX}SELECT ?t WHERE {{ VALUES ?t {{ \
           <<( ex:a ex:hasAge ex:b )>> \
           <<( ex:c ex:hasAge ex:d )>> \
         }} }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(got.len(), 2, "got={got:#?}");
    let expect_triple = |s: &str, o: &str| {
        Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked(s),
            NamedNode::new_unchecked("http://example.com/hasAge"),
            iri(o),
        )))
    };
    let mut got_terms: Vec<&Term> = got.iter().map(|r| &r["t"]).collect();
    got_terms.sort_by_key(|t| t.to_string());
    let mut expected = vec![
        expect_triple("http://example.com/a", "http://example.com/b"),
        expect_triple("http://example.com/c", "http://example.com/d"),
    ];
    expected.sort_by_key(ToString::to_string);
    assert_eq!(
        got_terms.into_iter().cloned().collect::<Vec<_>>(),
        expected,
        "got={got:#?}"
    );
}

#[test]
fn values_ground_triple_matched_against_real_reifies_data() {
    // ADR-0032 D3 item 2's "decomposed matched" cell: `?t` is ALSO reified
    // via a real `?r rdf:reifies ?t` pattern — `star::rewrite_triple`'s
    // reifies-bare-variable case and `star::rewrite_values`'s decomposition
    // BOTH register `?t` in the SAME env entry (lookup-before-mint), so they
    // share the SAME component vars and correlate via an ORDINARY
    // shared-variable join: the VALUES-supplied ground components constrain
    // which REAL reifier row(s) match. Uses the SAME engine's own baseline
    // age Term (never a hand-typed XSD lexical form, per this file's module
    // doc) to build the VALUES literal for person 1.
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    let age1 = ages
        .get(&NamedNode::new_unchecked("http://ex.org/person/1"))
        .expect("person 1 must have a baseline age")
        .clone();
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r WHERE {{ \
           ?r rdf:reifies ?t . \
           VALUES ?t {{ <<( <http://ex.org/person/1> ex:hasAge {age1} )>> }} \
         }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(
        got.len(),
        1,
        "exactly one reifier for person 1's real (?p,ex:hasAge,?age) proposition: got={got:#?}"
    );
    let r = match &got[0]["r"] {
        Term::NamedNode(n) => n,
        other => panic!("?r must bind an IRI, got {other:?}"),
    };
    assert!(
        r.as_str().starts_with("urn:sf-star:r:"),
        "?r must bind the reifier-family synthetic IRI: {r}"
    );
}

#[test]
fn is_triple_true_and_false_cells() {
    // ADR-0032 D3 item 3, §17.4.6 asymmetry: isTRIPLE NEVER errors, always a
    // definite boolean — `star::rewrite_expr` resolves it STATICALLY:
    // composed → `true` (every row survives `FILTER isTRIPLE(?t)`);
    // non-composed → `false` (`FILTER isTRIPLE(?age)` eliminates every row,
    // and `FILTER !isTRIPLE(?age)` — the negation — keeps every row,
    // confirming `false` is a real computed value, not some OTHER
    // elimination reason).
    let rdf = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ";

    let composed_true =
        format!("{EX}{rdf}SELECT ?r WHERE {{ ?r rdf:reifies ?t . FILTER isTRIPLE(?t) }}");
    let got_true = diff(CENSUS_SQL, CENSUS_R2RML, &composed_true);
    assert_eq!(got_true.len(), 3, "got={got_true:#?}");

    let non_composed_false =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:hasAge ?age . FILTER isTRIPLE(?age) }}");
    let got_false = diff(CENSUS_SQL, CENSUS_R2RML, &non_composed_false);
    assert!(got_false.is_empty(), "got={got_false:#?}");

    let non_composed_negated =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:hasAge ?age . FILTER (!isTRIPLE(?age)) }}");
    let got_negated = diff(CENSUS_SQL, CENSUS_R2RML, &non_composed_negated);
    assert_eq!(got_negated.len(), 3, "got={got_negated:#?}");
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

/// ADR-0032 D3 item 2 — INVESTIGATED, no reachable bug: `iq::lower::
/// lower_iq_exists` passes an empty `extra_keep` to its inner `lower_node`
/// call (a documented, narrow suspected gap — a composed variable's
/// component vars referenced ONLY inside a FILTER EXISTS/NOT EXISTS/MINUS
/// body might not survive that inner lowering's own projection-restrict
/// retain). This is the best near-miss shape found after genuinely trying 7
/// distinct ones (see that function's own doc comment for the full list and
/// the three structural reasons none of them reach it): `?t` is composed
/// OUTSIDE (and projected), and the SAME `?t` is ALSO referenced via a
/// SECOND, independent `rdf:reifies` occurrence INSIDE the FILTER EXISTS
/// body — reusing the SAME component-variable names across occurrences
/// (`reifies_bare_variable_env_lookup_reuses_component_vars_across_
/// occurrences`, `star/tests.rs`). This survives WITHOUT `extra_keep`
/// because the EXISTS body's own top-level scope has no explicit SPARQL
/// SELECT list, so its default projection is every variable the body binds
/// (broad, `output_vars()`) — the 3 component vars, each bound by a pattern
/// DIRECTLY inside this same body, are already in that broad set regardless
/// of `extra_keep`'s emptiness. Locks the CURRENT, CORRECT behavior — this
/// is a regression guard, not a reachable-bug repro.
#[test]
fn star_pattern_reused_inside_filter_exists_survives_without_extra_keep() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?t WHERE {{ \
           ?r rdf:reifies ?t . \
           FILTER EXISTS {{ ?r2 rdf:reifies ?t . ?r2 ex:assertedBy ?src }} \
         }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    // Every row is asserted (CENSUS_R2RML's #PersonAgeAssertion), so EXISTS
    // holds for every row — the same 3-row shape as this file's sibling
    // `star_pattern_inside_filter_exists_matches_hand_computed_bindings`.
    assert_eq!(got.len(), 3, "got={got:#?}");
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    let mut expected: Vec<Term> = ages
        .iter()
        .map(|(p, age)| {
            Term::Triple(Box::new(Triple::new(
                p.clone(),
                NamedNode::new_unchecked("http://example.com/hasAge"),
                age.clone(),
            )))
        })
        .collect();
    let mut got_terms: Vec<Term> = got
        .into_iter()
        .map(|mut r| r.remove("t").unwrap())
        .collect();
    got_terms.sort_by_key(ToString::to_string);
    expected.sort_by_key(ToString::to_string);
    assert_eq!(
        got_terms, expected,
        "?t must still realize as a genuine native Term::Triple — got={got_terms:#?}\n\
         expected={expected:#?}"
    );
}

// ============================================================================
// 10 — a star pattern at a property-path endpoint (rule R5b: the identity's 4
// patterns joined alongside the Path node). Item 5's align_templates
// literal-prefix lift ORIGINALLY surfaced an unanticipated flat/tree
// divergence HERE (reported to the team lead as a NEW finding): the TREE path
// proves this query PROVABLY EMPTY (the quoted identity's proposition-form
// template, `urn:sf-star:pf:...`, and `ex:knows`'s own subject template,
// `http://ex.org/person/...` read via the path's canonical `sf_s` key column,
// have CONFLICTING literal prefixes from the very first character —
// `ex:knows`'s domain is disjoint from a proposition identity's range BY
// CONSTRUCTION, so `?pf ex:knows+ ?x` can never match ANY row) BEFORE the
// PRE-EXISTING, unrelated "no join onto any path branch" boundary is ever
// reached in ITS OWN pipeline, while the FLAT path's `unfold::merge` checked
// `left.path.is_some() || right.path.is_some()` UNCONDITIONALLY, as its very
// FIRST statement — before ever attempting `unify()` — so it STILL 501'd,
// unimproved. ADR-0032 D6's follow-up ("mirror the prefix check in
// `unfold::merge`") CLOSES that divergence: `merge` now runs the SAME
// leading-literal-prefix disjointness proof (`unify::templates_provably_
// disjoint`, sharing `align_templates`'s exact mechanism, not duplicating it)
// over the join-correlated bindings BEFORE its unconditional path-join 501,
// so flat now ALSO proves this join empty instead of 501ing — both engines
// AGREE (0 rows), verified through the strict `diff()` helper (flat/tree
// row-bag parity), not the looser divergence-locking pattern this slot used
// before the fix landed. UPDATE (ADR-0033): the general "no join onto any
// path branch" boundary this empty-proof pre-empted is now LIFTED on BOTH
// engines (a path-carrying branch converts to an ordinary derived-table
// `Scan` at the two tree join sites and, since Run 4 A2, at flat's own
// `GraphPattern::Join` arm via the same conversion) — but THIS query stays
// empty on BOTH
// engines regardless, unaffected: after conversion, `unfold::merge`'s
// disjointness pre-check simply no longer fires (its own `path.is_some()`
// guard is gone), so the SAME `align_templates` proof now runs as part of
// ORDINARY `unify()` instead — still `Unify::Empty`, same 0 rows, just
// reached one call deeper. See the ANSWERABLE case right below, where the
// join var is a PERSON (not a proposition-form id) — the templates are NOT
// disjoint there, so the lift actually produces rows.
// ============================================================================

#[test]
fn star_pattern_at_property_path_endpoint_flat_and_tree_both_prove_it_empty() {
    let query = format!("{EX}SELECT ?age ?x WHERE {{ <<( ?p ex:hasAge ?age )>> ex:knows+ ?x }}");
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(
        got.is_empty(),
        "both engines must agree this join is PROVABLY EMPTY (ADR-0032 D6: the quoted \
         identity's proposition-form template and ex:knows's own subject template have \
         conflicting literal prefixes from the first character): got={got:#?}"
    );
}

/// The ANSWERABLE D6 case ADR-0033 finally unlocks: the quoted triple's own
/// SUBJECT COMPONENT (`?p`, a PERSON IRI — `http://ex.org/person/{person_id}`,
/// the IDENTICAL domain `#Knows`'s own subject/object templates use) feeds the
/// closure, not the reifier/proposition-form id — so the join genuinely
/// correlates instead of being provably empty. Both engines now answer: the
/// ADR-0033 conversion also runs at flat's own `GraphPattern::Join` arm (the
/// same `convert_path_branches`, mirrored there in Run 4 A2), so `diff()`
/// applies — strict flat/tree row-bag parity PLUS the hand-computed
/// expectation below. `ex:knows` edges (from `friend_id`): (1,2) and (3,1) — row 2
/// (Bob, friend_id NULL) contributes no edge. `ex:knows+` closure:
/// {(1,2),(3,1),(3,2)}. Every census row IS an `#PersonAgeAssertion` (`?p`
/// ranges over all 3 person ids), so joining with the closure keeps only
/// p∈{1,3} (2 has no outgoing edge): (p=1,x=2), (p=3,x=1), (p=3,x=2) — 3
/// rows. `?age` is cross-checked against the SAME engine's own
/// `baseline_ages` rather than hand-typed (the module doc's established
/// rationale — never hand-encode an `rr:column`-sourced literal's exact XSD
/// lexical form).
#[test]
fn star_pattern_at_property_path_endpoint_now_answers_on_both_engines() {
    // A DEDICATED fixture, not `CENSUS_R2RML`'s own `#Knows` (`friend_id`, which
    // is NULLABLE — row 2 leaves it NULL): a PRE-EXISTING gap in the path
    // closure's one-hop relation (`emit::hop_sql`'s `HopExpr::Pred` case had no
    // `IS NOT NULL` guard on the object column) let that NULL flow into the base
    // hop as a phantom `(2, NULL)` pair, which then TRANSITIVELY poisoned every
    // node that can reach it (`1→2→NULL`, `3→1→2→NULL`) — unrelated to join
    // composition (pre-existing on the standalone, non-joined path too,
    // `emit_path_branch`, untouched by ADR-0033), so kept separate from THIS
    // test's own D6 join-lift concern deliberately. FIXED (F4a): `hop_sql`'s
    // `HopExpr::Pred` arm and `reflexive_sql` both now guard every
    // column-valued endpoint (`differential_paths.rs`'s `*_nullable_object_
    // column_*` tests). `#KnowsClean` is kept as its own NOT NULL fixture
    // regardless — this test isolates the D6 join-lift question specifically,
    // same {(1,2),(3,1)} shape as `#Knows`.
    const KNOWS_CLEAN_SQL: &str = r#"
CREATE TABLE census_row (
    person_id INTEGER PRIMARY KEY,
    age INTEGER NOT NULL,
    friend_id INTEGER
);
INSERT INTO census_row VALUES (1, 30, 2);
INSERT INTO census_row VALUES (2, 40, NULL);
INSERT INTO census_row VALUES (3, 30, 1);
CREATE TABLE knows_clean (a INTEGER NOT NULL, b INTEGER NOT NULL);
INSERT INTO knows_clean VALUES (1, 2);
INSERT INTO knows_clean VALUES (3, 1);
"#;
    const KNOWS_CLEAN_R2RML: &str = r#"
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

<#KnowsClean>
    rr:logicalTable [ rr:tableName "knows_clean" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{a}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:knowsClean ;
        rr:objectMap [ rr:template "http://ex.org/person/{b}" ]
    ] .
"#;

    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?x WHERE {{ ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
         ?p ex:knowsClean+ ?x }}"
    );
    let got = diff(KNOWS_CLEAN_SQL, KNOWS_CLEAN_R2RML, &query);

    // `ex:knowsClean` = {(1,2),(3,1)}; closure `+` = {(1,2),(3,1),(3,2)}. Every
    // census row IS a `#PersonAgeAssertion` (`?p` ranges over all 3 person
    // ids), so joining with the closure keeps p in {1,3} (2 has no outgoing
    // edge): (p=1,x=2), (p=3,x=1), (p=3,x=2) — 3 rows. `?age` is cross-checked
    // against the SAME engine's own `baseline_ages` rather than hand-typed
    // (the module doc's established rationale for an `rr:column`-sourced
    // literal).
    let ages = baseline_ages(KNOWS_CLEAN_SQL, KNOWS_CLEAN_R2RML);
    let person = |id: i32| NamedNode::new_unchecked(format!("http://ex.org/person/{id}"));
    let expected: Vec<BTreeMap<String, Term>> = [(1, 2), (3, 1), (3, 2)]
        .into_iter()
        .map(|(p_id, x_id)| {
            row3(
                "p",
                Term::NamedNode(person(p_id)),
                "age",
                ages[&person(p_id)].clone(),
                "x",
                Term::NamedNode(person(x_id)),
            )
        })
        .collect();
    assert!(
        oracle::solutions_bag_eq(&got, &expected),
        "got={got:#?}\nexpected={expected:#?}"
    );
}

// ============================================================================
// Wave 2b (ADR-0032 D3 items 2-4, 6) — the composed-variable environment: the
// reifies-bare-variable decode-at-boundary acceptance test, the five
// triple-term functions over a composed / non-composed argument, TRIPLE(...)
// as a BIND target, composed-aware `=`/`sameTerm`, the UNION uniform-
// composed-ness boundary, and ORDER BY determinism.
// ============================================================================

#[test]
fn reifies_object_variable_projects_as_a_native_triple_term() {
    // THE decode-at-boundary acceptance test (ADR-0032 D2's headline mandate:
    // "every visible surface speaks native reification form"): `?r rdf:reifies
    // ?t` composes `?t` (item 2's reifies-bare-variable case) — projecting it
    // directly must realize the REAL native `Term::Triple`, never the
    // internal proposition-form IRI.
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?t WHERE {{ ?r rdf:reifies ?t }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(got.len(), 3, "got={got:#?}");
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    let mut expected: Vec<Term> = ages
        .iter()
        .map(|(p, age)| {
            Term::Triple(Box::new(Triple::new(
                p.clone(),
                NamedNode::new_unchecked("http://example.com/hasAge"),
                age.clone(),
            )))
        })
        .collect();
    let mut got_terms: Vec<Term> = got
        .into_iter()
        .map(|mut r| r.remove("t").unwrap())
        .collect();
    got_terms.sort_by_key(ToString::to_string);
    expected.sort_by_key(ToString::to_string);
    assert_eq!(
        got_terms, expected,
        "got={got_terms:#?}\nexpected={expected:#?}"
    );
    assert!(
        matches!(got_terms[0], Term::Triple(_)),
        "must be a genuine native Term::Triple: {:?}",
        got_terms[0]
    );
}

#[test]
fn subject_predicate_object_on_composed_variable_bind_the_components() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?s ?p ?o WHERE {{ \
           ?r rdf:reifies ?t . \
           BIND(SUBJECT(?t) AS ?s) . BIND(PREDICATE(?t) AS ?p) . BIND(OBJECT(?t) AS ?o) \
         }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(got.len(), 3, "got={got:#?}");
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    for row in &got {
        let s = match &row["s"] {
            Term::NamedNode(n) => n.clone(),
            other => panic!("?s must be an IRI, got {other:?}"),
        };
        assert_eq!(row["p"], iri("http://example.com/hasAge"), "row={row:#?}");
        assert_eq!(row.get("o"), ages.get(&s), "row={row:#?}");
    }
}

#[test]
fn subject_predicate_object_on_a_non_composed_variable_error_observably() {
    // §17.4.6: SUBJECT/PREDICATE/OBJECT on a provably-non-composed argument
    // is the spec ERROR (engine-totality — `star::rewrite_function_call`'s
    // doc comment) — in FILTER context it eliminates the row; in BIND
    // context the target var stays genuinely unbound (R5: never a wrong
    // value silently substituted for the error).
    let filter_query =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:hasAge ?age . FILTER(SUBJECT(?age)) }}");
    let filtered = diff(CENSUS_SQL, CENSUS_R2RML, &filter_query);
    assert!(filtered.is_empty(), "got={filtered:#?}");

    let bind_query =
        format!("{EX}SELECT ?p ?s WHERE {{ ?p ex:hasAge ?age . BIND(SUBJECT(?age) AS ?s) }}");
    let bound = diff(CENSUS_SQL, CENSUS_R2RML, &bind_query);
    assert_eq!(
        bound.len(),
        3,
        "every row still appears, just with ?s left unbound: got={bound:#?}"
    );
    for row in &bound {
        assert!(
            !row.contains_key("s"),
            "?s must be genuinely UNBOUND, not present: {row:#?}"
        );
    }
}

#[test]
fn triple_function_bind_projects_as_a_native_triple_term() {
    let query = format!(
        "{EX}SELECT ?p ?age ?t WHERE {{ \
           ?p ex:hasAge ?age . BIND(TRIPLE(?p, ex:hasAge, ?age) AS ?t) \
         }}"
    );
    let got = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(got.len(), 3, "got={got:#?}");
    for row in &got {
        let p = match &row["p"] {
            Term::NamedNode(n) => n.clone(),
            other => panic!("?p must be an IRI, got {other:?}"),
        };
        let expected = Term::Triple(Box::new(Triple::new(
            p,
            NamedNode::new_unchecked("http://example.com/hasAge"),
            row["age"].clone(),
        )));
        assert_eq!(row["t"], expected, "row={row:#?}");
        assert!(matches!(row["t"], Term::Triple(_)));
    }
}

#[test]
fn equality_and_same_term_over_composed_variables() {
    // `#AssertionA`/`#AssertionB` (CENSUS_R2RML_TWO_ASSERTIONS) both reify the
    // SAME `#PersonAge` shape — `?t1`/`?t2`, drawn from two INDEPENDENT
    // reifies patterns (no shared variable besides the cartesian join),
    // structurally correlate ONLY through equality: same person ⇒ equal
    // OBJECT component ⇒ `=`/`sameTerm` true; different person ⇒ false.
    // Compares `OBJECT(?t1)`/`OBJECT(?t2)` specifically (the `age` column, a
    // bare `rr:column`), not the whole `?t1 = ?t2` — `#PersonAge`'s SUBJECT
    // is `rr:template`-valued, and `unify::filter_cond`'s `var_col` (which
    // both the pre-existing `Equal`/`Greater`/etc. machinery and this wave's
    // new var-vs-var `cmp` arm share) only resolves a bare-column binding —
    // a PRE-EXISTING v1 `filter_cond` scope limit, inherited (not
    // introduced) by composed-variable equality once component-wise
    // recursion reaches a template-bound component; see the SEPARATE
    // `whole_composed_variable_equality_over_a_template_bound_component_is_a_sound_501`
    // test below for that exact, honestly-documented boundary.
    // `=` is infix; `sameTerm` is a FUNCTION CALL (`sameTerm(a, b)`), not an
    // infix operator — spargebra `parser.rs`'s `sameTerm` grammar rule.
    let filter_eq = |expr: &str| {
        format!(
            "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
             SELECT ?rA ?rB WHERE {{ \
               ?rA rdf:reifies ?t1 . ?rA ex:assertedBy ex:SourceA . \
               ?rB rdf:reifies ?t2 . ?rB ex:assertedBy ex:SourceB . \
               FILTER({expr}) \
             }}"
        )
    };
    // CENSUS_SQL's ages (30, 40, 30) are NOT unique per person (persons 1
    // and 3 share age 30), so the equal-age pair count is computed from the
    // SAME engine's own baseline — never hand-guessed — rather than assumed
    // to be "one pairing per person" (3x3 cartesian, one match per row).
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML_TWO_ASSERTIONS);
    let age_values: Vec<Term> = ages.values().cloned().collect();
    let expected_equal: usize = age_values
        .iter()
        .map(|a| age_values.iter().filter(|b| a == *b).count())
        .sum();
    let total = age_values.len() * age_values.len();

    let equal = diff(
        CENSUS_SQL,
        CENSUS_R2RML_TWO_ASSERTIONS,
        &filter_eq("OBJECT(?t1) = OBJECT(?t2)"),
    );
    assert_eq!(equal.len(), expected_equal, "got={equal:#?}");
    let same_term = diff(
        CENSUS_SQL,
        CENSUS_R2RML_TWO_ASSERTIONS,
        &filter_eq("sameTerm(OBJECT(?t1), OBJECT(?t2))"),
    );
    assert_eq!(
        same_term.len(),
        expected_equal,
        "sameTerm agrees with = here (same-typed integer literals): got={same_term:#?}"
    );
    let unequal = diff(
        CENSUS_SQL,
        CENSUS_R2RML_TWO_ASSERTIONS,
        &filter_eq("OBJECT(?t1) != OBJECT(?t2)"),
    );
    assert_eq!(unequal.len(), total - expected_equal, "got={unequal:#?}");

    // Exactly-one-composed → constant FALSE (never an error, resolved
    // STATICALLY at rewrite time — never reaches `var_col` at all).
    let mixed = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p WHERE {{ \
           ?r rdf:reifies ?t . ?p ex:hasAge ?age . FILTER(?t = ?age) \
         }}"
    );
    let one_composed = diff(CENSUS_SQL, CENSUS_R2RML, &mixed);
    assert!(
        one_composed.is_empty(),
        "a triple term can never equal a non-triple-term value: got={one_composed:#?}"
    );
}

#[test]
fn whole_composed_variable_equality_over_a_template_bound_component_now_resolves() {
    // FORMERLY a locked 501 (ledger closeout, boundary B): the FULL `?t1 =
    // ?t2` (both composed, component-wise conjunction —
    // `star::rewrite_equality`) recurses into comparing the SUBJECT
    // components directly (`http://ex.org/person/{person_id}`, an
    // `rr:template`) AND the PREDICATE components (`ex:hasAge`, a CONSTANT —
    // RDF 1.2 §3.1 predicates are always IRIs, and `sf-mapping`'s quoted-
    // shape compiler bakes a quoted predicate in as a fixed constant, never a
    // per-row column, `r2rml/star.rs`'s `quote_shape`). `unify::filter_cond`'s
    // `var_col` only resolves a bare `rr:column` binding (pre-existing v1
    // scope) — but `cmp`'s new `var_var_eq_beyond_column` (`unify.rs`)
    // resolves both shapes directly: two SAME-SHAPE templates align
    // pairwise-column-equal (`unify::align_templates`, reused verbatim from
    // the ordinary join-key case), two equal constants resolve to the
    // "always true" sentinel. The OBJECT component (`age`, a bare
    // `rr:column`) already worked
    // (`equality_and_same_term_over_composed_variables`). So the WHOLE `?t1 =
    // ?t2` now resolves: `?t1` and `?t2` (as native triple terms) are equal
    // IFF `rA`/`rB` reify the SAME person's `#PersonAge` proposition — exactly
    // the diagonal of the 3x3 cartesian, one pair per `census_row` row —
    // verified against the independent spareval oracle, not hand-counted
    // (unlike `expected_equal` in the sibling test above, which is
    // object-only equality and so also counts the spurious same-age-
    // different-person pairs that FULL triple equality correctly excludes).
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?rA ?rB WHERE {{ \
           ?rA rdf:reifies ?t1 . ?rA ex:assertedBy ex:SourceA . \
           ?rB rdf:reifies ?t2 . ?rB ex:assertedBy ex:SourceB . \
           FILTER(?t1 = ?t2) \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML_TWO_ASSERTIONS, &query);
    assert_eq!(rows.len(), 3, "one (rA,rB) pair per person: got={rows:#?}");
}

#[test]
fn union_arms_disagreeing_on_composed_ness_resolves_at_the_top_level() {
    // FORMERLY a locked 501 (ledger closeout, boundary A): ADR-0032 D3 item
    // 2's uniform-composed-ness law — the left arm composes `?t` (via
    // `rdf:reifies`, to the PROPOSITION); the right arm binds the SAME `?t`
    // as an ordinary, non-composing pattern variable (to the REIFIER, a
    // DIFFERENT value — `#PersonAgeAssertion`'s own subject). A FILTER
    // wrapping this same union now ALSO resolves, per arm (see the companion
    // `_wrapped_in_a_filter_now_resolves_per_arm` test below; deeper
    // nesting — under a JOIN, or a CONSTRUCT template — is still rejected,
    // pinned in `differential_star_observers.rs`) — but this
    // EXACT query's union is the SELECT's own top-level pattern (nothing
    // else references `?t`), where `star::rewrite_top_level_pattern` proves
    // it observationally safe: each top-level `Plan` branch reconstructs
    // independently (`exec_core::run_branches`, never a single SQL-level
    // `UNION` requiring uniform column arity), so the left arm's `?t`
    // realizes a native `Term::Triple` and the right arm's stays an ordinary
    // `Term::NamedNode`, with nothing in the query ever needing ONE static
    // answer about which. 6 rows total: the 3 propositions (same triples as
    // `reifies_object_variable_projects_as_a_native_triple_term`) plus the 3
    // reifiers (same IRIs `SELECT ?t WHERE {?t ex:assertedBy
    // ex:CensusRecord2026}` would bind) — verified against the independent
    // spareval oracle.
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?t WHERE {{ \
           {{ ?r rdf:reifies ?t }} \
           UNION \
           {{ ?t ex:assertedBy ex:CensusRecord2026 }} \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 6, "got={rows:#?}");
    let (triples, plain): (Vec<_>, Vec<_>) = rows
        .iter()
        .map(|r| &r["t"])
        .partition(|t| matches!(t, Term::Triple(_)));
    assert_eq!(triples.len(), 3, "the 3 propositions: got={rows:#?}");
    assert_eq!(plain.len(), 3, "the 3 reifiers: got={rows:#?}");
    assert!(
        plain.iter().all(|t| matches!(t, Term::NamedNode(_))),
        "a non-composed reifier is an ordinary IRI: got={rows:#?}"
    );
}

#[test]
fn union_arms_disagreeing_on_composed_ness_wrapped_in_a_filter_now_resolves_per_arm() {
    // FORMERLY a locked 501 (the F4b boundary pin): the IDENTICAL
    // disagreement as the previous test, wrapped in a FILTER referencing
    // `?t` (`isTRIPLE`, a genuinely sensitive consumer — the OLD
    // `star::rewrite_and_check_composed` resolved it to ONE static boolean
    // for the WHOLE query). Run 4 Wave B2's `rewrite_filter_over_union`
    // now resolves the FILTER expression PER ARM, each against its own
    // arm's composedness (mirroring `iq::normalize`'s later
    // Filter-over-Union distribution, done before composedness collapses
    // to one static answer): the composed arm's `isTRIPLE(?t)` is
    // statically true — its 3 propositions survive — and the plain arm's
    // is statically false — its 3 reifiers are dropped. Verified against
    // the independent spareval oracle. Deeper nesting (under a JOIN, or a
    // CONSTRUCT template) is still a locked 501, pinned in
    // `differential_star_observers.rs`.
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?t WHERE {{ \
           {{ {{ ?r rdf:reifies ?t }} \
           UNION \
           {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
           FILTER(isTRIPLE(?t)) \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(
        rows.len(),
        3,
        "only the composed arm's 3 propositions survive isTRIPLE: got={rows:#?}"
    );
    assert!(
        rows.iter().all(|r| matches!(&r["t"], Term::Triple(_))),
        "every surviving ?t is a native triple term: got={rows:#?}"
    );
}

#[test]
fn values_mixed_triple_and_plain_cells_resolves_at_the_top_level() {
    // FORMERLY a locked 501 (ledger closeout, boundary A): a VALUES column
    // mixing a ground triple-term cell with a plain-IRI cell for the SAME
    // variable is a genuine shape ambiguity ONE flat table cannot represent
    // (`star::decompose_column`'s doc comment) — but at the SELECT's own top
    // level, `star::partition_values_by_triple_shape` row-partitions it into
    // TWO uniform VALUES blocks, unioned, reducing it to the (now-resolved)
    // union-mixed case above. A FILTER wrapping it now also resolves, per
    // arm (see the companion `_wrapped_in_a_filter_now_resolves_per_arm`
    // test below); deeper nesting is still rejected, pinned in
    // `differential_star_observers.rs`.
    let query =
        format!("{EX}SELECT ?t WHERE {{ VALUES ?t {{ <<( ex:a ex:hasAge ex:b )>> ex:plain }} }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 2, "got={rows:#?}");
    let expected_triple = Triple::new(
        NamedNode::new_unchecked("http://example.com/a"),
        NamedNode::new_unchecked("http://example.com/hasAge"),
        iri("http://example.com/b"),
    );
    assert!(
        rows.iter()
            .any(|r| matches!(&r["t"], Term::Triple(t) if **t == expected_triple)),
        "got={rows:#?}"
    );
    assert!(
        rows.iter()
            .any(|r| r["t"] == iri("http://example.com/plain")),
        "got={rows:#?}"
    );
}

#[test]
fn values_mixed_triple_and_plain_cells_wrapped_in_a_filter_now_resolves_per_arm() {
    // FORMERLY a locked 501 (the F4b boundary pin): the IDENTICAL mixed
    // VALUES column, wrapped in a FILTER referencing `?t`.
    // `star::partition_values_by_triple_shape` row-partitions the column
    // into two uniform VALUES blocks (exactly as in the sibling top-level
    // test above), and Run 4 Wave B2's per-arm FILTER resolution then
    // evaluates `isTRIPLE(?t)` against EACH block's own shape: statically
    // true for the ground-triple cell (kept), statically false for the
    // plain-IRI cell (dropped). Verified against the spareval oracle.
    let query = format!(
        "{EX}SELECT ?t WHERE {{ \
           VALUES ?t {{ <<( ex:a ex:hasAge ex:b )>> ex:plain }} \
           FILTER(isTRIPLE(?t)) \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(
        rows.len(),
        1,
        "only the ground-triple cell survives isTRIPLE: got={rows:#?}"
    );
    let expected_triple = Triple::new(
        NamedNode::new_unchecked("http://example.com/a"),
        NamedNode::new_unchecked("http://example.com/hasAge"),
        iri("http://example.com/b"),
    );
    assert!(
        matches!(&rows[0]["t"], Term::Triple(t) if **t == expected_triple),
        "the surviving cell is the ground triple term: got={rows:#?}"
    );
}

#[test]
fn order_by_composed_var_is_deterministic_across_runs() {
    // ADR-0032 D3 item 6: triple terms are SPARQL's highest ORDER BY category
    // (§15.1); order AMONG them is spec-undefined. This engine's choice
    // (sort last, by lexical form — `exec_core::term_rank`'s doc comment) is
    // merely a PERMISSIBLE, deterministic one — verified by running the SAME
    // query twice and requiring the SAME row order both times (mirrors
    // `explicit_reifier_variable_binds_synthetic_iri_deterministically`'s own
    // "run twice, compare" pattern).
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?t WHERE {{ ?r rdf:reifies ?t }} ORDER BY ?t"
    );
    let run1 = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    let run2 = diff(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(run1.len(), 3, "got={run1:#?}");
    assert_eq!(
        run1, run2,
        "the same ORDER BY query must produce the SAME row order across independent runs"
    );
}

// ============================================================================
// Wave 3 (ADR-0032 D0 / R6) — the END-TO-END SPAREVAL ORACLE. Every test
// above hand-computes its expectation and cross-checks the tree/flat SQL
// translators against EACH OTHER ([`diff`]/[`diff_construct`]) — real, but
// not independent of the rewrite itself (`sf_sparql::star`): a bug shared by
// both translators (they share the SAME `star::rewrite_query` pre-pass, per
// the module doc) would sail through undetected. This section adds the THIRD,
// genuinely independent leg D0 demands: materialize the mapping's FULL
// encoded graph (every triples map, including the synthetic description maps
// — `exec::dump_quads`, the SAME mapping-IR walk `runner.rs`'s named-graph
// path already uses), decode it to native RDF 1.2 reification form
// (`sf_conformance::star_decode`, verified in isolation by its own unit
// tests), and run the query AS THE USER WROTE IT — never rewritten — through
// `spareval` (verified fully SPARQL-star-native: reifies triples, triple-term
// objects, the five functions, CONSTRUCT templates, all natively evaluated
// over the DECODED graph, no rewrite pass involved on this side at all).
//
// Since ADR-0032 D1 ids are all deterministic IRIs (`urn:sf-star:pf:...` /
// `urn:sf-star:r:...`, never a blank node anywhere in this encoding), the
// SAME mapping run through BOTH the engine's SQL path and this decode path
// mints the IDENTICAL reifier/proposition IRIs — so SELECT bindings compare
// with PLAIN bag equality ([`oracle::solutions_bag_eq`]), no canonicalization
// needed (this file's module doc, "Oracle strategy", is updated by this
// finding: the SAME-graph decode now makes the spareval oracle usable after
// all — see that doc comment for why it was previously ruled out). CONSTRUCT
// still goes through [`graph::isomorphic`] (the crate's established
// graph-comparison primitive), even though no blank node ever actually
// appears in this particular data.
// ============================================================================

/// Materialize `r2rml`'s FULL encoded graph over `create` — every triples
/// map, synthetic description maps included ([`exec::dump_quads`]'s
/// mapping-IR walk, not a SPARQL CONSTRUCT dump: it needs no translation at
/// all, so it also exercises the description maps a `?s ?p ?o` CONSTRUCT
/// would technically also reach, but more directly) — then decode it to
/// native RDF 1.2 form (ADR-0032 D2).
fn decoded_graph(create: &str, r2rml: &str) -> Dataset {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let encoded = graph::quads_to_dataset(&quads);
    decode_proposition_forms(&encoded)
        .expect("decode must succeed for a well-formed ADR-0032 D1 emission")
}

/// The D0 oracle answer: `query` (the ORIGINAL SPARQL-star surface syntax,
/// never rewritten) evaluated by `spareval` over the decoded native graph.
fn oracle_star(create: &str, r2rml: &str, query: &str) -> OracleAnswer {
    oracle::evaluate(&decoded_graph(create, r2rml), query).expect("oracle eval")
}

fn oracle_star_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    match oracle_star(create, r2rml, query) {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected Solutions, got {other:?}"),
    }
}

/// The engine's (tree/flat-agreed, [`diff`]) row bag vs the decoded-graph
/// spareval oracle's row bag: both must agree EXACTLY (ADR-0032 R6's
/// acceptance bar). Returns the agreed rows for the caller's own additional
/// assertions (row count, structural sanity).
fn assert_oracle_agrees(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let engine = diff(create, r2rml, query);
    let oracle_rows = oracle_star_bag(create, r2rml, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "ADR-0032 R6 divergence on `{query}`:\n \
         engine (SQL-rewritten encoding) = {engine:#?}\n \
         oracle (decoded native graph, spareval) = {oracle_rows:#?}"
    );
    engine
}

/// [`assert_oracle_agrees`]'s CONSTRUCT counterpart: the engine's produced
/// triples vs spareval's CONSTRUCT output over the decoded graph, compared by
/// [`graph::isomorphic`].
fn assert_oracle_agrees_construct(create: &str, r2rml: &str, query: &str) -> Vec<Triple> {
    let engine = diff_construct(create, r2rml, query);
    let oracle_graph = match oracle_star(create, r2rml, query) {
        OracleAnswer::Graph(g) => *g,
        other => panic!("expected Graph, got {other:?}"),
    };
    let engine_graph = graph::triples_to_dataset(&engine);
    assert!(
        graph::isomorphic(&engine_graph, &oracle_graph),
        "ADR-0032 R6 CONSTRUCT divergence on `{query}`:\n engine={engine:#?}\n oracle={oracle_graph:?}"
    );
    engine
}

// --- Matrix cells (ADR-0032 Test plan): each companions a hand-computed test
// above — same fixture, same query, cross-checked against the independent
// decoded-graph oracle instead of (or in addition to) the hand-computed
// expectation. ---

/// Companions [`bare_syntax_reifies_elision_matches_hand_computed_bindings`].
#[test]
fn bare_syntax_reifies_elision_oracle_agrees() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<?p ex:hasAge ?age>> ex:assertedBy ?src }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
}

/// Companions [`parenthesized_subject_position_triple_term_is_statically_empty`]
/// — the subject-position statically-empty matrix cell: the oracle (a real
/// evaluator, not a stub) independently agrees the answer is empty, over data
/// that DOES contain the reified statement (proving the emptiness is a
/// genuine syntactic-position law, not merely "no matching data").
#[test]
fn parenthesized_subject_position_triple_term_oracle_agrees_empty() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<( ?p ex:hasAge ?age )>> ex:assertedBy ?src }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(rows.is_empty(), "rows={rows:#?}");
}

/// Companions [`subject_side_nested_quoted_triple_is_statically_empty`] — the
/// SAME law, one level of (statically-empty, spec-impossible) subject-side
/// nesting deeper.
#[test]
fn subject_side_nested_quoted_triple_oracle_agrees_empty() {
    let query = format!(
        "{EX}SELECT * WHERE {{ <<( <<( ?a ex:hasAge ?b )>> ex:assertedBy ?c )>> ex:assertedBy ?d }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(rows.is_empty(), "rows={rows:#?}");
}

/// Companions [`object_position_star_pattern_matches_hand_computed_bindings`]
/// — object-position `<<( )>>` TripleTerm match.
#[test]
fn object_position_star_pattern_oracle_agrees() {
    let query =
        format!("{EX}SELECT ?q ?p ?age WHERE {{ ?q ex:hasQuote <<( ?p ex:hasAge ?age )>> }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML_OBJECT, &query);
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
}

/// Companions
/// [`bare_syntax_in_object_position_does_not_match_an_unreified_triple_term`]
/// — the bare-in-object EMPTY cell.
#[test]
fn bare_syntax_in_object_position_oracle_agrees_empty() {
    let query = format!("{EX}SELECT ?q ?p ?age WHERE {{ ?q ex:hasQuote << ?p ex:hasAge ?age >> }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML_OBJECT, &query);
    assert!(rows.is_empty(), "rows={rows:#?}");
}

/// Companions
/// [`reifier_multiplicity_two_star_maps_same_shape_yield_distinct_reifiers`]
/// — reifier multiplicity (two reifiers, one proposition).
#[test]
fn reifier_multiplicity_oracle_agrees() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?r ?src WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
           ?r ex:assertedBy ?src \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML_TWO_ASSERTIONS, &query);
    assert_eq!(rows.len(), 6, "rows={rows:#?}");
}

/// Companions [`annotation_sugar_asserts_and_reifies_matches_same_rows_as_bare_sugar`]
/// — annotation sugar, asserted.
#[test]
fn annotation_sugar_asserts_and_reifies_oracle_agrees() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ ?p ex:hasAge ?age {{| ex:assertedBy ?src |}} }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
}

/// Companions [`annotation_sugar_also_requires_the_plain_triple_unlike_bare_sugar`]
/// — the non-asserted EMPTY distinguisher: spareval's OWN annotation-sugar
/// desugaring (parser-level, independent of `sf_sparql::star`'s rewrite)
/// empirically corroborates the engine's plain-triple-required reading,
/// end to end through the SAME decoded graph.
#[test]
fn annotation_sugar_non_asserted_oracle_agrees_empty() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ ?p ex:hasAge ?age {{| ex:assertedBy ?src |}} }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML_NON_ASSERTED, &query);
    assert!(rows.is_empty(), "rows={rows:#?}");
}

/// Companions [`explicit_reifier_sugar_e2e_matches_same_rows_as_manual_reifies_pattern`]
/// — explicit reifier sugar `<< s p o ~ ?r >>`.
#[test]
fn explicit_reifier_sugar_oracle_agrees() {
    let query = format!(
        "{EX}SELECT ?p ?age ?r ?src WHERE {{ << ?p ex:hasAge ?age ~ ?r >> . ?r ex:assertedBy ?src }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
}

/// Companions [`object_side_nesting_depth_2_e2e_matches_hand_computed_bindings`]
/// — nested depth-2 bindings.
#[test]
fn object_side_nesting_depth_2_oracle_agrees() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r ?p ?leaf ?score WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge <<( ?leaf ex:hasScore ?score )>> )>> \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, STAR_NESTED_DEPTH2_R2RML, &query);
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
}

/// Companions [`values_projected_only_ground_triple_decomposes_and_reprojects_natively`]
/// — VALUES, projected-only native ground triple.
#[test]
fn values_projected_only_ground_triple_oracle_agrees() {
    let query = format!("{EX}SELECT ?t WHERE {{ VALUES ?t {{ <<( ex:a ex:hasAge ex:b )>> }} }}");
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
}

/// Companions [`values_ground_triple_matched_against_real_reifies_data`] —
/// VALUES, matched against real reifies data.
#[test]
fn values_matched_against_real_reifies_data_oracle_agrees() {
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML);
    let age1 = ages
        .get(&NamedNode::new_unchecked("http://ex.org/person/1"))
        .expect("person 1 must have a baseline age")
        .clone();
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r WHERE {{ \
           ?r rdf:reifies ?t . \
           VALUES ?t {{ <<( <http://ex.org/person/1> ex:hasAge {age1} )>> }} \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
}

/// Companions [`subject_predicate_object_on_composed_variable_bind_the_components`]
/// — SUBJECT/PREDICATE/OBJECT on a composed variable.
#[test]
fn subject_predicate_object_functions_oracle_agrees() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?s ?p ?o WHERE {{ \
           ?r rdf:reifies ?t . \
           BIND(SUBJECT(?t) AS ?s) . BIND(PREDICATE(?t) AS ?p) . BIND(OBJECT(?t) AS ?o) \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
}

/// Companions [`is_triple_true_and_false_cells`] — isTRIPLE true/false cells.
#[test]
fn is_triple_cells_oracle_agrees() {
    let rdf = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ";

    let composed_true =
        format!("{EX}{rdf}SELECT ?r WHERE {{ ?r rdf:reifies ?t . FILTER isTRIPLE(?t) }}");
    let rows_true = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &composed_true);
    assert_eq!(rows_true.len(), 3, "rows={rows_true:#?}");

    let non_composed_false =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:hasAge ?age . FILTER isTRIPLE(?age) }}");
    let rows_false = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &non_composed_false);
    assert!(rows_false.is_empty(), "rows={rows_false:#?}");
}

/// Companions [`equality_and_same_term_over_composed_variables`] — equality
/// cells (`=` and `sameTerm` over composed variables).
#[test]
fn equality_and_same_term_oracle_agrees() {
    let filter_eq = |expr: &str| {
        format!(
            "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
             SELECT ?rA ?rB WHERE {{ \
               ?rA rdf:reifies ?t1 . ?rA ex:assertedBy ex:SourceA . \
               ?rB rdf:reifies ?t2 . ?rB ex:assertedBy ex:SourceB . \
               FILTER({expr}) \
             }}"
        )
    };
    let ages = baseline_ages(CENSUS_SQL, CENSUS_R2RML_TWO_ASSERTIONS);
    let age_values: Vec<Term> = ages.values().cloned().collect();
    let expected_equal: usize = age_values
        .iter()
        .map(|a| age_values.iter().filter(|b| a == *b).count())
        .sum();

    let equal = assert_oracle_agrees(
        CENSUS_SQL,
        CENSUS_R2RML_TWO_ASSERTIONS,
        &filter_eq("OBJECT(?t1) = OBJECT(?t2)"),
    );
    assert_eq!(equal.len(), expected_equal, "rows={equal:#?}");

    let same_term = assert_oracle_agrees(
        CENSUS_SQL,
        CENSUS_R2RML_TWO_ASSERTIONS,
        &filter_eq("sameTerm(OBJECT(?t1), OBJECT(?t2))"),
    );
    assert_eq!(same_term.len(), expected_equal, "rows={same_term:#?}");
}

/// Companions [`reifies_object_variable_projects_as_a_native_triple_term`] —
/// `?r rdf:reifies ?t` with `?t` projected: native TT binding equality
/// against spareval's OWN native TT binding (both sides genuinely native
/// here, unlike the hand-computed test, which only asserts the ENGINE side
/// is native).
#[test]
fn reifies_object_variable_projection_oracle_agrees() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?t WHERE {{ ?r rdf:reifies ?t }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    for row in &rows {
        assert!(matches!(row["t"], Term::Triple(_)), "row={row:#?}");
    }
}

/// Companions [`construct_template_quoting_a_triple_in_object_position_produces_real_triples`]
/// — CONSTRUCT producing TT objects: graph isomorphism against spareval's OWN
/// CONSTRUCT output (both sides independently build the triple-term object).
#[test]
fn construct_object_position_triple_term_oracle_agrees() {
    let query = format!(
        "{EX}CONSTRUCT {{ ?p ex:hasQuote <<( ?p ex:hasAge ?age )>> }} \
         WHERE {{ ?p ex:hasAge ?age }}"
    );
    let triples = assert_oracle_agrees_construct(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(triples.len(), 3, "triples={triples:#?}");
}

// ============================================================================
// F4a Bug 3 — ADR-0032 D3 cross-boundary gap (confirmed and designed by a
// prior review pass; see `sf_sparql::star::apply_composed_bindings`'s own doc
// comment for the full analysis this test proves). When a composed
// (triple-term) variable is one of a SubPlan's declared `vars` but its
// component vars (`s_var`/`p_var`/`o_var`) are NOT, `iq::lower::lower_as_subplan`
// used to freeze the outer positional-column remap from the arm's RAW
// (pre-composition) binding — projecting the internal
// `urn:sf-star:pf:...`-shaped proposition-form identity `NamedNode` instead
// of a native `Term::Triple`. This is TREE-ONLY (`lower_as_subplan` is
// exclusively tree machinery — flat has no derived-table/positional-column
// abstraction to lose the components across), so this test drives
// `translate_with` (tree) directly rather than the shared `diff()` helper
// (which requires flat/tree parity — not the property being tested here).
// ============================================================================

/// The team-lead's exact confirmed repro: `?t` is the SubPlan's own declared
/// `vars` entry (`SELECT DISTINCT ?t`), cross-joined (no shared variable) with
/// an outer `?p ex:knows ?friend` pattern, and projected. `ex:knows` edges
/// (row 2's NULL `friend_id` excluded, R2RML §11 / Bug 1 above): (1,2) (3,1)
/// — 2 rows. Distinct `?t` values: one native quoted triple per census row's
/// `#PersonAgeAssertion` (3 rows, subjects differ even where ages repeat). No
/// shared variable between the two sides ⇒ a plain cross product: 2 * 3 = 6
/// rows, every one of which must carry a genuine `Term::Triple` for `?t`.
#[test]
fn composed_var_crossing_subplan_boundary_projects_as_native_triple_term() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?t ?friend WHERE {{ \
           ?p ex:knows ?friend . \
           {{ SELECT DISTINCT ?t WHERE {{ ?r rdf:reifies ?t }} }} \
         }}"
    );
    let maps = sf_mapping::parse_r2rml(CENSUS_R2RML).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(&query)
        .expect("query parses");
    let conn = sqlite::load(CENSUS_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let tree = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("tree must answer this SubPlan-crossing composed var");
    let got = run_select(&tree, &conn);

    assert_eq!(
        got.len(),
        6,
        "2 ex:knows edges * 3 distinct ?t values: got={got:#?}"
    );
    for row in &got {
        assert!(
            matches!(row.get("t"), Some(Term::Triple(_))),
            "?t crossing the SubPlan boundary must reconstruct as a native Term::Triple, \
             never the raw internal proposition-form identity IRI: row={row:#?}"
        );
    }

    let oracle_rows = oracle_star_bag(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(
        oracle::solutions_bag_eq(&got, &oracle_rows),
        "engine vs decoded-graph oracle divergence:\n engine={got:#?}\n oracle={oracle_rows:#?}"
    );
}

// ============================================================================
// Wave A3 (Run 4) — co-identification, GENERAL mappings (the RDF-star ledger's
// open item 6, and the general-mapping half of the oracle Wave 3 never tried:
// every fixture above quotes a shape from exactly ONE logical source, with
// zero duplicate rows). Two independent questions, both answered here with
// hard evidence, NOT hypothesized:
//
// (a) Two INDEPENDENT triples maps, on DIFFERENT logical sources, quoting
//     "the same shape": `cross_source_same_actual_triple_*` (below) confirms
//     co-identity's CORRECTNESS claim holds — genuinely the same real triple,
//     asserted from two unrelated tables, collapses to ONE proposition,
//     realized as the identical `Term::Triple`. `cross_source_colliding_
//     shape_*` originally found a SEPARATE, sharper problem one layer down:
//     `ids::proposition_template` (`sf-mapping/src/r2rml/star/ids.rs`) built
//     a proposition id from the predicate slug plus each term map's
//     referenced COLUMN VALUES only, so a quoted map's OWN subject-template
//     literal prefix (e.g. `http://ex.org/person/`) never reached the id at
//     all — "the same shape" as implemented meant "same predicate + same
//     column ARITY", not "the same triple", so two UNRELATED quoted maps
//     over unrelated entities could mint the IDENTICAL proposition id
//     whenever a row from each happened to carry equal column values, and
//     the query engine genuinely cross-attributed data between them (not
//     just an oracle-decode technicality). **Run 4 Fix-1 closes this**:
//     `ids::push_term` (same file) now folds each component's FULL rendered
//     lexical form into the id — a template's own literal segments spliced
//     in verbatim alongside its columns, an `rr:constant`'s value rendered
//     and percent-encoded in, and a fixed term-kind/datatype/language tag —
//     so id-equality implies decoded-triple-equality by construction (RDF
//     1.2 Semantics §5's injective `IT`). `cross_source_colliding_shape_*`
//     (below) now confirms the fix instead: the decoder no longer finds an
//     ambiguous `PropositionForm` node, and the engine no longer
//     cross-attributes a reifier to the wrong source's triple.
// (b) A single source with a literal duplicate row feeding one quoted shape:
//     `duplicate_source_row_*` measures a large, real bag-multiplicity gap
//     between the engine's SQL-joined answer and the decoded (set-based)
//     native graph, confirms it is NOT star-specific (the plain-pattern
//     baseline already shows it, smaller), and shows the star rewrite's
//     extra shared-variable join positions (the 4 basic-encoding predicates,
//     doubled again by nesting) amplify it multiplicatively. UNCHANGED by
//     Fix-1 — a separate bug in `sf-sparql`'s unification, not the mapping
//     layer's id construction, being designed separately (see below).
// (c) The cross-product (`cross_source_with_duplicate_*`): co-identity's
//     correctness (a) survives being layered on top of a duplicate row (b)
//     unchanged — but the two mechanisms' multiplicities COMPOUND. Also
//     unchanged by Fix-1.
//
// Root cause of (b)/(c)'s multiplicity (Fix-1 does NOT touch this — a
// separate, `sf-sparql`-side bug, deliberately left red here): a star-
// rewritten BGP's shared `?pf`/`?r` variables are matched by the GENERAL
// "overlapping candidate triples maps" unification (`sf-sparql`'s
// `unify`/`unfold`) the SAME way any two ordinary, independently-asserting
// triples maps would be — but nothing in that unification tracks "these N
// pattern positions must all resolve through the SAME physical source row";
// it only requires the shared variable's SQL VALUE to agree at each position
// independently. When exactly one candidate row's value matches, this is
// unobservable. When TWO OR MORE rows (duplicate physical rows) produce the
// identical shared value, every one of the (4, or more with nesting/
// reifies/assertedBy) description-predicate positions independently
// re-picks among them, so the combinations multiply. Before Fix-1, (a)'s id
// collision manufactured this SAME trigger condition too (two UNRELATED
// rows sharing one non-injective `?pf` value, exactly like a duplicate row
// would) — which is why the pre-fix (a) cells additionally bound genuinely
// WRONG data, not just duplicate correct rows: their candidates' OTHER
// (non-shared) components actually differed. Fix-1 closes that pathway by
// making `?pf` injective; (b)/(c)'s own direct pathway (a genuine duplicate
// physical row within one source) is untouched and remains open.
// ============================================================================

// --- (a) POSITIVE: co-identity's own correctness claim, across sources -----

const CROSS_SOURCE_SQL: &str = r#"
CREATE TABLE people_2020 (person_id INTEGER PRIMARY KEY, age INTEGER NOT NULL);
INSERT INTO people_2020 VALUES (1, 30);
INSERT INTO people_2020 VALUES (2, 40);

CREATE TABLE people_2021 (person_id INTEGER PRIMARY KEY, age INTEGER NOT NULL);
INSERT INTO people_2021 VALUES (1, 30);
INSERT INTO people_2021 VALUES (3, 25);
"#;

/// `#PersonAge2020`/`#PersonAge2021`: two INDEPENDENT quoted triples maps,
/// each its own logical source, sharing the identical subject template,
/// predicate, and object column — genuinely the same shape. Person 1 (age
/// 30) appears in BOTH sources: the SAME real triple
/// `<<http://ex.org/person/1 ex:hasAge 30>>`, asserted twice from two
/// unrelated tables. Persons 2 (2020-only, age 40) and 3 (2021-only, age 25)
/// are the negative control — no cross-source overlap in the DATA, so no
/// cross-source multiplicity for them either.
const CROSS_SOURCE_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge2020>
    rr:logicalTable [ rr:tableName "people_2020" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#AssertFrom2020>
    rr:logicalTable [ rr:tableName "people_2020" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#PersonAge2020> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:Src2020 ]
    ] .

<#PersonAge2021>
    rr:logicalTable [ rr:tableName "people_2021" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#AssertFrom2021>
    rr:logicalTable [ rr:tableName "people_2021" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#PersonAge2021> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:Src2021 ]
    ] .
"#;

/// Co-identity's CORRECTNESS claim, verified: the DISTINCT (deduplicated)
/// solution set is exactly the 4 semantically-right rows — person 1
/// co-identifies to ONE proposition with TWO per-declaration reifiers (the
/// general-mapping generalization of `reifier_multiplicity_two_star_maps_
/// same_shape_yield_distinct_reifiers`, which only ever tries this within a
/// SINGLE source); persons 2/3 each keep exactly their own single reifier,
/// with NO spurious cross-identification. Deduplicated deliberately — see
/// `cross_source_same_actual_triple_bag_multiplicity_diverges_from_oracle`
/// immediately below for the SEPARATE finding that the RAW (non-dedup'd) bag
/// is wrong (measured 34 rows, not 4).
#[test]
fn cross_source_same_actual_triple_coidentifies_correctly() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?r ?src WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
           ?r ex:assertedBy ?src \
         }}"
    );
    let got = diff(CROSS_SOURCE_SQL, CROSS_SOURCE_R2RML, &query);
    let mut distinct: Vec<&BTreeMap<String, Term>> = Vec::new();
    for row in &got {
        if !distinct.contains(&row) {
            distinct.push(row);
        }
    }
    assert_eq!(distinct.len(), 4, "distinct combos={distinct:#?}");

    let mut by_person: BTreeMap<NamedNode, Vec<&&BTreeMap<String, Term>>> = BTreeMap::new();
    for row in &distinct {
        let p = match &row["p"] {
            Term::NamedNode(n) => n.clone(),
            other => panic!("?p must be an IRI, got {other:?}"),
        };
        by_person.entry(p).or_default().push(row);
    }
    assert_eq!(by_person.len(), 3, "3 distinct persons: {by_person:#?}");
    let person1 = NamedNode::new_unchecked("http://ex.org/person/1");
    let p1_rows = &by_person[&person1];
    assert_eq!(
        p1_rows.len(),
        2,
        "person 1 (age 30 in BOTH sources) must co-identify to ONE proposition with TWO \
         per-declaration reifiers: {p1_rows:#?}"
    );
    assert_ne!(
        p1_rows[0]["r"], p1_rows[1]["r"],
        "the two reifiers sharing person 1's cross-source proposition must be distinct IRIs"
    );
    let src_2020 = iri("http://example.com/Src2020");
    let src_2021 = iri("http://example.com/Src2021");
    assert!(
        p1_rows.iter().any(|r| r["src"] == src_2020)
            && p1_rows.iter().any(|r| r["src"] == src_2021),
        "person 1 must have exactly one reifier from EACH source: {p1_rows:#?}"
    );
    let person2 = NamedNode::new_unchecked("http://ex.org/person/2");
    assert_eq!(
        by_person[&person2].len(),
        1,
        "person 2 (2020-only) must NOT spuriously cross-identify: {:#?}",
        by_person[&person2]
    );
    let person3 = NamedNode::new_unchecked("http://ex.org/person/3");
    assert_eq!(
        by_person[&person3].len(),
        1,
        "person 3 (2021-only) must NOT spuriously cross-identify: {:#?}",
        by_person[&person3]
    );
}

/// The sharpest direct proof of co-identity: person 1's two reifiers (one per
/// source) must bind `?t` to the exact SAME native `Term::Triple` value, not
/// merely "the SQL join happened to correlate them". `?age`'s exact literal
/// form comes from `baseline_ages` (this file's established rule: never
/// hand-type an `rr:column`-sourced XSD lexical form).
#[test]
fn cross_source_same_actual_triple_composed_term_is_structurally_identical() {
    let ages = baseline_ages(CROSS_SOURCE_SQL, CROSS_SOURCE_R2RML);
    let age30 = ages
        .get(&NamedNode::new_unchecked("http://ex.org/person/1"))
        .expect("person 1 has a baseline age")
        .clone();
    let expected_t = Term::Triple(Box::new(Triple::new(
        NamedNode::new_unchecked("http://ex.org/person/1"),
        NamedNode::new_unchecked("http://example.com/hasAge"),
        age30,
    )));

    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r ?src ?t WHERE {{ ?r rdf:reifies ?t . ?r ex:assertedBy ?src }}"
    );
    let got = diff(CROSS_SOURCE_SQL, CROSS_SOURCE_R2RML, &query);
    let src_2020 = iri("http://example.com/Src2020");
    let src_2021 = iri("http://example.com/Src2021");
    assert!(
        got.iter()
            .any(|r| r["src"] == src_2020 && r["t"] == expected_t),
        "2020's person-1 reifier must bind ?t to the expected triple: got={got:#?}"
    );
    assert!(
        got.iter()
            .any(|r| r["src"] == src_2021 && r["t"] == expected_t),
        "2021's person-1 reifier must bind ?t to the IDENTICAL expected triple (co-identity): \
         got={got:#?}"
    );
}

/// The SEPARATE, real problem the two tests above deliberately dedup around:
/// the RAW engine bag for this exact query and fixture is 34 rows, not 4 —
/// measured breakdown: person 1's two (semantically CORRECT) combos each
/// appear **16 times** (32 total), while persons 2/3 (no cross-source
/// overlap, so no colliding candidate row) appear exactly once each. The
/// decoded-graph oracle (a real `oxrdf::Dataset`, which is a SET — duplicate
/// quads collapse) correctly returns 4. Root cause: see this section's own
/// header comment — every one of the 4 basic-encoding description patterns
/// independently re-picks between `desc(#PersonAge2020)` and
/// `desc(#PersonAge2021)` for person 1's shared `?pf` value (both genuinely
/// produce it), and nothing constrains all 4 (plus the `rdf:reifies` and
/// `ex:assertedBy` patterns) to agree on ONE candidate's row identity.
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn cross_source_same_actual_triple_bag_multiplicity_diverges_from_oracle() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?r ?src WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
           ?r ex:assertedBy ?src \
         }}"
    );
    assert_oracle_agrees(CROSS_SOURCE_SQL, CROSS_SOURCE_R2RML, &query);
}

/// The bare reifies-sugar surface form (`<<?p ex:hasAge ?age>>`, no
/// parentheses) over the SAME fixture — confirms the bag-multiplicity finding
/// is not an artifact of the parenthesized `<<( )>>` TripleTerm spelling
/// specifically.
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn cross_source_bare_reifies_sugar_bag_multiplicity_diverges_from_oracle() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<?p ex:hasAge ?age>> ex:assertedBy ?src }}");
    assert_oracle_agrees(CROSS_SOURCE_SQL, CROSS_SOURCE_R2RML, &query);
}

// --- (a) ADVERSARIAL: superficial shape collision, genuinely different -----
// --- triples (this WAS a real bug, confirmed both at decode time and at ---
// --- ordinary query time — FIXED by Run 4 Fix-1, see the two tests below) -

const COLLIDING_SHAPE_SQL: &str = r#"
CREATE TABLE people (person_id INTEGER PRIMARY KEY, age INTEGER NOT NULL);
INSERT INTO people VALUES (1, 30);

CREATE TABLE widgets (widget_id INTEGER PRIMARY KEY, weight INTEGER NOT NULL);
INSERT INTO widgets VALUES (1, 30);
"#;

/// `#Person`/`#Widget`: two UNRELATED quoted triples maps (different logical
/// sources, different subject-template NAMESPACES: `.../person/{person_id}`
/// vs `.../widget/{widget_id}`) that merely happen to share a predicate
/// (`ex:hasValue` — ordinary predicate reuse across classes, e.g. exactly how
/// `rdfs:label` or a generic `ex:hasValue` gets reused in real ontologies)
/// and column ARITY (one subject column, one plain object column), with row
/// values that ALSO coincide (1, 30 for both) — chosen specifically so that,
/// pre-Fix-1, the two maps' proposition ids collided
/// (`urn:sf-star:pf:ex_hasValue|1|30|` for BOTH — predicate slug plus raw
/// column VALUES only) despite denoting GENUINELY DIFFERENT triples:
/// `<<http://ex.org/person/1 ex:hasValue 30>>` vs
/// `<<http://ex.org/widget/1 ex:hasValue 30>>`. Post-fix, `ids::push_term`'s
/// full-lexical-form treatment keeps the two subject templates' own literal
/// prefixes (`.../person/` vs `.../widget/`) in the id — the actual ids are
/// now `urn:sf-star:pf:http_example_com_hasValue|I|http://ex.org/person/1|L|30|`
/// and `.../widget/1|L|30|` — so they no longer collide; see the two tests
/// below.
const COLLIDING_SHAPE_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#Person>
    rr:logicalTable [ rr:tableName "people" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasValue ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#PersonAssertion>
    rr:logicalTable [ rr:tableName "people" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#Person> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:PersonSrc ]
    ] .

<#Widget>
    rr:logicalTable [ rr:tableName "widgets" ] ;
    rr:subjectMap [ rr:template "http://ex.org/widget/{widget_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasValue ;
        rr:objectMap [ rr:column "weight" ]
    ] .

<#WidgetAssertion>
    rr:logicalTable [ rr:tableName "widgets" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#Widget> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:WidgetSrc ]
    ] .
"#;

/// **FIXED by Run 4 Fix-1** (`ids::push_term`, `sf-mapping/src/r2rml/star/
/// ids.rs`) — previously a **CONFIRMED BUG** (not merely theoretical —
/// reproduced): materializing this mapping's full graph and decoding it
/// (`sf_conformance::star_decode`, the same decoder every oracle cell in
/// this file relies on) used to fail. The dumped graph literally contained
/// `<urn:sf-star:pf:...hasValue|1|30|> rdf:propositionFormSubject
/// <http://ex.org/widget/1>` AND `... rdf:propositionFormSubject
/// <http://ex.org/person/1>` on the SAME pfid node (one triple from each
/// quoted map's own standalone description map, `sf-mapping/src/r2rml/
/// star.rs`'s `quote_shape` — each keyed on ITS OWN `quoted_id`, never
/// deduplicated against the OTHER map, since nothing compared pfid VALUES
/// across declarations). `decode_proposition_forms`'s `one_component` check
/// correctly rejected this ("2 rdf:propositionFormSubject components,
/// expected exactly one") — proving the decode contract (ADR-0032 D2,
/// "report an error if it cannot unambiguously determine s, p, or o") was
/// violated by an entirely ordinary mapping shape (two classes reusing one
/// predicate), not an adversarially malformed one: R2 ("the two [id]
/// families never collide") did not hold as implemented.
///
/// Now that `ids::push_term` folds each component's FULL rendered lexical
/// form into the id (see `COLLIDING_SHAPE_R2RML`'s own doc comment for the
/// two new, distinct ids), this decodes cleanly into TWO separate
/// `PropositionForm` nodes, restoring R2 for this previously-colliding shape.
#[test]
fn cross_source_colliding_shape_decode_finds_no_ambiguous_proposition_form_node() {
    let conn = sqlite::load(COLLIDING_SHAPE_SQL).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(COLLIDING_SHAPE_R2RML).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let encoded = graph::quads_to_dataset(&quads);
    let decoded = decode_proposition_forms(&encoded);
    assert!(
        decoded.is_ok(),
        "two UNRELATED quoted shapes sharing a predicate + column arity must mint DISTINCT \
         urn:sf-star:pf: ids for GENUINELY DIFFERENT triples (RDF 1.2 Semantics §5's injective \
         IT) — an ambiguous PropositionForm node means they collided again: {decoded:?}"
    );
}

/// **FIXED by Run 4 Fix-1** — previously a **CONFIRMED BUG**, reproduced
/// through ORDINARY querying (not just the oracle's decode step above):
/// `?r`'s own id family never collided (it is keyed on the DECLARING map,
/// `sf-mapping/src/r2rml/star/ids.rs`'s `reifier_template`, unchanged by
/// Fix-1), so this query anchors on `?r` first. Every row where `?src =
/// ex:PersonSrc` must reify the PERSON triple; every row where `?src =
/// ex:WidgetSrc` must reify the WIDGET triple — pre-fix this failed on BOTH
/// flat AND tree (which additionally DISAGREED with each other here: 64 rows
/// flat vs 32 rows tree — a symptom of the SAME non-injective `?pf` value
/// feeding both translators' shared candidate-unification machinery, not an
/// independent flat/tree bug of its own): `PersonAssertion`'s own reifier
/// (`src = PersonSrc`) used to bind `?s` to `http://ex.org/widget/1` — i.e.
/// person 1's own reifier was reported as reifying WIDGET's triple, one it
/// had nothing to do with. Genuine cross-source data corruption reaching the
/// engine's real query answers, not merely an oracle-construction
/// technicality.
///
/// Now that `?pf` is injective (see the decode-level companion above), flat
/// and tree agree (2 rows each) and both attribute correctly, so this uses
/// `diff()` like the rest of the file rather than the old manual per-engine
/// check this test used while the two translators disagreed.
#[test]
fn cross_source_colliding_shape_engine_attributes_reifiers_to_the_correct_source() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r ?src ?s ?v WHERE {{ \
           ?r rdf:reifies ?t . ?r ex:assertedBy ?src . \
           BIND(SUBJECT(?t) AS ?s) . BIND(OBJECT(?t) AS ?v) \
         }}"
    );
    let person = iri("http://ex.org/person/1");
    let widget = iri("http://ex.org/widget/1");
    let person_src = iri("http://example.com/PersonSrc");
    let widget_src = iri("http://example.com/WidgetSrc");
    let got = diff(COLLIDING_SHAPE_SQL, COLLIDING_SHAPE_R2RML, &query);
    assert_eq!(got.len(), 2, "one reifier per source: got={got:#?}");
    for row in &got {
        if row["src"] == person_src {
            assert_eq!(
                row["s"], person,
                "a PersonSrc-asserted reifier must reify the PERSON triple, not the widget's: \
                 row={row:#?}"
            );
        } else if row["src"] == widget_src {
            assert_eq!(
                row["s"], widget,
                "a WidgetSrc-asserted reifier must reify the WIDGET triple, not the person's: \
                 row={row:#?}"
            );
        } else {
            panic!("unexpected src: {row:#?}");
        }
    }
}

// --- (b) Literal duplicate row in ONE source: bag multiplicity ------------

/// `census_row`'s own shape (matches `CENSUS_SQL`/`CENSUS_R2RML` exactly, so
/// `CENSUS_R2RML` and `CENSUS_R2RML_OBJECT`/`STAR_NESTED_DEPTH2_R2RML` are all
/// reused unchanged below), but person 1's row is LITERALLY duplicated — the
/// SAME triples map, the SAME underlying fact, asserted twice by two
/// physically distinct rows (not two different maps, unlike (a) above).
/// `person_id` deliberately has no `PRIMARY KEY`/uniqueness constraint, so
/// SQLite accepts the exact duplicate.
const CENSUS_SQL_DUPLICATE_ROW: &str = r#"
CREATE TABLE census_row (
    person_id INTEGER,
    age INTEGER NOT NULL,
    friend_id INTEGER
);
INSERT INTO census_row VALUES (1, 30, 2);
INSERT INTO census_row VALUES (1, 30, 2);
INSERT INTO census_row VALUES (2, 40, NULL);
INSERT INTO census_row VALUES (3, 30, 1);
"#;

/// Root-cause isolation, run FIRST: is bag-multiplicity from a literal
/// duplicate row a star-rewrite artifact, or does it already exist in
/// ORDINARY (non-star) BGP matching? A plain `?p ex:hasAge ?age` pattern (no
/// star machinery at all) is the control — **measured 4 rows, not 3**: even
/// ordinary R2RML row-to-triple mapping does not deduplicate a literal
/// duplicate source row into one triple under set-based RDF semantics. This
/// is the SAME general "not star-specific" mechanism the RDF-star ledger's
/// item 6 flagged and deferred; the star-specific cells below show the
/// SAME root cause amplified by the extra shared-variable join positions a
/// star rewrite introduces (66 rows for the analogous star query, not 4).
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn duplicate_source_row_plain_pattern_baseline_bag_multiplicity() {
    let query = format!("{EX}SELECT ?p ?age WHERE {{ ?p ex:hasAge ?age }}");
    assert_oracle_agrees(CENSUS_SQL_DUPLICATE_ROW, CENSUS_R2RML, &query);
}

/// The star reifies/TripleTerm-pattern form over the SAME duplicated-row
/// fixture — **measured 66 engine rows vs 3 correct** (a real
/// `differential_star`-visible divergence; `diff()` inside `assert_oracle_
/// agrees` confirms flat and tree AGREE with each other on the wrong 66, so
/// this is a bug shared by both translators' common candidate-unification
/// machinery, exactly the blind spot this file's own module doc names: "a
/// bug shared by both translators... would sail through undetected" by the
/// flat/tree differential alone — only the independent oracle catches it).
/// 66 = 64 (person 1: all 6 shared-variable positions — `rdf:reifies`, the 4
/// description predicates, `ex:assertedBy` — independently re-pick between
/// person 1's 2 duplicate candidate rows, 2^6) + 1 (person 2) + 1 (person 3).
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn duplicate_source_row_reifies_triple_term_pattern_bag_multiplicity() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?r ?src WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
           ?r ex:assertedBy ?src \
         }}"
    );
    assert_oracle_agrees(CENSUS_SQL_DUPLICATE_ROW, CENSUS_R2RML, &query);
}

/// Object-position TripleTerm match (`#Quote`'s own shape, `CENSUS_R2RML_
/// OBJECT`, reused unchanged — it maps the SAME `census_row` table) —
/// measured 34 engine rows vs 3 correct, confirming the mechanism is not
/// specific to the reifies/subject-position surface form.
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn duplicate_source_row_object_position_triple_term_bag_multiplicity() {
    let query =
        format!("{EX}SELECT ?q ?p ?age WHERE {{ ?q ex:hasQuote <<( ?p ex:hasAge ?age )>> }}");
    assert_oracle_agrees(CENSUS_SQL_DUPLICATE_ROW, CENSUS_R2RML_OBJECT, &query);
}

/// Annotation-sugar surface form (`s p o {| ... |}`) — measured 130 engine
/// rows vs 3 correct, the WORST-observed multiplier: annotation sugar
/// desugars to three conjuncts (the plain triple, a fresh reifier, and the
/// annotation's own POM), so it carries even MORE shared-variable positions
/// than the bare reifies form for the SAME duplicate candidate set to
/// combine across.
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn duplicate_source_row_annotation_sugar_bag_multiplicity() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ ?p ex:hasAge ?age {{| ex:assertedBy ?src |}} }}");
    assert_oracle_agrees(CENSUS_SQL_DUPLICATE_ROW, CENSUS_R2RML, &query);
}

/// CONSTRUCT round-trip — measured 4 engine triples vs 3 correct. Notably the
/// SMALLEST divergence of this group: this CONSTRUCT's WHERE clause
/// (`?p ex:hasAge ?age`) is the PLAIN pattern, not a star pattern (the
/// quoting is only in the TEMPLATE) — so it inherits exactly the baseline's
/// own 4-vs-3 gap, not the star-amplified one, and — unlike every SELECT cell
/// above — the produced `Vec<Triple>` is compared directly (this file's
/// `diff_construct`/`assert_oracle_agrees_construct` doc comments both note
/// multiplicity IS preserved, "though this suite's cases don't happen to
/// produce duplicates" — this is now the first case that does).
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn duplicate_source_row_construct_round_trip_bag_multiplicity() {
    let query = format!(
        "{EX}CONSTRUCT {{ ?p ex:hasQuote <<( ?p ex:hasAge ?age )>> }} \
         WHERE {{ ?p ex:hasAge ?age }}"
    );
    let engine = diff_construct(CENSUS_SQL_DUPLICATE_ROW, CENSUS_R2RML, &query);
    let oracle_graph = match oracle_star(CENSUS_SQL_DUPLICATE_ROW, CENSUS_R2RML, &query) {
        OracleAnswer::Graph(g) => *g,
        other => panic!("expected Graph, got {other:?}"),
    };
    assert_eq!(
        engine.len(),
        oracle_graph.len(),
        "engine triples={engine:#?}\noracle graph len={}",
        oracle_graph.len()
    );
}

/// One nested shape (depth 2, `STAR_NESTED_DEPTH2_R2RML` reused unchanged —
/// `#Leaf`/`#Mid`/`#Outer` all map `census_row`, so the duplicate row feeds
/// the LEAF quote too) — measured 514 engine rows vs 3 correct, the WORST of
/// this group by far: nesting doubles the number of shared-variable
/// description-predicate positions (4 for `#Mid`'s own shape, 4 more for the
/// nested `#Leaf`), so the SAME 2-candidate-row collision combines across
/// far more join positions.
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn duplicate_source_row_nested_shape_bag_multiplicity() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?r ?p ?leaf ?score WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge <<( ?leaf ex:hasScore ?score )>> )>> \
         }}"
    );
    assert_oracle_agrees(CENSUS_SQL_DUPLICATE_ROW, STAR_NESTED_DEPTH2_R2RML, &query);
}

// --- (c) Cross-product: co-identity across sources, one source dup'd ------

const CROSS_SOURCE_ONE_DUP_SQL: &str = r#"
CREATE TABLE people_a (person_id INTEGER, age INTEGER NOT NULL);
INSERT INTO people_a VALUES (1, 30);
INSERT INTO people_a VALUES (1, 30);

CREATE TABLE people_b (person_id INTEGER, age INTEGER NOT NULL);
INSERT INTO people_b VALUES (1, 30);
"#;

/// (a)'s positive cross-source shape (`#PersonAgeA`/`#PersonAgeB`, identical
/// template/predicate/column, genuinely the same real triple) layered with
/// (b)'s literal duplicate row — source A's own row for person 1 is ALSO
/// physically duplicated.
const CROSS_SOURCE_ONE_DUP_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAgeA>
    rr:logicalTable [ rr:tableName "people_a" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#AssertFromA>
    rr:logicalTable [ rr:tableName "people_a" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#PersonAgeA> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:SrcFromA ]
    ] .

<#PersonAgeB>
    rr:logicalTable [ rr:tableName "people_b" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#AssertFromB>
    rr:logicalTable [ rr:tableName "people_b" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#PersonAgeB> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:SrcFromB ]
    ] .
"#;

/// Co-identity's correctness claim STILL holds with a duplicate layered in:
/// the reifier id is likewise a pure function of `(outer_tm_id, row values)`,
/// so source A's two duplicate rows collapse to the SAME reifier id — the
/// DISTINCT (deduplicated) solution set is exactly 2 rows (one reifier per
/// source, both reifying the identical co-identified proposition), not 3.
#[test]
fn cross_source_with_duplicate_reifier_identity_stays_correct() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?r ?src WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
           ?r ex:assertedBy ?src \
         }}"
    );
    let got = diff(CROSS_SOURCE_ONE_DUP_SQL, CROSS_SOURCE_ONE_DUP_R2RML, &query);
    let mut distinct: Vec<&BTreeMap<String, Term>> = Vec::new();
    for row in &got {
        if !distinct.contains(&row) {
            distinct.push(row);
        }
    }
    assert_eq!(
        distinct.len(),
        2,
        "co-identification must still collapse to exactly 2 DISTINCT reifiers (one per \
         source) despite source A's internal row duplication: distinct combos={distinct:#?}"
    );
    assert_ne!(
        distinct[0]["r"], distinct[1]["r"],
        "distinct combos={distinct:#?}"
    );
    let src_a = iri("http://example.com/SrcFromA");
    let src_b = iri("http://example.com/SrcFromB");
    assert!(
        distinct.iter().any(|r| r["src"] == src_a) && distinct.iter().any(|r| r["src"] == src_b),
        "distinct combos={distinct:#?}"
    );
}

/// The two mechanisms' multiplicities COMPOUND: measured 405 engine rows vs
/// 2 correct — far worse than either (a)'s 34 or (b)'s 66 alone, consistent
/// with this section's header comment ("nothing constrains all [join
/// positions] to agree on ONE candidate's row identity"): here BOTH sources
/// contribute a colliding candidate (the cross-source co-identity itself, by
/// design) AND source A alone contributes a second, independent duplicate-row
/// collision on top.
#[test]
#[ignore = "ADR-0034 red phase (virtual-graph set semantics) — un-ignored by Run 4 Wave C0"]
fn cross_source_with_duplicate_bag_multiplicity_diverges_from_oracle() {
    let query = format!(
        "{EX}PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         SELECT ?p ?age ?r ?src WHERE {{ \
           ?r rdf:reifies <<( ?p ex:hasAge ?age )>> . \
           ?r ex:assertedBy ?src \
         }}"
    );
    assert_oracle_agrees(CROSS_SOURCE_ONE_DUP_SQL, CROSS_SOURCE_ONE_DUP_R2RML, &query);
}
