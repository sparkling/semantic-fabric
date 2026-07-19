//! ADVERSARIAL REFUTE-ONLY review of the ADR-0033 "path-as-derived-table
//! join composition" implementation
//! (`crates/sf-sparql/src/emit.rs::path_as_derived_table_sql` +
//! `crates/sf-sparql/src/iq/lower.rs::convert_path_branches`, wired at the
//! `IqNode::InnerJoin`/`IqNode::LeftJoin` arms).
//!
//! Every fixture/query here targets an attack surface NOT already covered by
//! the ADR-0033 tests landed in `differential_paths.rs`/`differential_star.rs`/
//! `differential_tree.rs`. Standalone probe: separate test binary (Cargo
//! integration tests are independent crates), so harness plumbing is
//! duplicated from `differential_tree.rs`/`adversarial_antijoin_review.rs`
//! rather than imported.
//!
//! Attack surfaces covered (verdicts in the accompanying report, not here):
//! 1. LIMIT/OFFSET on a path-carrying branch as a join operand (nested
//!    sub-SELECT slice; OFFSET-only; OPTIONAL-right) — silent-drop suspicion.
//!    Plus a positive control: a WHOLE-QUERY LIMIT over a joined path.
//! 2. Cascade interactions: self-join elimination with the SAME closure shape
//!    joined twice on a shared var; DISTINCT-removal over a joined closure.
//! 3. `ZeroOrMore`/`ZeroOrOne` reflexive pairs joined with a class pattern.
//! 4. OPTIONAL null semantics (a shared var OPTIONAL-bound on the LEFT
//!    feeding a path JOIN on the right) + MINUS/FILTER-NOT-EXISTS bodies
//!    containing a joined path (anti-join correctness over Query-sourced scans).
//! 5. The identical path pattern occurring twice in one BGP — distinct
//!    aliases, no SQL collision, correct bag multiplicity.
//! 6. Hand-derivation (not oracle-echoed) of `differential_tree.rs`'s
//!    `item1d_r3_path_left_with_subplan_optional_right_now_answers_on_tree`.
//! 7. Property path inside `GRAPH <iri> { }`, joined vs standalone.
//! 8. A path branch carrying a WHERE condition beyond its own endpoints
//!    (FILTER pushed onto the path leaf before conversion); a path inside a
//!    UNION arm, joined AFTER the union.

use rusqlite::Connection;
use sf_conformance::graph::parse_turtle;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::sqlite;
use sf_sparql::iq::{Branch, Scan};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, Tbox};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};
use std::collections::BTreeMap;

const BASE: &str = "http://ex/";

// ============================================================================
// Harness plumbing — duplicated from `differential_tree.rs` (separate test
// binary; no `pub` cross-file surface to import from). Semantics identical.
// ============================================================================

fn parse(q: &str) -> Query {
    SparqlParser::new().parse_query(q).expect("query parses")
}

fn flat(
    maps: &[sf_core::ir::TriplesMap],
    q: &Query,
    schema: &[TableSchema],
) -> sf_sparql::Result<Plan> {
    translate_with_flat(q, maps, Dialect::Sqlite, &Tbox::default(), schema)
}

fn tree(
    maps: &[sf_core::ir::TriplesMap],
    q: &Query,
    schema: &[TableSchema],
) -> sf_sparql::Result<Plan> {
    translate_with(q, maps, Dialect::Sqlite, &Tbox::default(), schema)
}

/// Engine answer: SPARQL -> SQL over a live SQLite source, normalised to the bag.
fn engine_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, oxrdf::Term>> {
    let conn: Connection = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let q = parse(query);
    let plan = tree(&maps, &q, &schema).expect("tree translates");
    oracle::engine_bag(&exec::select(&plan, &conn).expect("exec"))
}

/// Oracle answer: the SAME SPARQL over the hand-authored expected graph (spareval).
fn oracle_bag(ttl: &str, query: &str) -> Vec<BTreeMap<String, oxrdf::Term>> {
    let g = parse_turtle(ttl, BASE).expect("expected graph parses");
    match oracle::evaluate(&g, query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected SELECT solutions, got {other:?}"),
    }
}

/// The core differential: the engine (tree) vs the independent `spareval` oracle
/// over the SAME hand-authored graph. Returns the row count for a sanity pin.
fn assert_differential(create: &str, r2rml: &str, ttl: &str, query: &str) -> usize {
    let engine = engine_bag(create, r2rml, query);
    let oracle = oracle_bag(ttl, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle),
        "engine vs oracle divergence on `{query}`:\n engine={engine:#?}\n oracle={oracle:#?}"
    );
    engine.len()
}

// ============================================================================
// Fixture RJ — a small DAG (1->2, 2->3, 1->4) + a name table + a class table.
// `ex:reaches+` closure = {(1,2),(1,3),(1,4),(2,3)} (4 pairs): from 1 you reach
// {2,3,4}; from 2 you reach {3}; 3 and 4 have no outgoing edges. Serves
// surfaces 1, 2, 5, 8.
// ============================================================================

const RJ_SQL: &str = r#"
CREATE TABLE rj_edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO rj_edge VALUES (1, 2);
INSERT INTO rj_edge VALUES (2, 3);
INSERT INTO rj_edge VALUES (1, 4);
CREATE TABLE rj_person (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
INSERT INTO rj_person VALUES (1, 'Ann');
INSERT INTO rj_person VALUES (2, 'Bob');
CREATE TABLE rj_thing (id INTEGER NOT NULL);
INSERT INTO rj_thing VALUES (2);
INSERT INTO rj_thing VALUES (3);
INSERT INTO rj_thing VALUES (4);
"#;

const RJ_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "rj_edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
<#Person>
    rr:logicalTable [ rr:tableName "rj_person" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
<#Thing>
    rr:logicalTable [ rr:tableName "rj_thing" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ; rr:class ex:Thing ] .
"#;

const RJ_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/1> ex:reaches <http://ex/n/4> .
<http://ex/n/1> ex:name "Ann" .
<http://ex/n/2> ex:name "Bob" .
<http://ex/n/2> a ex:Thing .
<http://ex/n/3> a ex:Thing .
<http://ex/n/4> a ex:Thing .
"#;

// ----------------------------------------------------------------------------
// Surface 1: LIMIT/OFFSET on a path-carrying branch as a join operand.
// ----------------------------------------------------------------------------

/// TOP SUSPICION. A path pattern wrapped in its OWN nested `{ SELECT ... LIMIT
/// 1 }` sub-SELECT, then joined with another pattern on the path's own object
/// endpoint. Pre-ADR-0033 this is a `Path`-joined-with-anything 501
/// (`unfold::merge`'s unconditional guard). ADR-0033 converts a path-carrying
/// branch to an ordinary `core` `Scan` at `InnerJoin` — but a *nested-Slice*
/// child never reaches `IqNode::Path` directly; it is `IqNode::Slice{ child:
/// Construction{Path}, limit: Some(1) }`, routed by `lower_node`'s catch-all
/// modifier arm to `lower_as_subplan`, which (pre-existing, ADR-0025 Tier-1,
/// UNTOUCHED by this diff) checks `nested_plan.limit.is_some() ||
/// nested_plan.offset > 0` and 501s BEFORE `convert_path_branches` (which only
/// ever inspects `Branch::path`) could see it. `Branch::path` and
/// `Branch::limit` are therefore mutually exclusive by construction — a nested
/// slice on a path NEVER reaches a raw `Branch{path: Some, limit: Some}`
/// combination. Verified live, not just read: both flat AND tree must 501.
#[test]
fn nested_limit_on_path_joined_with_class_pattern_stays_501() {
    let maps = sf_mapping::parse_r2rml(RJ_R2RML).expect("R2RML parses");
    let q = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?t WHERE { \
         { SELECT ?s ?o WHERE { ?s ex:reaches+ ?o } LIMIT 1 } . ?o ex:reaches ?t }",
    );
    let f = flat(&maps, &q, &[]);
    let t = tree(&maps, &q, &[]);
    assert!(
        matches!(f, Err(Error::Unsupported(_))),
        "expected 501 on flat (pre-existing path-join guard): {f:?}"
    );
    match &t {
        Err(Error::Unsupported(msg)) => {
            // Evidence this is the SLICE guard, not some unrelated 501.
            assert!(
                msg.contains("LIMIT") || msg.contains("OFFSET") || msg.contains("slice"),
                "tree 501'd but NOT via the expected slice guard — investigate: {msg}"
            );
        }
        other => panic!(
            "CRITICAL: tree did NOT 501 a sliced-path join operand — potential silent-drop: {other:?}"
        ),
    }
}

/// Same shape, `OFFSET` with NO `LIMIT` — `lower_as_subplan`'s guard is `||
/// offset > 0`, a separate disjunct from `limit.is_some()`; a bare OFFSET is
/// itself a genuine, syntactically valid SPARQL slice, so it must be checked
/// independently, not assumed to ride along with the LIMIT case.
#[test]
fn nested_offset_only_on_path_joined_stays_501() {
    let maps = sf_mapping::parse_r2rml(RJ_R2RML).expect("R2RML parses");
    let q = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?t WHERE { \
         { SELECT ?s ?o WHERE { ?s ex:reaches+ ?o } OFFSET 1 } . ?o ex:reaches ?t }",
    );
    let f = flat(&maps, &q, &[]);
    let t = tree(&maps, &q, &[]);
    assert!(
        matches!(f, Err(Error::Unsupported(_))),
        "expected 501 on flat: {f:?}"
    );
    assert!(
        matches!(t, Err(Error::Unsupported(_))),
        "CRITICAL: tree did NOT 501 an offset-sliced-path join operand — potential silent-drop: {t:?}"
    );
}

/// The SAME slice-on-a-path suspicion, but as an OPTIONAL's right operand
/// (`IqNode::LeftJoin`'s right-side `is_single_subplan_branch`/
/// `left_join_over_subplan` path) rather than an `InnerJoin` operand — a
/// DIFFERENT call site of `convert_path_branches`.
#[test]
fn nested_limit_on_path_subselect_as_optional_right_stays_501() {
    let maps = sf_mapping::parse_r2rml(RJ_R2RML).expect("R2RML parses");
    let q = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?n ?o WHERE { \
         ?s ex:name ?n OPTIONAL { SELECT ?s ?o WHERE { ?s ex:reaches+ ?o } LIMIT 1 } }",
    );
    let f = flat(&maps, &q, &[]);
    let t = tree(&maps, &q, &[]);
    assert!(
        matches!(f, Err(Error::Unsupported(_))),
        "expected 501 on flat: {f:?}"
    );
    assert!(
        matches!(t, Err(Error::Unsupported(_))),
        "CRITICAL: tree did NOT 501 a sliced-path OPTIONAL right side — potential silent-drop: {t:?}"
    );
}

/// POSITIVE CONTROL: a WHOLE-QUERY (plan-level) `LIMIT`, not a nested
/// sub-SELECT slice — this is SOUND SPARQL (LIMIT is a solution modifier over
/// the entire WHERE clause) and, after the join collapses to ONE branch,
/// `Plan::prepared_branches` pushes `Plan.limit`/`.offset` straight into that
/// branch's OWN `Branch.limit`/`.offset` (the "single branch in the whole
/// plan" SQL-pushdown case) — a genuinely NEW interaction this diff doesn't
/// exercise anywhere: a converted path's `core` `Query` scan sitting inside a
/// branch that ALSO carries a SQL-level LIMIT. Un-limited baseline = 4 rows
/// (the full closure joined with the class fact, all of 2/3/4 typed
/// `ex:Thing`); LIMIT 2 must return EXACTLY 2 rows, and — since there is no
/// ORDER BY, so SPARQL does not mandate WHICH 2 — every returned row must
/// still be a genuine MEMBER of the correct 4-row set (no phantom/duplicated
/// rows from a mis-sliced derived table).
#[test]
fn plan_level_limit_over_joined_path_returns_correct_subset() {
    let full = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o . ?o a ex:Thing }",
    );
    assert_eq!(full, 4, "closure {{(1,2),(1,3),(1,4),(2,3)}}, all o typed ex:Thing");

    let conn = sqlite::load(RJ_SQL).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(RJ_R2RML).expect("R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let full_bag = oracle_bag(
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o . ?o a ex:Thing }",
    );

    let q = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o . ?o a ex:Thing } LIMIT 2",
    );
    let plan = tree(&maps, &q, &schema).expect("tree must answer a plan-level LIMIT over a joined path");
    let limited = oracle::engine_bag(&exec::select(&plan, &conn).expect("exec"));
    assert_eq!(
        limited.len(),
        2,
        "LIMIT 2 over a 4-row joined-path result must return exactly 2 rows, got {limited:#?}"
    );
    for row in &limited {
        assert!(
            full_bag.iter().any(|r| r == row),
            "LIMIT returned a row NOT in the correct unlimited set — a mis-sliced derived \
             table: {row:#?}\nfull correct set={full_bag:#?}"
        );
    }
}

// ----------------------------------------------------------------------------
// Surface 2 (+5): cascade interactions and same-closure-shape-joined-twice.
// ----------------------------------------------------------------------------

/// The SAME closure shape (`ex:reaches+`) joined TWICE on the shared subject
/// `?a`, with DIFFERENT object variables `?b`/`?c` — the shape self-join
/// elimination would need to (WRONGLY) collapse if its `LogicalSource::Table`
/// guard (`cascade/mod.rs::scan_table_in`, `find_self_join_in`) did not
/// exclude `LogicalSource::Query` scans. A wrongful collapse would either
/// error or force `?b == ?c` (diagonal only, 4 rows); the CORRECT answer is
/// the independent cross product per shared `?a`: a=1 reaches {2,3,4}, so
/// 3x3=9 (b,c) pairs; a=2 reaches {3}, so 1 pair (3,3) — 10 rows total.
#[test]
fn two_path_occurrences_shared_subject_cross_product_not_collapsed() {
    let n = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?a ?b ?c WHERE { ?a ex:reaches+ ?b . ?a ex:reaches+ ?c }",
    );
    assert_eq!(
        n, 10,
        "a=1: {{2,3,4}}x{{2,3,4}} = 9 pairs; a=2: {{3}}x{{3}} = 1 pair; NOT the 4-row \
         diagonal a wrongful self-join collapse would produce"
    );
}

/// The IDENTICAL path pattern (same shape, SAME variables) twice in one BGP:
/// `?s ex:reaches+ ?o . ?s ex:reaches+ ?o`. An inner join of X with itself on
/// ALL shared variables is idempotent (`X join X = X`) — the result must be
/// BYTE-IDENTICAL to the single-occurrence closure (4 rows), never
/// multiplied (16, if the two occurrences were wrongly treated as
/// independent/uncorrelated) nor erroring (an alias collision).
#[test]
fn identical_path_pattern_twice_is_idempotent_join() {
    let solo = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o }",
    );
    let twice = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o . ?s ex:reaches+ ?o }",
    );
    assert_eq!(solo, 4, "the base closure has 4 pairs");
    assert_eq!(
        twice, solo,
        "repeating the IDENTICAL path pattern must be a no-op (idempotent self-join), \
         not a multiplication or a crash"
    );
}

/// `SELECT DISTINCT ?s` over a joined closure: `distinct_removal`
/// (`cascade/mod.rs`) must NOT wrongly prove the DISTINCT redundant just
/// because a Query-sourced (converted-path) scan sits in `b.core` alongside
/// an ordinary table scan — its `LogicalSource::Table` destructure-or-bail
/// inside the `.all()` multi-scan proof must reject the Query scan. Proven
/// empirically, not just by code reading: the NON-distinct join has 4 rows
/// (s=1 x3, s=2 x1) — if DISTINCT were silently dropped, `SELECT DISTINCT ?s`
/// would STILL return those same underlying dupes wherever the SQL-level
/// DISTINCT failed to apply; the CORRECT answer collapses to 2 (s in {1,2}).
#[test]
fn distinct_over_joined_path_actually_dedups() {
    let non_distinct = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:reaches+ ?o . ?o a ex:Thing }",
    );
    assert_eq!(non_distinct, 4, "s=1 (o in {{2,3,4}}) + s=2 (o=3) — 4 pre-dedup rows");
    let distinct = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT DISTINCT ?s WHERE { ?s ex:reaches+ ?o . ?o a ex:Thing }",
    );
    assert_eq!(
        distinct, 2,
        "DISTINCT ?s must collapse to {{1,2}} — a silently-dropped DISTINCT would leak the \
         same 4 rows `non_distinct` proved exist"
    );
}

/// Surface 5, structural half: after lowering, the Plan's sole branch must
/// carry TWO DISTINCT `Scan`s with `LogicalSource::Query` (one per path
/// occurrence), with DISTINCT outer aliases (`pc.alias`, assigned uniquely
/// by RESOLVE to each `IqNode::Path` leaf independently of
/// `convert_path_branches`) and DISTINCT embedded SQL text (each closure's
/// SQL is rebased onto its own fresh internal CTE alias by
/// `path_as_derived_table_sql`). A Rust-level proof alongside the row-level
/// proof (`two_path_occurrences_shared_subject_cross_product_not_collapsed`)
/// that there is no aliasing collision, not just correct output.
#[test]
fn two_path_scans_get_distinct_aliases_structural_check() {
    let maps = sf_mapping::parse_r2rml(RJ_R2RML).expect("R2RML parses");
    let q = parse(
        "PREFIX ex: <http://ex/> SELECT ?a ?b ?c WHERE { ?a ex:reaches+ ?b . ?a ex:reaches+ ?c }",
    );
    let plan = tree(&maps, &q, &[]).expect("tree translates");
    assert_eq!(plan.branches.len(), 1, "a plain multi-path inner join is one branch");
    let b: &Branch = &plan.branches[0];
    let query_scans: Vec<&Scan> = b
        .core
        .iter()
        .filter(|s| matches!(s.source, sf_core::ir::LogicalSource::Query(_)))
        .collect();
    assert_eq!(
        query_scans.len(),
        2,
        "expected exactly 2 converted-path Query scans, got {query_scans:#?}"
    );
    assert_ne!(
        query_scans[0].alias, query_scans[1].alias,
        "the two path occurrences' OUTER scan aliases must differ (else the SQL FROM \
         clause has a duplicate table alias — a crash, not a silent bug)"
    );
    let (sf_core::ir::LogicalSource::Query(sql0), sf_core::ir::LogicalSource::Query(sql1)) =
        (&query_scans[0].source, &query_scans[1].source)
    else {
        unreachable!("filtered to Query sources above");
    };
    assert_ne!(
        sql0, sql1,
        "the two closures' embedded derived-table SQL must be rebased onto DISTINCT \
         internal CTE aliases"
    );
}

// ============================================================================
// Fixture ZR — a single-predicate 3-CYCLE (1->2->3->1). `P*`/`p?`'s reflexive
// enumeration is gated by `unfold::graph_is_single_predicate`, which scans the
// ENTIRE mapping document (every triples map, including any `rr:class`
// shortcut) — a SEPARATE class table (as first attempted here) trips this
// PRE-EXISTING, ADR-0033-unrelated guard immediately: "P*/p? reflexive
// ZeroLengthPath: ... supported only over a single-predicate, single-table
// mapping" (`path.rs`), confirmed live. So the join partner here reuses the
// SAME predicate (a direct 1-hop `ex:reaches`) instead of a class pattern —
// this still exercises "reflexive pairs interacting with a join", just not
// literally with `rdf:type`. The 3-cycle makes `P+`/`P*` reach ALL THREE
// nodes from ANY start (a cycle back to the origin IS a genuine transitive
// member, not merely the reflexive addition), while `p?` stays just
// hop-plus-reflexive — a crisp, non-coincidental differentiator between the
// two path kinds. Serves surface 3.
// ============================================================================

const ZR_SQL: &str = r#"
CREATE TABLE zr_edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO zr_edge VALUES (1, 2);
INSERT INTO zr_edge VALUES (2, 3);
INSERT INTO zr_edge VALUES (3, 1);
"#;

const ZR_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "zr_edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
"#;

const ZR_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/3> ex:reaches <http://ex/n/1> .
"#;

/// `ex:reaches*` joined, on its OBJECT endpoint, with a direct 1-hop
/// `ex:reaches` (the SAME predicate — required by `graph_is_single_predicate`).
/// In a 3-cycle, `P+`/`P*` from ANY node reaches ALL THREE nodes (the cycle
/// closes), so the full `*` closure is the complete 3x3 = 9 pairs. Every node
/// has an outgoing direct edge (it's a cycle), so EVERY one of the 9 pairs
/// survives the join — 9 rows. This specifically exercises the REFLEXIVE
/// pairs `(1,1)`, `(2,2)`, `(3,3)` (present only because P* adds them, NOT
/// because the cycle makes a node reach itself in fewer than 3 hops)
/// surviving a join through the converted derived table's `sf_o` column.
#[test]
fn zero_or_more_reflexive_pairs_join_correctly_with_self_predicate() {
    let n = assert_differential(
        ZR_SQL,
        ZR_R2RML,
        ZR_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?t WHERE { ?s ex:reaches* ?o . ?o ex:reaches ?t }",
    );
    assert_eq!(
        n, 9,
        "3-cycle: P* from any node reaches all 3 (full 3x3=9 pairs); every node has an \
         outgoing edge, so the join drops nothing"
    );
}

/// The SAME join, `ex:reaches?` (ZeroOrOne) instead of `*`. Unlike the cyclic
/// closure, `p?` is just ONE optional hop: {(1,2),(2,3),(3,1)} UNION reflexive
/// {(1,1),(2,2),(3,3)} = 6 pairs (NOT the full 9 `*` reaches) — a crisp,
/// non-coincidental differentiator from the `P*` case above, still every node
/// has an outgoing edge so still nothing drops in the join.
#[test]
fn zero_or_one_reflexive_pairs_join_correctly_with_self_predicate() {
    let n = assert_differential(
        ZR_SQL,
        ZR_R2RML,
        ZR_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?t WHERE { ?s ex:reaches? ?o . ?o ex:reaches ?t }",
    );
    assert_eq!(
        n, 6,
        "p? = hop (3) + reflexive (3) = 6 pairs, all surviving the join (every node has an \
         outgoing edge) — deliberately LESS than P*'s 9 (no cycle-closure for a single hop)"
    );
}

// ============================================================================
// Fixture OM — a nested-OPTIONAL chain feeding a path join, plus a MINUS/NOT
// EXISTS anti-join body containing a joined path. Ann has a `mid` (10),
// reaching {20,30} via `ex:next+`; Bob has NO `mid` row at all (absent row,
// not a NULL column — sidesteps the pre-existing, unrelated `rr:column`
// NULL-handling imprecision this file's siblings already document). Serves
// surface 4.
// ============================================================================

const OM_SQL: &str = r#"
CREATE TABLE om_person (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
INSERT INTO om_person VALUES (1, 'Ann');
INSERT INTO om_person VALUES (2, 'Bob');
CREATE TABLE om_mid (person_id INTEGER NOT NULL, mid INTEGER NOT NULL);
INSERT INTO om_mid VALUES (1, 10);
CREATE TABLE om_edge (a INTEGER NOT NULL, b INTEGER NOT NULL);
INSERT INTO om_edge VALUES (10, 20);
INSERT INTO om_edge VALUES (20, 30);
"#;

const OM_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Person>
    rr:logicalTable [ rr:tableName "om_person" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
<#Mid>
    rr:logicalTable [ rr:tableName "om_mid" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{person_id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:mid ; rr:objectMap [ rr:template "http://ex/n/{mid}" ] ] .
<#Edge>
    rr:logicalTable [ rr:tableName "om_edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:next ; rr:objectMap [ rr:template "http://ex/n/{b}" ] ] .
"#;

const OM_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:name "Ann" .
<http://ex/n/2> ex:name "Bob" .
<http://ex/n/1> ex:mid <http://ex/n/10> .
<http://ex/n/10> ex:next <http://ex/n/20> .
<http://ex/n/20> ex:next <http://ex/n/30> .
"#;

/// `{?id ex:name ?name OPTIONAL {?id ex:mid ?m}} OPTIONAL {?m ex:next+
/// ?reached}` — SPARQL left-associates same-level OPTIONALs, so this is
/// `LeftJoin(LeftJoin(Name,Mid), Path)`: the OUTER OPTIONAL's right side is a
/// property path whose join variable `?m` is ONLY conditionally bound (from
/// the INNER OPTIONAL). This is the R1 null-safe ON machinery
/// (`leftjoin::null_safe`/`def_is_nullable`) exercised against a Query-sourced
/// `sf_s` column for the first time.
///
/// CORRECTED EXPECTATION (first draft wrongly assumed SQL NULL-never-matches
/// semantics and asserted 3 — that failed with `left=5, right=3`, and
/// `assert_differential`'s own internal engine-vs-spareval-oracle check
/// passed at 5, meaning the flaw was in my hand-derivation, not the engine).
/// SPARQL compatibility is defined over VARIABLE DOMAINS, not SQL-style NULL
/// propagation: for Bob, `?m` is ABSENT from his solution's domain (the inner
/// OPTIONAL never matched him), so `dom(Bob) ∩ dom(path-branch-row) = {}` for
/// EVERY path-branch row — an EMPTY shared-variable domain is *vacuously*
/// compatible, so Bob merges with ALL THREE rows of the FULL `ex:next+`
/// closure ({(10,20),(10,30),(20,30)} — from 10 AND from 20), not zero.
/// `null_safe`'s `NullSafeEq` renders as `(a = b OR a IS NULL OR b IS NULL)`
/// (`emit.rs::render_conjunction`) — EITHER side NULL makes the whole
/// condition TRUE unconditionally, which is exactly this "absent ⇒
/// unconstrained" semantics, not a stricter "IS NOT DISTINCT FROM". Ann
/// (m=10, bound): matches only the 2 rows with sf_s=10 -> (10,20),(10,30).
/// Bob (m absent -> SQL NULL): the OR-IS-NULL disjunct fires unconditionally
/// -> matches ALL 3 rows, INCLUDING (20,30) (pulling m=20 in from the right
/// side, overwriting Bob's own absent `?m`). Total 2 + 3 = 5 rows. This is a
/// SURVIVED verdict with strong evidence: the engine implements the CORRECT,
/// subtle SPARQL semantics here (a well-known nested-OPTIONAL variable-scoping
/// gotcha), not a naive SQL-NULL approximation of it.
#[test]
fn optional_bound_var_from_nested_optional_feeds_path_join_null_safe() {
    let n = assert_differential(
        OM_SQL,
        OM_R2RML,
        OM_TTL,
        "PREFIX ex: <http://ex/> SELECT ?id ?name ?m ?reached WHERE { \
         ?id ex:name ?name OPTIONAL { ?id ex:mid ?m } OPTIONAL { ?m ex:next+ ?reached } }",
    );
    assert_eq!(
        n, 5,
        "Ann: (10,20)+(10,30) = 2 rows; Bob (m ABSENT, vacuously compatible with the FULL \
         next+ closure {{(10,20),(10,30),(20,30)}}) = 3 rows"
    );
}

/// MINUS whose body is a JOINED path (`?id ex:mid ?m . ?m ex:next+ ?x`) — the
/// anti-join correlation must probe the Query-sourced scan correctly. Ann's
/// MINUS body has solutions (mid=10, next+ reaches {20,30}, both correlating
/// on id=Ann) -> Ann is REMOVED. Bob's MINUS body has NO solutions at all
/// (Bob has no `ex:mid` triple) -> Bob is KEPT. Expect exactly {Bob}.
#[test]
fn minus_body_with_joined_path_anti_join_correctness() {
    let n = assert_differential(
        OM_SQL,
        OM_R2RML,
        OM_TTL,
        "PREFIX ex: <http://ex/> SELECT ?id ?name WHERE { \
         ?id ex:name ?name MINUS { ?id ex:mid ?m . ?m ex:next+ ?x } }",
    );
    assert_eq!(n, 1, "only Bob survives — Ann's MINUS body has solutions via her mid+closure");
}

/// The SAME anti-join, via `FILTER NOT EXISTS` instead of `MINUS` — a
/// DIFFERENT variable-scoping path into the same `SqlCond::NotExists`/
/// `not_exists_cond_for` machinery (`lower_iq_exists` reusing `lower_node`
/// inside the correlated subquery, per the ADR's own coverage claim).
#[test]
fn filter_not_exists_body_with_joined_path_anti_join_correctness() {
    let n = assert_differential(
        OM_SQL,
        OM_R2RML,
        OM_TTL,
        "PREFIX ex: <http://ex/> SELECT ?id ?name WHERE { \
         ?id ex:name ?name FILTER NOT EXISTS { ?id ex:mid ?m . ?m ex:next+ ?x } }",
    );
    assert_eq!(n, 1, "only Bob survives, same as the MINUS form");
}

// ============================================================================
// Surface 6 — hand-derivation of the most complex flipped `differential_tree`
// pin: `item1d_r3_path_left_with_subplan_optional_right_now_answers_on_tree`.
// Reproduces that test's EXACT query + PE_SQL/PE_R2RML/PE_TTL fixture
// (verbatim copy — not `pub`, cannot import across test binaries) and asserts
// against a FULLY HAND-TYPED expected row set, independent of the `spareval`
// oracle entirely (never calls `oracle::evaluate`) — a from-scratch check
// that the pin's flipped expectation is semantically correct, not merely
// self-consistent with whatever `spareval` happens to compute.
// ============================================================================

const PE_SQL: &str = r#"
CREATE TABLE edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO edge VALUES (1, 2);
INSERT INTO edge VALUES (2, 3);
INSERT INTO edge VALUES (3, 4);
INSERT INTO edge VALUES (1, 5);
"#;

const PE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
"#;

/// By hand: edges 1->2, 2->3, 3->4, 1->5. `ex:reaches+` closure: from 1,
/// {2,3,4,5}; from 2, {3,4}; from 3, {4}. 7 pairs: (1,2) (1,3) (1,4) (1,5)
/// (2,3) (2,4) (3,4). The OPTIONAL's right side computes, for each node `?o`,
/// the COUNT of DIRECT (1-hop) edges INTO it (`GROUP BY ?o`): in-degree(2)=1
/// (from 1), in-degree(3)=1 (from 2), in-degree(4)=1 (from 3), in-degree(5)=1
/// (from 1); node 1 has in-degree 0 (no row). Since this graph is a simple
/// chain/star with no node having in-degree > 1, and every closure pair's
/// `?o` is one of {2,3,4,5} (never 1, which is only ever a source), EVERY
/// closure pair's `?o` matches the subselect with count=1 — no row ever
/// null-pads. 7 rows, `?c` = 1 in every one.
#[test]
fn hand_derived_item1d_r3_path_left_with_subplan_optional_right() {
    let conn = sqlite::load(PE_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(PE_R2RML).expect("R2RML parses");
    let query = "PREFIX ex: <http://ex/> SELECT ?s ?o ?c WHERE { ?s ex:reaches+ ?o \
                 OPTIONAL { SELECT ?o (COUNT(?x) AS ?c) WHERE { ?x ex:reaches ?o } GROUP BY ?o } }";
    let q = parse(query);
    let plan = tree(&maps, &q, &schema)
        .expect("tree must answer this SubPlan-OPTIONAL over a path LEFT (ADR-0033)");
    let got = oracle::engine_bag(&exec::select(&plan, &conn).expect("exec"));

    let hand_derived_pairs: [(i64, i64); 7] =
        [(1, 2), (1, 3), (1, 4), (1, 5), (2, 3), (2, 4), (3, 4)];
    assert_eq!(
        got.len(),
        hand_derived_pairs.len(),
        "expected exactly {} hand-derived rows, got {}:\n{got:#?}",
        hand_derived_pairs.len(),
        got.len()
    );
    let node = |i: i64| format!("http://ex/n/{i}");
    for (s, o) in hand_derived_pairs {
        let row = got.iter().find(|r| {
            r.get("s").map(|t| t.to_string()) == Some(format!("<{}>", node(s)))
                && r.get("o").map(|t| t.to_string()) == Some(format!("<{}>", node(o)))
        });
        let row = row.unwrap_or_else(|| {
            panic!("hand-derived pair (s={s},o={o}) missing from engine output: {got:#?}")
        });
        let c = row.get("c").unwrap_or_else(|| panic!("row (s={s},o={o}) has no ?c: {row:#?}"));
        let oxrdf::Term::Literal(lit) = c else {
            panic!("?c must be a literal, got {c:?}")
        };
        assert_eq!(
            lit.value().parse::<i64>(),
            Ok(1),
            "every hand-derived pair's ?c must be 1 (this DAG has no in-degree > 1 among \
             reachable nodes) — got {:?} for (s={s},o={o})",
            lit.value()
        );
    }
}

// ============================================================================
// Surface 7 — property path inside `GRAPH <iri> { }`. Quick check: is the
// SOLO (unjoined, pre-existing, `convert_path_branches`-independent) case's
// outcome the SAME KIND of outcome as the JOINED (ADR-0033-territory) case?
// If both agree (both 501 the same way, or both succeed identically), the
// join lift changes nothing about GRAPH handling.
//
// CONFIRMED FINDING (out-of-charter, pre-existing, NOT introduced or fixed by
// ADR-0033): BOTH translate `Ok` and BOTH silently IGNORE the `GRAPH <g>`
// constraint entirely — the emitted plan queries the full `rj_edge` table
// regardless of `<http://ex/g1>`, byte-identical to the SAME query with the
// `GRAPH` wrapper removed. Root cause traced (not just observed): `build.rs`
// threads `current_graph` into `Intensional.graph`/`UnresolvedPath.graph`,
// but `iq/resolve.rs` (grepped directly) never reads either field when
// compiling to `Extensional`/`Path` — the constraint is captured, then
// dropped, for EVERY pattern kind (ordinary triples AND paths alike), a
// mapping-level gap that predates and is orthogonal to join composition. The
// join lift is confirmed UNCHANGED here: solo and joined both ignore GRAPH
// the same way, so this is a pointer for a SEPARATE ticket, not an ADR-0033
// regression.
// ============================================================================

#[test]
fn path_inside_named_graph_joined_behaves_same_as_standalone() {
    let maps = sf_mapping::parse_r2rml(RJ_R2RML).expect("R2RML parses");
    let solo = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { GRAPH <http://ex/g1> { ?s ex:reaches+ ?o } }",
    );
    let joined = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?t WHERE { \
         GRAPH <http://ex/g1> { ?s ex:reaches+ ?o } . ?o ex:reaches ?t }",
    );
    let joined_no_graph = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?t WHERE { ?s ex:reaches+ ?o . ?o ex:reaches ?t }",
    );
    let solo_t = tree(&maps, &solo, &[]);
    let joined_t = tree(&maps, &joined, &[]);
    match (&solo_t, &joined_t) {
        (Err(Error::Unsupported(_)), Err(Error::Unsupported(_))) => {
            // A future GRAPH-support fix would land here — both refuse, so
            // the join lift changed nothing about GRAPH handling.
        }
        (Ok(_), Ok(_)) => {
            // The CONFIRMED current outcome: GRAPH is silently ignored
            // engine-wide. Pin exact parity with the SAME query minus the
            // GRAPH wrapper — proves the join composes IDENTICALLY whether or
            // not a (currently-unenforced) GRAPH constraint is textually
            // present, i.e. no NEW divergence from the join lift specifically.
            let conn = sqlite::load(RJ_SQL).expect("fixture loads");
            let schema = sqlite::introspect_all(&conn).expect("introspect");
            let joined_plan = tree(&maps, &joined, &schema).expect("re-translate with schema");
            let no_graph_plan =
                tree(&maps, &joined_no_graph, &schema).expect("re-translate with schema");
            let joined_bag = oracle::engine_bag(&exec::select(&joined_plan, &conn).expect("exec"));
            let no_graph_bag =
                oracle::engine_bag(&exec::select(&no_graph_plan, &conn).expect("exec"));
            assert!(
                oracle::solutions_bag_eq(&joined_bag, &no_graph_bag),
                "GRAPH-wrapped joined-path answer must be byte-identical to the same query \
                 with GRAPH removed (both currently ignore it) — a divergence here would mean \
                 the join lift changed (not fixed) GRAPH handling:\n \
                 graph-wrapped={joined_bag:#?}\n no-graph={no_graph_bag:#?}"
            );
        }
        (a, b) => panic!(
            "solo vs joined GRAPH-path DISAGREE in kind — the join lift changed GRAPH \
             behavior: solo={a:?}\n joined={b:?}"
        ),
    }
}

// ----------------------------------------------------------------------------
// Surface 8: a path branch carrying a WHERE condition beyond its own
// endpoints; a path inside a UNION arm, joined AFTER the union.
// ----------------------------------------------------------------------------

/// A `FILTER` directly on the path's OWN endpoint variable, in the SAME BGP
/// as the path and a further join — an attempt to construct "a branch
/// carrying BOTH path and where_conds beyond its endpoints" via NORMALIZE's
/// filter pushdown (`IqNode::Filter{ child: IqNode::Path{closure}, cond }`
/// would lower to a branch carrying BOTH `path: Some(_)` and a non-empty
/// `where_conds` BEFORE `convert_path_branches` ever runs).
///
/// CONFIRMED FINDING (not the bug I was probing for): this 501s — but for a
/// SEPARATE, PRE-EXISTING, GENERIC reason, unrelated to join composition. A
/// path's only bindings are its two endpoints, and BOTH are ALWAYS
/// `TermMap::Template`-reconstructed (a full IRI, e.g. `http://ex/n/{sf_s}`),
/// never a bare `rr:column` — and FILTER-lowering (`unify`'s v1 scope, the
/// SAME boundary `iq/lower.rs`'s LeftJoin-right-operand comment already
/// documents for the R5 inner-FILTER case: "a filter directly on the path's
/// OWN endpoint variable ... still 501s on that separate, generic ground")
/// only supports a FILTER over a plain-column binding. So THIS specific
/// sub-case is STRUCTURALLY UNREACHABLE regardless of ADR-0033: there is no
/// SPARQL query that filters a bare path-endpoint variable and does NOT hit
/// this v1 boundary, whether or not the path is joined. Verified live on
/// BOTH the subject and object endpoints, both pre- and post- this diff's
/// join sites (`InnerJoin`), confirming NO NEW regression — the same 501,
/// for the same reason, that a solo (unjoined) path would already hit.
#[test]
fn filter_on_path_endpoint_hits_the_same_pre_existing_v1_boundary() {
    let maps = sf_mapping::parse_r2rml(RJ_R2RML).expect("R2RML parses");
    let subj_filtered = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?nm WHERE { \
         ?s ex:reaches+ ?o . FILTER(?s != <http://ex/n/1>) . ?o ex:name ?nm }",
    );
    let obj_filtered = parse(
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?nm WHERE { \
         ?s ex:reaches+ ?o . FILTER(?o != <http://ex/n/2>) . ?o ex:name ?nm }",
    );
    let solo_subj_filtered =
        parse("PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o . FILTER(?s != <http://ex/n/1>) }");
    for (label, q) in [
        ("joined, subject-endpoint filter", &subj_filtered),
        ("joined, object-endpoint filter", &obj_filtered),
        ("solo (unjoined), subject-endpoint filter", &solo_subj_filtered),
    ] {
        let t = tree(&maps, q, &[]);
        assert!(
            matches!(t, Err(Error::Unsupported(ref msg)) if msg.contains("plain column") || msg.contains("FILTER")),
            "{label}: expected the pre-existing v1 FILTER-on-template 501, got {t:?}"
        );
    }
}

/// The positive counterpart: a FILTER on a variable bound ALONGSIDE the path
/// (not the path's own endpoint) — `?nm`, a plain `rr:column` binding from the
/// JOIN PARTNER — is unaffected by the path's presence in the same query, and
/// composes normally. Baseline (no filter): exactly 1 row (s=1,o=2,nm=Bob) —
/// the only closure pair whose `?o` has an `ex:name`. Filtered to exclude
/// nm="Bob": must become EXACTLY 0 rows.
#[test]
fn filter_on_a_plain_column_var_bound_alongside_the_path_still_applies() {
    let baseline = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?nm WHERE { ?s ex:reaches+ ?o . ?o ex:name ?nm }",
    );
    assert_eq!(baseline, 1, "only (1,2,Bob) — o=3/4 from other closure pairs have no ex:name");

    let filtered = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?nm WHERE { \
         ?s ex:reaches+ ?o . ?o ex:name ?nm . FILTER(?nm != \"Bob\") }",
    );
    assert_eq!(
        filtered, 0,
        "FILTER(?nm != \"Bob\") must exclude the ONLY baseline row — proving a filter on a \
         plain-column var bound alongside a converted path scan is correctly applied"
    );
}

/// A path in ONE arm of a `UNION`, an ORDINARY (non-path) pattern in the
/// OTHER arm, joined with a THIRD pattern AFTER the union closes:
/// `Join(Union(Path, Ordinary), C)`. `IqNode::Union`'s handler returns
/// multiple branches (one per arm); `InnerJoin`'s per-child loop calls
/// `convert_path_branches` on EACH child's `lower_node` output BEFORE
/// `join_branches` — so the Union child's own multi-branch output (one
/// path-carrying, one not) is converted branch-by-branch, THEN cross-joined
/// with `C`'s branch. `ex:reaches+` closure (4 pairs) UNION `ex:reaches`
/// direct (bag, 3 pairs: (1,2),(2,3),(1,4)) = 7-row bag (with (1,2),(2,3),
/// (1,4) each appearing twice — once via each arm), every row's `?s` in
/// {1,2}, both named — so ALL 7 survive the join with `?s ex:name ?nm`.
#[test]
fn path_inside_union_arm_joined_after_union_matches_oracle() {
    let n = assert_differential(
        RJ_SQL,
        RJ_R2RML,
        RJ_TTL,
        "PREFIX ex: <http://ex/> SELECT ?s ?o ?nm WHERE { \
         { { ?s ex:reaches+ ?o } UNION { ?s ex:reaches ?o } } . ?s ex:name ?nm }",
    );
    assert_eq!(
        n, 7,
        "closure (4) + direct-bag (3), every row's ?s named — full 7-row bag union survives"
    );
}
