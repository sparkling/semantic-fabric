//! ADR-0023 M8 **flat-oracle differential** — after the M8 default flip, the flat
//! [`sf_sparql::translate_with_flat`] path is the `=_bag` oracle and the tree
//! ([`sf_sparql::translate_tree`]) is now the production default. This harness
//! still verifies tree `=_bag` flat over the full corpus (W3C, multiplicity-stress,
//! spareval cross-check) so any regression is caught immediately.
//!
//! For every query in the corpus this runs BOTH translators and asserts:
//!
//! * **(a) row-bag parity** — when both `Ok`, execute both `Plan`s over the SAME SQLite
//!   fixture and compare the result MULTISETS *with counts* (sort + count, never
//!   set-dedup, which hides multiplicity bugs): SELECT via [`oracle::solutions_bag_eq`],
//!   CONSTRUCT via a sorted N-Triples multiset, ASK via boolean equality.
//! * **(b) identical 501 set** — the tree path returns `Err(Unsupported)` EXACTLY when
//!   the flat path does (no new silent passes, no new failures).
//! * **(c) modifier interaction** — LIMIT/OFFSET/ORDER/DISTINCT, single-branch SQL push
//!   vs multi-branch exec, are all in the corpus.
//! * **(d) independent oracle** — where a hand-authored expected graph exists, the tree
//!   result is ALSO diffed against the independent `spareval` oracle (a shared-primitive
//!   defect cannot hide from a tree-vs-flat diff alone — R5).
//!
//! Plus **R5 multiplicity-stress fixtures** (§7): the W3C corpus is predominantly
//! primary-keyed / set-like and will NOT surface a multiplicity bug, so this adds
//! duplicate-row union arms, overlapping/redundant triples-maps emitting the same
//! predicate, non-unique self-/refObject-join keys, OPTIONAL null-pad over duplicates,
//! and aggregate-over-union — and runs the same tree-vs-flat (and, where set-faithful,
//! vs spareval) multiset assertion over them.

use std::path::PathBuf;

use rusqlite::Connection;
use sf_conformance::graph::{isomorphic, parse_turtle, triples_to_dataset};
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::sqlite;
use sf_sparql::iq::SqlCond;
use sf_sparql::{exec, translate_tree, translate_with_flat, Error, Plan, PlanForm, Tbox};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};

/// Base IRI for the hand-authored expected graphs (matches the mapping templates).
const BASE: &str = "http://ex/";

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
    translate_tree(q, maps, &Tbox::default(), Dialect::Sqlite, schema)
}

/// A plan's executed answer, in a form directly comparable as a MULTISET-with-counts.
#[derive(Debug)]
enum Answer {
    /// SELECT — a bag of solutions (compared via [`oracle::solutions_bag_eq`]).
    Select(Vec<std::collections::BTreeMap<String, oxrdf::Term>>),
    /// CONSTRUCT/DESCRIBE — the produced triples as a SORTED N-Triples multiset
    /// (duplicates kept, so a multiplicity bug fails the `Vec` equality).
    Construct(Vec<String>),
    /// ASK.
    Ask(bool),
}

/// Execute a plan over the live SQLite fixture into its comparable [`Answer`].
fn run(plan: &Plan, conn: &Connection) -> Answer {
    match &plan.form {
        PlanForm::Select { .. } => Answer::Select(oracle::engine_bag(
            &exec::select(plan, conn).expect("select exec"),
        )),
        PlanForm::Construct { .. } => {
            let mut v: Vec<String> = exec::construct_triples(plan, conn)
                .expect("construct exec")
                .iter()
                .map(|t| t.to_string())
                .collect();
            v.sort();
            Answer::Construct(v)
        }
        PlanForm::Ask => Answer::Ask(exec::ask(plan, conn).expect("ask exec")),
    }
}

/// Multiset equality of two answers (counts significant).
fn answers_eq(a: &Answer, b: &Answer) -> bool {
    match (a, b) {
        (Answer::Select(x), Answer::Select(y)) => oracle::solutions_bag_eq(x, y),
        (Answer::Construct(x), Answer::Construct(y)) => x == y, // both pre-sorted
        (Answer::Ask(x), Answer::Ask(y)) => x == y,
        _ => false,
    }
}

/// The core differential over one inline fixture + query. Asserts (a) flat/tree row-bag
/// parity and (b) identical 501 outcome; when `ttl` is `Some`, also (d) the tree answer
/// vs the independent `spareval` oracle over the hand-authored expected graph.
fn diff(create: &str, r2rml: &str, ttl: Option<&str>, query: &str) {
    let conn = sqlite::load(create).expect("fixture loads");
    // Introspect the live fixture so BOTH paths optimize with the SAME schema (ADR-0023
    // M4 wave 1): the cascade is `=_bag`-preserving, so the optimized tree result still
    // equals the optimized flat result — the property this differential proves.
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);
    let f = flat(&maps, &q, &schema);
    let t = tree(&maps, &q, &schema);

    match (&f, &t) {
        // (b) identical 501 set — both defer.
        (Err(Error::Unsupported(_)), Err(Error::Unsupported(_))) => {}
        (Ok(fp), Ok(tp)) => {
            // (a) row-bag parity with counts.
            let fa = run(fp, &conn);
            let ta = run(tp, &conn);
            assert!(
                answers_eq(&fa, &ta),
                "flat vs tree row-bag divergence on `{query}`:\n flat={fa:#?}\n tree={ta:#?}"
            );
            // (d) independent spareval oracle over the hand-authored expected graph.
            if let Some(ttl) = ttl {
                assert_vs_spareval(ttl, query, tp, &conn);
            }
        }
        _ => panic!(
            "501-set mismatch on `{query}` (flat and tree must agree on Unsupported):\n flat={f:?}\n tree={t:?}"
        ),
    }
}

/// (d) Diff the tree plan's live answer against the independent `spareval` oracle over
/// the hand-authored expected graph. SELECT/ASK compare as bags/booleans; CONSTRUCT
/// compares by blank-node-aware dataset isomorphism (the gold is set-valued).
fn assert_vs_spareval(ttl: &str, query: &str, tp: &Plan, conn: &Connection) {
    let g = parse_turtle(ttl, BASE).expect("expected graph parses");
    let oracle = oracle::evaluate(&g, query).expect("oracle eval");
    match (&tp.form, oracle) {
        (PlanForm::Select { .. }, OracleAnswer::Solutions(orows)) => {
            let tb = oracle::engine_bag(&exec::select(tp, conn).expect("select exec"));
            assert!(
                oracle::solutions_bag_eq(&tb, &orows),
                "tree vs spareval divergence on `{query}`:\n tree={tb:#?}\n oracle={orows:#?}"
            );
        }
        (PlanForm::Ask, OracleAnswer::Boolean(ob)) => {
            assert_eq!(
                exec::ask(tp, conn).expect("ask exec"),
                ob,
                "tree vs spareval ASK divergence on `{query}`"
            );
        }
        (PlanForm::Construct { .. }, OracleAnswer::Graph(og)) => {
            let tris = exec::construct_triples(tp, conn).expect("construct exec");
            assert!(
                isomorphic(&triples_to_dataset(&tris), &og),
                "tree vs spareval CONSTRUCT divergence on `{query}`"
            );
        }
        (form, other) => panic!("oracle/plan form mismatch on `{query}`: {form:?} vs {other:?}"),
    }
}

/// `diff` with no spareval oracle — for fixtures whose virtual graph is NOT set-faithful
/// (overlapping triples-maps emit identical triples that collapse in a real RDF graph
/// but stay a bag in the engine; R5's overlapping-map coverage gap). flat-vs-tree IS the
/// `=_bag` gate here — the property this milestone proves.
fn diff_bag_only(create: &str, r2rml: &str, query: &str) {
    diff(create, r2rml, None, query);
}

// ============================================================================
// Fixture P — person ⟕ dept (the proven differential_oracle fixture), with a
// hand-authored EXPECTED graph so every query is also diffed vs spareval (d).
// ============================================================================

const P_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    dept_id INTEGER NOT NULL,
    email TEXT,
    FOREIGN KEY (dept_id) REFERENCES dept(id)
);
INSERT INTO dept VALUES (10, 'Sales');
INSERT INTO person VALUES (1, 'Ann', 10, 'ann@x');
INSERT INTO person VALUES (2, 'Bob', 10, NULL);
INSERT INTO person VALUES (3, 'Zed', 10, 'zed@x');
"#;

const P_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name  ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:email ; rr:objectMap [ rr:column "email" ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:dept ;
        rr:objectMap [
            rr:parentTriplesMap <#Dept> ;
            rr:joinCondition [ rr:child "dept_id" ; rr:parent "id" ]
        ]
    ] .
<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const P_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/person/1> ex:name "Ann" ; ex:dept <http://ex/dept/10> ; ex:email "ann@x" .
<http://ex/person/2> ex:name "Bob" ; ex:dept <http://ex/dept/10> .
<http://ex/person/3> ex:name "Zed" ; ex:dept <http://ex/dept/10> ; ex:email "zed@x" .
<http://ex/dept/10> ex:label "Sales" .
"#;

/// Diff a query over fixture P against BOTH the flat oracle AND spareval. Use only for
/// SET-FAITHFUL queries — ones that do NOT expose the nullable `email` column as a bare
/// (non-OPTIONAL) arm, because the flat oracle emits a NULL column object as an UNBOUND
/// row (R2RML-imprecise but the established oracle — out of M3 scope; flat==tree still
/// holds), which a strict set-graph oracle like spareval does not.
fn diff_p(query: &str) {
    diff(P_SQL, P_R2RML, Some(P_TTL), query);
}

/// Diff a query over fixture P flat-vs-tree ONLY (no spareval) — for queries that expose
/// the nullable `email` column as a bare/EXISTS/MINUS arm, where the flat oracle's
/// NULL-as-unbound behavior diverges from a set-graph oracle. flat==tree (the `=_bag`
/// gate this milestone proves) still holds and is asserted.
fn diff_p_bag(query: &str) {
    diff(P_SQL, P_R2RML, None, query);
}

const PFX: &str = "PREFIX ex: <http://ex/>";

#[test]
fn p_bgp_join_optional_filter_union() {
    // BGP + JOIN + OPTIONAL + FILTER (the canonical differential query). OPTIONAL
    // correctly leaves Bob's NULL email unbound — matching spareval.
    diff_p(&format!(
        "{PFX} SELECT ?name ?label ?email WHERE {{
            ?p ex:name ?name . ?p ex:dept ?d . ?d ex:label ?label .
            OPTIONAL {{ ?p ex:email ?email }} FILTER (?name != \"Zed\") }}"
    ));
    // Plain BGP.
    diff_p(&format!(
        "{PFX} SELECT ?label WHERE {{ ?d ex:label ?label }}"
    ));
    // Self-join on the PK subject.
    diff_p(&format!(
        "{PFX} SELECT ?name ?name2 WHERE {{ ?p ex:name ?name . ?p ex:name ?name2 }}"
    ));
    // refObjectMap join (2-scan InnerJoin).
    diff_p(&format!("{PFX} SELECT ?p ?d WHERE {{ ?p ex:dept ?d }}"));
    // UNION (bag union, multi-branch exec) over NOT-NULL columns — set-faithful.
    diff_p(&format!(
        "{PFX} SELECT ?v WHERE {{ {{ ?p ex:name ?v }} UNION {{ ?d ex:label ?v }} }}"
    ));
}

#[test]
fn p_minus_exists_union_nullable() {
    // These expose the nullable `email` column as a bare/EXISTS/MINUS/UNION arm — the
    // flat oracle's NULL-as-unbound behavior makes them flat-vs-tree-only (still =_bag).
    diff_p_bag(&format!(
        "{PFX} SELECT ?v WHERE {{ {{ ?p ex:name ?v }} UNION {{ ?p ex:email ?v }} }}"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name MINUS {{ ?p ex:email ?e }} }}"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name FILTER EXISTS {{ ?p ex:email ?e }} }}"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name FILTER NOT EXISTS {{ ?p ex:email ?e }} }}"
    ));
}

#[test]
fn p_modifier_interaction() {
    // (c) DISTINCT, ORDER BY, LIMIT/OFFSET — single-branch SQL push paths.
    diff_p(&format!(
        "{PFX} SELECT DISTINCT ?label WHERE {{ ?d ex:label ?label }}"
    ));
    diff_p(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name }} ORDER BY ?name"
    ));
    diff_p(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name }} ORDER BY DESC(?name) LIMIT 2"
    ));
    diff_p(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name }} ORDER BY ?name LIMIT 1 OFFSET 1"
    ));
    // LIMIT/OFFSET over a multi-branch (UNION) inner — exec-applied modifiers (set-
    // faithful: both arms are NOT-NULL columns).
    diff_p(&format!(
        "{PFX} SELECT ?v WHERE {{ {{ ?p ex:name ?v }} UNION {{ ?d ex:label ?v }} }} ORDER BY ?v LIMIT 2"
    ));
}

#[test]
fn p_bind_values_construct_ask() {
    // BIND (symbolic BindDef::Expr resolved per leaf-CQ at LOWER).
    diff_p(&format!(
        "{PFX} SELECT ?name ?u WHERE {{ ?p ex:name ?name . BIND(UCASE(?name) AS ?u) }}"
    ));
    // VALUES.
    diff_p(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name . VALUES ?name {{ \"Ann\" \"Bob\" }} }}"
    ));
    // CONSTRUCT.
    diff_p(&format!(
        "{PFX} CONSTRUCT {{ ?p ex:n ?name }} WHERE {{ ?p ex:name ?name }}"
    ));
    // The ?s ?p ?o dump CONSTRUCT (every virtual triple).
    diff_p("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }");
    // ASK.
    diff_p(&format!("{PFX} ASK WHERE {{ ?p ex:name \"Ann\" }}"));
    diff_p(&format!("{PFX} ASK WHERE {{ ?p ex:name \"Nobody\" }}"));
    // SELECT * (visible-var projection).
    diff_p(&format!("{PFX} SELECT * WHERE {{ ?p ex:name ?name }}"));
}

#[test]
fn p_aggregation() {
    // GROUP BY + COUNT over a single-branch inner (SQL GROUP BY).
    diff_p(&format!(
        "{PFX} SELECT ?d (COUNT(?p) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d"
    ));
    // COUNT(*) over the whole pattern.
    diff_p(&format!(
        "{PFX} SELECT (COUNT(*) AS ?c) WHERE {{ ?p ex:name ?name }}"
    ));
}

// ============================================================================
// Fixture OD — person ⟕ dept (2 DISTINCT labels) + person ⟕ person (nullable
// self-join `mentor`) — regression coverage for the OPTIONAL anti-join FILTER
// bug (`leftjoin.rs` `not_exists_cond_for`): a multi-scan OPTIONAL right (the
// dept refObjectMap join + the label triple, `core.len() > 1` ⇒ the
// `left_join_branches`/`left_join_decomposed` decomposition, NOT the
// single-scan `build_left_join` shortcut) whose OWN inner FILTER removes the
// only candidate match must NULL-pad the left row, not drop it. Before the
// fix, `not_exists_cond_for` omitted the inner FILTER from its `NOT EXISTS`
// condition, so a right row that exists-but-fails-the-filter still counted
// as "a match exists" for the anti-join — the row vanished from BOTH the
// match branch (filtered out) and the no-match branch (NOT EXISTS wrongly
// false): a silent wrong answer (violates ADR-0007). Fixture P (single dept
// "Sales") can't expose this — the filter needs a SECOND label to discriminate.
// ============================================================================

const OD_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    dept_id INTEGER NOT NULL,
    mentor_id INTEGER,
    FOREIGN KEY (dept_id) REFERENCES dept(id),
    FOREIGN KEY (mentor_id) REFERENCES person(id)
);
INSERT INTO dept VALUES (10, 'Sales');
INSERT INTO dept VALUES (20, 'Eng');
INSERT INTO person VALUES (1, 'Ann', 20, NULL);
INSERT INTO person VALUES (2, 'Bob', 10, 1);
INSERT INTO person VALUES (3, 'Zed', 10, 2);
"#;

const OD_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:dept ;
        rr:objectMap [
            rr:parentTriplesMap <#Dept> ;
            rr:joinCondition [ rr:child "dept_id" ; rr:parent "id" ]
        ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:mentor ;
        rr:objectMap [
            rr:parentTriplesMap <#Person> ;
            rr:joinCondition [ rr:child "mentor_id" ; rr:parent "id" ]
        ]
    ] .
<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const OD_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/person/1> ex:name "Ann" ; ex:dept <http://ex/dept/20> .
<http://ex/person/2> ex:name "Bob" ; ex:dept <http://ex/dept/10> ; ex:mentor <http://ex/person/1> .
<http://ex/person/3> ex:name "Zed" ; ex:dept <http://ex/dept/10> ; ex:mentor <http://ex/person/2> .
<http://ex/dept/10> ex:label "Sales" .
<http://ex/dept/20> ex:label "Eng" .
"#;

fn diff_od(query: &str) {
    diff(OD_SQL, OD_R2RML, Some(OD_TTL), query);
}

#[test]
fn optional_anti_join_filter_match_removing() {
    // THE REPRO: Ann's ONLY candidate dept-label is "Eng", which the inner
    // FILTER excludes. Correct (spareval): Ann NULL-padded on ?label, Bob/Zed
    // keep "Sales" -- 3 rows. Buggy (`not_exists_cond_for` missing the filter):
    // Ann's row vanishes entirely (excluded from the match branch by the
    // filter, but ALSO excluded from the no-match branch because the
    // (unfiltered) NOT EXISTS sees her valid dept FK and wrongly concludes a
    // match exists) -- 2 rows.
    diff_od(&format!(
        "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }}"
    ));
}

#[test]
fn optional_anti_join_filter_no_op_guard() {
    // GUARD against over-correction: a FILTER that never actually excludes any
    // candidate (no dept is labelled "ZZZ") must stay correct before AND after
    // the fix -- every person keeps their real label, 3 rows, nothing unbound.
    diff_od(&format!(
        "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"ZZZ\") }} }}"
    ));
}

#[test]
fn optional_anti_join_filter_nested_optional() {
    // VARIANT (a): the OPTIONAL's own inner FILTER sits ALONGSIDE a nested
    // OPTIONAL in the same group -- per the SPARQL translation algorithm the
    // FILTER's scope is the WHOLE group (it wraps the nested OPTIONAL's
    // LeftJoin too), so this becomes a right-NESTED LeftJoin whose OWN `expr`
    // is the label filter (`left_join_decomposed`'s §5.3 nested-right closure
    // re-feeding `left_join_branches`) -- proves the fix's `expr` threading
    // survives the nested-OPTIONAL flattening, not just the flat multi-scan case.
    diff_od(&format!(
        "{PFX} SELECT ?name ?label ?m WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") \
         OPTIONAL {{ ?p ex:mentor ?m }} }} }}"
    ));
}

#[test]
fn optional_anti_join_filter_nullable_determinant() {
    // VARIANT (c): the shared variable feeding the filtered multi-scan
    // OPTIONAL (?m) is ITSELF nullable -- bound by a PRIOR OPTIONAL that may
    // not match (Ann has no mentor). Exercises the fix alongside the
    // pre-existing R1 null-safe-compat / R2 COALESCE machinery
    // (`def_is_nullable`/`null_safe`) for a left-nullable determinant, not
    // just a mandatory one.
    diff_od(&format!(
        "{PFX} SELECT ?name ?label2 WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:mentor ?m }} \
         OPTIONAL {{ ?m ex:dept ?d2 . ?d2 ex:label ?label2 FILTER(?label2 != \"Eng\") }} }}"
    ));
}

// ============================================================================
// R5 multiplicity-stress fixtures (§7) — force the multiplicity to actually appear.
// ============================================================================

// --- STRESS: emp (non-unique name/dept) + tag (non-unique join key), set-faithful
// (distinct triples per row), so every stress query is ALSO diffed vs spareval. ---

const STRESS_SQL: &str = r#"
CREATE TABLE emp (id INTEGER PRIMARY KEY, name TEXT NOT NULL, dept TEXT NOT NULL);
INSERT INTO emp VALUES (1, 'A', 'd10');
INSERT INTO emp VALUES (2, 'A', 'd10');
INSERT INTO emp VALUES (3, 'B', 'd20');
CREATE TABLE tag (eid INTEGER NOT NULL, lbl TEXT NOT NULL);
INSERT INTO tag VALUES (1, 'x');
INSERT INTO tag VALUES (1, 'y');
INSERT INTO tag VALUES (3, 'z');
"#;

const STRESS_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Emp>
    rr:logicalTable [ rr:tableName "emp" ] ;
    rr:subjectMap [ rr:template "http://ex/emp/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:dept ; rr:objectMap [ rr:template "http://ex/dept/{dept}" ] ] .
<#Tag>
    rr:logicalTable [ rr:tableName "tag" ] ;
    rr:subjectMap [ rr:template "http://ex/emp/{eid}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:tag ; rr:objectMap [ rr:column "lbl" ] ] .
"#;

const STRESS_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/emp/1> ex:name "A" ; ex:dept <http://ex/dept/d10> ; ex:tag "x", "y" .
<http://ex/emp/2> ex:name "A" ; ex:dept <http://ex/dept/d10> .
<http://ex/emp/3> ex:name "B" ; ex:dept <http://ex/dept/d20> ; ex:tag "z" .
"#;

fn diff_stress(query: &str) {
    diff(STRESS_SQL, STRESS_R2RML, Some(STRESS_TTL), query);
}

#[test]
fn r5_i_duplicate_union_arms() {
    // (i) the SAME arm UNIONed with itself ⇒ every solution counted twice (bag union,
    // no dedup). Names {A,A,B} per arm ⇒ a 6-row bag.
    diff_stress(&format!(
        "{PFX} SELECT ?o WHERE {{ {{ ?p ex:name ?o }} UNION {{ ?p ex:name ?o }} }}"
    ));
}

#[test]
fn r5_iii_non_unique_self_join() {
    // (iii) non-unique join key: self-join on the shared dept ⇒ a cartesian per dept.
    // dept d10 has {emp1,emp2} (both name A) ⇒ 4 (A,A) pairs; d20 ⇒ 1 (B,B). Bag of 5.
    diff_stress(&format!(
        "{PFX} SELECT ?n1 ?n2 WHERE {{ ?p ex:name ?n1 . ?q ex:name ?n2 . ?p ex:dept ?d . ?q ex:dept ?d }}"
    ));
}

#[test]
fn r5_iv_optional_null_pad_over_duplicates() {
    // (iv) OPTIONAL whose right side has duplicate matches: emp1 has 2 tags (2 rows),
    // emp2 has 0 (1 null-padded row), emp3 has 1. Bag of 4 with one unbound ?l.
    diff_stress(&format!(
        "{PFX} SELECT ?n ?l WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:tag ?l }} }}"
    ));
}

#[test]
fn r5_v_aggregate_over_union_and_unbound() {
    // (v) aggregate over a UNION (multi-branch ⇒ Rust-group) is the agg-over-UNION case the
    // FLAT oracle defers (ADR-0023 design §4.14): the tree now CLOSES it, so a flat-vs-tree
    // diff would fail the 501-set assertion. Those specs moved to `agg_over_union_tree_*`
    // below, gated vs the independent spareval oracle (the tree EXCEEDS flat by design).
    //
    // Unbound variable carried through a multiplicity-> 1 projection (engine_bag drops
    // the unbound ?missing; tree must match flat AND spareval).
    diff_stress(&format!(
        "{PFX} SELECT ?o ?missing WHERE {{ ?p ex:name ?o }}"
    ));
}

// ============================================================================
// AGG-OVER-UNION — the HEADLINE bug the operator-tree closes (ADR-0023 §4.14): an
// aggregate over a MULTI-branch inner (UNION/VALUES) is lowered to a `Plan::rust_group`
// with the `Aggregation` node owning its scope, so the outer `(agg AS ?v)` Extend renames
// the Rust-group output instead of folding into the pre-group branches (which lack the
// aggregate column → the FLAT path's "BIND references unbound" 501). The FLAT oracle
// DEFERS this shape, so the differential's flat-vs-tree 501-set assertion does NOT apply;
// the rigorous gate is the TREE result `=_bag` the INDEPENDENT spareval oracle over a
// hand-authored set-faithful graph (the tree EXCEEDS flat by design).
// ============================================================================

// All union-arm columns are NOT NULL so the virtual graph is SET-FAITHFUL even for
// COUNT(*) (a nullable column object would be emitted as an UNBOUND solution row that
// COUNT(*) counts but a set-graph oracle does not — the established NULL-as-unbound gap,
// out of scope here; see `diff_p` vs `diff_p_bag`).
const AGG_SQL: &str = r#"
CREATE TABLE m (
    id INTEGER PRIMARY KEY,
    grp TEXT NOT NULL, s1 TEXT NOT NULL, s2 TEXT NOT NULL,
    n1 INTEGER NOT NULL, n2 INTEGER NOT NULL
);
INSERT INTO m VALUES (1, 'g1', 'a', 'b', 10, 1);
INSERT INTO m VALUES (2, 'g1', 'c', 'f', 20, 2);
INSERT INTO m VALUES (3, 'g2', 'd', 'e', 30, 3);
"#;

const AGG_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#M>
    rr:logicalTable [ rr:tableName "m" ] ;
    rr:subjectMap [ rr:template "http://ex/m/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:grp ; rr:objectMap [ rr:column "grp" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p1  ; rr:objectMap [ rr:column "s1" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p2  ; rr:objectMap [ rr:column "s2" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:q1  ; rr:objectMap [ rr:column "n1" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:q2  ; rr:objectMap [ rr:column "n2" ] ] .
"#;

// Set-faithful expected graph (distinct triples per row): an INTEGER column maps to an
// xsd:integer literal (R2RML natural mapping), matching the bare integer literals here.
const AGG_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/m/1> ex:grp "g1" ; ex:p1 "a" ; ex:p2 "b" ; ex:q1 10 ; ex:q2 1 .
<http://ex/m/2> ex:grp "g1" ; ex:p1 "c" ; ex:p2 "f" ; ex:q1 20 ; ex:q2 2 .
<http://ex/m/3> ex:grp "g2" ; ex:p1 "d" ; ex:p2 "e" ; ex:q1 30 ; ex:q2 3 .
"#;

/// An agg-over-UNION spec (ADR-0023 M4 wave 1): assert the FLAT oracle DEFERS (documents
/// the headline gap the tree closes) AND the TREE result `=_bag` the independent spareval
/// oracle over `ttl` (the rigorous gate — not a hand-computed expected alone).
fn agg_union(query: &str) {
    let conn = sqlite::load(AGG_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(AGG_R2RML).expect("R2RML parses");
    let q = parse(query);
    assert!(
        matches!(flat(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "flat oracle must DEFER agg-over-UNION (else not a tree-exceeds-flat spec): `{query}`"
    );
    let tp = tree(&maps, &q, &schema).expect("tree must close agg-over-UNION");
    assert!(
        agg_over_union_is_lowered(&tp),
        "agg-over-UNION must lower to a `rust_group` OR a SQL-pushed-down \
         Aggregation-over-SubPlan (q9 agg-pushdown wave): `{query}`"
    );
    assert_vs_spareval(AGG_TTL, query, &tp, &conn);
}

/// A multi-branch agg-over-UNION closes EITHER via the Rust-level [`Plan::rust_group`]
/// buffer-and-group (the correctness oracle/fallback) OR the SQL pushdown — one
/// [`Branch`] carrying both an `agg` (the `Aggregation`) and a non-empty
/// `subplan_joins` (the pooled `UNION ALL` derived table `try_sql_group_over_union`
/// builds). Exactly one of the two ever fires for a given query (never both, never
/// neither) — this only asserts SOME closure happened, the specific choice is an
/// applicability detail the `=_bag` spareval check right after this is what actually
/// gates correctness.
fn agg_over_union_is_lowered(tp: &Plan) -> bool {
    tp.rust_group.is_some()
        || tp
            .branches
            .iter()
            .any(|b| b.agg.is_some() && !b.subplan_joins.is_empty())
}

/// The per-UNION-arm branches an agg-over-UNION plan actually scans, regardless of
/// which of the two closures (above) fired: the Rust-group path keeps them as
/// `tp.branches` directly; the SQL-pushdown path nests them inside the pooled
/// `SubPlanJoin`'s own `Plan`.
fn agg_over_union_arm_branches(tp: &Plan) -> Vec<&sf_sparql::iq::Branch> {
    if tp.rust_group.is_some() {
        tp.branches.iter().collect()
    } else {
        tp.branches
            .iter()
            .flat_map(|b| b.subplan_joins.iter())
            .flat_map(|sp| sp.plan.branches.iter())
            .collect()
    }
}

#[test]
fn agg_over_union_count() {
    // 1. COUNT(?v) over a UNION, no GROUP BY: {a,b,c,f,d,e} ⇒ 6.
    agg_union(&format!(
        "{PFX} SELECT (COUNT(?v) AS ?c) WHERE {{ {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }}"
    ));
    // 2. COUNT(*) over a UNION, no GROUP BY ⇒ 6 (3 p1 + 3 p2 solutions).
    agg_union(&format!(
        "{PFX} SELECT (COUNT(*) AS ?c) WHERE {{ {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }}"
    ));
    // 3. The HEADLINE: COUNT(?v) GROUP BY a key bound OUTSIDE the union — g1:{a,b,c,f}=4, g2:{d,e}=2.
    agg_union(&format!(
        "{PFX} SELECT ?g (COUNT(?v) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    ));
    // 4. COUNT(*) GROUP BY the outside key — g1=4, g2=2.
    agg_union(&format!(
        "{PFX} SELECT ?g (COUNT(*) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    ));
}

#[test]
fn agg_over_union_min_max() {
    // 5. MIN over the string UNION GROUP BY grp — g1: min{a,b,c,f}=a, g2: min{d,e}=d.
    agg_union(&format!(
        "{PFX} SELECT ?g (MIN(?v) AS ?m) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    ));
    // 6. MAX over the string UNION GROUP BY grp (the `agg_union_max_group_by` spec) —
    //    g1: max{a,b,c,f}=f, g2: max{d,e}=e. Gated vs spareval (no hand-expected).
    agg_union(&format!(
        "{PFX} SELECT ?g (MAX(?v) AS ?m) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    ));
    // 7. MAX over the union with NO group (implicit single group).
    agg_union(&format!(
        "{PFX} SELECT (MAX(?v) AS ?m) WHERE {{ {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }}"
    ));
}

#[test]
fn agg_over_union_sum_avg() {
    // 8. SUM over a NUMERIC UNION (xsd:integer), no group: {10,1,20,2,30,3} ⇒ 66.
    agg_union(&format!(
        "{PFX} SELECT (SUM(?v) AS ?t) WHERE {{ {{ ?x ex:q1 ?v }} UNION {{ ?x ex:q2 ?v }} }}"
    ));
    // 9. SUM over the numeric union GROUP BY grp — g1: 10+1+20+2=33, g2: 30+3=33.
    agg_union(&format!(
        "{PFX} SELECT ?g (SUM(?v) AS ?t) WHERE {{ ?x ex:grp ?g . \
         {{ ?x ex:q1 ?v }} UNION {{ ?x ex:q2 ?v }} }} GROUP BY ?g"
    ));
    // 10. Two aggregates at once over the union GROUP BY grp.
    agg_union(&format!(
        "{PFX} SELECT ?g (COUNT(?v) AS ?c) (MAX(?v) AS ?m) WHERE {{ ?x ex:grp ?g . \
         {{ ?x ex:q1 ?v }} UNION {{ ?x ex:q2 ?v }} }} GROUP BY ?g"
    ));
}

/// A pooled arm's `Aggregation` branch: `Some((agg, subplan_arm_count))` when the
/// SQL pushdown fired (`try_sql_group_over_union`), `None` when it fell back to
/// `RustGroup`. Distinguishes the two `agg_over_union_is_lowered` closures the
/// SAME query could legitimately take, so a test can assert WHICH one fired
/// (not just that "some" closure did) — needed here because concern #3
/// (`COUNT(DISTINCT ?v)`) and the compound-key case are exactly the shapes the
/// pushdown must actually exercise for these regression guards to mean anything.
fn has_not_exists(conds: &[SqlCond]) -> bool {
    conds.iter().any(|c| matches!(c, SqlCond::NotExists { .. }))
}

fn pushdown_agg(tp: &Plan) -> Option<&sf_sparql::iq::Aggregation> {
    tp.branches
        .iter()
        .find(|b| b.agg.is_some() && !b.subplan_joins.is_empty())
        .and_then(|b| b.agg.as_ref())
}

/// ADR-0023 optimizer-residue wave, q9 agg-pushdown follow-up (concern #3 LIVE
/// proof): `COUNT(DISTINCT ?v)` over a UNION must push down to SQL `COUNT(DISTINCT
/// col)` over the pooled `UNION ALL` — ONE SQL scope dedupes across BOTH arms,
/// matching the `RustAgg.distinct` per-group manual dedup the oracle would use.
/// The union here is a SELF-union (`?s ex:p1 ?v` twice) so every value is
/// deliberately DUPLICATED: g1's `?v`s are `{a,c,a,c}` (COUNT=4) but DISTINCT
/// must dedupe to `{a,c}` (COUNT=2); g2's `{d,d}` → 1. A DISTINCT-not-applied
/// regression would silently return 4/1 (double-counted) instead of 2/1 — caught
/// by the spareval `=_bag` gate inside `agg_union`, not just a row-count check.
#[test]
fn agg_over_union_count_distinct_pushes_down() {
    let query = format!(
        "{PFX} SELECT ?g (COUNT(DISTINCT ?v) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p1 ?v }} }} GROUP BY ?g"
    );
    agg_union(&query);
    let conn = sqlite::load(AGG_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(AGG_R2RML).expect("R2RML parses");
    let tp = tree(&maps, &parse(&query), &schema).expect("tree must close agg-over-UNION");
    let agg = pushdown_agg(&tp).expect(
        "COUNT(DISTINCT ?v) over a self-union must SQL-pushdown, not fall back to RustGroup",
    );
    assert!(agg.aggs[0].distinct, "AggCol.distinct carried through");
}

/// ADR-0023 optimizer-residue wave, q9 agg-pushdown follow-up (Wave A.1, compound
/// grouping key LIVE proof): a TWO-variable `GROUP BY ?g ?s` over a UNION must
/// push down to a multi-column SQL `GROUP BY c0, c1` over the pooled `UNION ALL`
/// — `?g` (grp, outside the union) and `?s` (the subject, also outside the union)
/// together split the p1/p2 union into one group PER SUBJECT (3 subjects ⇒ 3
/// groups of 2), never collapsing rows across different `?s` values the way a
/// single-key (`?g` only) grouping would (m/1 and m/2 share `?g="g1"` but must
/// stay separate groups here).
#[test]
fn agg_over_union_compound_grouping_key_pushes_down() {
    let query = format!(
        "{PFX} SELECT ?g ?s (COUNT(?v) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g ?s"
    );
    agg_union(&query);
    let conn = sqlite::load(AGG_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(AGG_R2RML).expect("R2RML parses");
    let tp = tree(&maps, &parse(&query), &schema).expect("tree must close agg-over-UNION");
    let agg = pushdown_agg(&tp)
        .expect("2-var GROUP BY over a union must SQL-pushdown, not fall back to RustGroup");
    assert_eq!(agg.keys.len(), 2, "both ?g and ?s are SQL GROUP BY keys");
}

// --- TEMPLATE GROUP KEY: the ?g grouping var is bound via a 2-column INJECTIVE
// `rr:template "{cc}-{num}"` (separator present) instead of a bare `rr:column`,
// proving the Wave A.2 pushdown extension end-to-end (real SQL execution, not
// just IR-shape unit tests in `iq::lower::tests`). ---

const TEMPLATE_KEY_SQL: &str = r#"
CREATE TABLE m4 (
    id INTEGER PRIMARY KEY,
    cc TEXT NOT NULL, num TEXT NOT NULL, p1 TEXT NOT NULL, p2 TEXT NOT NULL
);
INSERT INTO m4 VALUES (1, 'g1', 'x', 'a', 'b');
INSERT INTO m4 VALUES (2, 'g1', 'x', 'c', 'f');
INSERT INTO m4 VALUES (3, 'g2', 'y', 'd', 'e');
"#;

const TEMPLATE_KEY_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#M4>
    rr:logicalTable [ rr:tableName "m4" ] ;
    rr:subjectMap [ rr:template "http://ex/m4/{id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:grp ;
        rr:objectMap [ rr:template "http://ex/g/{cc}-{num}" ]
    ] ;
    rr:predicateObjectMap [ rr:predicate ex:p1  ; rr:objectMap [ rr:column "p1" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p2  ; rr:objectMap [ rr:column "p2" ] ] .
"#;

const TEMPLATE_KEY_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/m4/1> ex:grp <http://ex/g/g1-x> ; ex:p1 "a" ; ex:p2 "b" .
<http://ex/m4/2> ex:grp <http://ex/g/g1-x> ; ex:p1 "c" ; ex:p2 "f" .
<http://ex/m4/3> ex:grp <http://ex/g/g2-y> ; ex:p1 "d" ; ex:p2 "e" .
"#;

/// ADR-0023 optimizer-residue wave, q9 agg-pushdown follow-up (Wave A.2, LIVE SQL
/// proof): `GROUP BY ?g` where `?g` is a 2-column injective Template must ACTUALLY
/// push down to a multi-column SQL `GROUP BY` and execute correctly — g1-x (rows
/// 1,2): `{a,b,c,f}` ⇒ COUNT 4; g2-y (row 3): `{d,e}` ⇒ COUNT 2. Gated vs the
/// independent spareval oracle (not just row-count).
#[test]
fn agg_over_union_template_group_key_pushes_down_and_executes_correctly() {
    let query = format!(
        "{PFX} SELECT ?g (COUNT(?v) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    );
    let conn = sqlite::load(TEMPLATE_KEY_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(TEMPLATE_KEY_R2RML).expect("R2RML parses");
    let q = parse(&query);
    let tp = tree(&maps, &q, &schema).expect("tree must close agg-over-UNION");
    let agg = pushdown_agg(&tp).expect(
        "a 2-column injective Template GROUP BY key must SQL-pushdown, not fall back to RustGroup",
    );
    assert_eq!(
        agg.keys[0].cols.len(),
        2,
        "both template columns are the GROUP BY key: {:?}",
        agg.keys[0].cols
    );
    assert_vs_spareval(TEMPLATE_KEY_TTL, &query, &tp, &conn);
}

/// Regression guard (optimizer-residue wave, post-M8): `?s ex:grp ?g` and each UNION
/// arm's `?s ex:p1`/`ex:p2 ?v` all resolve against the SAME table `m` (single-column PK
/// `id`), joined on the shared subject `?s` — a self-join on the primary key the cascade
/// must collapse to one scan per branch, exactly the shape the live GTFS q9 shootout
/// exposed (`routes`/`agency` self-joins surviving into emitted SQL, HANDOVER-2026-07-01).
/// Before this fix `translate_tree` skipped `cascade::run` wholesale for any `rust_group`
/// plan (to protect the aggregate-arg bindings from the projection-shrinking pass), so
/// self-join elimination never ran on a `rust_group` plan's branches either; now `project`
/// is forced to `None` instead, so self-join elimination still fires.
#[test]
fn agg_over_union_self_join_eliminated_on_shared_table() {
    let conn = sqlite::load(AGG_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(AGG_R2RML).expect("R2RML parses");
    let q = parse(&format!(
        "{PFX} SELECT ?g (COUNT(?v) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    ));
    let tp = tree(&maps, &q, &schema).expect("tree must close agg-over-UNION");
    assert!(
        agg_over_union_is_lowered(&tp),
        "must lower to a rust_group or a SQL-pushed-down Aggregation-over-SubPlan"
    );
    for b in agg_over_union_arm_branches(&tp) {
        let m_scans = b
            .core
            .iter()
            .filter(|s| matches!(&s.source, sf_core::ir::LogicalSource::Table(t) if t == "m"))
            .count();
        assert_eq!(
            m_scans, 1,
            "self-join on `m`'s PK must collapse to a single scan"
        );
    }
}

/// Test-integrity fix (leftjoin-antijoin-filter wave): `agg_over_union_self_join_eliminated_on_shared_table`
/// above is now MASKED — its query qualifies for the SQL-pushdown
/// (`try_sql_group_over_union`, both `?v` arms are plain-TEXT columns with
/// identical `TermSpec`), so `agg_over_union_arm_branches` reads the arms out
/// of `subplan_joins`, whose self-join elimination is protected by
/// `cascade_subplans`, NOT the `rust_group`-specific `ctx.project = None` fix
/// (commit `4eca009`) that test was written to guard. Reverting `4eca009` no
/// longer fails it. This fixture forces the ORIGINAL rust_group path instead:
/// `?v`'s two arms are a TEXT column (`s1`) and an INTEGER column (`n2`) — a
/// deliberate cross-arm `TermSpec` mismatch `try_sql_group_over_union` declines
/// on (COUNT doesn't care about the value's type, so this is semantically
/// sound; it only exists to force the applicability gate to bail) — while
/// keeping the IDENTICAL self-join topology (`?s ex:grp ?g` + each arm's
/// `?s ex:pN ?v`, all against the same PK-keyed table `sj`). Note: a plain
/// `rr:column` (no explicit `rr:datatype`) always gets `TermSpec::plain_literal()`
/// here regardless of the underlying SQL column type (no natural-mapping
/// auto-inference for hand-written R2RML) — so the mismatch needs an EXPLICIT
/// `rr:datatype` on one arm, not just a differently-typed SQL column.
const SJ_SQL: &str = r#"
CREATE TABLE sj (id INTEGER PRIMARY KEY, grp TEXT NOT NULL, s1 TEXT NOT NULL, n2 INTEGER NOT NULL);
INSERT INTO sj VALUES (1, 'g1', 'a', 10);
INSERT INTO sj VALUES (2, 'g1', 'c', 20);
INSERT INTO sj VALUES (3, 'g2', 'd', 30);
"#;

const SJ_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://ex/> .
<#SJ>
    rr:logicalTable [ rr:tableName "sj" ] ;
    rr:subjectMap [ rr:template "http://ex/sj/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:grp ; rr:objectMap [ rr:column "grp" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p1  ; rr:objectMap [ rr:column "s1" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p2  ; rr:objectMap [ rr:column "n2" ; rr:datatype xsd:integer ] ] .
"#;

#[test]
fn agg_over_union_self_join_eliminated_via_rust_group() {
    let conn = sqlite::load(SJ_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(SJ_R2RML).expect("R2RML parses");
    let q = parse(&format!(
        "{PFX} SELECT ?g (COUNT(?v) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    ));
    let tp = tree(&maps, &q, &schema).expect("tree must close agg-over-UNION");
    assert!(
        tp.rust_group.is_some(),
        "the cross-arm TermSpec mismatch (TEXT vs INTEGER) must decline the SQL \
         pushdown, so this exercises the rust_group path the guard is meant for"
    );
    for b in &tp.branches {
        let sj_scans = b
            .core
            .iter()
            .filter(|s| matches!(&s.source, sf_core::ir::LogicalSource::Table(t) if t == "sj"))
            .count();
        assert_eq!(
            sj_scans, 1,
            "self-join on `sj`'s PK must collapse to a single scan (rust_group path)"
        );
    }
}

/// Test-integrity fix: `d69daa6` (guard q9 agg-pushdown against post-cascade
/// column-count/position drift) had NO regression test. This constructs the
/// exact divergent shape the fix guards: TWO union arms sharing the outer
/// `?s ex:grp ?g` clause, where arm 1 (`?s ex:p1 ?v`) is a SELF-join on the
/// SAME table `sj2` (eliminable — collapses 2 scans to 1, shrinking its
/// `where_conds`-derived trailing projection columns), while arm 2
/// (`?s ex:p2 ?v`) reads a SEPARATE table `other2` joined on `sj2.id =
/// other2.sid` (a genuine 2-table join, NOT a self-join — self-join
/// elimination has nothing to collapse there, so its column count is
/// unaffected). Both arms still qualify for the SQL pushdown (`?v`'s TermSpec
/// is the same plain-TEXT literal in both `s1`/`s2`), so `cascade_subplans`
/// runs on the pooled arms and must catch the resulting cross-arm column-count
/// mismatch post-cascade and revert arm 1 to its PRE-cascade (2-scan) form —
/// still correct SQL, just forgoing the self-join-elimination optimization for
/// this one SubPlan, rather than emitting a `UNION ALL` with mismatched arms.
const CS_SQL: &str = r#"
CREATE TABLE sj2 (id INTEGER PRIMARY KEY, grp TEXT NOT NULL, s1 TEXT NOT NULL);
CREATE TABLE other2 (sid INTEGER NOT NULL, s2 TEXT NOT NULL);
INSERT INTO sj2 VALUES (1, 'g1', 'a');
INSERT INTO sj2 VALUES (2, 'g1', 'c');
INSERT INTO sj2 VALUES (3, 'g2', 'd');
INSERT INTO other2 VALUES (1, 'x');
INSERT INTO other2 VALUES (2, 'y');
INSERT INTO other2 VALUES (3, 'z');
"#;

const CS_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#SJ2>
    rr:logicalTable [ rr:tableName "sj2" ] ;
    rr:subjectMap [ rr:template "http://ex/sj2/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:grp ; rr:objectMap [ rr:column "grp" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p1  ; rr:objectMap [ rr:column "s1" ] ] .
<#Other2>
    rr:logicalTable [ rr:tableName "other2" ] ;
    rr:subjectMap [ rr:template "http://ex/sj2/{sid}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p2 ; rr:objectMap [ rr:column "s2" ] ] .
"#;

const CS_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/sj2/1> ex:grp "g1" ; ex:p1 "a" ; ex:p2 "x" .
<http://ex/sj2/2> ex:grp "g1" ; ex:p1 "c" ; ex:p2 "y" .
<http://ex/sj2/3> ex:grp "g2" ; ex:p1 "d" ; ex:p2 "z" .
"#;

#[test]
fn agg_over_union_pushdown_survives_asymmetric_self_join_elim() {
    let conn = sqlite::load(CS_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(CS_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?g (COUNT(?v) AS ?c) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
    );
    let q = parse(&query);
    let tp = tree(&maps, &q, &schema).expect("tree must close agg-over-UNION");

    // Must take the SQL-pushdown path (subplan_joins), not rust_group -- that's
    // the path `cascade_subplans`'s guard protects.
    let pushdown_branch = tp
        .branches
        .iter()
        .find(|b| b.agg.is_some() && !b.subplan_joins.is_empty())
        .expect("must lower to the SQL-pushdown Aggregation-over-SubPlan shape");
    let arms = &pushdown_branch.subplan_joins[0].plan.branches;
    assert_eq!(arms.len(), 2, "one pooled arm per UNION branch");
    let lens: Vec<usize> = arms.iter().map(|b| b.projection().len()).collect();
    assert_eq!(
        lens[0], lens[1],
        "cascade_subplans must keep both pooled arms' column counts equal \
         (reverting the self-joinable arm's elimination if needed), not emit a \
         column-count-mismatched UNION ALL: got {lens:?}"
    );

    // Correctness (not just shape): row-bag vs the independent spareval oracle,
    // regardless of whether self-join-elimination fired or was safely declined.
    let conn2 = sqlite::load(CS_SQL).expect("fixture reloads");
    assert_vs_spareval(CS_TTL, &query, &tp, &conn2);
}

/// ADR-0023 optimizer-residue wave (the Group-D-adjacent SQL-shape cosmetic fix):
/// `?p ex:name ?name OPTIONAL { ?p ex:dept ?d . ?d ex:label ?label }` — the
/// OPTIONAL right is a 2-scan `refObjectMap` join (`person` re-scanned to reach
/// `dept_id`, joined to `dept`) — Group C's decomposition re-derives the right
/// side from scratch inside its `NOT EXISTS` anti-join branch, which (before this
/// fix) left a REDUNDANT extra `dept` scan there (self-joined on `dept.id`, not
/// collapsed the way the matched-arm `InnerJoin` branch's own self-join
/// elimination already collapses the redundant `person` scan). After this fix the
/// `NOT EXISTS` subquery scans `dept` exactly once too — proven both `=_bag`
/// (via `diff_p`) and structurally (scan count).
#[test]
fn join_transfer_not_exists_self_join_eliminated() {
    let query = format!(
        "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label }} }}"
    );
    diff_p(&query);

    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let tp = tree(&maps, &parse(&query), &schema)
        .expect("tree must lower OPTIONAL over multi-atom right");
    let no_match = tp
        .branches
        .iter()
        .find(|b| has_not_exists(&b.where_conds))
        .expect("a NOT EXISTS no-match branch is present (Group C decomposition)");
    fn dept_scans_in_not_exists(conds: &[SqlCond]) -> usize {
        conds
            .iter()
            .map(|c| {
                match c {
                SqlCond::NotExists { scans, .. } | SqlCond::Exists { scans, .. } => scans
                    .iter()
                    .filter(|s| {
                        matches!(&s.source, sf_core::ir::LogicalSource::Table(t) if t == "dept")
                    })
                    .count(),
                SqlCond::Not(c) => dept_scans_in_not_exists(std::slice::from_ref(c)),
                SqlCond::And(cs) | SqlCond::Or(cs) => dept_scans_in_not_exists(cs),
                _ => 0,
            }
            })
            .sum()
    }
    assert_eq!(
        dept_scans_in_not_exists(&no_match.where_conds),
        1,
        "the NOT EXISTS anti-join subquery must scan `dept` exactly once (no \
         redundant self-join): {:#?}",
        no_match.where_conds
    );
}

// ============================================================================
// ADVERSARIAL REVIEW (commit 14b53ab: self-join elimination inside NOT
// EXISTS/EXISTS subqueries, plus the `find_self_join_in` `?` -> `continue`
// generalization). Four targeted counter-example attempts; see each test's doc.
// ============================================================================

/// Count scans of `table` within `conds`, recursing into NOT EXISTS/EXISTS/Not/
/// And/Or wrappers (generalizes `dept_scans_in_not_exists` above to any table
/// name, reused across the adversarial probes below).
fn table_scans_in(conds: &[SqlCond], table: &str) -> usize {
    conds
        .iter()
        .map(|c| match c {
            SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
                scans
                    .iter()
                    .filter(|s| {
                        matches!(&s.source, sf_core::ir::LogicalSource::Table(t) if t == table)
                    })
                    .count()
                    + table_scans_in(conds, table)
            }
            SqlCond::Not(c) => table_scans_in(std::slice::from_ref(c), table),
            SqlCond::And(cs) | SqlCond::Or(cs) => table_scans_in(cs, table),
            _ => 0,
        })
        .sum()
}

/// Collect every scan alias reachable inside `conds`' NOT EXISTS/EXISTS
/// subqueries (recursing into nested ones too) — used to prove no alias number
/// is reused across independent (sibling or nested) subquery scopes.
fn collect_subquery_aliases(conds: &[SqlCond], out: &mut Vec<usize>) {
    for c in conds {
        match c {
            SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
                out.extend(scans.iter().map(|s| s.alias));
                collect_subquery_aliases(conds, out);
            }
            SqlCond::Not(c) => collect_subquery_aliases(std::slice::from_ref(c), out),
            SqlCond::And(cs) | SqlCond::Or(cs) => collect_subquery_aliases(cs, out),
            _ => {}
        }
    }
}

const MSJ_SQL: &str = r#"
CREATE TABLE emp (id INTEGER PRIMARY KEY, name TEXT NOT NULL, dept TEXT NOT NULL);
INSERT INTO emp VALUES (1, 'A', 'd10');
INSERT INTO emp VALUES (2, 'A', 'd10');
INSERT INTO emp VALUES (3, 'B', 'd20');
INSERT INTO emp VALUES (4, 'C', 'd10');
"#;

const MSJ_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Emp>
    rr:logicalTable [ rr:tableName "emp" ] ;
    rr:subjectMap [ rr:template "http://ex/emp/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:dept ; rr:objectMap [ rr:template "http://ex/dept/{dept}" ] ] .
"#;

const MSJ_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/emp/1> ex:name "A" ; ex:dept <http://ex/dept/d10> .
<http://ex/emp/2> ex:name "A" ; ex:dept <http://ex/dept/d10> .
<http://ex/emp/3> ex:name "B" ; ex:dept <http://ex/dept/d20> .
<http://ex/emp/4> ex:name "C" ; ex:dept <http://ex/dept/d10> .
"#;

/// Adversarial angle 1: `?p ex:dept ?d . ?q ex:dept ?d . ?q ex:name "A"` inside a
/// MINUS anti-join is a same-table (`emp`) self-join on the NON-unique `dept`
/// column — the merge precondition (`t.is_unique_key`) must reject it. `emp/4`
/// ("C", dept d10) is the trap: it shares `dept` with the two `"A"`-named rows
/// but its OWN name is NOT "A", so a WRONG merge (collapsing the `?q` scan into
/// the `?p` scan and rewriting `?q ex:name "A"` onto the `?p` alias) would
/// corrupt the anti-join into checking `?p`'s OWN name = "A" instead of some
/// OTHER row's — `emp/4` would then wrongly SURVIVE the MINUS (its own name is
/// "C" != "A", so the corrupted check would fail and the MINUS would incorrectly
/// pass it through). Correct semantics: `emp/4` shares dept d10 with `emp/1`/
/// `emp/2` (both named "A") ⇒ the MINUS body IS satisfiable ⇒ `emp/4` is
/// CORRECTLY excluded. `assert_vs_spareval` (inside `diff`) is the decisive
/// check here — it is untainted by cascade (a tree-vs-flat diff alone would NOT
/// catch this, since both translators route through the SAME `cascade::run`).
#[test]
fn adversarial_minus_non_unique_self_join_not_merged() {
    let query = format!(
        "{PFX} SELECT ?n WHERE {{ ?p ex:name ?n \
         MINUS {{ ?p ex:dept ?d . ?q ex:dept ?d . ?q ex:name \"A\" }} }}"
    );
    diff(MSJ_SQL, MSJ_R2RML, Some(MSJ_TTL), &query);
}

/// Adversarial angle 2a: `FILTER EXISTS { ?p ex:dept ?d . ?d ex:label ?label }`
/// mirrors the OPTIONAL right side `join_transfer_not_exists_self_join_eliminated`
/// exercises (a `refObjectMap` join re-scans `dept` against itself, redundantly),
/// but here it's a literal user-written `FILTER EXISTS` (the `Exists` variant),
/// not a `NotExists` the Group C decomposition manufactured. Every person's dept
/// (10) has a label ("Sales"), so EXISTS is true for all 3 — proves the merge
/// fires (structural: exactly one `dept` scan survives) without corrupting the
/// semi-join into always-false.
#[test]
fn adversarial_filter_exists_self_join_eliminated_stays_true() {
    let query = format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name \
         FILTER EXISTS {{ ?p ex:dept ?d . ?d ex:label ?label }} }}"
    );
    diff_p(&query);

    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let tp = tree(&maps, &parse(&query), &schema).expect("tree must lower FILTER EXISTS");
    let has_exists = tp.branches.iter().any(|b| {
        b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::Exists { .. }))
    });
    assert!(has_exists, "FILTER EXISTS must lower to a SqlCond::Exists");
    let dept_scans: usize = tp
        .branches
        .iter()
        .map(|b| table_scans_in(&b.where_conds, "dept"))
        .sum();
    assert_eq!(
        dept_scans, 1,
        "the FILTER EXISTS semi-join subquery must scan `dept` exactly once (no \
         redundant self-join): {:#?}",
        tp.branches
    );
}

/// Adversarial angle 2b: the same redundant-self-join shape, but the EXISTS
/// condition is UNSATISFIABLE (`?d ex:label "Nonexistent"` — no dept has that
/// label) — every person must be EXCLUDED. A merge bug that corrupts the
/// correlation (e.g. rewrites the wrong alias when collapsing the redundant
/// `dept` scan) could flip this to always-true instead.
#[test]
fn adversarial_filter_exists_self_join_eliminated_stays_false() {
    let query = format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name \
         FILTER EXISTS {{ ?p ex:dept ?d . ?d ex:label \"Nonexistent\" }} }}"
    );
    diff_p(&query);
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let tp = tree(&maps, &parse(&query), &schema).expect("tree must lower FILTER EXISTS");
    let rows = oracle::engine_bag(&exec::select(&tp, &conn).expect("select exec"));
    assert!(
        rows.is_empty(),
        "no dept has label 'Nonexistent' -- EXISTS must be false for every person: {rows:#?}"
    );
}

/// Adversarial angle 3: a MINUS anti-join whose OWN body contains a FURTHER
/// nested `FILTER EXISTS`, each with its OWN independent redundant same-table
/// (`dept`) self-join. `lower_iq_exists` re-lowers EVERY EXISTS/NOT EXISTS body
/// (including a nested one) via a FRESH lowering-time alias counter (`&mut 0`)
/// that is independent of the outer plan's counter AND of any ancestor EXISTS
/// body's own counter — this probes whether that construction can ever produce
/// a NUMBER coincidence `find_self_join_in` could mistake for an in-scope scan.
/// It must not: every `Scan.alias` is assigned by `resolve()`'s single GLOBAL
/// counter, which explicitly descends into `IqCond::Exists`/`NotExists`
/// subtrees BEFORE `lower()` (and its per-EXISTS-call fresh counter) ever runs —
/// the lowering-time counter only allocates NEW derived aliases (subplan/agg),
/// never re-numbers a `Scan` already embedded in the resolved tree. Verified
/// both structurally (each level collapses independently) and by an explicit
/// no-duplicate-alias scan across every NOT EXISTS/EXISTS scope in the plan.
#[test]
fn adversarial_nested_exists_in_minus_no_cross_scope_alias_collision() {
    let query = format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name \
         MINUS {{ ?p ex:dept ?d . ?d ex:label ?label \
                  FILTER EXISTS {{ ?p ex:dept ?d2 . ?d2 ex:label ?label2 }} }} }}"
    );
    diff_p_bag(&query);

    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let tp = tree(&maps, &parse(&query), &schema)
        .expect("tree must lower a nested FILTER EXISTS inside MINUS");

    let dept_scans: usize = tp
        .branches
        .iter()
        .map(|b| table_scans_in(&b.where_conds, "dept"))
        .sum();
    assert_eq!(
        dept_scans, 2,
        "both the outer MINUS's own redundant dept self-join AND the nested \
         FILTER EXISTS's own redundant dept self-join must independently \
         collapse to 1 scan each (2 total): {:#?}",
        tp.branches
    );

    let mut aliases = Vec::new();
    for b in &tp.branches {
        collect_subquery_aliases(&b.where_conds, &mut aliases);
    }
    let mut sorted = aliases.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        aliases.len(),
        "every scan alias inside the nested MINUS/EXISTS subqueries must be \
         unique -- a duplicate would mean two DIFFERENT scans (possibly at \
         different nesting levels) share a number: {aliases:?}"
    );
}

/// Adversarial angle 4: the `find_self_join_in` `?` -> `continue` behavior
/// change, exercised at the OUTER BRANCH level (`b.core`/`b.where_conds`) — NOT
/// inside a subquery. `?p ex:name ?name OPTIONAL { ?p ex:dept ?d }` binds `?d`
/// to the OptJoin's OWN scan alias (living in `b.opts`, NOT `b.core`); a LATER,
/// unrelated mandatory pattern that re-joins on `?d` (`?p2 ex:dept ?d`) unifies
/// against that OPT-owned alias, pushing a ColEq that references an
/// out-of-`b.core` alias into `b.where_conds` — BEFORE a later, genuinely
/// eliminable PK self-join (`?p2 ex:name ?name2` re-scans `emp` for `?p2`,
/// self-joined against the dept-triple's own `?p2` scan on the PK `id`).
///
/// Under the OLD `scan_table(b, alias)?` code this out-of-scope ColEq would
/// have ABORTED the entire search (the `?` short-circuits the whole function,
/// not just that loop iteration) — the LATER, legitimate self-join would NEVER
/// have been found. This is a REACHABLE natural-SPARQL shape (not just a
/// hypothetical Branch construction), so it demonstrates the commit's "a no-op
/// for the existing branch-level call site" claim is not literally true: the
/// NEW code finds and performs an elimination the OLD code provably would have
/// missed. The merge itself is sound (`id` is genuinely `emp`'s PK) — this is a
/// missed-optimization/claim-accuracy finding, not a correctness regression.
// `dept` is schema-NULLABLE (so the cascade's OPTIONAL-always-matches downgrade
// cannot fire — it needs a schema-proven NOT NULL to collapse `b.opts`, per the
// `adversarial_branch_level_*` doc below) but every ACTUAL row has a non-NULL
// value, so the (separate, pre-existing, NOT part of this commit) "later
// mandatory join against a variable that may be genuinely UNBOUND from an
// OPTIONAL" gap can never fire either — isolating the ONE thing this probe
// targets from two unrelated confounds (confirmed empirically: see commit
// message / handover notes for both confounds' independent repros). `dept` lives
// on a SEPARATE table (`emp_dept`, correlated to `emp` only by sharing the SAME
// subject URI scheme/`id`, a common R2RML multi-table-per-entity pattern) so the
// UNRELATED `self_left_join_elimination` cascade pass (which collapses an
// OptJoin whenever it is a SAME-TABLE unique-key self-match — confirmed
// empirically: a single-table `emp.dept` version of this fixture gets its
// OptJoin dissolved by THAT pass before this commit's code ever sees it) cannot
// fire either — its precondition explicitly requires the opt table to match an
// existing CORE scan's table, which `emp` vs `emp_dept` never does.
const A4_SQL: &str = r#"
CREATE TABLE emp (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE emp_dept (id INTEGER PRIMARY KEY, dept TEXT NOT NULL, code TEXT NOT NULL);
INSERT INTO emp VALUES (1, 'Ann');
INSERT INTO emp VALUES (2, 'Bob');
INSERT INTO emp_dept VALUES (1, 'd10', 'C1');
INSERT INTO emp_dept VALUES (2, 'd10', 'C2');
"#;

const A4_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Emp>
    rr:logicalTable [ rr:tableName "emp" ] ;
    rr:subjectMap [ rr:template "http://ex/emp/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
<#EmpDept>
    rr:logicalTable [ rr:tableName "emp_dept" ] ;
    rr:subjectMap [ rr:template "http://ex/emp/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:dept ; rr:objectMap [ rr:template "http://ex/dept/{dept}" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:code ; rr:objectMap [ rr:column "code" ] ] .
"#;

const A4_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/emp/1> ex:name "Ann" ; ex:dept <http://ex/dept/d10> ; ex:code "C1" .
<http://ex/emp/2> ex:name "Bob" ; ex:dept <http://ex/dept/d10> ; ex:code "C2" .
"#;

/// Adversarial angle 4: the `find_self_join_in` `?` -> `continue` behavior
/// change, exercised at the OUTER BRANCH level (`b.core`/`b.where_conds`) — NOT
/// inside a subquery (no MINUS/EXISTS anywhere in this query, so
/// `self_join_elimination_in_subqueries`, this commit's OTHER change, is never
/// even called here — this isolates the `find_self_join_in` generalization's
/// effect on the PRE-EXISTING branch-level call site specifically).
///
/// `?p ex:name ?name OPTIONAL { ?p ex:dept ?d }` binds `?d` to the OptJoin's OWN
/// scan alias (living in `b.opts`, NOT `b.core`). A LATER, unrelated mandatory
/// pattern that re-joins on `?d` (`?p2 ex:dept ?d`) unifies against that
/// OPT-owned alias, pushing a ColEq that references an out-of-`b.core` alias
/// into `b.where_conds` — BEFORE a later, genuinely eliminable PK self-join
/// (`?p2 ex:code ?c` re-scans `emp_dept` for `?p2`, self-joined against the
/// dept-triple's own `?p2` scan on the PK `id`).
///
/// Under the OLD `scan_table(b, alias)?` code this out-of-scope ColEq would
/// have ABORTED the entire search (the `?` short-circuits the whole function,
/// not just that loop iteration) — the LATER, legitimate self-join would NEVER
/// have been found, leaving 2 separate `emp_dept` core scans for `?p2` instead
/// of 1. This is a REACHABLE natural-SPARQL shape (not just a hypothetical
/// Branch construction), so it demonstrates the commit's "a no-op for the
/// existing branch-level call site" claim is not literally true: the NEW code
/// finds and performs an elimination the OLD code provably would have missed.
/// The merge itself is sound (`id` is genuinely `emp_dept`'s PK, confirmed by
/// the `=_bag` vs spareval check below) — this is a missed-optimization/
/// claim-accuracy finding, not a correctness regression.
#[test]
fn adversarial_branch_level_continue_finds_later_self_join_past_opt_alias() {
    let query = format!(
        "{PFX} SELECT ?name ?c WHERE {{ \
         ?p ex:name ?name OPTIONAL {{ ?p ex:dept ?d }} . \
         ?p2 ex:dept ?d . ?p2 ex:code ?c }}"
    );
    diff(A4_SQL, A4_R2RML, Some(A4_TTL), &query);

    let conn = sqlite::load(A4_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(A4_R2RML).expect("R2RML parses");
    let tp =
        tree(&maps, &parse(&query), &schema).expect("tree must lower OPTIONAL + re-join on ?d");
    for b in &tp.branches {
        assert!(
            !b.opts.is_empty(),
            "the OPTIONAL must remain a genuine b.opts OptJoin (not collapsed by \
             the unrelated self_left_join_elimination/lj_to_ij_fk_downgrade \
             passes) for this claim to be exercised: {b:#?}"
        );
        let empdept_core_scans = b
            .core
            .iter()
            .filter(
                |s| matches!(&s.source, sf_core::ir::LogicalSource::Table(t) if t == "emp_dept"),
            )
            .count();
        assert_eq!(
            empdept_core_scans, 1,
            "?p2's dept-scan and code-scan (a genuine PK self-join on `id`) must \
             collapse to ONE `emp_dept` core scan, even though an EARLIER \
             where_conds entry (the ?p2/?d correlation) references the OPT's \
             out-of-core alias -- if this is 2, the later self-join was NOT \
             found (the OLD `?`-abort behavior): {b:#?}",
        );
    }
}

// ============================================================================
// GROUP-BY-TEMPLATE INJECTIVITY (ADR-0023 optimizer-residue wave, q9 agg-pushdown
// follow-up): `group_key_columns` lowers a `GROUP BY ?v` key to ?v's raw R2RML
// columns (ADR-0007 term-lifting: grouping by the raw key ≡ grouping by the
// constructed term) — sound ONLY when the term map is INJECTIVE. A non-injective
// `rr:template` (two adjacent `{col}` slots with no literal separator between
// them) can map two DISTINCT raw-column tuples to the SAME constructed term;
// grouping by the raw columns then silently SPLITS one SPARQL group into two,
// under-counting. `crate::cascade::binding_is_injective` (already the gate
// DISTINCT-removal uses for the identical soundness condition) closes this.
// ============================================================================

const NONINJ_SQL: &str = r#"
CREATE TABLE nj (id INTEGER PRIMARY KEY, a TEXT NOT NULL, b TEXT NOT NULL, tag TEXT NOT NULL);
INSERT INTO nj VALUES (1, '1', '23', 't1');
INSERT INTO nj VALUES (2, '12', '3', 't2');
"#;

/// `{a}{b}` — adjacent column slots, NO separator: row 1 ('1','23') and row 2
/// ('12','3') both expand to the SAME subject `http://ex/x/123`.
const NONINJ_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#NJ>
    rr:logicalTable [ rr:tableName "nj" ] ;
    rr:subjectMap [ rr:template "http://ex/x/{a}{b}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:tag ; rr:objectMap [ rr:column "tag" ] ] .
"#;

#[test]
fn group_by_non_injective_template_key_defers_not_miscounts() {
    // Both DB rows construct the SAME subject `http://ex/x/123` (non-injective
    // template) but emit DIFFERENT `ex:tag` objects, so the graph is genuinely
    // 2 distinct triples under 1 collapsed subject — grouping by the RAW (a,b)
    // columns (the pre-fix behaviour) would wrongly produce 2 groups of count 1
    // instead of 1 group of count 2. `diff()` asserts flat/tree agree AND defer
    // (rather than asserting a — now provably wrong — numeric answer): a 501 is
    // the established "never silently wrong" outcome this file's other deferred
    // shapes (GROUP_CONCAT, SAMPLE, …) already use.
    diff(
        NONINJ_SQL,
        NONINJ_R2RML,
        None,
        &format!("{PFX} SELECT ?s (COUNT(?t) AS ?c) WHERE {{ ?s ex:tag ?t }} GROUP BY ?s"),
    );
    let conn = sqlite::load(NONINJ_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(NONINJ_R2RML).expect("R2RML parses");
    let q = parse(&format!(
        "{PFX} SELECT ?s (COUNT(?t) AS ?c) WHERE {{ ?s ex:tag ?t }} GROUP BY ?s"
    ));
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "GROUP BY on a non-injective template key must defer (501), not silently \
         miscount by grouping on the raw non-injective columns"
    );
    assert!(
        matches!(flat(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "flat oracle must defer identically"
    );
}

/// Positive control: an INJECTIVE template key (single column slot) must still
/// group correctly — proves the injectivity gate isn't overly conservative and
/// doesn't regress the common (single-placeholder subject IRI) case that every
/// other `agg_over_union_*`/R2RML fixture in this file relies on.
#[test]
fn group_by_injective_template_key_still_groups() {
    let ttl = r#"
@prefix ex: <http://ex/> .
<http://ex/x/123> ex:tag "t1" .
<http://ex/x/23> ex:tag "t2" .
"#;
    // `{id}` alone is trivially injective (single column slot, ADR-0007 R2).
    let r2rml = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#NJ2>
    rr:logicalTable [ rr:tableName "nj2" ] ;
    rr:subjectMap [ rr:template "http://ex/x/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:tag ; rr:objectMap [ rr:column "tag" ] ] .
"#;
    let sql = r#"
CREATE TABLE nj2 (id TEXT PRIMARY KEY, tag TEXT NOT NULL);
INSERT INTO nj2 VALUES ('123', 't1');
INSERT INTO nj2 VALUES ('23', 't2');
"#;
    diff(
        sql,
        r2rml,
        Some(ttl),
        &format!("{PFX} SELECT ?s (COUNT(?t) AS ?c) WHERE {{ ?s ex:tag ?t }} GROUP BY ?s"),
    );
}

// --- OVERLAP: two triples-maps over the SAME table both emit ex:val (identical
// triples), and one map carries the SAME predicate twice. The virtual graph is NOT
// set-faithful (the duplicate triples collapse in a real RDF graph), so flat-vs-tree
// is the `=_bag` gate (R5's overlapping-map coverage gap; no spareval). ---

const OVERLAP_SQL: &str = r#"
CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL);
INSERT INTO t VALUES (1, 'p');
INSERT INTO t VALUES (2, 'q');
"#;

const OVERLAP_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#A>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rr:template "http://ex/t/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:val ; rr:objectMap [ rr:column "v" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:val ; rr:objectMap [ rr:column "v" ] ] .
<#B>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rr:template "http://ex/t/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:val ; rr:objectMap [ rr:column "v" ] ] .
"#;

#[test]
fn r5_ii_overlapping_maps_same_predicate() {
    // (ii) overlapping/redundant maps + a doubled POM on the SAME predicate over the
    // SAME rows: 3 ex:val POMs × 2 rows ⇒ a 6-row bag. flat == tree (the =_bag gate).
    diff_bag_only(
        OVERLAP_SQL,
        OVERLAP_R2RML,
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:val ?o }}"),
    );
    // The bare dump over the overlapping maps — same multiplicity through CONSTRUCT.
    diff_bag_only(
        OVERLAP_SQL,
        OVERLAP_R2RML,
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
    );
}

// ============================================================================
// PROPERTY PATHS (ADR-0023 M5 Wave 1) — the tree path now compiles a SPARQL
// property-path closure (`P+`/`P*`/`p?`, sequence `p/q`, alternative `p|q`, inverse
// `^p`, negated property set `!p`) by REUSING the flat `path_branch` VERBATIM at
// RESOLVE (`UnresolvedPath` → `IqNode::Path`), so the closure semantics are identical
// to the flat path by construction. Each case asserts flat-vs-tree row-bag parity (the
// `=_bag` gate) AND — set-faithful node-pair graphs — tree-vs-spareval (the independent
// oracle). These fixtures are the proven `differential_paths.rs` path fixtures.
//
// Before M5 Wave 1 every non-NamedNode path 501'd in the TREE while the FLAT path
// resolved it — the `diff` (b) identical-501-set assertion would FAIL on the alternative
// `p|q` case (M3d Finding 2: tree 501 vs flat Ok). With the path now resolved in the
// tree, `diff` passes (b), proving the alt-path 501-divergence is GONE.
// ============================================================================

// --- Fixture A: two predicates ex:p / ex:q, all nodes in the ex:n/ domain (one node
// shape), so sequence/alternative/inverse/NPS are all soundly composable. ---

const PA_SQL: &str = r#"
CREATE TABLE pe (ps INTEGER NOT NULL, pm INTEGER NOT NULL);
CREATE TABLE qe (qm INTEGER NOT NULL, qo INTEGER NOT NULL);
INSERT INTO pe VALUES (1, 2);
INSERT INTO pe VALUES (2, 3);
INSERT INTO qe VALUES (2, 20);
INSERT INTO qe VALUES (3, 30);
"#;

const PA_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "pe" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{ps}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:template "http://ex/n/{pm}" ] ] .
<#Q>
    rr:logicalTable [ rr:tableName "qe" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{qm}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:template "http://ex/n/{qo}" ] ] .
"#;

const PA_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:p <http://ex/n/2> .
<http://ex/n/2> ex:p <http://ex/n/3> .
<http://ex/n/2> ex:q <http://ex/n/20> .
<http://ex/n/3> ex:q <http://ex/n/30> .
"#;

// --- Single-predicate edge fixture: the reflexive (P*/p?) shapes are sound here
// because the hop's node set equals the active graph's node set. ---

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

const PE_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/3> ex:reaches <http://ex/n/4> .
<http://ex/n/1> ex:reaches <http://ex/n/5> .
"#;

// --- Cyclic edge fixture (1→2→3→1 + chord 1→3): a composite closure must terminate
// (depth bound) and return each reachable pair exactly once. ---

const PC_SQL: &str = r#"
CREATE TABLE edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO edge VALUES (1, 2);
INSERT INTO edge VALUES (2, 3);
INSERT INTO edge VALUES (3, 1);
INSERT INTO edge VALUES (1, 3);
"#;

const PC_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/3> ex:reaches <http://ex/n/1> .
<http://ex/n/1> ex:reaches <http://ex/n/3> .
"#;

#[test]
fn path_recursive_plus_star_tree_eq_flat_and_spareval() {
    // P+ (transitive closure) over the single-predicate edge fixture — recursive CTE.
    diff(
        PE_SQL,
        PE_R2RML,
        Some(PE_TTL),
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches+ ?o }}"),
    );
    // P* — P+ ∪ the reflexive (x,x) over every graph node (§9.3), sound here because the
    // graph is single-predicate/single-table (the hop node set equals the graph's).
    diff(
        PE_SQL,
        PE_R2RML,
        Some(PE_TTL),
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches* ?o }}"),
    );
    // P+ over a CYCLIC graph must terminate (depth bound) and return each pair once.
    diff(
        PC_SQL,
        PE_R2RML,
        Some(PC_TTL),
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches+ ?o }}"),
    );
}

#[test]
fn path_zero_or_one_tree_eq_flat_and_spareval() {
    // p? — the hop ∪ the reflexive (x,x) over the graph's nodes (single-predicate).
    diff(
        PE_SQL,
        PE_R2RML,
        Some(PE_TTL),
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches? ?o }}"),
    );
}

#[test]
fn path_alternative_tree_eq_flat_and_spareval() {
    // p|q ALTERNATIVE — the M3d Finding 2 case: before M5 Wave 1 the TREE 501'd while
    // the FLAT path resolved it (the alt-path 501-divergence). `diff` asserts (b) the
    // identical-501-set — so its PASS proves that divergence is GONE — AND (a) flat-vs-tree
    // row-bag parity AND (d) tree-vs-spareval (set union of the two hop relations).
    diff(
        PA_SQL,
        PA_R2RML,
        Some(PA_TTL),
        &format!("{PFX} SELECT ?x ?y WHERE {{ ?x ex:p|ex:q ?y }}"),
    );
}

#[test]
fn path_negated_property_set_tree_eq_flat_and_spareval() {
    // !p NEGATED PROPERTY SET — the complement of ex:p is {ex:q} (bag semantics per
    // matching triple, §18.2.2; faithful here as no pair is reached twice).
    diff(
        PA_SQL,
        PA_R2RML,
        Some(PA_TTL),
        &format!("{PFX} SELECT ?x ?y WHERE {{ ?x !ex:p ?y }}"),
    );
}

#[test]
fn path_composite_inverse_and_sequence_plus_tree_eq_flat_and_spareval() {
    // (^p)+ — a composite closure over an INVERSE one-hop relation (HopExpr::Inverse),
    // closed transitively over the cyclic graph (must terminate, each pair once).
    diff(
        PC_SQL,
        PE_R2RML,
        Some(PC_TTL),
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s (^ex:reaches)+ ?o }}"),
    );
    // (p/q)+ — a composite closure over a SEQUENCE one-hop relation (HopExpr::Seq): the
    // middle node of p/q is the junction the raw-key join meets on (matching node shape).
    const SEQ_SQL: &str = r#"
CREATE TABLE pe (ps INTEGER NOT NULL, pm INTEGER NOT NULL);
CREATE TABLE qe (qm INTEGER NOT NULL, qo INTEGER NOT NULL);
INSERT INTO pe VALUES (1, 1);
INSERT INTO pe VALUES (2, 2);
INSERT INTO qe VALUES (1, 2);
INSERT INTO qe VALUES (2, 1);
"#;
    const SEQ_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:p <http://ex/n/1> .
<http://ex/n/2> ex:p <http://ex/n/2> .
<http://ex/n/1> ex:q <http://ex/n/2> .
<http://ex/n/2> ex:q <http://ex/n/1> .
"#;
    diff(
        SEQ_SQL,
        PA_R2RML,
        Some(SEQ_TTL),
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s (ex:p/ex:q)+ ?o }}"),
    );
}

#[test]
fn path_blank_node_sequence_tree_eq_flat_bag() {
    // M3d Finding 1 — the BLANK-NODE-connected sequence path. spargebra lowers a bare
    // top-level sequence `p/q` to a BGP of two triples joined on a fresh ANONYMOUS BLANK
    // NODE (`?x ex:p _:b . _:b ex:q ?z`), NOT a property-path closure — so it resolves
    // through ordinary triple resolution. The FROZEN flat oracle binds a query blank node
    // as a CONSTANT term (no blank nodes in the source ⇒ empty), which a set-graph oracle
    // like spareval (blank node = non-distinguished variable) does not — so this is a
    // flat-vs-tree BAG-ONLY case (the established flat blank-node behavior, out of scope;
    // flat==tree, the `=_bag` gate this milestone proves, still holds and is asserted).
    diff(
        PA_SQL,
        PA_R2RML,
        None,
        &format!("{PFX} SELECT ?x ?z WHERE {{ ?x ex:p/ex:q ?z }}"),
    );
}

#[test]
fn path_bare_inverse_tree_eq_flat_and_spareval() {
    // A bare top-level inverse `^p` is lowered to the reversed triple `?y p ?x` (the
    // single-predicate fast-path, a real join variable) — tree == flat == spareval.
    diff(
        PA_SQL,
        PA_R2RML,
        Some(PA_TTL),
        &format!("{PFX} SELECT ?x ?y WHERE {{ ?x ^ex:p ?y }}"),
    );
}

#[test]
fn path_deferred_shapes_tree_eq_flat_501() {
    // Shapes the FLAT path itself 501s must ALSO 501 in the tree (identical-501-set): the
    // tree reuses `path_branch` verbatim, so a `path_branch` 501 propagates unchanged.
    // `diff` asserts both paths agree on Unsupported.
    // p? reflexive over a MULTI-predicate graph (ZeroLengthPath node set ≠ hop node set).
    diff(
        PA_SQL,
        PA_R2RML,
        None,
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:p? ?o }}"),
    );
    // A nested closure operator inside a composite hop relation.
    diff(
        PA_SQL,
        PA_R2RML,
        None,
        &format!("{PFX} SELECT ?s ?o WHERE {{ ?s (ex:p+)/ex:q ?o }}"),
    );
    // A bound endpoint — outside `?s PATH ?o` (v1 path surface).
    diff(
        PE_SQL,
        PE_R2RML,
        None,
        &format!("{PFX} SELECT ?o WHERE {{ <http://ex/n/1> ex:reaches+ ?o }}"),
    );
}

// ============================================================================
// W3C RDB2RDF corpus — the ?s ?p ?o dump CONSTRUCT through BOTH paths over every
// loadable vendored case (R2RML + Direct Mapping), asserting flat-vs-tree row-bag
// parity and identical 501 outcomes (the breadth half of the differential).
// ============================================================================

const DUMP: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
const W3C_BASE: &str = "http://example.com/base/";

fn w3c_cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf/cases")
}

/// Run the DUMP through both paths over `maps`/`conn`; assert flat-vs-tree parity and
/// identical 501 outcome. Returns `Some(true)` when compared, `Some(false)` when both
/// deferred, `None` when a runtime (exec) 501/skip made the case non-comparable.
fn w3c_compare(
    maps: &[sf_core::ir::TriplesMap],
    conn: &Connection,
    schema: &[TableSchema],
    id: &str,
) -> Option<bool> {
    let q = parse(DUMP);
    let f = translate_with_flat(&q, maps, Dialect::Sqlite, &Tbox::default(), schema);
    let t = translate_tree(&q, maps, &Tbox::default(), Dialect::Sqlite, schema);
    match (&f, &t) {
        // Both paths error identically (a deferred/negative case) — parity holds; we
        // require the Unsupported SET to match, and tolerate identical non-501 errors
        // (a malformed-fixture negative case both paths reject the same way).
        (Err(fe), Err(te)) => {
            assert_eq!(
                matches!(fe, Error::Unsupported(_)),
                matches!(te, Error::Unsupported(_)),
                "W3C {id}: translate 501-set mismatch:\n flat={f:?}\n tree={t:?}"
            );
            Some(false)
        }
        (Ok(fp), Ok(tp)) => {
            // construct_triples can itself defer to a runtime 501 / source error; require
            // the same Ok/Err shape, then compare the sorted N-Triples multisets.
            let ft = exec::construct_triples(fp, conn);
            let tt = exec::construct_triples(tp, conn);
            match (ft, tt) {
                (Ok(ftr), Ok(ttr)) => {
                    let mut fs: Vec<String> = ftr.iter().map(|t| t.to_string()).collect();
                    let mut ts: Vec<String> = ttr.iter().map(|t| t.to_string()).collect();
                    fs.sort();
                    ts.sort();
                    assert_eq!(
                        fs,
                        ts,
                        "W3C {id}: flat vs tree DUMP row-bag divergence ({} vs {} triples)",
                        fs.len(),
                        ts.len()
                    );
                    Some(true)
                }
                (Err(_), Err(_)) => None,
                (a, b) => panic!("W3C {id}: exec Ok/Err mismatch:\n flat={a:?}\n tree={b:?}"),
            }
        }
        _ => panic!("W3C {id}: translate Ok/Err mismatch:\n flat={f:?}\n tree={t:?}"),
    }
}

#[test]
fn w3c_rdb2rdf_dump_flat_eq_tree() {
    let root = w3c_cases_dir();
    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .expect("cases dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let mut compared = 0usize;
    let mut deferred = 0usize;
    let mut noncomparable = 0usize;

    for dir in dirs {
        let Ok(manifest_text) = std::fs::read_to_string(dir.join("manifest.ttl")) else {
            continue;
        };
        let Ok(cases) = sf_conformance::manifest::parse(&manifest_text) else {
            continue;
        };
        // Each scenario directory shares ONE create.sql fixture.
        let Ok(create) = std::fs::read_to_string(dir.join("create.sql")) else {
            continue;
        };
        let Ok(conn) = sqlite::load(&create) else {
            continue;
        };
        let Ok(schemas) = sqlite::introspect_all(&conn) else {
            continue;
        };

        for case in &cases {
            let maps = match case.kind {
                sf_conformance::Kind::R2rml => {
                    let Some(doc) = &case.mapping_document else {
                        continue;
                    };
                    let Ok(ttl) = std::fs::read_to_string(dir.join(doc)) else {
                        continue;
                    };
                    match sf_mapping::parse_r2rml(&ttl) {
                        Ok(m) => m,
                        Err(_) => continue, // a parse-error (negative) case — not a translate diff
                    }
                }
                sf_conformance::Kind::DirectMapping => {
                    match sf_mapping::direct_mapping(&schemas, W3C_BASE) {
                        Ok(m) => m,
                        Err(_) => continue,
                    }
                }
            };
            match w3c_compare(&maps, &conn, &schemas, &case.identifier) {
                Some(true) => compared += 1,
                Some(false) => deferred += 1,
                None => noncomparable += 1,
            }
        }
    }

    eprintln!(
        "W3C dump differential: compared={compared} both-deferred={deferred} non-comparable={noncomparable}"
    );
    assert!(
        compared > 0,
        "the W3C dump differential must compare at least one case"
    );
}

// ============================================================================
// SUBPLAN (ADR-0023 M5 Wave 2) — a modifier-bearing subquery appearing as a JOIN
// operand (not the spine) is lowered via a SubPlan derived table in the TREE path.
// The FLAT path handles some of these shapes by translating the subquery algebra
// inline (transparent to the JOIN), but produces results that differ from the tree
// on queries where shared variables cross the subquery boundary (flat is imprecise
// here). The rigorous gate is: the TREE result =_bag the independent spareval oracle.
// ============================================================================

/// A SubPlan-as-join tree-only spec (ADR-0023 M5 Wave 2): assert the TREE produces
/// a subplan_join branch AND the tree result =_bag the independent spareval oracle.
/// Does NOT compare with the flat oracle (flat may produce incomplete results for
/// the subquery-join algebra shape).
fn subplan_tree_eq_spareval(sql: &str, r2rml: &str, ttl: &str, query: &str) {
    let conn = sqlite::load(sql).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);
    let tp = tree(&maps, &q, &schema).expect("tree must succeed for subquery-as-join");
    assert!(
        tp.branches.iter().any(|b| !b.subplan_joins.is_empty()),
        "tree must produce a subplan_join branch (SubPlan derived-table, §5.4): {tp:?}"
    );
    assert_vs_spareval(ttl, query, &tp, &conn);
}

#[test]
fn subplan_aggregate_as_join_operand() {
    // Test 1 (ADR-0023 M5 Wave 2, §5.4): a SPARQL aggregate subquery appearing as a
    // JOIN operand — `{ SELECT ?s (COUNT(?n) AS ?c) WHERE { ?s ex:name ?n } GROUP BY ?s }
    // ?s ex:name ?m`. The tree lowers the inner SELECT via a SubPlan derived table
    // (`(SELECT …) AS t{alias}`) joined with the outer `?s ex:name ?m` pattern.
    //
    // Expected over STRESS fixture (emp has 1 name per id):
    //   (emp/1, c=1, m="A"), (emp/2, c=1, m="A"), (emp/3, c=1, m="B") — 3 rows.
    //
    // flat = Ok (the flat path handles the subquery inline but the tree uses a
    // derived-table SubPlan; the independent spareval oracle gates correctness).
    subplan_tree_eq_spareval(
        STRESS_SQL,
        STRESS_R2RML,
        STRESS_TTL,
        &format!(
            "{PFX} SELECT ?s ?c ?m WHERE {{ \
             {{ SELECT ?s (COUNT(?n) AS ?c) WHERE {{ ?s ex:name ?n }} GROUP BY ?s }} \
             ?s ex:name ?m }}"
        ),
    );
}

#[test]
fn subplan_distinct_as_join_operand() {
    // Test 2 (ADR-0023 M5 Wave 2, §5.4): a DISTINCT subquery appearing as a JOIN
    // operand — `{ SELECT DISTINCT ?n WHERE { ?s ex:name ?n } } ?s ex:name ?n`.
    // The tree lowers the inner SELECT DISTINCT as a SubPlan derived table.
    //
    // Expected over STRESS fixture:
    //   DISTINCT names: {"A", "B"};
    //   joined with ?s ex:name ?n:
    //   (n="A", s=emp/1), (n="A", s=emp/2), (n="B", s=emp/3) — 3 rows.
    //
    // flat = Ok for this shape (distinct subquery with non-overlapping outer var).
    // tree-vs-spareval is the independent oracle gate.
    diff_stress(&format!(
        "{PFX} SELECT ?s ?n WHERE {{ \
         {{ SELECT DISTINCT ?n WHERE {{ ?s ex:name ?n }} }} \
         ?s ex:name ?n }}"
    ));
}
