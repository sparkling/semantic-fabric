//! ADR-0035 — `GRAPH ?g` (variable-graph querying) differential coverage: the
//! test contract's items 1-4, 6-7 (item 5, star-under-named-graph, lives in
//! `differential_star.rs` per the ADR's own file split — it needs the star
//! encoding's fixtures already established there).
//!
//! Copied-helper pattern: a Cargo integration test is its own crate, so the
//! harness plumbing is duplicated from `adversarial_testaudit.rs` (the newest
//! landed idiom: the oracle is the mapping's OWN materialization —
//! `exec::dump_quads` -> `graph::quads_to_dataset`, a genuine SET — rather
//! than a hand-authored, independently-transcribed Turtle/N-Quads graph that
//! could silently drift out of sync with the SQL fixture) rather than
//! imported (no `pub` cross-file surface). `require_translate_then_rows`
//! forces BOTH engines to actually translate before comparing rows, so a
//! regression to `Unsupported` reds instead of a `diff().is_empty()`-shaped
//! cell passing vacuously on a mutual 501 (the same anti-false-green argument
//! `adversarial_testaudit.rs` makes).

use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, Tbox};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};
use std::collections::BTreeMap;

use oxrdf::Term;

/// A SELECT answer as a comparable bag.
type Rows = Vec<BTreeMap<String, Term>>;

// ============================================================================
// Harness plumbing — duplicated from `adversarial_testaudit.rs`. Semantics
// identical.
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

fn run_select(plan: &Plan, conn: &Connection) -> Rows {
    oracle::engine_bag(&exec::select(plan, conn).expect("select exec"))
}

/// The spareval oracle's SELECT row bag over the mapping's own materialization
/// (a genuine SET — `graph::quads_to_dataset` folds physically-duplicate quads,
/// exactly what item 7's ADR-0034 D1 interaction cell needs on the oracle side
/// too).
fn oracle_bag(create: &str, r2rml: &str, query: &str) -> Rows {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let g = graph::quads_to_dataset(&quads);
    match oracle::evaluate(&g, query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected Solutions, got {other:?}"),
    }
}

/// Strengthened differential: BOTH engines MUST translate WITHOUT a 501 (never
/// masking a regression as a vacuous mutual-501 empty match), then both engines'
/// row bags AND the oracle's must all agree, and the row count is pinned.
fn assert_both_match_oracle(
    create: &str,
    r2rml: &str,
    query: &str,
    expected_len: usize,
    msg: &str,
) {
    let (flat, tree) = translate_both(create, r2rml, query);
    let fp = flat.unwrap_or_else(|e| panic!("flat 501'd `{query}` ({msg}): {e:?}"));
    let tp = tree.unwrap_or_else(|e| panic!("tree 501'd `{query}` ({msg}): {e:?}"));
    let conn = sqlite::load(create).expect("fixture loads");
    let flat_rows = run_select(&fp, &conn);
    let tree_rows = run_select(&tp, &conn);
    let oracle_rows = oracle_bag(create, r2rml, query);
    assert!(
        oracle::solutions_bag_eq(&flat_rows, &oracle_rows),
        "flat vs oracle divergence on `{query}` ({msg}):\n flat={flat_rows:#?}\n oracle={oracle_rows:#?}"
    );
    assert!(
        oracle::solutions_bag_eq(&tree_rows, &oracle_rows),
        "tree vs oracle divergence on `{query}` ({msg}):\n tree={tree_rows:#?}\n oracle={oracle_rows:#?}"
    );
    assert_eq!(tree_rows.len(), expected_len, "{msg}: {tree_rows:#?}");
}

/// Both engines must refuse (sound 501), for a pinned boundary cell.
fn assert_both_501(create: &str, r2rml: &str, query: &str, msg: &str) {
    let (flat, tree) = translate_both(create, r2rml, query);
    assert!(
        matches!(flat, Err(Error::Unsupported(_))),
        "flat must 501 ({msg}): {flat:?}"
    );
    assert!(
        matches!(tree, Err(Error::Unsupported(_))),
        "tree must 501 ({msg}): {tree:?}"
    );
}

// ============================================================================
// Fixture TG — "two graphs": TWO constant named graphs (g1, g2) each carrying
// BOTH an `ex:reaches` and an `ex:owns` triples-map, plus a THIRD, default-
// graph-only triples-map (`ex:note`, no `rr:graphMap` at all). Serves
// contract items 1 (bgp wildcard), 2 (same-graph join correlation), 4 (path
// enumeration over the declared constant graphs). No table declares a
// PRIMARY KEY (matching `adversarial_adr0033_refute.rs`'s GJ_* convention) —
// harmless here since none of TG's data is physically duplicated, so ADR-0034
// D1's conservative DISTINCT never changes a row count.
// ============================================================================

const TG_SQL: &str = r#"
CREATE TABLE reaches_g1 (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO reaches_g1 VALUES (1, 2);
INSERT INTO reaches_g1 VALUES (2, 3);
CREATE TABLE reaches_g2 (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO reaches_g2 VALUES (5, 6);
CREATE TABLE owns_g1 (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO owns_g1 VALUES (1, 100);
CREATE TABLE owns_g2 (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO owns_g2 VALUES (5, 500);
CREATE TABLE notes_default (id INTEGER NOT NULL, txt TEXT NOT NULL);
INSERT INTO notes_default VALUES (1, 'hello');
"#;

const TG_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#ReachesG1>
    rr:logicalTable [ rr:tableName "reaches_g1" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ; rr:graphMap [ rr:constant <http://ex/g1> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
<#ReachesG2>
    rr:logicalTable [ rr:tableName "reaches_g2" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ; rr:graphMap [ rr:constant <http://ex/g2> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
<#OwnsG1>
    rr:logicalTable [ rr:tableName "owns_g1" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ; rr:graphMap [ rr:constant <http://ex/g1> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:owns ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
<#OwnsG2>
    rr:logicalTable [ rr:tableName "owns_g2" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ; rr:graphMap [ rr:constant <http://ex/g2> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:owns ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
<#NotesDefault>
    rr:logicalTable [ rr:tableName "notes_default" ] ;
    rr:subjectMap [ rr:template "http://ex/note/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:note ; rr:objectMap [ rr:column "txt" ] ] .
"#;

/// Contract item 1: `GRAPH ?g { ?s ?p ?o }` over 2 constant named graphs plus a
/// default-graph-only triples-map — only the named-graph triples answer (the
/// default-graph `ex:note` row is excluded, SPARQL §13.3 named-graphs-only),
/// `?g` bound to the right constant per row.
#[test]
fn graph_var_bgp_wildcard_only_named_triples_default_excluded() {
    assert_both_match_oracle(
        TG_SQL,
        TG_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }",
        5, // reaches_g1 (2) + reaches_g2 (1) + owns_g1 (1) + owns_g2 (1); notes_default excluded
        "wildcard BGP under GRAPH ?g: named triples only",
    );
}

/// Contract item 2: same-graph join correlation. `ex:reaches` has 3 branches
/// (2 in g1, 1 in g2); `ex:owns` has 2 (1 in g1, 1 in g2) — a full cross
/// product would be 3*2=6, but sharing `?g` inside ONE `GRAPH ?g` block means
/// only SAME-graph combinations survive (g1: 2*1=2, g2: 1*1=1) via ordinary
/// unification — no special-casing, exactly the ADR's "free" claim.
#[test]
fn graph_var_same_graph_join_correlation_excludes_cross_graph_combos() {
    assert_both_match_oracle(
        TG_SQL,
        TG_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o ?s2 ?o2 WHERE { \
         GRAPH ?g { ?s ex:reaches ?o . ?s2 ex:owns ?o2 } }",
        3, // g1: 2 reaches x 1 owns = 2; g2: 1 x 1 = 1; cross-graph (would-be 6-3=3) excluded
        "same-?g correlation: cross-graph combinations excluded",
    );
}

/// Contract item 4: `GRAPH ?g` + a property path over the mapping's declared
/// CONSTANT named graphs — compiled as the union-over-declared-constant-graphs
/// (`declared_constant_graphs`/`path_branches_for_graph_var`), each arm's own
/// closure already proven correct by `adversarial_adr0033_refute.rs`'s
/// `path_plus_inside_named_graph_returns_only_that_graphs_closure`.
#[test]
fn graph_var_path_enumerates_declared_constant_graphs() {
    assert_both_match_oracle(
        TG_SQL,
        TG_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:reaches+ ?o } }",
        4, // g1 closure {(1,2),(2,3),(1,3)} = 3, g2 closure {(5,6)} = 1
        "path closure enumerated per declared constant graph",
    );
}

// ============================================================================
// Fixture PD — "person/dept": a triples-map whose graph is a TEMPLATE
// (`http://ex/{dept}`, rendered per row) sharing the SAME graph namespace as
// TG's constant `<http://ex/g1>`/`<http://ex/g2>` graphs, plus its OWN copy of
// the `ex:reaches` constant-graph triples-maps so a query can join across the
// two shapes on `?g` (Const/Template unification, contract item 3). Also
// serves contract item 6: this mapping has a template `rr:graphMap`
// SOMEWHERE, so a property path under `GRAPH ?g` here must hit the pinned
// 501 (`has_non_constant_graph_map`) — unlike the identical query shape
// against TG (no template graph maps anywhere), which fully answers (item 4
// above).
// ============================================================================

const PD_SQL: &str = r#"
CREATE TABLE person_dept (id INTEGER NOT NULL, name TEXT NOT NULL, dept TEXT NOT NULL);
INSERT INTO person_dept VALUES (1, 'Alice', 'g1');
INSERT INTO person_dept VALUES (2, 'Bob', 'g2');
INSERT INTO person_dept VALUES (3, 'Carola', 'g3');
CREATE TABLE reaches_g1 (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO reaches_g1 VALUES (1, 2);
INSERT INTO reaches_g1 VALUES (2, 3);
CREATE TABLE reaches_g2 (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO reaches_g2 VALUES (5, 6);
"#;

const PD_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#PersonDept>
    rr:logicalTable [ rr:tableName "person_dept" ] ;
    rr:subjectMap [
        rr:template "http://ex/n/p{id}" ;
        rr:graphMap [ rr:template "http://ex/{dept}" ; rr:termType rr:IRI ]
    ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
<#ReachesG1>
    rr:logicalTable [ rr:tableName "reaches_g1" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ; rr:graphMap [ rr:constant <http://ex/g1> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
<#ReachesG2>
    rr:logicalTable [ rr:tableName "reaches_g2" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ; rr:graphMap [ rr:constant <http://ex/g2> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
"#;

/// Contract item 3 (binding half): `?g` bound from a rendered TEMPLATE graph
/// map, one branch per row (`http://ex/{dept}` evaluated per `person_dept` row).
#[test]
fn graph_var_template_graph_map_binds_rendered_graph_per_row() {
    assert_both_match_oracle(
        PD_SQL,
        PD_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?n WHERE { GRAPH ?g { ?s ex:name ?n } }",
        3,
        "template graph map: one ?g binding per row, rendered from `dept`",
    );
}

/// Contract item 3 (unification half): the template-rendered `?g` (from
/// `ex:name`) must unify with the CONSTANT `?g` bound by `ex:reaches`'s branches
/// (Const/Template unification, `crate::unify::unify_const_derived` — the SAME
/// mechanism ordinary subject/object positions already exercise, reused here
/// for the graph position). `dept` values `"g1"`/`"g2"`/`"g3"` render to
/// exactly `http://ex/g1`/`http://ex/g2`/`http://ex/g3` — matching TG's own
/// constant graphs for g1/g2, with g3 present only on the template side (no
/// `ex:reaches` data there) to prove the unification actually EXCLUDES a
/// non-matching graph rather than admitting every combination.
#[test]
fn graph_var_template_graph_unifies_with_constant_graph_from_sibling_branch() {
    assert_both_match_oracle(
        PD_SQL,
        PD_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?n ?s2 ?o2 WHERE { \
         GRAPH ?g { ?s ex:name ?n . ?s2 ex:reaches ?o2 } }",
        3, // g1: 1 name x 2 reaches = 2; g2: 1 x 1 = 1; g3: 1 x 0 = 0 (no reaches data)
        "Const/Template unification on ?g: g3 (template-only) contributes nothing",
    );
}

/// Contract item 6: pinned 501 — a property path under `GRAPH ?g` where the
/// mapping declares a TEMPLATE/column `rr:graphMap` SOMEWHERE
/// (`PersonDept`'s) is row-dependent, not statically enumerable, so
/// `path_branches_for_graph_var` refuses outright rather than guessing over
/// just the constant subset. Contrast with `graph_var_path_enumerates_
/// declared_constant_graphs` above: the IDENTICAL query shape against TG (no
/// template graph maps anywhere in that mapping) fully answers.
#[test]
fn graph_var_path_under_template_graph_map_is_pinned_501() {
    assert_both_501(
        PD_SQL,
        PD_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:reaches+ ?o } }",
        "a template rr:graphMap exists in this mapping (PersonDept) — row-dependent graph set",
    );
}

// ============================================================================
// Fixture DG — "dup graph": an UNKEYED table with two PHYSICALLY IDENTICAL
// rows (no declared PRIMARY KEY — ADR-0034 D1's "cannot prove duplicate-free"
// trigger), plus a second, single-row table whose triples-map declares TWO
// `rr:graphMap`s at once (R2RML §7.4: one triple visible in two named graphs
// simultaneously). Serves contract item 5 (ADR-0034 interaction): "?g is one
// more output-determining binding; dedup/elision extend unchanged... a POM
// with multiple graph maps multiplies branches, not rows-within-a-branch".
// ============================================================================

const DG_SQL: &str = r#"
CREATE TABLE dup_g1 (a INTEGER NOT NULL, b INTEGER NOT NULL);
INSERT INTO dup_g1 VALUES (1, 2);
INSERT INTO dup_g1 VALUES (1, 2);
CREATE TABLE multi_g (a INTEGER NOT NULL, b INTEGER NOT NULL);
INSERT INTO multi_g VALUES (7, 8);
"#;

const DG_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#DupG1>
    rr:logicalTable [ rr:tableName "dup_g1" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{a}" ; rr:graphMap [ rr:constant <http://ex/g1> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:dup ; rr:objectMap [ rr:template "http://ex/n/{b}" ] ] .
<#MultiG>
    rr:logicalTable [ rr:tableName "multi_g" ] ;
    rr:subjectMap [
        rr:template "http://ex/n/{a}" ;
        rr:graphMap [ rr:constant <http://ex/g1> ] ;
        rr:graphMap [ rr:constant <http://ex/g2> ]
    ] ;
    rr:predicateObjectMap [ rr:predicate ex:multi ; rr:objectMap [ rr:template "http://ex/n/{b}" ] ] .
"#;

/// Contract item 5 (dedup half): `dup_g1` is UNKEYED with 2 physically-identical
/// rows — ADR-0034 D1 forces `SELECT DISTINCT`, and that must still collapse
/// them to ONE row even though the branch now ALSO carries a `?g` binding
/// (`TermDef::Const`, contributing zero raw columns to the projection — see
/// `Branch::projection`/`TermDef::columns` — so it cannot widen the dedup key).
/// The oracle side proves the same thing independently: `graph::quads_to_
/// dataset` folds the two physically-identical materialized quads into ONE
/// dataset entry (a genuine SET), so `expected_len == 1` is not engine-specific.
#[test]
fn graph_var_adr0034_d1_dedup_still_collapses_unkeyed_duplicate_rows() {
    assert_both_match_oracle(
        DG_SQL,
        DG_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:dup ?o } }",
        1,
        "D1 dedup survives a ?g binding: 2 physical rows collapse to 1",
    );
}

/// Contract item 5 (fan-out half): a POM declaring TWO `rr:graphMap`s
/// (R2RML §7.4 — one triple visible in both named graphs at once) must
/// multiply into TWO branches, one per graph, for the SAME single underlying
/// row — never collapsed together (they carry DIFFERENT `?g` values, so they
/// are different solutions) and never treated as an ambiguity/501.
#[test]
fn graph_var_single_pom_multiple_graph_maps_multiplies_branches() {
    assert_both_match_oracle(
        DG_SQL,
        DG_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:multi ?o } }",
        2, // (g=g1, s=n/7, o=n/8) and (g=g2, s=n/7, o=n/8) — same row, two graphs
        "one POM, two rr:graphMaps: one branch per declared graph",
    );
}

const BN_SQL: &str = r#"
CREATE TABLE bn (id TEXT NOT NULL, left_value TEXT NOT NULL, right_value TEXT NOT NULL, named_graph TEXT NOT NULL, default_graph TEXT NOT NULL);
INSERT INTO bn VALUES ('shared', 'left', 'right', 'http://ex/g1', 'http://www.w3.org/ns/r2rml#defaultGraph');
"#;

const BN_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#DefaultStatic> rr:logicalTable [ rr:tableName "bn" ]; rr:subjectMap [ rr:template "{id}"; rr:termType rr:BlankNode ];
  rr:predicateObjectMap [ rr:predicate ex:ds; rr:objectMap [ rr:column "left_value" ] ].
<#DefaultDynamic> rr:logicalTable [ rr:tableName "bn" ]; rr:subjectMap [ rr:template "{id}"; rr:termType rr:BlankNode; rr:graphMap [ rr:column "default_graph"; rr:termType rr:IRI ] ];
  rr:predicateObjectMap [ rr:predicate ex:dd; rr:objectMap [ rr:column "right_value" ] ].
<#NamedConstant> rr:logicalTable [ rr:tableName "bn" ]; rr:subjectMap [ rr:template "{id}"; rr:termType rr:BlankNode; rr:class ex:Scoped; rr:graphMap [ rr:constant <http://ex/g1> ] ];
  rr:predicateObjectMap [ rr:predicate ex:nc; rr:objectMap [ rr:column "left_value" ] ].
<#NamedDynamic> rr:logicalTable [ rr:tableName "bn" ]; rr:subjectMap [ rr:template "{id}"; rr:termType rr:BlankNode; rr:graphMap [ rr:column "named_graph"; rr:termType rr:IRI ] ];
  rr:predicateObjectMap [ rr:predicate ex:nd; rr:objectMap [ rr:column "right_value" ] ].
<#NamedOther> rr:logicalTable [ rr:tableName "bn" ]; rr:subjectMap [ rr:template "{id}"; rr:termType rr:BlankNode; rr:graphMap [ rr:constant <http://ex/g2> ] ];
  rr:predicateObjectMap [ rr:predicate ex:no; rr:objectMap [ rr:column "right_value" ] ].
<#ChildRef> rr:logicalTable [ rr:tableName "bn" ]; rr:subjectMap [ rr:template "http://ex/child/{id}"; rr:graphMap [ rr:constant <http://ex/g2> ] ];
  rr:predicateObjectMap [ rr:predicate ex:ref; rr:objectMap [
    rr:parentTriplesMap <#NamedConstant>; rr:joinCondition [ rr:child "id"; rr:parent "id" ] ] ].
<#DirectObject> rr:logicalTable [ rr:tableName "bn" ]; rr:subjectMap [ rr:template "http://ex/owner/{id}"; rr:graphMap [ rr:constant <http://ex/g1> ] ];
  rr:predicateObjectMap [ rr:predicate ex:blank; rr:objectMap [ rr:template "{id}"; rr:termType rr:BlankNode ] ].
"#;

fn assert_bn(query: &str, expected_len: usize, msg: &str) {
    assert_both_match_oracle(BN_SQL, BN_R2RML, query, expected_len, msg);
}

#[test]
fn generated_blank_nodes_share_identity_only_within_the_effective_graph() {
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:ds ?a; ex:dd ?b }",
        1,
        "implicit and dynamically generated rr:defaultGraph share identity",
    );
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?g ?s WHERE { GRAPH ?g { ?s ex:nc ?a; ex:nd ?b } }",
        1,
        "constant and dynamic graph maps resolving to g1 share identity",
    );
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { GRAPH <http://ex/g1> { ?s a ex:Scoped; ex:nc ?v } }",
        1,
        "class atoms and ordinary triples reconstruct the same scoped subject",
    );
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?v WHERE { GRAPH <http://ex/g1> { <http://ex/owner/shared> ex:blank/ex:nc ?v } }",
        1,
        "direct blank objects remain scoped through property-path endpoints",
    );
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { { SELECT DISTINCT ?s WHERE { GRAPH <http://ex/g1> { ?s ex:nc ?a } } } GRAPH <http://ex/g1> { ?s ex:nd ?b } }",
        1,
        "DISTINCT subplan projection preserves the graph component",
    );
}

#[test]
fn generated_blank_nodes_never_join_across_target_graphs() {
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { GRAPH <http://ex/g1> { ?s ex:nc ?a } GRAPH <http://ex/g2> { ?s ex:no ?b } }",
        0, "equal generated identifiers in g1 and g2 denote different blank nodes",
    );
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:ds ?a GRAPH <http://ex/g1> { ?s ex:nc ?b } }",
        0, "default-graph and named-graph blank nodes are distinct",
    );
}

#[test]
fn rr_default_graph_is_not_addressable_as_a_named_graph() {
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { GRAPH <http://www.w3.org/ns/r2rml#defaultGraph> { ?s ex:dd ?o } }",
        0, "rr:defaultGraph denotes the default destination, never a named graph",
    );
}

#[test]
fn referencing_object_blank_node_uses_the_child_triples_target_graph() {
    assert_bn(
        "PREFIX ex: <http://ex/> SELECT ?o WHERE { GRAPH <http://ex/g2> { ?c ex:ref ?o . ?o ex:no ?v } }",
        1,
        "the parent lexical identifier is scoped to the child reference triple's g2 target",
    );
}

#[test]
fn quad_dump_carries_the_graph_scoped_identity_directly() {
    let conn = sqlite::load(BN_SQL).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(BN_R2RML).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let subject = |predicate: &str| {
        quads
            .iter()
            .find(|quad| quad.predicate.as_str() == predicate)
            .expect("predicate quad")
            .subject
            .to_string()
    };
    let default = subject("http://ex/ds");
    let g1 = subject("http://ex/nc");
    let g2 = subject("http://ex/no");
    assert_eq!(default, subject("http://ex/dd"));
    assert_eq!(g1, subject("http://ex/nd"));
    assert_ne!(default, g1);
    assert_ne!(g1, g2);
    let referenced = quads
        .iter()
        .find(|quad| quad.predicate.as_str() == "http://ex/ref")
        .expect("reference quad")
        .object
        .to_string();
    assert_eq!(referenced, g2);
}
