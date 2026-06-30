//! ADR-0023 M3d **shadow differential** — empirically prove the operator-tree (IQ)
//! translation path ([`sf_sparql::translate_tree`]) is `=_bag` (multiset-equivalent,
//! counts significant) to the proven flat [`sf_sparql::translate`] oracle, WITHOUT
//! switching the default (M3 design `docs/design/ADR-0023-M3-resolution-pipeline.md`
//! §7). The flat `unfold.rs` path stays the default and the ORACLE.
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
use sf_sparql::{exec, translate_tree, translate_with, Error, Plan, PlanForm, Tbox};
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
    translate_with(q, maps, Dialect::Sqlite, &Tbox::default(), schema)
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
        tp.rust_group.is_some(),
        "agg-over-UNION must lower to a rust_group: `{query}`"
    );
    assert_vs_spareval(AGG_TTL, query, &tp, &conn);
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
    let f = translate_with(&q, maps, Dialect::Sqlite, &Tbox::default(), schema);
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
