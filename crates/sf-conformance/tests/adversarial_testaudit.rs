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
// is_empty` (adversarial_run4b_refute.rs) was meant to lock "an IRI object
// can never equal a Literal object", but its ORIGINAL fixture's IRI/Literal
// templates rendered DIFFERENT strings (.../X vs .../X/Y), so the join was
// empty even with BOTH cross-kind guards (unify.rs:137 and :263) removed —
// verified empirically in the W5 run. This fixture makes iriv and litv render
// the IDENTICAL lexical text (same template shape, same column), so emptiness
// depends SOLELY on the kind guard: a real translation is forced, so removing
// :137 (which drops to a :263 501) reds it, and removing both (a spurious
// TemplateEq match) reds it — neither of which the s3a JOIN cell caught at
// the time. W5b repair: `s3a_cross_kind_shared_join_is_empty`'s own fixture
// has SINCE been fixed to the same same-rendered shape (belt-and-braces, not
// a replacement for this cell) — both cells now independently depend on the
// guard.
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

// ============================================================================
// E — MySQL `group_concat_max_len` truncation (live MySQL, W5b). The IRI
// percent-encoder's `percent_encode_col_mysql` (emit.rs) raises
// `group_concat_max_len` via a `/*+ SET_VAR(...) */` hint specifically
// because MySQL's GROUP_CONCAT defaults to SILENTLY TRUNCATING at 1024 bytes
// with no error — reached whenever a `TemplateEq` comparison needs to
// percent-encode a column value in SQL (Run 4 B-repair FIX 2). No existing
// test drives this hint over a value long enough to actually HIT the
// 1024-byte default: this fixture's rendered IRIs cross it. Live-gated on
// SF_MYSQL_URL (graceful skip; mirrors `differential_pg_sqlite.rs`'s own
// convention).
// ============================================================================

const GCML_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#L> rr:logicalTable [ rr:tableName "tgcml" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:leftv ; rr:objectMap [ rr:template "http://ex.org/v/{va}" ] ] .
<#R> rr:logicalTable [ rr:tableName "tgcml" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:rightv ; rr:objectMap [ rr:template "http://ex.org/v/{vb1}({vb2}" ] ] .
"#;

/// Base MySQL URL: `SF_MYSQL_URL` if set, else the `mysql_e2e` default —
/// duplicated from `differential_pg_sqlite.rs` per this file's own
/// copied-helper convention (no `pub` cross-file surface to import).
fn mysql_url() -> String {
    std::env::var("SF_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:sftest@127.0.0.1:13306/sftest".to_owned())
}

/// Probe; `None` ⇒ graceful skip.
async fn try_connect_mysql() -> Option<mysql_async::Conn> {
    let opts = mysql_async::Opts::from_url(&mysql_url()).ok()?;
    mysql_async::Conn::new(opts).await.ok()
}

/// LOCK: `va` is 400 repetitions of `/` (every byte percent-encodes to
/// `%2F`, 3 bytes each ⇒ 1200 encoded bytes); `vb1`/`vb2` split the SAME 400
/// raw slashes as 395+5 across the right template's OWN `(` literal
/// separator (scales `adversarial_run4b_refute.rs`'s `s3d1` false-positive
/// shape past MySQL's `group_concat_max_len` DEFAULT of 1024 bytes): the two
/// encoded IRIs are IDENTICAL for their first 1201 bytes (395 shared `%2F`
/// groups after the common prefix) — past 1024 — and diverge ONLY at byte
/// 1201 onward (`%2F` vs `(`+`%2F`). Without the `SET_VAR(group_concat_max_
/// len=...)` hint, MySQL's default GROUP_CONCAT truncation at 1024 bytes
/// would cut BOTH sides down to their (identical) common prefix, wrongly
/// reporting them EQUAL; with the hint, the full, genuinely-DIFFERENT values
/// compare correctly and the shared-variable join is empty. The SQLite-side
/// sanity assertion (oracle) runs unconditionally — independent of live
/// MySQL — so a fixture-design mistake is caught even without a server.
#[tokio::test]
async fn mysql_group_concat_max_len_hint_prevents_1024_byte_truncation() {
    let va = "/".repeat(400);
    let vb1 = "/".repeat(395);
    let vb2 = "/".repeat(5);
    let sqlite_create = format!(
        "CREATE TABLE tgcml (id INTEGER PRIMARY KEY, va TEXT NOT NULL, vb1 TEXT NOT NULL, vb2 TEXT NOT NULL);\n\
         INSERT INTO tgcml VALUES (1, '{va}', '{vb1}', '{vb2}');"
    );
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:leftv ?v . ?p ex:rightv ?v }}");
    let oracle_rows = oracle_bag(&sqlite_create, GCML_R2RML, &query);
    assert!(
        oracle_rows.is_empty(),
        "sanity: the two 400-slash values genuinely differ past byte 1024, the \
         SQLite/materialized oracle must be empty regardless of live MySQL: {oracle_rows:#?}"
    );

    let Some(mut conn) = try_connect_mysql().await else {
        eprintln!("no MySQL server reachable — skipping group_concat_max_len cell");
        return;
    };
    use mysql_async::prelude::Queryable;

    let db = format!("sf_gcml_{}", std::process::id());
    conn.query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await
        .expect("drop pre-existing throwaway db");
    conn.query_drop(format!("CREATE DATABASE {db}"))
        .await
        .expect("create throwaway db");
    conn.query_drop(format!("USE {db}"))
        .await
        .expect("use throwaway db");
    conn.query_drop(
        "CREATE TABLE tgcml (id INTEGER PRIMARY KEY, va TEXT NOT NULL, vb1 TEXT NOT NULL, vb2 TEXT NOT NULL)",
    )
    .await
    .expect("create table");
    conn.query_drop(format!(
        "INSERT INTO tgcml VALUES (1, '{va}', '{vb1}', '{vb2}')"
    ))
    .await
    .expect("insert long values");

    let maps = sf_mapping::parse_r2rml(GCML_R2RML).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(&query)
        .expect("query parses");
    let plan = translate_with(&q, &maps, Dialect::MySql, &Tbox::default(), &[])
        .expect("MySQL translate must succeed");
    let solutions = sf_sparql::exec_mysql::select_mysql(&plan, &mut conn)
        .await
        .expect("MySQL execution");
    let engine = oracle::engine_bag(&solutions);
    assert!(
        engine.is_empty(),
        "MySQL group_concat_max_len default truncation would wrongly MATCH these two \
         >1024-byte encoded IRIs at their shared prefix; the SET_VAR hint must keep them \
         correctly UNEQUAL: {engine:#?}"
    );

    let _ = conn
        .query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await;
}

// ============================================================================
// F — CONSTRUCT template blank-node gate (§16.2 fresh-bnode-per-solution).
// `dedup_construct_template_projected_vars` (lib.rs, ADR-0034 Item 2) must
// SKIP its own template-projection dedup whenever the CONSTRUCT template
// carries a blank node (`template_has_blank_node` guard) — a template bnode
// label denotes a FRESH blank node PER SOLUTION (§16.2), so two solutions
// that agree on every OTHER template variable are still meant to mint TWO
// distinct blank nodes, never merge into one triple. No existing template
// anywhere in the suite carries a blank node AND drops a WHERE variable, so
// this specific gate has never been exercised.
// ============================================================================

const BNODE_GATE_SQL: &str = r#"
CREATE TABLE tbn (id INTEGER PRIMARY KEY, a INTEGER NOT NULL);
INSERT INTO tbn VALUES (1, 30);
INSERT INTO tbn VALUES (2, 30);
"#;

const BNODE_GATE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#T>
    rr:logicalTable [ rr:tableName "tbn" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rr:column "a" ] ] .
"#;

/// LOCK: two DIFFERENT people (subjects `person/1`/`person/2`) share the SAME
/// age (30) — the WHERE BGP `?p ex:age ?a` yields 2 distinct solutions, but a
/// template that DROPS `?p` and keeps only `?a` would, WITHOUT the bnode
/// gate, look like a duplicate under `dedup_construct_template_projected_
/// vars`'s narrowing (both solutions agree on the kept var `?a`=30) and
/// wrongly collapse to ONE triple. This locks that the gate correctly stays
/// OFF for a blank-node-carrying template: 2 solutions in, 2 bnode
/// instantiations out (4 triples: 2 template triples x 2 solutions), on both
/// engines — neither narrows/dedups them away.
///
/// ALSO locks the SPARQL §16.2 fix (Run 5 W6): each solution's instantiation
/// of a template blank node must be a FRESH, DISTINCT node ("blank nodes
/// created from the same label in different solutions will be different"),
/// while the SAME label used across a SINGLE solution's multiple template
/// triples (this fixture's template repeats `_:x` in a second triple) must
/// resolve to the SAME node. `exec_core::instantiate_term`'s
/// `TermPattern::BlankNode(b) => Some(Term::BlankNode(b.clone()))` used to
/// clone the SAME parsed-AST blank node for every solution — a genuine,
/// pre-existing §16.2 nonconformance previously reported here rather than
/// fixed (see git blame for the earlier revision of this doc comment) — now
/// fixed by per-solution freshening (a monotonic counter scoped to the whole
/// CONSTRUCT execution). Checked against the independent `spareval` oracle's
/// OWN CONSTRUCT evaluation by blank-node-aware graph ISOMORPHISM
/// (`graph::isomorphic` — the SAME comparator the W3C runner uses), never by
/// exact label text, which is an unspecified implementation detail on both
/// sides.
#[test]
fn construct_template_blank_node_subject_is_not_merged_by_projection_dedup() {
    let query =
        format!("{EX}CONSTRUCT {{ _:x ex:hasAge ?a . _:x a ex:Person }} WHERE {{ ?p ex:age ?a }}");
    let maps = sf_mapping::parse_r2rml(BNODE_GATE_R2RML).expect("R2RML parses");
    let conn = sqlite::load(BNODE_GATE_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let q = parse(&query);
    let flat = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("flat translates");
    let tree = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("tree translates");
    let ft = exec::construct_triples(&flat, &conn).expect("flat construct");
    let tt = exec::construct_triples(&tree, &conn).expect("tree construct");
    assert_eq!(
        ft.len(),
        tt.len(),
        "flat vs tree triple-count divergence: flat={ft:#?} tree={tt:#?}"
    );
    assert_eq!(
        tt.len(),
        4,
        "2 template triples x 2 WHERE solutions (2 people, same age) — a fresh \
         bnode per solution must NOT be merged by the template-projection dedup \
         that drops ?p: {tt:#?}"
    );

    // §16.2 freshness, checked against the independent spareval oracle by
    // blank-node-aware isomorphism (never exact label text).
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let materialized = graph::quads_to_dataset(&quads);
    let oracle_graph = match oracle::evaluate(&materialized, &query).expect("oracle eval") {
        OracleAnswer::Graph(g) => *g,
        other => panic!("expected Graph, got {other:?}"),
    };
    let engine_graph = graph::triples_to_dataset(&tt);
    assert!(
        graph::isomorphic(&engine_graph, &oracle_graph),
        "engine CONSTRUCT output must be isomorphic to the oracle's — 2 DISTINCT \
         fresh bnodes, each anchoring its OWN hasAge+type pair, not 1 bnode shared \
         across both solutions:\n engine={tt:#?}\n oracle={oracle_graph:?}"
    );
}

// ============================================================================
// G — `pool_rendered` POSITIVE differential (D2 Mechanism B, width
// mismatch). `iq::lower::pool_rendered` is the D2 pooling FALLBACK for
// candidate-map arms whose positional (raw-column) projections have
// DIFFERING WIDTHS (W3C R2RMLTC0011a: subject templates with 3 vs 2 column
// slots) — every existing reference to it is a doc comment or an
// emission-shape check; no cell runs it to a SUCCESSFUL, value-correct
// answer against the oracle. This fixture forces the width mismatch (3-slot
// vs 2-slot subject templates sharing a leading-literal prefix, so they are
// NOT provably disjoint and must pool) AND exercises percent-encoding
// inside the rendered projection (`Bloggs Jr` -> `Bloggs%20Jr`), so a
// successful, correctly-encoded answer is the closable half `pool_rendered`'s
// own doc comment describes.
// ============================================================================

const POOL_RENDERED_SQL: &str = r#"
CREATE TABLE pra (id INTEGER PRIMARY KEY, first TEXT NOT NULL, last TEXT NOT NULL);
INSERT INTO pra VALUES (1, 'Jo', 'Bloggs Jr');
CREATE TABLE prb (id INTEGER PRIMARY KEY, descr TEXT NOT NULL);
INSERT INTO prb VALUES (2, 'X');
"#;

const POOL_RENDERED_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#A>
    rr:logicalTable [ rr:tableName "pra" ] ;
    rr:subjectMap [ rr:template "http://ex.org/s/{id}/{first};{last}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:constant ex:Marker ] ] .
<#B>
    rr:logicalTable [ rr:tableName "prb" ] ;
    rr:subjectMap [ rr:template "http://ex.org/s/{id}/{descr}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:constant ex:Marker ] ] .
"#;

/// LOCK (positive control): two candidate maps for `?s ex:p ex:Marker` whose
/// subject templates have DIFFERENT column-slot counts (3 vs 2) — not
/// provably disjoint (identical leading literal prefix `http://ex.org/s/`),
/// so the D2 pooling machinery must combine them; the positional-width
/// mismatch forces `pool_rendered`'s rendered-projection fallback. Both
/// engines must translate AND answer correctly: the two arms' rendered
/// subjects (one requiring percent-encoding of an embedded space), matching
/// the materialized-graph oracle exactly.
#[test]
fn pool_rendered_width_mismatch_answers_correctly_with_percent_encoding() {
    let query = format!("{EX}SELECT ?s WHERE {{ ?s ex:p ex:Marker }}");
    let (flat, tree) = translate_both(POOL_RENDERED_SQL, POOL_RENDERED_R2RML, &query);
    let fp = flat.expect("flat must translate the width-mismatch pool (pool_rendered fallback)");
    let tp = tree.expect("tree must translate the width-mismatch pool (pool_rendered fallback)");
    // Confirm `pool_rendered`'s OWN rendered-projection shape actually fired
    // (not some other pooling path): its per-var output aliases are `rv{i}`.
    let tree_sql: String = tp
        .emitted()
        .expect("tree emits")
        .into_iter()
        .map(|e| e.sql)
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        tree_sql.contains("AS rv0"),
        "expected pool_rendered's own `rv{{i}}` rendered-projection aliasing to fire \
         for the width-mismatched arms: {tree_sql}"
    );

    let conn = sqlite::load(POOL_RENDERED_SQL).expect("fixture loads");
    let fa = run_select(&fp, &conn);
    let ta = run_select(&tp, &conn);
    assert!(
        oracle::solutions_bag_eq(&fa, &ta),
        "flat vs tree divergence: flat={fa:#?} tree={ta:#?}"
    );
    let oracle_rows = oracle_bag(POOL_RENDERED_SQL, POOL_RENDERED_R2RML, &query);
    assert!(
        oracle::solutions_bag_eq(&ta, &oracle_rows),
        "engine vs oracle divergence on the width-mismatch pool:\n \
         engine={ta:#?}\n oracle={oracle_rows:#?}"
    );
    assert_eq!(
        ta.len(),
        2,
        "one row per (non-disjoint) candidate arm: {ta:#?}"
    );
    let subjects: std::collections::BTreeSet<String> =
        ta.iter().map(|r| r["s"].to_string()).collect();
    assert!(
        subjects.contains("<http://ex.org/s/1/Jo;Bloggs%20Jr>"),
        "3-slot arm's subject must be percent-encoded via the rendered projection: {ta:#?}"
    );
    assert!(
        subjects.contains("<http://ex.org/s/2/X>"),
        "2-slot arm's subject: {ta:#?}"
    );
}

// ============================================================================
// H — D2 maximal-group partition TRANSITIVITY (`unfold::disjoint_groups`'
// union-find). Three candidate arms for ONE pattern: A-B share a leading
// subject-template prefix (not provably disjoint), B-C ALSO share one, but
// A-C's OWN prefixes conflict (provably disjoint) — a chain, not a clique.
// The union-find must pool ALL THREE as ONE group (transitive closure
// through B), not leave any pair unpooled just because A and C themselves
// are disjoint: the fixture's actual row data makes A's row and B's first
// row the SAME triple, and B's second row and C's row the SAME triple, so a
// partition that drops either bridging edge double-counts one collision. No
// existing cell has 3+ arms; every `disjoint_groups`/`pool_group` cell
// elsewhere is 2 arms. C's subject template also has an extra column slot
// (forcing `pool_rendered`'s width-mismatch fallback for the whole group),
// so this is deliberately a DIFFERENT trigger shape from Cell G above (both
// paths must handle 3-arm transitivity correctly).
// ============================================================================

const D2_TRANS_SQL: &str = r#"
CREATE TABLE d2ta (id INTEGER PRIMARY KEY, k TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO d2ta VALUES (1, '1', 'X');
CREATE TABLE d2tb (id INTEGER PRIMARY KEY, k TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO d2tb VALUES (1, 'a1', 'X');
INSERT INTO d2tb VALUES (2, 'b1-Z', 'Y');
CREATE TABLE d2tc (id INTEGER PRIMARY KEY, k1 TEXT NOT NULL, k2 TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO d2tc VALUES (1, '1', 'Z', 'Y');
"#;

const D2_TRANS_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#A>
    rr:logicalTable [ rr:tableName "d2ta" ] ;
    rr:subjectMap [ rr:template "http://example.com/aa{k}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
<#B>
    rr:logicalTable [ rr:tableName "d2tb" ] ;
    rr:subjectMap [ rr:template "http://example.com/a{k}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
<#C>
    rr:logicalTable [ rr:tableName "d2tc" ] ;
    rr:subjectMap [ rr:template "http://example.com/ab{k1}-{k2}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// LOCK: A's subject template (`aa{k}`) and C's (`ab{k1}-{k2}`) have
/// CONFLICTING leading literal prefixes (`aa` vs `ab`) — provably disjoint on
/// their own. B's (`a{k}`) is a PREFIX of both, so A-B and B-C are each NOT
/// provably disjoint — a chain, A~B~C, not a clique. Row data: A's one row
/// and B's first row render the IDENTICAL triple
/// (`http://example.com/aa1`, ex:p, "X"); B's second row and C's one row
/// render the IDENTICAL triple (`http://example.com/ab1-Z`, ex:p, "Y"). The
/// union-find must pool all three as ONE group (transitivity through B),
/// deduping BOTH collisions; a partition that drops either bridging edge
/// leaves one collision un-deduped (3 rows instead of 2).
#[test]
fn d2_three_arm_chain_pools_transitively_through_the_bridging_arm() {
    let query = format!("{EX}SELECT ?s ?o WHERE {{ ?s ex:p ?o }}");
    let (fa, ta) = require_translate_then_rows(D2_TRANS_SQL, D2_TRANS_R2RML, &query);
    assert!(
        oracle::solutions_bag_eq(&fa, &ta),
        "flat vs tree divergence: flat={fa:#?} tree={ta:#?}"
    );
    let oracle_rows = oracle_bag(D2_TRANS_SQL, D2_TRANS_R2RML, &query);
    assert!(
        oracle::solutions_bag_eq(&ta, &oracle_rows),
        "engine vs oracle divergence on the 3-arm chain:\n \
         engine={ta:#?}\n oracle={oracle_rows:#?}"
    );
    assert_eq!(
        ta.len(),
        2,
        "exactly 2 distinct triples: A/B's shared (aa1,\"X\") and B/C's shared \
         (ab1-Z,\"Y\") each deduped ONCE — a broken (non-transitive) partition \
         would leave one collision un-deduped and return 3: {ta:#?}"
    );
}

// ============================================================================
// I — R2RMLTC0002c rejection shape: `wrap_scan_distinct`'s alias-qualified
// quoting defense. A `rr:column` referencing an UNDEFINED column must be
// REJECTED (a SQL execution error), never silently succeed by falling back
// to a bogus STRING LITERAL — the exact regression `cascade::mod::
// wrap_col_ref`'s own doc comment names (a deliberately undefined
// `rr:column`, found via the REAL W3C R2RMLTC0002c case, "silently produced
// a bogus triple instead"). The defense is alias-QUALIFYING every projected
// column reference in the D1 per-scan DISTINCT wrap (`sfsN.col`, not bare
// `col`): SQLite's own "double-quoted string" misfeature falls back to a
// STRING LITERAL only for a BARE, unqualified double-quoted token that fails
// to resolve — a QUALIFIED `alias.col` reference cannot parse any other way,
// so a genuinely missing column is a hard error. Mutation-verified: the
// SLOW, full `w3c_suite.rs` conformance run DOES already catch a regression
// here (via the real R2RMLTC0002c fixture), but no cell in THIS fast
// adversarial suite drives an undefined `rr:column` through the per-scan
// wrap — this fixture is a minimal, hermetic repro of the identical shape
// for fast-iteration defense-in-depth.
// ============================================================================

const TC0002C_SQL: &str = r#"
CREATE TABLE tc0002c (a TEXT NOT NULL, b TEXT NOT NULL);
INSERT INTO tc0002c VALUES ('1', 'X');
"#;

const TC0002C_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#T>
    rr:logicalTable [ rr:tableName "tc0002c" ] ;
    rr:subjectMap [ rr:template "http://ex.org/s/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "nonexistent" ] ] .
"#;

/// LOCK: `tc0002c` has NO declared PK/UNIQUE, so D1's per-scan wrap
/// (`wrap_scan_distinct`) engages — both bindings (the subject template and
/// the object column) are individually injective, so this takes the
/// per-scan-wrap path, not the branch-level-flag fallback. The
/// `rr:objectMap`'s `rr:column "nonexistent"` names a column that does NOT
/// exist in the table (only `a`/`b` do) — R2RML mapping parse doesn't
/// validate column existence against a live schema, so this reaches SQL
/// execution, which MUST reject it (a hard error), never silently succeed
/// with the column's OWN NAME as a bogus literal value.
#[test]
fn tc0002c_undefined_column_is_rejected_not_a_bogus_literal() {
    let conn = sqlite::load(TC0002C_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let maps = sf_mapping::parse_r2rml(TC0002C_R2RML).expect("R2RML parses");
    let query = format!("{EX}SELECT ?s ?o WHERE {{ ?s ex:p ?o }}");
    let q = parse(&query);
    for (label, plan) in [
        (
            "flat",
            translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema),
        ),
        (
            "tree",
            translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema),
        ),
    ] {
        let plan = plan.unwrap_or_else(|e| {
            panic!(
                "{label} must translate (a real column-existence check belongs at exec \
                 time, not translate time): {e:?}"
            )
        });
        match exec::select(&plan, &conn) {
            Err(_) => {} // expected: a real "no such column" execution error
            Ok(sols) => panic!(
                "{label}: an undefined rr:column must be REJECTED at exec time, never \
                 silently succeed with the column's own name as a bogus literal value \
                 (W3C R2RMLTC0002c); got {} row(s): {:?}",
                sols.rows.len(),
                sols.rows
            ),
        }
    }
}
