//! W5 TEST-SOUNDNESS AUDIT — gap-killing cells (Run 5).
//!
//! This file is NOT a new feature lock: it exists to CLOSE soundness/coverage
//! holes the W5 audit found in the EXISTING conformance suite. Each cell here
//! either (a) kills a mutation that the current suite lets survive silently, or
//! (b) strengthens a structurally-false-green shape (a `diff().is_empty()` cell
//! whose mutual-501 arm passes vacuously when both engines refuse). Every cell
//! is GREEN against current code and was verified to RED under the exact
//! regression it guards (mutation verdicts are in the accompanying W5 report).
//!
//! Copied-helper pattern: a Cargo integration test is its own crate, so the
//! harness plumbing is duplicated from `differential_star.rs` /
//! `adversarial_run4b_refute.rs` rather than imported (no `pub` cross-file
//! surface). The oracle is the mapping's OWN materialization
//! (`exec::dump_quads` → `graph::quads_to_dataset`, a genuine SET) — so
//! materialization is the reference and the SQL-rewrite (OBDA) path is the
//! system-under-test.

use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::star_decode::decode_proposition_forms;
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, Tbox};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};
use std::collections::BTreeMap;

use oxrdf::{Dataset, Term};

const EX: &str = "PREFIX ex: <http://example.com/> ";

/// A SELECT answer as a comparable bag (the suite's `oracle::solutions_bag_eq`
/// operand shape).
type Rows = Vec<BTreeMap<String, Term>>;

// ============================================================================
// Harness plumbing — duplicated from `differential_star.rs` (separate test
// binary; no `pub` cross-file surface to import). Semantics identical.
// ============================================================================

fn parse(q: &str) -> Query {
    SparqlParser::new().parse_query(q).expect("query parses")
}

fn schema_of(create: &str) -> Vec<TableSchema> {
    let conn = sqlite::load(create).expect("fixture loads");
    sqlite::introspect_all(&conn).expect("introspection")
}

/// Translate a query through BOTH engines (flat `=_bag` oracle/fallback and the
/// production tree path). Loads the fixture only for its schema.
fn translate_both(
    create: &str,
    r2rml: &str,
    query: &str,
) -> (sf_sparql::Result<Plan>, sf_sparql::Result<Plan>) {
    let schema = schema_of(create);
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);
    let f = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    let t = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    (f, t)
}

fn run_select(plan: &Plan, conn: &Connection) -> Vec<BTreeMap<String, Term>> {
    oracle::engine_bag(&exec::select(plan, conn).expect("select exec"))
}

/// The mapping's own materialization, decoded to native RDF 1.2 form (ADR-0032
/// D2). Returns the decoder's `Result` UNWRAPPED-INTO-CALLER so a cell can
/// assert on decode success/failure directly (unlike `differential_star.rs`'s
/// `decoded_graph`, which `.expect()`s — that would panic on the very collision
/// a soundness cell wants to observe as a red assertion).
fn decode_materialized(create: &str, r2rml: &str) -> Result<Dataset, String> {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    decode_proposition_forms(&graph::quads_to_dataset(&quads))
}

/// The spareval oracle's SELECT row bag over the materialized (set) graph.
fn oracle_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let g = graph::quads_to_dataset(&quads);
    match oracle::evaluate(&g, query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected Solutions, got {other:?}"),
    }
}

/// Strengthened `diff`: BOTH engines MUST translate WITHOUT a 501, then returns
/// `(flat_rows, tree_rows)`. This is the anti-false-green primitive: the
/// `diff().is_empty()` shape three `differential_star.rs` cells use silently
/// passes when BOTH engines return `Unsupported` (its mutual-501 arm yields
/// `Vec::new()`), so a regression to 501 is indistinguishable from a correct
/// empty answer. Forcing a translation here makes a 501 a hard failure.
fn require_translate_then_rows(create: &str, r2rml: &str, query: &str) -> (Rows, Rows) {
    let (flat, tree) = translate_both(create, r2rml, query);
    let fp = flat.unwrap_or_else(|e| {
        panic!(
            "flat 501'd `{query}` — a `diff().is_empty()` cell would MASK this regression as \
             green (mutual-501 arm returns an empty bag); {e:?}"
        )
    });
    let tp = tree.unwrap_or_else(|e| {
        panic!(
            "tree 501'd `{query}` — a `diff().is_empty()` cell would MASK this regression as \
             green; {e:?}"
        )
    });
    let conn = sqlite::load(create).expect("fixture loads");
    (run_select(&fp, &conn), run_select(&tp, &conn))
}

// ============================================================================
// Fixtures copied verbatim from `differential_star.rs` (the three false-green
// cells' own fixtures) + this file's own two new ones.
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
"#;

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

// Cell A fixture: two propositions differing ONLY by object term-KIND (IRI vs
// Literal) with the SAME lexical text — the id-collision `ids::kind_tag` is the
// sole defense against.
const KIND_TAG_SQL: &str = r#"
CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL);
INSERT INTO t VALUES (1, 'http://ex.org/x');
"#;

const KIND_TAG_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PropIri>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rr:template "http://ex.org/s/{id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:p ;
        rr:objectMap [ rr:column "v" ; rr:termType rr:IRI ]
    ] .

<#PropLit>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rr:template "http://ex.org/s/{id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:p ;
        rr:objectMap [ rr:column "v" ]
    ] .

<#AssertIri>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PropIri> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:SrcIri ] ] .

<#AssertLit>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PropLit> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:SrcLit ] ] .
"#;

// Cell C fixture: a plain numeric column for SUM/AVG (SUM=60, AVG=20).
const AGG_SQL: &str = r#"
CREATE TABLE nums (id INTEGER PRIMARY KEY, v INTEGER NOT NULL);
INSERT INTO nums VALUES (1, 10);
INSERT INTO nums VALUES (2, 20);
INSERT INTO nums VALUES (3, 30);
"#;

const AGG_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#N>
    rr:logicalTable [ rr:tableName "nums" ] ;
    rr:subjectMap [ rr:template "http://ex.org/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:v ; rr:objectMap [ rr:column "v" ] ] .
"#;

// Cell D fixture: an IRI-typed and a Literal-typed object over the SAME template
// shape + SAME column, so both render the IDENTICAL lexical text — the ONLY
// thing keeping a shared-var join empty is the cross-kind (IRI≠Literal) guard.
const CROSS_KIND_SQL: &str = r#"
CREATE TABLE ck (id INTEGER PRIMARY KEY, v TEXT NOT NULL);
INSERT INTO ck VALUES (1, 'X');
"#;

const CROSS_KIND_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#Iri>
    rr:logicalTable [ rr:tableName "ck" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:iriv ;
        rr:objectMap [ rr:template "http://ex.org/v/{v}" ] ] .
<#Lit>
    rr:logicalTable [ rr:tableName "ck" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:litv ;
        rr:objectMap [ rr:template "http://ex.org/v/{v}" ; rr:termType rr:Literal ] ] .
"#;

// ============================================================================
// A — pf:-id injectivity HOLE: ids::kind_tag (IRI-vs-Literal discriminator).
//
// The existing `cross_source_colliding_shape_*` pair (differential_star.rs:2374,
// 2410) keys injectivity on distinct subject-template PREFIXES (.../person/ vs
// .../widget/), so BOTH sides are IRI-subject/Literal-object and kind_tag emits
// the identical (I, L) bytes on both — dropping kind_tag reds NOTHING there.
// This cell makes two propositions identical in every id component EXCEPT the
// object kind byte, so it is the ONLY thing that reds when kind_tag is dropped.
// ============================================================================

#[test]
fn kind_tag_iri_object_vs_literal_object_same_lexical_mint_distinct_pfids() {
    let decoded = decode_materialized(KIND_TAG_SQL, KIND_TAG_R2RML);
    assert!(
        decoded.is_ok(),
        "an IRI-object proposition <<s p <http://ex.org/x>>> and a Literal-object proposition \
         <<s p \"http://ex.org/x\">> with the SAME lexical text and SAME subject/predicate must \
         mint DISTINCT urn:sf-star:pf: ids (ids::kind_tag, RDF 1.2 Semantics §5 injective IT). \
         An ambiguous PropositionForm node (two rdf:propositionFormObject values on one node) \
         means kind_tag was dropped and the two collided: {decoded:?}"
    );
}

// ============================================================================
// B — false-green enumeration: `diff().is_empty()` cells whose mutual-501 arm
// passes vacuously. `differential_star.rs`'s `diff` returns `Vec::new()` when
// BOTH engines 501, so `assert!(got.is_empty())` cannot tell "correct empty
// answer" from "both engines refused". Three cells have this exact shape:
//   - parenthesized_subject_position_triple_term_is_statically_empty     (:396)
//   - bare_syntax_in_object_position_does_not_match_an_unreified_triple  (:443)
//   - annotation_sugar_also_requires_the_plain_triple_unlike_bare_sugar  (:589)
// Each cell below re-runs the SAME query/fixture but FORCES a real translation
// first (`require_translate_then_rows`), so a regression to 501 reds instead of
// passing green.
// ============================================================================

#[test]
fn b1_parenthesized_subject_position_translates_then_empty_not_a_masked_501() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ <<( ?p ex:hasAge ?age )>> ex:assertedBy ?src }}");
    let (fa, ta) = require_translate_then_rows(CENSUS_SQL, CENSUS_R2RML, &query);
    assert!(
        fa.is_empty() && ta.is_empty(),
        "SPARQL 1.2 §18.1.3: a triple term in subject position is statically empty — but via a \
         REAL translation, not a masked mutual 501: flat={fa:#?} tree={ta:#?}"
    );
}

#[test]
fn b2_bare_syntax_object_position_translates_then_empty_not_a_masked_501() {
    let query = format!("{EX}SELECT ?q ?p ?age WHERE {{ ?q ex:hasQuote << ?p ex:hasAge ?age >> }}");
    let (fa, ta) = require_translate_then_rows(CENSUS_SQL, CENSUS_R2RML_OBJECT, &query);
    assert!(
        fa.is_empty() && ta.is_empty(),
        "bare syntax over an object-position StarMap mints no rdf:reifies row, so the desugared \
         reifies conjunct never matches — but via a REAL translation: flat={fa:#?} tree={ta:#?}"
    );
}

#[test]
fn b3_annotation_sugar_non_asserted_translates_then_empty_not_a_masked_501() {
    let query =
        format!("{EX}SELECT ?p ?age ?src WHERE {{ ?p ex:hasAge ?age {{| ex:assertedBy ?src |}} }}");
    let (fa, ta) = require_translate_then_rows(CENSUS_SQL, CENSUS_R2RML_NON_ASSERTED, &query);
    assert!(
        fa.is_empty() && ta.is_empty(),
        "annotation sugar's plain-triple conjunct is a real requirement the non-asserted map \
         suppresses — empty via a REAL translation, not a masked 501: flat={fa:#?} tree={ta:#?}"
    );
}

// ============================================================================
// C — AVG/SUM flat/tree ASYMMETRY (empirical W5 finding). The audit set out to
// close a supposed hole ("no cell runs FLAT to a SUCCESSFUL AVG/SUM value
// check") but probing revealed WHY: the flat `=_bag` fallback 501s a
// `(SUM/AVG(?x) AS ?y)` aggregate projection outright ("BIND references
// unbound"), so a flat-success value check has no green form — it is a
// BOUNDARY, not an untested success. This cell pins that boundary AND
// value-checks the TREE path (SUM=60, AVG=20) against the materialized-graph
// oracle, so the closable half is locked and the asymmetry is documented.
// ============================================================================

#[test]
fn flat_501s_aggregate_projection_while_tree_value_matches_oracle() {
    let query = format!("{EX}SELECT (SUM(?x) AS ?s) (AVG(?x) AS ?a) WHERE {{ ?p ex:v ?x }}");
    let (flat, tree) = translate_both(AGG_SQL, AGG_R2RML, &query);
    assert!(
        matches!(flat, Err(Error::Unsupported(_))),
        "the flat =_bag fallback 501s aggregate SELECT-expressions; if this ever succeeds, the \
         flat AVG/SUM VALUE now needs its own oracle check (today only tree computes it): {flat:?}"
    );
    let plan = tree.expect("tree must compute an aggregate projection");
    let conn = sqlite::load(AGG_SQL).expect("fixture loads");
    let got = run_select(&plan, &conn);
    let oracle_rows = oracle_bag(AGG_SQL, AGG_R2RML, &query);
    assert_eq!(got.len(), 1, "exactly one aggregate row: {got:#?}");
    assert!(
        oracle::solutions_bag_eq(&got, &oracle_rows),
        "the TREE engine's SUM/AVG value must equal the materialized-graph oracle:\n \
         tree={got:#?}\n oracle={oracle_rows:#?}"
    );
}

// ============================================================================
// D — cross-kind disjointness OVER-DETERMINATION. `s3a_cross_kind_shared_join_
// is_empty` (adversarial_run4b_refute.rs:468) is meant to lock "an IRI object
// can never equal a Literal object", but its fixture's IRI/Literal templates
// render DIFFERENT strings (.../X vs .../X/Y), so the join is empty even with
// BOTH cross-kind guards (unify.rs:137 and :263) removed — verified empirically
// in the W5 run. This fixture makes iriv and litv render the IDENTICAL lexical
// text (same template shape, same column), so emptiness depends SOLELY on the
// kind guard: a real translation is forced, so removing :137 (which drops to a
// :263 501) reds it, and removing both (a spurious TemplateEq match) reds it —
// neither of which the s3a JOIN cell catches.
// ============================================================================

#[test]
fn cross_kind_same_rendered_string_join_is_empty_only_via_the_kind_guard() {
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:iriv ?v . ?p ex:litv ?v }}");
    let (fa, ta) = require_translate_then_rows(CROSS_KIND_SQL, CROSS_KIND_R2RML, &query);
    assert!(
        fa.is_empty() && ta.is_empty(),
        "an IRI object <http://ex.org/v/X> and a Literal object \"http://ex.org/v/X\" with the \
         SAME lexical text are never sameTerm, so the shared-?v join is empty — via a REAL \
         translation, not a masked 501: flat={fa:#?} tree={ta:#?}"
    );
}
