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

/// RED (pre-fix): `FILTER NOT EXISTS { <body sharing no variable with the outer
/// scope> }` is a PURE existence test (SPARQL §11.4.7) -- it must evaluate the
/// same regardless of whether the body happens to share a variable with the
/// outer scope. `MINUS`, in contrast, has a documented no-op exception for
/// exactly this case (SPARQL §8.3.2: disjoint variable domains mean the right
/// side can never remove a left solution). `lower_iq_exists` conflated the two
/// (both build to the same `IqCond::NotExists` node, `build.rs`) and applied
/// MINUS's no-op skip to FILTER NOT EXISTS too -- silently keeping every row
/// instead of correctly testing existence. `dept` has a `label='Sales'` row
/// (fixture), so `NOT EXISTS { ?x ex:label "Sales" }` is unconditionally FALSE
/// for every person (the pattern always matches, uncorrelated) -- the correct
/// answer is 0 rows; the MINUS analog is a documented no-op -- the correct
/// answer is all 3 rows, unfiltered. `diff_p` (not `_bag`): both sides fully
/// set-faithful here (no nullable column exposed).
#[test]
fn not_exists_with_no_shared_variable_is_not_a_minus_no_op() {
    diff_p(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name \
         FILTER NOT EXISTS {{ ?x ex:label \"Sales\" }} }}"
    ));
    // Companion guard: MINUS's own no-op exception must NOT regress while fixing
    // the above -- same shape, different (correct) SPARQL semantics.
    diff_p(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name \
         MINUS {{ ?x ex:label \"Sales\" }} }}"
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
    diff_p(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name }} ORDER BY ?name OFFSET 1"
    ));
    // A bare OFFSET (no LIMIT, no ORDER BY, single-branch) is the ONLY shape that
    // triggers Plan::prepared_branches' SQL push-down (single unordered branch),
    // which is what actually exercises emit.rs's LIMIT/OFFSET rendering -- the
    // fixed SQLite/MySQL "OFFSET with no LIMIT" syntax-error bug. `diff_p_bag`
    // (flat vs tree only), not `diff_p`: with no ORDER BY, WHICH row a bare
    // OFFSET skips is legitimately implementation-defined (spareval's own
    // iteration order may pick a different-but-equally-valid row than this
    // engine's as-written-order convention -- the SAME established exception
    // this file already documents for LIMIT).
    diff_p_bag(&format!(
        "{PFX} SELECT ?name WHERE {{ ?p ex:name ?name }} OFFSET 1"
    ));
}

/// Pre-existing bug (confirmed live: `sqlite error: no such column: t0.name` for
/// every dialect, not just SQLite), root-caused and fixed here: a core-less
/// `LeftJoin` LEFT side (`{}`, the empty-BGP identity — always exactly one
/// solution to extend) meant `Branch.core.is_empty()`, and the FROM-decision
/// guard in `emit_branch_with`/`emit_agg_branch` checked only `core`/
/// `subplan_joins`, never `opts` — so `from = None` was chosen even though
/// `Branch.opts` (the OPTIONAL's own scan) was non-empty, and the SELECT list
/// still projected the opt's column, referencing a table alias no FROM clause
/// ever introduced. Fixed in `render_from`'s core-less branch: when no
/// `subplan_joins` exist either, synthesize a portable `(SELECT 1)` single-row
/// anchor (correctly preserving OPTIONAL's "guaranteed at least one row, even
/// with zero matches" semantics — unlike naively promoting the opt itself to a
/// hard FROM anchor, which would wrongly drop that guaranteed row whenever it
/// has zero matches) and LEFT JOIN every opt onto it, same as the core-anchor
/// case already did.
#[test]
fn bare_group_as_leftjoin_left_no_longer_mis_aliases() {
    diff_p(&format!(
        "{PFX} SELECT ?n WHERE {{ {{}} OPTIONAL {{ ?p ex:name ?n }} }}"
    ));
    // The identical bug via a Construction-wrapped (BIND-only) core-less left
    // side, not just a bare `{}` — same root cause, same fix.
    diff_p(&format!(
        "{PFX} SELECT ?x ?n WHERE {{ BIND(1 AS ?x) OPTIONAL {{ ?p ex:name ?n }} }}"
    ));
    // The same core-less-plus-opts shape reaching `emit_agg_branch` (a SEPARATE
    // FROM-decision guard, fixed identically) via an aggregate over the OPTIONAL.
    diff_p(&format!(
        "{PFX} SELECT (COUNT(?n) AS ?c) WHERE {{ {{}} OPTIONAL {{ ?p ex:name ?n }} }}"
    ));
}

/// Correctness-backlog assessment (per team-lead handoff): "a flat-oracle
/// limitation aggregating over a BIND-only union" was flagged as a possible
/// bug during Wave C. Assessed here and found to be NEITHER a bug nor
/// something needing a fix: `COUNT` over a `UNION` whose every arm is a bare
/// `BIND` (no real pattern at all) makes FLAT's own aggregation-over-UNION
/// mechanism introduce an internal synthetic variable it cannot itself bind,
/// so flat honestly defers (`Error::Unsupported("BIND references unbound
/// ?<synthetic>")`) rather than risk a wrong answer — this is flat's own
/// documented 501 discipline working exactly as designed, not a silent
/// failure. TREE, unlike flat, computes this correctly via its `rust_group`
/// mechanism — confirmed independently against `spareval` directly (bypassing
/// flat entirely, since `diff()`'s own "both sides must 501 together"
/// requirement would otherwise treat "tree succeeds where flat honestly
/// defers" as a mismatch, even though tree's answer is genuinely correct, not
/// wrong). This is `=_bag`-safe strengthening (tree is a strict capability
/// superset of flat here), not a regression to guard against — flat's 501 is
/// asserted explicitly below specifically so a FUTURE flat-side capability
/// improvement doesn't silently invalidate this test's own premise unnoticed.
#[test]
fn count_over_bind_only_union_is_flats_inherent_limitation_not_a_bug() {
    let q = format!(
        "{PFX} SELECT (COUNT(?x) AS ?c) WHERE {{ {{ BIND(1 AS ?x) }} UNION {{ BIND(2 AS ?x) }} }}"
    );
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let parsed = parse(&q);
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "flat is expected to honestly 501 here (its own inherent limitation) \
         -- if this now succeeds, flat has gained the capability and this \
         test's premise needs revisiting, not silently relaxing"
    );
    let tp = tree(&maps, &parsed, &schema).expect("tree translates");
    assert_vs_spareval(P_TTL, &q, &tp, &conn);
}

/// Adversarial-review-caught regression (ADR-0023 optimizer-residue Wave C,
/// Distinct-over-Values dedup): `?y` is bound by VALUES but not SELECTed, so
/// `project = [x]` NARROWS the Values leaf's own `vars = [x, y]`. DISTINCT applies
/// AFTER Project (SPARQL 18.2.5) — deduping the leaf's full `(x,y)` tuples before
/// projection would keep `(1,2)`/`(1,3)` as distinct (only the exact `(1,2)`
/// duplicate pair collapses), leaving 2 post-projection `x=1` rows where DISTINCT
/// must yield exactly 1. `diff_p_bag`, not spareval: the projected-away `?y` makes
/// this a `p`-fixture-style nullable/nondeterminism case outside `diff_p`'s
/// set-faithful scope (mirrors this file's own `diff_p_bag` convention).
#[test]
fn distinct_over_values_does_not_dedup_before_a_narrowing_projection() {
    diff_p_bag("SELECT DISTINCT ?x WHERE { VALUES (?x ?y) { (1 2) (1 3) (1 2) } }");
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

/// ADR-0023 optimizer-residue Wave C (Ontop `ValuesNodeOptimization::test1/
/// test2normalizationSlice`): LIMIT/OFFSET directly over a literal VALUES table (no
/// other pattern) truncates the row list at NORMALIZE instead of lowering every row
/// to its own branch and relying on `Plan.limit`/`Plan.offset` to hide the rest.
#[test]
fn values_slice_truncates_at_normalize_not_plan_level() {
    diff_p("SELECT ?x WHERE { VALUES ?x { 1 2 3 } } LIMIT 1");
    diff_p("SELECT ?x WHERE { VALUES ?x { 1 2 3 } } LIMIT 5 OFFSET 1");

    // Shape: exactly as many branches as survive the slice (1), not `rows.len()`
    // branches (3) plus a Plan-level limit/offset the executor applies afterward.
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let q = parse("SELECT ?x WHERE { VALUES ?x { 1 2 3 } } LIMIT 1");
    let tp = tree(&maps, &q, &schema).expect("tree translates");
    assert_eq!(
        tp.branches.len(),
        1,
        "the Values table itself must shrink to 1 row/branch: {:?}",
        tp.branches
    );
    assert!(
        tp.limit.is_none() && tp.offset == 0,
        "the Slice must be fully absorbed into the Values leaf, leaving nothing for \
         the Plan's own limit/offset to do: limit={:?} offset={}",
        tp.limit,
        tp.offset
    );
}

/// ADR-0023 optimizer-residue Wave C (Ontop `ValuesNodeOptimization::
/// test14ConstructionUnionTrueTrue`): a `Union` of bare-constant `BIND`-only arms
/// folds to one `Values` leaf at NORMALIZE, so no per-arm scan survives to LOWER.
#[test]
fn constant_union_folds_to_values_leaf() {
    diff_p("SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } }");
    diff_p(
        "SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } \
         UNION { BIND(\"c\" AS ?x) } }",
    );
    // A DATA arm (a real triple pattern) alongside a constant arm must NOT fold --
    // the fixture's own name data mixed with one constant arm.
    diff_p(&format!(
        "{PFX} SELECT ?x WHERE {{ {{ BIND(\"extra\" AS ?x) }} UNION {{ ?p ex:name ?x }} }}"
    ));

    // Shape: the fold must actually happen at NORMALIZE (not just lower to the same
    // core-less-branches-either-way shape a plain Union of BIND arms already would).
    // Proof that distinguishes fold-vs-no-fold: composing with the Slice-over-Values
    // rule (this same Wave C batch) -- a `Union` of 3 constant arms that does NOT
    // fold stays a `Union` (`Slice{child: Union{..}}` doesn't match the Slice rule's
    // `Values`/`Construction{Values}` patterns, so all 3 arms would lower to 3
    // branches with the LIMIT applied at the Plan level); folded into ONE `Values`
    // first, the Slice rule then truncates it in place, to exactly 1 branch.
    //
    // flat-vs-tree only (`diff_p_bag`), NOT spareval: LIMIT with no ORDER BY over a
    // multi-arm UNION is implementation-defined tie-breaking (confirmed empirically --
    // spareval picks the LAST arm here, sf's tree the FIRST, as-written order; neither
    // is wrong, they just disagree, exactly the risk `diff_p_bag` exists for).
    diff_p_bag(
        "SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } \
         UNION { BIND(\"c\" AS ?x) } } LIMIT 1",
    );
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let q = parse(
        "SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } \
         UNION { BIND(\"c\" AS ?x) } } LIMIT 1",
    );
    let tp = tree(&maps, &q, &schema).expect("tree translates");
    assert_eq!(
        tp.branches.len(),
        1,
        "the constant-fold must run before Slice-over-Values sees it, truncating to \
         1 branch, not all 3 arms plus a Plan-level LIMIT: {:?}",
        tp.branches
    );
    assert!(
        tp.limit.is_none(),
        "the Slice must be fully absorbed once the Union folds to Values: limit={:?}",
        tp.limit
    );
}

/// ADR-0023 optimizer-residue Wave C (Ontop `ValuesNodeOptimization::
/// test3normalizationDistinct`): `DISTINCT` directly over a literal `Values` table
/// dedups the row list at NORMALIZE, so `Distinct` and the duplicate branches never
/// reach LOWER at all.
#[test]
fn distinct_over_values_dedups_at_normalize_not_exec() {
    diff_p("SELECT DISTINCT ?x WHERE { VALUES ?x { 1 1 2 2 2 3 } }");
    // A non-Const cell (CONCAT of constants, via the constant-Union fold) declines
    // the in-tree dedup but must still be CORRECT end-to-end -- Distinct still runs
    // downstream, just not collapsed into the Values leaf itself.
    diff_p(
        "SELECT DISTINCT ?x WHERE { { BIND(CONCAT(\"a\",\"b\") AS ?x) } \
         UNION { BIND(CONCAT(\"a\",\"b\") AS ?x) } }",
    );

    // Shape: exactly as many branches as survive the dedup (2), not `rows.len()`
    // branches (6) relying on `Plan.distinct`/the executor to collapse them.
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let q = parse("SELECT DISTINCT ?x WHERE { VALUES ?x { 1 1 2 2 2 3 } }");
    let tp = tree(&maps, &q, &schema).expect("tree translates");
    assert_eq!(
        tp.branches.len(),
        3,
        "the Values table itself must dedup to 3 rows/branches: {:?}",
        tp.branches
    );
    assert!(
        !tp.distinct,
        "DISTINCT must be fully absorbed into the Values leaf, leaving nothing for \
         the Plan's own distinct flag to do"
    );
}

/// Ontop `ValuesNodeOptimization::test4SliceUnionValuesValues`: covered FOR FREE by
/// composing the constant-Union fold (Wave C batch 2) with Slice-over-Values (Wave C
/// batch 1) -- no new production code, verified here as its own named scenario.
#[test]
fn slice_over_union_of_bare_values_folds_and_truncates() {
    // LIMIT with no ORDER BY over a multi-arm shape: diff_p_bag (flat-vs-tree only),
    // not spareval, for the same implementation-defined-tie-break reason as this
    // file's other LIMIT-no-ORDER-BY multi-arm tests.
    diff_p_bag("SELECT ?x WHERE { { VALUES ?x { 1 2 } } UNION { VALUES ?x { 3 4 } } } LIMIT 3");

    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let q = parse("SELECT ?x WHERE { { VALUES ?x { 1 2 } } UNION { VALUES ?x { 3 4 } } } LIMIT 3");
    let tp = tree(&maps, &q, &schema).expect("tree translates");
    assert_eq!(
        tp.branches.len(),
        3,
        "both bare-VALUES arms fold to one Values leaf, then truncate to 3, not 4 \
         branches plus a Plan-level limit: {:?}",
        tp.branches
    );
    assert!(
        tp.limit.is_none(),
        "the Slice must be fully absorbed: limit={:?}",
        tp.limit
    );
}

/// Ontop `ValuesNodeOptimization::test5-7SliceUnionValuesNonValues`: a `Slice` over
/// `Union[Values, DATA-arm]` drops the data arm entirely when the `Values` arm alone
/// (after OFFSET/LIMIT) already satisfies the window, or keeps it under a reduced
/// residual `Slice` when it doesn't.
#[test]
fn slice_over_union_arm_drop_and_residual_limit() {
    // LIMIT with no ORDER BY over a multi-arm shape: diff_p_bag, not spareval, same
    // implementation-defined-tie-break reason as this file's other such tests.
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 2"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Bob\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 2 OFFSET 1"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Bob\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 5 OFFSET 1"
    ));
    // Adversarial-review-caught regression: OFFSET exceeds the Values arm's entire
    // row count (3) while the data arm still remains. A first draft hardcoded the
    // residual Slice's offset to 0, silently dropping the 1-row shortfall and
    // leaking an extra row ("Ann") the true OFFSET should have skipped -- confirmed
    // as a real flat-vs-tree divergence (flat=2 rows, buggy tree=3 rows) before the
    // fix (`offset: offset.saturating_sub(cursor)`, not a hardcoded 0).
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Bob\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 5 OFFSET 4"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Bob\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 5 OFFSET 10"
    ));

    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");

    // Shape: the data arm is unreachable (Values alone covers the window) -- fully
    // resolves to a bare Values leaf (2 rows -> 2 core-less branches, ADR-0023's
    // one-branch-per-row lowering), no branch touches the person/dept tables.
    let q_drop = parse(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 2"
    ));
    let tp_drop = tree(&maps, &q_drop, &schema).expect("tree translates");
    assert_eq!(
        tp_drop.branches.len(),
        2,
        "the data arm's scan must not survive, only the 2 Values rows: {:?}",
        tp_drop.branches
    );
    assert!(
        tp_drop.branches.iter().all(|b| b.core.is_empty()),
        "every surviving branch must be core-less (pure Values, no scan): {:?}",
        tp_drop.branches
    );

    // Shape: the data arm survives (Values alone doesn't cover the window), but
    // under a residual Slice(offset=0, ORIGINAL limit) -- not the original offset.
    let q_residual = parse(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Bob\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 5 OFFSET 1"
    ));
    let tp_residual = tree(&maps, &q_residual, &schema).expect("tree translates");
    assert_eq!(
        tp_residual.offset, 0,
        "the offset skip is already baked into the surviving Values rows"
    );
    assert_eq!(
        tp_residual.limit,
        Some(5),
        "the ORIGINAL limit, not reduced"
    );
    // OFFSET 1 drops "Ann", leaving 2 survivor rows (Bob, Zed) -> 2 core-less
    // branches, plus the data arm's own (real-scan) branch = 3 total.
    assert_eq!(
        tp_residual.branches.len(),
        3,
        "2 surviving Values branches (Bob, Zed) + the data arm's own branch: {:?}",
        tp_residual.branches
    );
    assert!(
        tp_residual.branches[..2].iter().all(|b| b.core.is_empty())
            && !tp_residual.branches[2].core.is_empty(),
        "first two branches are the core-less Values survivors, third is the real scan: {:?}",
        tp_residual.branches
    );
}

/// Ontop `ValuesNodeOptimization::test8/9DistinctUnionValuesNonValues`: `Distinct`
/// over `Union[Values, DATA-arm]` dedups the `Values` arm's own internal duplicates
/// in place, leaving the outer `Distinct` (cross-arm dedup isn't provable) and the
/// data arm untouched; an already-duplicate-free `Values` arm is a genuine no-op.
#[test]
fn distinct_over_union_dedups_values_arm_only() {
    diff_p(&format!(
        "{PFX} SELECT DISTINCT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }}"
    ));
    diff_p(&format!(
        "{PFX} SELECT DISTINCT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Ann\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }}"
    ));

    // Shape: the Values arm's own duplicate collapses (3 rows -> 2), the data arm's
    // own branch is untouched, and the outer Distinct flag survives (cross-arm
    // duplicates -- e.g. "Ann" appearing in both the Values arm and the person
    // table -- are NOT provable statically, so Distinct still has real work to do).
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let q = parse(&format!(
        "{PFX} SELECT DISTINCT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Ann\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }}"
    ));
    let tp = tree(&maps, &q, &schema).expect("tree translates");
    assert!(
        tp.distinct,
        "the outer DISTINCT must still be requested: {tp:?}"
    );
    assert_eq!(
        tp.branches.len(),
        3,
        "2 deduped Values branches (Ann, Zed) + the data arm's own branch: {:?}",
        tp.branches
    );
    assert!(
        tp.branches[..2].iter().all(|b| b.core.is_empty()) && !tp.branches[2].core.is_empty(),
        "first two branches are the core-less deduped Values survivors, third is \
         the real scan: {:?}",
        tp.branches
    );
}

/// Spot-check prompted by a genuine arm-order bug found (and fixed, commit
/// `84365ff`) in the SIBLING test15/17 partial-fold rule: unlike that rule,
/// `dedup_one_arm` (test8/9) dedups each `Union` arm IN PLACE -- it never
/// merges or repositions arms relative to each other -- so there is no
/// analogous reordering mechanism here. Verified anyway, under a bare LIMIT
/// with no ORDER BY, rather than assumed from reading the code alone.
#[test]
fn distinct_over_union_has_no_arm_order_concern_under_limit() {
    diff_p_bag(&format!(
        "{PFX} SELECT DISTINCT ?n WHERE {{ {{ VALUES ?n {{ \"Ann\" \"Ann\" \"Zed\" }} }} \
         UNION {{ ?p ex:name ?n }} }} LIMIT 2"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT DISTINCT ?n WHERE {{ {{ ?p ex:name ?n }} \
         UNION {{ VALUES ?n {{ \"Ann\" \"Ann\" \"Zed\" }} }} }} LIMIT 2"
    ));
}

/// Ontop `ValuesNodeOptimization::test26MergeableCombination`: two `VALUES` blocks
/// binding the SAME variables in DIFFERENT header order still fold into one
/// `Values` leaf, cells reordered by name (not position) -- no transposition.
#[test]
fn constant_union_folds_reordered_columns() {
    diff_p(
        "SELECT ?x ?y WHERE { { VALUES (?x ?y) { (1 2) } } UNION { VALUES (?y ?x) { (3 4) } } }",
    );

    // Shape proof that distinguishes fold-vs-no-fold. `LIMIT 2` (the TOTAL row
    // count, not less) is deliberate: with a smaller limit, the first arm ALONE
    // would already satisfy the window regardless of whether the second arm is
    // ever recognized, making the shape assertion vacuous (a mistake caught while
    // authoring this test) -- LIMIT 2 forces `try_slice_over_union` to actually
    // examine the second arm. Without the reordering fold, the column-order
    // mismatch makes `static_rows_of` (the Slice-over-Union rule's own arm
    // recognizer) treat that second arm as unknown-cardinality too, leaving a
    // residual `Slice` in the tree (`tp.limit == Some(2)`); folded first, the
    // whole thing collapses to Slice-over-Values's own bare-Values case, which
    // ALWAYS eliminates the Slice node outright (`tp.limit.is_none()`).
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let q = parse(
        "SELECT ?x ?y WHERE { { VALUES (?x ?y) { (1 2) } } \
         UNION { VALUES (?y ?x) { (3 4) } } } LIMIT 2",
    );
    let tp = tree(&maps, &q, &schema).expect("tree translates");
    assert!(
        tp.limit.is_none() && tp.offset == 0,
        "both arms must fold to one Values leaf BEFORE Slice-over-Values sees it, \
         which then always eliminates its own Slice node outright -- a residual \
         Slice surviving here means the second (reordered) arm was never \
         recognized as foldable: limit={:?} offset={}",
        tp.limit,
        tp.offset
    );
    assert_eq!(
        tp.branches.len(),
        2,
        "both rows present either way (this alone doesn't prove the fold fired -- \
         see the limit/offset assertion above): {:?}",
        tp.branches
    );
}

/// Ontop `ValuesNodeOptimization::test25NoVariableTrueNodesAndValuesNodes`: a
/// zero-var Union of bare `{}` groups folds to a "counting" Values leaf.
#[test]
fn zero_var_union_folds_to_counting_values() {
    diff_p("SELECT * WHERE { {} UNION {} UNION {} }");

    // Shape proof that distinguishes fold-vs-no-fold: a plain branch-count check
    // does NOT (each bare True arm already lowers to its own core-less branch
    // either way, so 3 branches survive with or without this rule -- the SAME
    // vacuous-test trap the test26 composition check ran into). Composing with
    // Slice DOES discriminate: `static_rows_of` (the Slice-over-Union rule's own
    // arm recognizer) does NOT recognize a BARE `True` arm (only `True` wrapped in
    // a Construction) as foldable, so without this fold firing first, the Slice
    // rule immediately hits "unknown" on arm 0 and declines entirely, leaving a
    // real `Slice` in the tree; folded first, Slice-over-Values's bare-Values case
    // eliminates its own Slice node outright.
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let q = parse("SELECT * WHERE { {} UNION {} UNION {} } LIMIT 2");
    let tp = tree(&maps, &q, &schema).expect("tree translates");
    assert!(
        tp.limit.is_none() && tp.offset == 0,
        "the zero-var Union must fold to a counting Values leaf BEFORE Slice sees \
         it, which then always eliminates its own Slice node outright: limit={:?} \
         offset={}",
        tp.limit,
        tp.offset
    );
    assert_eq!(
        tp.branches.len(),
        2,
        "LIMIT 2 truncates the 3-row counting Values to 2 (this alone doesn't \
         prove the fold fired -- see the limit/offset assertion above): {:?}",
        tp.branches
    );
}

/// Ontop `ValuesNodeOptimization::test15ConstructionUnionTrueTrueDataNode`: two
/// constant arms partially fold to one Values arm alongside a genuine data arm.
/// The rule-sensitive SHAPE proof already lives in `normalize.rs`'s own unit test
/// (`partial_fold_combines_multiple_constant_arms_keeps_data_arm`, which inspects
/// the pre-lowering `IqNode::Union` arm count directly); a bare branch-count check
/// here would be vacuous either way (a folded 2-row Values and 2 separate
/// single-row constant arms both lower to 2 core-less branches) -- this test's
/// job is confirming END-TO-END `=_bag` correctness against the independent
/// spareval oracle, covering the constant arms, the real data arm, and their
/// combination in one bag.
#[test]
fn partial_fold_combines_constant_arms_keeps_data_arm() {
    // The data arm is deliberately FIRST (`A UNION B UNION C` is left-associative
    // -- with the data arm last, the inner pair of BIND arms would fully fold via
    // the pre-existing test14 rule before this one ever ran, exercising the wrong
    // code path -- see the unit-test-level revert-proof note in normalize.rs).
    diff_p(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ ?p ex:name ?n }} UNION {{ BIND(\"X\" AS ?n) }} \
         UNION {{ BIND(\"Y\" AS ?n) }} }}"
    ));
}

/// Ontop `ValuesNodeOptimization::test17DBConstant` / `test18RDFConstant` (diff
/// datatypes -> NO fold, a documented Ontop-only restriction, free-pass negative
/// per the worklist): Ontop needs a homogeneous-cell-type gate before folding
/// constant arms into a SQL VALUES clause (a real column-type constraint at the
/// SQL level). semantic-fabric's `Values` IR node has no such constraint -- it
/// stores `Option<TermDef>` cells directly, not raw typed SQL columns -- so
/// `try_fold_constant_union` already folds constant arms of ANY types together
/// unconditionally (confirmed: an integer + a string literal, and separately an
/// IRI + a language-tagged literal, both already fold to one `Values` with no
/// gate at all). test17 (homogeneous case) is thus a strict SUBSET of what's
/// already correct and already happening -- nothing to implement.
#[test]
fn constant_fold_needs_no_type_homogeneity_gate() {
    diff_p("SELECT ?x WHERE { { BIND(1 AS ?x) } UNION { BIND(\"a\" AS ?x) } }");
    diff_p("SELECT ?x WHERE { { BIND(<http://ex/a> AS ?x) } UNION { BIND(\"a\"@en AS ?x) } }");
}

/// Ontop `ValuesNodeOptimization::test19/test21/test22/test23/test24` (the
/// RDF-term "binding-lift" family — splitting a constant/data arm's RDF term
/// into lexical value + datatype and hoisting a SHARED datatype/wrapper
/// Construction above the whole `Union`, so every arm underneath only supplies
/// its own raw lexical value): Ontop needs this hoist to collapse the SQL shape
/// (fewer `RDFTermFunctionNode`s, one shared wrapper instead of one per arm) --
/// but, exactly like test17's finding, semantic-fabric's lowering does not
/// depend on it for `=_bag` correctness. `IqNode::Union` always explodes into N
/// independent branches regardless of what (if anything) wraps it, and
/// `IqNode::Construction` folds its substitution into each resulting branch
/// separately -- neither behavior is conditioned on whether a shared wrapper
/// was hoisted, so the UNCHANGED tree (no fold attempted, no wrapper hoisted)
/// is already correct for every representative shape below: same-type
/// constants + a data arm (test19); constant IRIs + a real IRI-template arm
/// (test21); constants + a data arm gated by a nullable-column FILTER (test22);
/// a 2-variable generalization (test23); and HETEROGENEOUS types where even
/// Ontop's own Values-fold declines, confirming correctness never depended on
/// folding at all (test24).
///
/// Caveat, stated plainly: these are REPRESENTATIVE constructions matching each
/// test's OWN description in the worklist, not a line-for-line port of Ontop's
/// Java test source (not available in this environment) -- the underlying
/// architectural reason (Union/Construction lowering is wrapper-agnostic) is
/// shape-invariant, not scenario-specific, which is why one reasoning covers
/// all five; but the exact Ontop fixtures were not independently cross-checked.
#[test]
fn rdf_term_binding_lift_family_needs_no_rewrite_for_correctness() {
    // test19: same-type (xsd:string) RDF consts + a data arm.
    diff_p(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ ?p ex:name ?n }} UNION {{ BIND(\"X\" AS ?n) }} \
         UNION {{ BIND(\"Y\" AS ?n) }} }}"
    ));
    // test21: constant IRI arms + a real IRI-template (subject) arm.
    diff_p(&format!(
        "{PFX} SELECT ?s WHERE {{ {{ ?s ex:name ?n }} UNION {{ BIND(<http://ex/c1> AS ?s) }} \
         UNION {{ BIND(<http://ex/c2> AS ?s) }} }}"
    ));
    // test22: constants + a data arm involving a nullable-column FILTER (an
    // IS-NOT-NULL-ish expression via BOUND()).
    diff_p(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ ?p ex:email ?n . FILTER(BOUND(?n)) }} \
         UNION {{ BIND(\"X\" AS ?n) }} UNION {{ BIND(\"Y\" AS ?n) }} }}"
    ));
    // test23: multi-column (2-var) version of test19.
    diff_p(&format!(
        "{PFX} SELECT ?s ?n WHERE {{ {{ ?s ex:name ?n }} \
         UNION {{ BIND(<http://ex/c1> AS ?s) BIND(\"X\" AS ?n) }} \
         UNION {{ BIND(<http://ex/c2> AS ?s) BIND(\"Y\" AS ?n) }} }}"
    ));
    // test24: HETEROGENEOUS types (no Values fold even attempted) + a data arm
    // -- confirms the UNCHANGED tree is still correct with no rewrite.
    diff_p(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ ?p ex:name ?n }} UNION {{ BIND(1 AS ?n) }} \
         UNION {{ BIND(\"Y\"@en AS ?n) }} }}"
    ));
}

/// Regression test for a bug an adversarial review caught in `try_partial_fold_
/// constant_union` (test15/test17, commit `9bb21b8`): an earlier version
/// unconditionally PREPENDED the folded Values arm to position 0, silently
/// reordering rows relative to the flat oracle's own as-written-arm-order
/// convention -- a real flat-vs-tree divergence under a bare LIMIT (no ORDER
/// BY), where the two engines are supposed to agree with EACH OTHER (the
/// `=_bag` gate this whole differential suite proves) even though SPARQL
/// itself leaves the tie-break implementation-defined. Covers: data arm first
/// (the fold moves to the END, matching where its own 2 source arms sat);
/// constant arms sandwiching a data arm (non-adjacent constants must NOT
/// combine at all -- combining them would necessarily move something out of
/// its as-written position no matter where the merged arm lands, so the fix
/// only ever folds a maximal CONTIGUOUS run of constant arms, declining
/// entirely here rather than risk it).
#[test]
fn partial_fold_preserves_as_written_arm_order_under_limit() {
    // Data arm first: the fold (2 contiguous constant arms) lands LAST.
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ ?p ex:name ?n }} UNION {{ BIND(\"X\" AS ?n) }} \
         UNION {{ BIND(\"Y\" AS ?n) }} }} LIMIT 2"
    ));
    // Constant arms sandwiching a data arm: non-adjacent, must not combine.
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ BIND(\"X\" AS ?n) }} UNION {{ ?p ex:name ?n }} \
         UNION {{ BIND(\"Y\" AS ?n) }} }} LIMIT 2"
    ));
}

/// A representative shape for Ontop `ValuesNodeSimpleQueryOptimization`/
/// `ValuesNodeComplexQueryOptimization::testTranslatedSQLQuery1` (end-to-end
/// LIMIT over a mapping union): a union with NO constant arms at all (both arms
/// are real data, from different tables via different predicates), under a bare
/// LIMIT/OFFSET. Confirms this doesn't touch `try_partial_fold_constant_union`'s
/// territory (no constant arm exists to fold), so the just-fixed arm-ordering
/// bug (which was specifically about constant-vs-data ordering) cannot recur
/// here, and there is no OTHER ordering concern for a pure-data union: neither
/// fold function ever runs, the Union passes through unchanged, and
/// `Slice`-over-Union declines (an unrecognized arm shape) exactly as it always
/// has -- verified end-to-end via `diff_p_bag` rather than assumed.
#[test]
fn pure_data_union_under_limit_has_no_ordering_concern() {
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?d ex:label ?n }} }} LIMIT 2"
    ));
    diff_p_bag(&format!(
        "{PFX} SELECT ?n WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?d ex:label ?n }} }} \
         LIMIT 1 OFFSET 1"
    ));
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

/// Pre-existing bug (found incidentally during a DIFFERENT bug's own adversarial
/// review, confirmed live on both flat and tree and confirmed pre-existing on
/// the base commit before this session touched anything — it lives in
/// `leftjoin.rs`, shared infrastructure): a property-path pattern used as an
/// OPTIONAL's RIGHT operand loses its path closure. `left_join_branches`
/// dispatches a non-single-scan right side (a path branch always has
/// `core.len() == 0`, so it can NEVER take the single-scan fast path) through
/// `inner_join_one`/`not_exists_cond_for` — the `(P ⋈ R) ∪ (P − R)`
/// decomposition — and BOTH used to silently drop `right.path`: `inner_join_one`
/// only ever carried `left.path` forward (`path: left.path.clone()`), and
/// `not_exists_cond_for` built `SqlCond::NotExists { scans: right.core.clone(),
/// .. }`, empty for a path branch, yet still referenced the path's own
/// CTE-only columns in its conditions. Confirmed live: `no such column:
/// t0.sf_o` for `{} OPTIONAL { ?s ex:reaches+ ?o }`.
///
/// A simple "copy `right.path` across instead" fix is NOT sound: `Branch` can
/// only ever represent ONE of {a plain core/opts scan, a path closure} at a
/// time (`emit_branch_with` dispatches unconditionally to `emit_path_branch`
/// whenever `b.path.is_some()`, which renders ONLY the path's own CTE +
/// projection — nothing about `b.core`/`b.opts`/other conditions). Adopting
/// `right.path` would silently drop `left`'s own data instead of `right`'s —
/// trading one silent-wrong-answer shape for another. Fixed with a sound 501
/// in both `inner_join_one` and `not_exists_cond_for` (matching this file's
/// own pre-existing convention for the analogous `right.subplan_joins`
/// boundary a few lines above each) rather than attempting a real fix, which
/// would need a genuinely new composition mechanism (e.g. wrapping the path's
/// own rendering as an opaque SubPlan derived table) — out of scope for a
/// crash-to-501 fix.
#[test]
fn path_as_optional_right_operand_is_a_sound_501_not_a_crash() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ {{}} OPTIONAL {{ ?s ex:reaches+ ?o }} }}");
    let conn = sqlite::load(PE_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(PE_R2RML).expect("R2RML parses");
    let parsed = parse(&q);
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "expected an honest 501 on flat, not a crash or a silently wrong answer"
    );
    assert!(
        matches!(tree(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "expected an honest 501 on tree, not a crash or a silently wrong answer"
    );
}

/// The SAME architectural gap found via a THIRD, independent entry point: a
/// property-path pattern as the OPTIONAL's own PRECEDING (left) pattern, routed
/// through the single-scan FAST path (`build_left_join`, not the multi-branch
/// decomposition above — reachable whenever the OPTIONAL's right side is a
/// plain single scan, regardless of what the left side is). `build_left_join`
/// never touches `left.path` at all — it only ever ADDS an `OptJoin` onto
/// whatever `left` already is — so a path-shaped `left` ends up with BOTH
/// `path: Some(_)` AND a non-empty `opts`, the same unrepresentable combination
/// as above, reached from the opposite side. Confirmed live: `no such column:
/// t1.child` for `?s ex:reaches+ ?o OPTIONAL { ?o ex:reaches ?o2 }`. Fixed with
/// the same sound-501 convention.
#[test]
fn path_as_optional_left_via_single_scan_fast_path_is_a_sound_501() {
    let q = format!(
        "{PFX} SELECT ?s ?o ?o2 WHERE {{ ?s ex:reaches+ ?o OPTIONAL {{ ?o ex:reaches ?o2 }} }}"
    );
    let conn = sqlite::load(PE_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(PE_R2RML).expect("R2RML parses");
    let parsed = parse(&q);
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "expected an honest 501 on flat, not a crash or a silently wrong answer"
    );
    assert!(
        matches!(tree(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "expected an honest 501 on tree, not a crash or a silently wrong answer"
    );
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

/// Regression guard (q6/q13 tiny-aggregate perf, 2026-07-03): a SINGLE-branch
/// `GROUP BY` + aggregate over a self-join — two predicate-object maps of the SAME
/// PK-keyed table (`ex:grp`, `ex:p` both on `m`), joined on the shared subject `?s`
/// (m's PK `id`) — must collapse to ONE scan of `m`. This is the exact shape the
/// live GTFS q6/q13 shootout exposed: `SELECT ?agencyName (COUNT(?route) …) … GROUP
/// BY ?agencyName` emitted `routes`×2 CROSS JOIN `agency`×2 self-joins (≈1.7–1.8×
/// slower than Ontop at scale ≥1000). Before this fix `cascade::run` passed every
/// `agg.is_some()` branch through UNTOUCHED, so self-join elimination never ran on a
/// single-branch aggregate (the multi-branch `rust_group`/pushdown paths were fixed
/// in earlier waves; this one was missed). Now self-join elimination runs on
/// aggregate branches and `rewrite_alias` follows the merge into the `Aggregation`'s
/// key/argument `ColRef`s. **Revert-proof**: reverting either half (the `run` gate or
/// the `rewrite_alias` agg-coverage) makes the scan-count assertion fail (2, not 1).
/// `=_bag` preserved — a 1:1 PK self-join changes neither the group nor the COUNT.
const SBAGG_SQL: &str = r#"
CREATE TABLE m (id INTEGER PRIMARY KEY, grp TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO m VALUES (1,'g1','a');
INSERT INTO m VALUES (2,'g1','b');
INSERT INTO m VALUES (3,'g1','c');
INSERT INTO m VALUES (4,'g2','d');
INSERT INTO m VALUES (5,'g2','e');
"#;

const SBAGG_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#M>
    rr:logicalTable [ rr:tableName "m" ] ;
    rr:subjectMap [ rr:template "http://ex/m/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:grp ; rr:objectMap [ rr:column "grp" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p   ; rr:objectMap [ rr:column "v" ] ] .
"#;

const SBAGG_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/m/1> ex:grp "g1" ; ex:p "a" .
<http://ex/m/2> ex:grp "g1" ; ex:p "b" .
<http://ex/m/3> ex:grp "g1" ; ex:p "c" .
<http://ex/m/4> ex:grp "g2" ; ex:p "d" .
<http://ex/m/5> ex:grp "g2" ; ex:p "e" .
"#;

#[test]
fn single_branch_group_by_self_join_collapses_to_one_scan() {
    let conn = sqlite::load(SBAGG_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(SBAGG_R2RML).expect("R2RML parses");
    let query =
        format!("{PFX} SELECT ?g (COUNT(?v) AS ?c) WHERE {{ ?s ex:grp ?g ; ex:p ?v }} GROUP BY ?g");
    let q = parse(&query);
    let tp = tree(&maps, &q, &schema).expect("single-branch GROUP BY translates");
    // A plain single-branch aggregate: one branch, `agg` set, NOT a UNION pushdown
    // (no `subplan_joins`) — the path that was passing through the cascade untouched.
    let b = tp
        .branches
        .iter()
        .find(|b| b.agg.is_some() && b.subplan_joins.is_empty())
        .expect("single-branch Aggregation");
    let m_scans = b
        .core
        .iter()
        .filter(|s| matches!(&s.source, sf_core::ir::LogicalSource::Table(t) if t == "m"))
        .count();
    assert_eq!(
        m_scans, 1,
        "the PK self-join must collapse to one scan of `m` (was 2 pre-fix): {:?}",
        b.core
    );
    assert_vs_spareval(SBAGG_TTL, &query, &tp, &conn);
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
// INTEGRATION-MERGE ADVERSARIAL PROBE (integ/optimizer-gaps-on-main): the ONE
// shape neither side's own test suite exercises. `not_exists_cond_for` (main,
// the OPTIONAL-anti-join-FILTER fix) now pushes the OPTIONAL's inner FILTER into
// the SAME `NotExists { conds, .. }` that `self_join_elimination_in_subqueries`
// (gapfix, 14b53ab) rewrites when it collapses a redundant same-table self-join
// inside that NOT EXISTS. This is `join_transfer_not_exists_self_join_eliminated`
// above PLUS a match-removing FILTER, so the self-join-eliminatable shape AND the
// newly-threaded filter cond are both present in the SAME `conds` vec at once —
// the composition risk is whether collapsing the redundant `dept` scan correctly
// rewrites the filter cond's alias too (it references the DROPPED alias's
// `label` column), not just the pre-existing correlation/`IsNotNull` conds both
// sides' tests already cover individually.
// ============================================================================

const MERGE_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (id INTEGER PRIMARY KEY, name TEXT NOT NULL, dept_id INTEGER NOT NULL);
INSERT INTO dept VALUES (10, 'Sales');
INSERT INTO dept VALUES (20, 'Legal');
INSERT INTO person VALUES (1, 'Ann', 10);
INSERT INTO person VALUES (2, 'Bob', 20);
INSERT INTO person VALUES (3, 'Cid', 10);
"#;

const MERGE_R2RML: &str = r#"
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
    ] .
<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const MERGE_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/person/1> ex:name "Ann" ; ex:dept <http://ex/dept/10> .
<http://ex/person/2> ex:name "Bob" ; ex:dept <http://ex/dept/20> .
<http://ex/person/3> ex:name "Cid" ; ex:dept <http://ex/dept/10> .
<http://ex/dept/10> ex:label "Sales" .
<http://ex/dept/20> ex:label "Legal" .
"#;

/// The decisive check: `?p ex:dept ?d . ?d ex:label ?label` is the SAME
/// refObjectMap 2-scan-collapsing-to-1 shape as `join_transfer_not_exists_...`
/// above, but now the OPTIONAL carries its OWN inner `FILTER(?label != "Sales")`
/// — a MATCH-REMOVING filter for Ann/Cid (both dept 10, label "Sales", so their
/// only candidate is filtered out -> NULL-padded) but a pass-through for Bob
/// (dept 20, "Legal" != "Sales" -> right satisfied, `?label` bound). `diff()`
/// proves flat vs tree row-bag parity AND both vs the independent `spareval`
/// oracle over the hand-authored graph — a corrupted alias rewrite inside the
/// NOT EXISTS (dropping or mis-scoping the filter cond when the redundant `dept`
/// scan collapses) would either error (dangling alias, no such column) or flip
/// Ann/Cid to wrongly bound / Bob to wrongly NULL, either way a flat/tree or
/// spareval divergence `diff()` catches.
#[test]
fn merge_optional_filter_composes_with_self_join_elim_in_not_exists() {
    let query = format!(
        "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Sales\") }} }}"
    );
    diff(MERGE_SQL, MERGE_R2RML, Some(MERGE_TTL), &query);

    // Structural: the self-join-elimination-in-subqueries pass must actually have
    // FIRED here too (not silently bailed on seeing the extra filter cond) --
    // otherwise the row-bag proof above isn't exercising the composition at all.
    let conn = sqlite::load(MERGE_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(MERGE_R2RML).expect("R2RML parses");
    let tp = tree(&maps, &parse(&query), &schema)
        .expect("tree must lower OPTIONAL over multi-atom right + inner FILTER");
    let no_match = tp
        .branches
        .iter()
        .find(|b| has_not_exists(&b.where_conds))
        .expect("a NOT EXISTS no-match branch is present (Group C decomposition)");
    assert_eq!(
        table_scans_in(&no_match.where_conds, "dept"),
        1,
        "the NOT EXISTS anti-join subquery must scan `dept` exactly once (self-join \
         eliminated) even with the OPTIONAL's own FILTER also present in `conds`: {:#?}",
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
                    .filter(
                        |s| matches!(&s.source, sf_core::ir::LogicalSource::Table(t) if t == table),
                    )
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
    // `lower_iq_exists` always wraps its per-branch `Exists`(es) in a `SqlCond::Or`
    // (even for a single inner branch -- "at least one must match"), so this must
    // recurse the same way `table_scans_in` below does, not just check the
    // top-level `where_conds` shape.
    fn has_exists_in(conds: &[SqlCond]) -> bool {
        conds.iter().any(|c| match c {
            SqlCond::Exists { .. } => true,
            SqlCond::NotExists { conds, .. } => has_exists_in(conds),
            SqlCond::Not(c) => has_exists_in(std::slice::from_ref(c)),
            SqlCond::And(cs) | SqlCond::Or(cs) => has_exists_in(cs),
            _ => false,
        })
    }
    let has_exists = tp.branches.iter().any(|b| has_exists_in(&b.where_conds));
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
// EXISTS / NOT EXISTS / MINUS over an OPTIONAL-bound (possibly-UNBOUND) shared
// variable — ADR-0023 parity backlog Item 3. The FLAT oracle DEFERS this shape
// (→ 501, its documented v1 restriction: a shared variable that may be UNBOUND on
// the outer side would emit a wrong `NULL = value` correlation); the TREE now
// closes it SOUNDLY. So the rigorous gate is the TREE result `=_bag` the
// INDEPENDENT `spareval` oracle over a hand-authored set-faithful graph (tree
// EXCEEDS flat by design — the bespoke `agg_union`-style pattern, NOT `diff`,
// whose identical-501-set assertion does not apply when the tree legitimately
// exceeds flat).
//
// The SOUND rule, per SPARQL semantics: when the outer determinant is UNBOUND the
// shared variable is FREE in the inner (§18.6 substitution), so its raw-key
// correlation is guarded `(determinant IS NULL OR eq)`. The DECISIVE property this
// fixture pins: on an outer row where the shared var is UNBOUND, MINUS
// (§8.3.2 disjoint-domain no-op) and FILTER NOT EXISTS (§11.4.7 pure existence)
// must DIVERGE — NOT EXISTS removes the row (free-var existence holds), MINUS keeps
// it (domains disjoint). A single null-safe rule for all three would be WRONG;
// MINUS additionally requires a per-row `at-least-one-shared-var-bound` guard.
//
// Fixture: Ann email 'x' g1, Bob email NULL g1, Zed email 'y' g2; tokens {'x','g1'}.
// ?e (email) is OPTIONAL-bound ⇒ UNBOUND for Bob.
// ============================================================================

const I3_SQL: &str = r#"
CREATE TABLE person (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT, grp TEXT NOT NULL);
CREATE TABLE token (id INTEGER PRIMARY KEY, val TEXT NOT NULL);
INSERT INTO person VALUES (1,'Ann','x','g1');
INSERT INTO person VALUES (2,'Bob',NULL,'g1');
INSERT INTO person VALUES (3,'Zed','y','g2');
INSERT INTO token VALUES (100,'x');
INSERT INTO token VALUES (101,'g1');
"#;

const I3_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:email ; rr:objectMap [ rr:column "email" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:grp ; rr:objectMap [ rr:column "grp" ] ] .
<#Token>
    rr:logicalTable [ rr:tableName "token" ] ;
    rr:subjectMap [ rr:template "http://ex/t/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:tok ; rr:objectMap [ rr:column "val" ] ] .
"#;

// Set-faithful: Bob's NULL email ⇒ no ex:email triple (matches OPTIONAL-unbound).
const I3_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/p/1> ex:name "Ann" ; ex:email "x" ; ex:grp "g1" .
<http://ex/p/2> ex:name "Bob" ; ex:grp "g1" .
<http://ex/p/3> ex:name "Zed" ; ex:email "y" ; ex:grp "g2" .
<http://ex/t/100> ex:tok "x" .
<http://ex/t/101> ex:tok "g1" .
"#;

/// Assert the FLAT oracle DEFERS (its documented v1 limitation) AND the TREE
/// result `=_bag` the independent spareval oracle over `I3_TTL` (the SELECT
/// projects only always-bound `?n`, so it is set-faithful).
fn item3(query: &str) {
    let conn = sqlite::load(I3_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I3_R2RML).expect("R2RML parses");
    let q = parse(query);
    assert!(
        matches!(flat(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "flat oracle must DEFER EXISTS/MINUS over an OPTIONAL-bound shared var \
         (else not a tree-exceeds-flat spec): `{query}`"
    );
    let tp = tree(&maps, &q, &schema).expect("tree must close the OPTIONAL-shared-var shape");
    assert_vs_spareval(I3_TTL, query, &tp, &conn);
}

#[test]
fn item3_exists_over_optional_bound_shared_var() {
    // EXISTS: Bob's ?e is UNBOUND ⇒ FREE in the inner ⇒ EXISTS true iff ANY token
    // exists ⇒ Bob KEPT. Ann e='x' matches token 'x' ⇒ kept. Zed e='y' no token ⇒ out.
    item3(&format!(
        "{PFX} SELECT ?n WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} \
         FILTER EXISTS {{ ?t ex:tok ?e }} }}"
    ));
}

#[test]
fn item3_not_exists_over_optional_bound_shared_var() {
    // NOT EXISTS: Bob's free-var existence holds ⇒ NOT EXISTS false ⇒ Bob REMOVED
    // (the DIVERGENCE from MINUS below). Only Zed survives.
    item3(&format!(
        "{PFX} SELECT ?n WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} \
         FILTER NOT EXISTS {{ ?t ex:tok ?e }} }}"
    ));
}

#[test]
fn item3_minus_over_optional_bound_shared_var_is_disjoint_domain_no_op() {
    // MINUS: Bob's ?e UNBOUND and it is the ONLY shared var ⇒ disjoint domain ⇒
    // MINUS is a NO-OP ⇒ Bob KEPT (diverges from NOT EXISTS, which removes Bob).
    // Ann e='x' compatible with token 'x' ⇒ removed. Zed no match ⇒ kept. {Bob,Zed}.
    item3(&format!(
        "{PFX} SELECT ?n WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} \
         MINUS {{ ?t ex:tok ?e }} }}"
    ));
}

#[test]
fn item3_minus_with_additional_mandatory_shared_var() {
    // A mandatory shared var ?g (grp, always bound) ALSO shared with the inner:
    // the domains always overlap on ?g, so the disjoint-domain no-op never applies
    // and the `any_mandatory_shared` guard-suppression path is exercised. Remove iff
    // inner agrees on ?g (any ?e): Ann/Bob g1 (token 'g1' exists) removed, Zed g2 kept.
    item3(&format!(
        "{PFX} SELECT ?n WHERE {{ ?p ex:name ?n ; ex:grp ?g OPTIONAL {{ ?p ex:email ?e }} \
         MINUS {{ ?t ex:tok ?e . ?t2 ex:tok ?g }} }}"
    ));
}

#[test]
fn item3_exists_inner_match_removing_filter() {
    // Adversarial (the 3-valued blind spot): the inner body's own FILTER removes the
    // only correlated match. Ann's token 'x' matches the correlation but FAILS
    // FILTER(?e != "x") ⇒ EXISTS false ⇒ Ann out. Bob free ⇒ a token with val != (free)
    // exists ⇒ kept. Zed no match ⇒ out. {Bob}. (Companion NOT EXISTS/MINUS variants
    // exercised in the sf-sparql validation harness; here the EXISTS sense pins it.)
    item3(&format!(
        "{PFX} SELECT ?n WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} \
         FILTER EXISTS {{ ?t ex:tok ?e FILTER(?e != \"x\") }} }}"
    ));
}

#[test]
fn item3_minus_multibranch_inner_over_optional_shared_var() {
    // Adversarial: a UNION (multi-branch) inner MINUS body with the OPTIONAL-unbound
    // shared var. Bob (unbound, only shared var) ⇒ disjoint no-op ⇒ kept regardless of
    // branch count. Correlation guarding must hold per inner branch. {Bob,Zed}.
    item3(&format!(
        "{PFX} SELECT ?n WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} \
         MINUS {{ {{ ?t ex:tok ?e }} UNION {{ ?t2 ex:name ?e }} }} }}"
    ));
}

// ============================================================================
// Nested GROUP BY — an aggregate OVER an aggregate (ADR-0023 parity backlog
// Item 6). The FLAT oracle DEFERS ("nested GROUP BY (aggregate over an aggregate)
// is deferred → 501"); the TREE already CLOSES it via the §5.4 SubPlan
// derived-table lowering (`lower_as_subplan`: the inner `Aggregation` lowers to
// its own `Plan`, joined as an `INNER JOIN (SELECT …) t{alias}`, so its grouped
// outputs become plain re-aggregatable columns of the outer group). No production
// change here — this LOCKS the already-shipped capability (which the flat path
// lacks) against a future regression, gated =_bag vs the independent spareval
// oracle. Fixture OD has two depts (10:{Bob,Zed}=2, 20:{Ann}=1), so the inner
// grouped counts are non-trivial (2 and 1) and the outer aggregate is meaningful.
// ============================================================================

fn item6_od(query: &str) {
    let conn = sqlite::load(OD_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(OD_R2RML).expect("R2RML parses");
    let q = parse(query);
    assert!(
        matches!(flat(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "flat oracle must DEFER nested GROUP BY (else not a tree-exceeds-flat spec): `{query}`"
    );
    let tp = tree(&maps, &q, &schema).expect("tree must close nested GROUP BY");
    assert_vs_spareval(OD_TTL, query, &tp, &conn);
}

#[test]
fn item6_nested_group_by_aggregate_over_aggregate() {
    // inner: GROUP BY ?d, COUNT(?p) ⇒ {dept/10 ↦ 2, dept/20 ↦ 1}.
    // outer COUNT(?c): counts the two grouped rows ⇒ 2.
    item6_od(&format!(
        "{PFX} SELECT (COUNT(?c) AS ?cc) WHERE \
         {{ SELECT ?d (COUNT(?p) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d }}"
    ));
    // outer SUM(?c): 2 + 1 ⇒ 3 (a value-bearing aggregate over the inner counts,
    // not just a row count — exercises the inner agg column as a real SUM argument).
    item6_od(&format!(
        "{PFX} SELECT (SUM(?c) AS ?t) WHERE \
         {{ SELECT ?d (COUNT(?p) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d }}"
    ));
    // outer GROUP BY on a re-projected inner group key + MAX over the inner count.
    item6_od(&format!(
        "{PFX} SELECT (MAX(?c) AS ?m) WHERE \
         {{ SELECT ?d (COUNT(?p) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d }}"
    ));
}

// ============================================================================
// A single modifier sub-SELECT (Aggregation/Distinct/OrderBy) as an OPTIONAL's
// RIGHT operand — ADR-0023 parity backlog Item 1d ("nested-OPTIONAL-inside-a-
// subselect" / LeftJoinJoinLimit family). The `(P⋈R)∪(P−R)` decomposition's
// anti-join half (`not_exists_cond_for`) cannot represent a SubPlan in
// `SqlCond::NotExists`, so the tree previously 501'd. `left_join_over_subplan`
// now attaches the nested SubPlan as a derived-table LEFT JOIN
// (`SubPlanJoin { left: true, on: <correlation> }`, reusing the already-shipped
// emit LEFT JOIN path) with R1/R5/R2 semantics.
//
// The FLAT path is NOT a usable oracle for these shapes: it translates without a 501
// but emits SQL referencing a derived-table alias it never introduces in the FROM (a
// hard SQL error at exec, not a silent wrong answer — a separate, pre-existing
// flat-path limitation, out of scope here). So the gate is the TREE result `=_bag`
// the INDEPENDENT spareval oracle (the tree exceeds flat in translating AND in
// executing). `diff` is deliberately NOT used (it would exec flat and crash).
//
// Fixture: dept 30 "Empty" has NO persons, so a correlated subplan has no group
// for it ⇒ the OPTIONAL NULL-pads (the LEFT JOIN no-match path is exercised).
// ============================================================================

const I1D_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (id INTEGER PRIMARY KEY, name TEXT NOT NULL, dept_id INTEGER NOT NULL);
INSERT INTO dept VALUES (10,'Sales');
INSERT INTO dept VALUES (20,'Eng');
INSERT INTO dept VALUES (30,'Empty');
INSERT INTO person VALUES (1,'Ann',20);
INSERT INTO person VALUES (2,'Bob',10);
INSERT INTO person VALUES (3,'Zed',10);
"#;

const I1D_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:dept ;
        rr:objectMap [ rr:parentTriplesMap <#Dept> ;
            rr:joinCondition [ rr:child "dept_id" ; rr:parent "id" ] ] ] .
<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/d/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const I1D_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/d/10> ex:label "Sales" .
<http://ex/d/20> ex:label "Eng" .
<http://ex/d/30> ex:label "Empty" .
<http://ex/p/1> ex:name "Ann" ; ex:dept <http://ex/d/20> .
<http://ex/p/2> ex:name "Bob" ; ex:dept <http://ex/d/10> .
<http://ex/p/3> ex:name "Zed" ; ex:dept <http://ex/d/10> .
"#;

fn item1d(query: &str) {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let q = parse(query);
    let tp = tree(&maps, &q, &schema).expect("tree must close the subplan-OPTIONAL shape");
    assert_vs_spareval(I1D_TTL, query, &tp, &conn);
}

#[test]
fn item1d_aggregation_subplan_as_optional_right() {
    // OPTIONAL over an AGGREGATION sub-SELECT, correlated on ?d. dept/30 (Empty) has no
    // persons ⇒ the inner has no group for it ⇒ ?c is NULL-padded (LEFT JOIN no-match).
    item1d(&format!(
        "{PFX} SELECT ?l ?c WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT ?d (COUNT(?p) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d }} }}"
    ));
}

#[test]
fn item1d_distinct_subplan_as_optional_right() {
    // OPTIONAL over a DISTINCT sub-SELECT, correlated on ?d (fans out per dept's names).
    item1d(&format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} }}"
    ));
}

#[test]
fn item1d_left_nullable_determinant_subplan_optional() {
    // R2 COALESCE / R1 null-safe: a PRIOR OPTIONAL binds the shared var ?d (nullable),
    // THEN the aggregation sub-SELECT OPTIONAL correlates on it — the determinant on the
    // preserved side may itself be UNBOUND (every person here has a dept, so it is bound,
    // but the lowering must still emit the null-safe ON / COALESCE without corrupting it).
    item1d(&format!(
        "{PFX} SELECT ?n ?tot WHERE {{ ?p ex:name ?n \
         OPTIONAL {{ ?p ex:dept ?d }} \
         OPTIONAL {{ SELECT ?d (COUNT(?p2) AS ?tot) WHERE {{ ?p2 ex:dept ?d }} GROUP BY ?d }} }}"
    ));
}

#[test]
fn item1d_uncorrelated_subplan_as_optional_right() {
    // OPTIONAL over a sub-SELECT sharing NO variable with the left ⇒ LEFT JOIN ON 1 = 1
    // (every left row extended by the single aggregate row; a non-empty subplan ⇒ never
    // NULL-padded here).
    item1d(&format!(
        "{PFX} SELECT ?l ?tc WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT (COUNT(?p) AS ?tc) WHERE {{ ?p ex:name ?n2 . ?p ex:dept ?dd }} }} }}"
    ));
}

/// Revert-proof lock on the ORDER-BY-+-LIMIT-inside-a-SubPlan-as-OPTIONAL-right SOUND
/// 501. `is_single_subplan_branch` rejects a nested plan carrying ORDER BY + LIMIT/
/// OFFSET (`subplan_emits_soundly_as_derived_table`) — a derived table is pure SQL with
/// no exec stage, but `Plan::prepared_branches` keeps ORDER BY OUT of SQL (exec applies
/// it type-aware), so an ORDER BY + LIMIT SubPlan would silently drop BOTH and let the
/// WRONG rows survive. Rejecting it makes the OPTIONAL fall through to the decomposition
/// and stay a 501 (via `not_exists_cond_for`'s SubPlan boundary). NOTE: the analogous
/// INNER-JOIN-input shape (`… . {SELECT … ORDER BY … LIMIT n}`) is a SEPARATE,
/// PRE-EXISTING `lower_as_subplan` wrong-answer (it drops ORDER BY/LIMIT), out of this
/// change's scope and reported for the team — it is deliberately NOT asserted here.
#[test]
fn item1d_orderby_limit_subplan_optional_stays_sound_501() {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} \
         ORDER BY ?nm LIMIT 1 }} }}"
    );
    let q = parse(&query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "ORDER BY + LIMIT inside a SubPlan-as-OPTIONAL-right must be a SOUND 501 (ORDER BY \
         is not emitted in SQL, so it cannot be faithfully sliced): `{query}`"
    );
}

/// Revert-proof lock on the SubPlan-OPTIONAL-with-inner-FILTER SOUND 501. An
/// OPTIONAL's own inner FILTER (R5) over a SubPlan right — e.g. `FILTER(?c > 1)` on a
/// subquery-aggregate output — placed in the LEFT JOIN ON did NOT evaluate correctly
/// (it wrongly NULL-padded rows the filter should have KEPT — a wrong answer verified
/// vs spareval during development). So `left_join_over_subplan` is gated on
/// `expr.is_none()`; the FILTER case falls through to the ordinary decomposition and
/// stays a 501. Reverting that gate turns this into a WRONG answer (the filter is
/// dropped, wrongly matching every row). dept/20 (Eng, ?c=1) FAILS `?c > 1` ⇒ must be
/// NULL-padded, dept/10 (Sales, ?c=2) must KEEP ?c=2 — a dropped filter loses that.
#[test]
fn item1d_subplan_optional_with_inner_filter_stays_sound_501() {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?l ?c WHERE {{ ?d ex:label ?l OPTIONAL {{ \
         {{ SELECT ?d (COUNT(?p) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d }} \
         FILTER(?c > 1) }} }}"
    );
    let q = parse(&query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "a SubPlan-OPTIONAL carrying an inner FILTER must stay a SOUND 501 (the \
         FILTER-in-ON over a SubPlan does not evaluate correctly): `{query}`"
    );
}

/// Revert-proof lock on the SubPlan-INSIDE-an-EXISTS/NOT-EXISTS/MINUS-body SOUND 501
/// (ADR-0023 Item 1d meta-audit, round 4). `lower_iq_exists` (which serves FILTER
/// EXISTS, FILTER NOT EXISTS, AND MINUS) lowers each inner body branch into a
/// `SqlCond::{Exists,NotExists} { scans: Vec<Scan>, conds }`. When the body JOINs a
/// modifier sub-SELECT (`{ ?a p ?nm . { SELECT DISTINCT/agg … } }`) the inner branch
/// carries a `subplan_joins` derived table whose alias the correlation `conds`
/// reference — but `scans` (a plain `Vec<Scan>`) cannot introduce it, so the emitted
/// correlated subquery references a FROM alias that does not exist ("no such column
/// t{sp}.c{i}" — a CRASH at exec, verified: `tree()` returned `Ok` then blew up). The
/// `!r.subplan_joins.is_empty()` guard turns that into a SOUND 501 (ADR-0007: a 501
/// beats a crash), mirroring `not_exists_cond_for`'s matching boundary for the OPTIONAL
/// anti-join half — the SAME `SqlCond`-cannot-hold-a-SubPlan limitation reached through
/// a DIFFERENT entry point than the round-1..3 OPTIONAL-decomposition guards. Reverting
/// the guard makes `tree()` return `Ok` again (these assertions fail) and executing the
/// plan is an invalid-SQL error, never a correct answer.
fn item1d_body_subplan_must_501(query: &str) {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let q = parse(query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "a SubPlan (modifier sub-SELECT) joined INSIDE an EXISTS/NOT EXISTS/MINUS body \
         must be a SOUND 501 — SqlCond::{{Exists,NotExists}} cannot carry a derived table, \
         so the un-guarded lowering crashes at exec: `{query}`"
    );
}

#[test]
fn item1d_exists_body_carrying_distinct_subplan_stays_sound_501() {
    item1d_body_subplan_must_501(&format!(
        "{PFX} SELECT ?l WHERE {{ ?d ex:label ?l FILTER EXISTS {{ ?px ex:name ?nm . \
         {{ SELECT DISTINCT ?d WHERE {{ ?p ex:dept ?d }} }} }} }}"
    ));
}

#[test]
fn item1d_not_exists_body_carrying_distinct_subplan_stays_sound_501() {
    item1d_body_subplan_must_501(&format!(
        "{PFX} SELECT ?l WHERE {{ ?d ex:label ?l FILTER NOT EXISTS {{ ?px ex:name ?nm . \
         {{ SELECT DISTINCT ?d WHERE {{ ?p ex:dept ?d }} }} }} }}"
    ));
}

#[test]
fn item1d_minus_body_carrying_distinct_subplan_stays_sound_501() {
    item1d_body_subplan_must_501(&format!(
        "{PFX} SELECT ?l WHERE {{ ?d ex:label ?l MINUS {{ ?d ex:label ?l2 . \
         {{ SELECT DISTINCT ?d WHERE {{ ?p ex:dept ?d }} }} }} }}"
    ));
}

#[test]
fn item1d_exists_body_carrying_aggregate_subplan_stays_sound_501() {
    // The aggregate-subplan variant crashes identically (the anti-join scans still
    // drop the derived table) — locked as a 501 too so a future SubPlan-in-SqlCond
    // capability must consciously revisit BOTH modifier kinds, not silently relax one.
    item1d_body_subplan_must_501(&format!(
        "{PFX} SELECT ?l WHERE {{ ?d ex:label ?l FILTER EXISTS {{ ?px ex:name ?nm . \
         {{ SELECT ?d (COUNT(?p) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d }} }} }}"
    ));
}

// ============================================================================
// Item 1d — outer SELECT DISTINCT over a ROW-MULTIPLYING SubPlan-OPTIONAL right
// (ADR-0023 parity backlog, round-4 review defect). `distinct_removal` proves the
// outer DISTINCT redundant from the core PK key alone (`?d` reads dept's non-null
// PK ⇒ injective), but a `left == true` `SubPlanJoin` (a modifier sub-SELECT
// attached as the OPTIONAL's right operand) MULTIPLIES a dept row into several
// output rows (its solution multiset LEFT-JOINed on the correlation) — so the
// DISTINCT is NOT redundant and dropping it leaves duplicates in the bag (=_bag
// broken, ADR-0007 silent wrong answer). The fix gates the `distinct_removal` call
// on `out[0].subplan_joins.is_empty()` (cascade/mod.rs), keeping the DISTINCT in the
// emitted SQL. These lock BOTH the multiplying variants (outer DISTINCT must collapse
// to 3) AND the non-multiplying / non-DISTINCT variants (must be unchanged) so the
// narrow fix does not overshoot. Fixture I1D: dept 10 "Sales" has 2 persons (Bob,
// Zed), so the per-label subplan fans dept/10 out to 2 rows; dept 20/30 stay 1 each.
// Gated =_bag vs the INDEPENDENT spareval oracle (`item1d` ⇒ `assert_vs_spareval`).
// ============================================================================

#[test]
fn item1d_distinct_over_multiplying_distinct_subplan_optional() {
    // Round-4 review repro (verbatim). Outer `SELECT DISTINCT ?d`; the OPTIONAL right
    // is a DISTINCT sub-SELECT correlated on the label ?l. dept/10 (Sales) fans out to
    // 2 subplan rows {(Sales,Bob),(Sales,Zed)} ⇒ the OPTIONAL yields d/10 twice; the
    // outer DISTINCT must collapse those ⇒ {d/10, d/20, d/30} = 3. Pre-fix:
    // `distinct_removal` dropped the DISTINCT (proved redundant from ?d=dept.PK alone,
    // ignoring `subplan_joins`) ⇒ emitted SQL had NO DISTINCT ⇒ 4 rows (d/10 ×2).
    item1d(&format!(
        "{PFX} SELECT DISTINCT ?d WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?l ?nm WHERE \
         {{ ?px ex:name ?nm . ?px ex:dept ?dx . ?dx ex:label ?l }} }} }}"
    ));
}

#[test]
fn item1d_distinct_over_multiplying_orderby_subplan_optional() {
    // Same defect via an ORDER-BY modifier sub-SELECT (no LIMIT/OFFSET ⇒ lowered as a
    // derived table by `subplan_emits_soundly_as_derived_table`). ORDER BY does not
    // dedupe, so dept/10 still fans out to 2 rows; the outer DISTINCT ⇒ 3. Pre-fix ⇒ 4.
    item1d(&format!(
        "{PFX} SELECT DISTINCT ?d WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT ?l ?nm WHERE \
         {{ ?px ex:name ?nm . ?px ex:dept ?dx . ?dx ex:label ?l }} ORDER BY ?nm }} }}"
    ));
}

#[test]
fn item1d_nondistinct_over_multiplying_subplan_optional_unchanged() {
    // No-overshoot lock — the fix is gated on `ctx.distinct`, so this path is untouched.
    // WITHOUT the outer DISTINCT the bag legitimately keeps every row: dept/10 ×2 +
    // dept/20 + dept/30 = 4. Must stay 4 both before and after the fix (proves the fix
    // does not force a spurious DISTINCT onto a non-DISTINCT query).
    item1d(&format!(
        "{PFX} SELECT ?d WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?l ?nm WHERE \
         {{ ?px ex:name ?nm . ?px ex:dept ?dx . ?dx ex:label ?l }} }} }}"
    ));
}

#[test]
fn item1d_distinct_over_nonmultiplying_aggregate_subplan_optional_unchanged() {
    // No-overshoot lock — an AGGREGATE sub-SELECT yields exactly 1 row per group ⇒ NO
    // row multiplication ⇒ the outer DISTINCT collapses nothing. Correct = 3 both before
    // and after the fix (pre-fix `distinct_removal` dropped a genuinely-redundant
    // DISTINCT, still 3; post-fix keeps the DISTINCT in SQL, still 3). Confirms the fix
    // narrows to the multiplying case and does not alter a case that was already right.
    item1d(&format!(
        "{PFX} SELECT DISTINCT ?d WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT ?l (COUNT(?px) AS ?c) WHERE \
         {{ ?px ex:dept ?dx . ?dx ex:label ?l }} GROUP BY ?l }} }}"
    ));
}

// ============================================================================
// Item 1d REGRESSION LOCKS — a variable bound by a LEFT-JOINed SubPlan
// (`left_join_over_subplan`, `SubPlanJoin { left: true }`) may be UNBOUND when the
// derived-table LEFT JOIN finds no match (dept/30 "Empty"). Any downstream
// EXISTS / NOT EXISTS / MINUS / second-OPTIONAL that correlates on it MUST treat it
// as nullable — the same rule the engine already applies to a prior-OPTIONAL scan
// alias. The `nullable_aliases()` detector (opts + left-subplan aliases) closes the
// gap that `e7cb7e6` opened by consulting only `opts`. These lock the exact
// counterexamples an adversarial review reproduced (silent wrong answers + a SQL
// emission crash), diffed against the INDEPENDENT `spareval` oracle over the I1D
// fixture (dept 30 "Empty" has no persons ⇒ the subplan var is genuinely UNBOUND).
// ============================================================================

#[test]
fn item1d_exists_over_subplan_bound_var() {
    // Reviewer CE1. `?nm` comes from a DISTINCT sub-SELECT LEFT-JOINed as the OPTIONAL
    // right; dept 30 (Empty) NULL-pads it. `FILTER EXISTS { ?p2 ex:name ?nm }` with an
    // UNBOUND `?nm` is a free-variable existence test (SPARQL §18.6 substitution) ⇒ TRUE
    // (people exist), so the Empty row is KEPT — spareval = 4 rows. The pre-fix bug
    // treated `?nm` as mandatory ⇒ `t.cNm = name` over a NULL ⇒ EXISTS false ⇒ Empty
    // silently DROPPED (3 rows).
    item1d(&format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         FILTER EXISTS {{ ?p2 ex:name ?nm }} }}"
    ));
}

#[test]
fn item1d_not_exists_over_subplan_bound_var() {
    // Reviewer CE2. NOT EXISTS of CE1's body: every row's EXISTS is TRUE ⇒ NOT EXISTS
    // FALSE ⇒ spareval = 0 rows. The pre-fix bug KEPT the Empty row (NULL never matched
    // the un-guarded equality ⇒ NOT EXISTS wrongly TRUE) — 1 silently-added row.
    item1d(&format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         FILTER NOT EXISTS {{ ?p2 ex:name ?nm }} }}"
    ));
}

#[test]
fn item1d_exists_multivar_mandatory_plus_subplan_nullable() {
    // Adversarial: a mandatory shared var (?l, from `?d ex:label ?l`) AND a
    // subplan-nullable shared var (?nm) in ONE EXISTS correlation. For the Empty row
    // ?l="Empty" is bound and DOES match the body's `?d2 ex:label ?l`, while ?nm is
    // UNBOUND (free ⇒ matches any person) ⇒ EXISTS TRUE ⇒ KEEP (spareval = 4). The bug's
    // mandatory-treatment of ?nm makes `t.cNm = name` over NULL false ⇒ EXISTS false ⇒
    // Empty wrongly dropped (3). Isolates the nullable-var handling from the mandatory one.
    item1d(&format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         FILTER EXISTS {{ ?d2 ex:label ?l . ?p2 ex:name ?nm }} }}"
    ));
}

#[test]
fn item1d_not_exists_multivar_mandatory_plus_subplan_nullable() {
    // NOT EXISTS of the multi-var body: all rows EXISTS-true ⇒ 0 rows (spareval). Bug
    // keeps Empty ⇒ 1 row.
    item1d(&format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         FILTER NOT EXISTS {{ ?d2 ex:label ?l . ?p2 ex:name ?nm }} }}"
    ));
}

#[test]
fn item1d_minus_multivar_mandatory_plus_subplan_nullable() {
    // MINUS body sharing mandatory ?l + subplan-nullable ?nm. For Empty (?l="Empty"
    // bound, ?nm unbound) the body DOES have rows with ?l="Empty" (dept 30 × every
    // person name), and μ agrees on the shared BOUND var ?l with a non-empty domain
    // overlap ⇒ COMPATIBLE ⇒ Empty is REMOVED (spareval = 0 rows). The bug treats ?nm as
    // mandatory ⇒ its NULL equality is never satisfied ⇒ NOT EXISTS wrongly TRUE ⇒ Empty
    // WRONGLY KEPT (1 row) — the null-safe `(t.cNm IS NULL OR …)` guard is what lets the
    // ?l-only match remove it.
    item1d(&format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         MINUS {{ ?d2 ex:label ?l . ?p2 ex:name ?nm }} }}"
    ));
}

#[test]
fn item1d_minus_single_subplan_nullable_var_no_regression() {
    // MINUS sharing ONLY the subplan-nullable ?nm. For Empty (?nm unbound) the domain is
    // disjoint from the body's ⇒ MINUS is a documented no-op (§8.3.2) ⇒ Empty KEPT; the
    // three bound rows share a bound ?nm with a compatible body row ⇒ removed. spareval =
    // 1 row (Empty). A lock that the fix does not over-remove the disjoint-domain row.
    item1d(&format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         MINUS {{ ?p2 ex:name ?nm }} }}"
    ));
}

#[test]
fn item1d_chained_subplan_optionals_propagate_nullability() {
    // Two SubPlan-OPTIONALs in sequence; the SECOND correlates on ?nm, bound by the
    // FIRST's LEFT-JOINed subplan (a left-subplan alias). Both go through
    // `left_join_over_subplan` (subplan RIGHT sides), which emits subplans in order
    // (sp1 before sp2) so sp2's ON referencing sp1 is valid SQL. With `nullable_aliases`
    // seeing sp1's alias, ?nm's left def is flagged nullable ⇒ null-safe ON + COALESCE:
    // for the Empty row (?nm unbound) the second join's `(t1.cNm IS NULL OR …)` is TRUE
    // ⇒ it fans out over every sp2 row (SPARQL: an unbound join var is free) — matching
    // spareval. Exercises the chained-nullability path that stays CORRECT (no 501).
    item1d(&format!(
        "{PFX} SELECT ?l ?nm ?cc WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         OPTIONAL {{ SELECT ?nm (COUNT(?p3) AS ?cc) WHERE {{ ?p3 ex:name ?nm }} GROUP BY ?nm }} }}"
    ));
}

/// Reviewer CE3 — a PLAIN-scan second OPTIONAL (`?p2 ex:name ?nm`) chained after a
/// SubPlan-OPTIONAL, correlating on the subplan-bound ?nm. `build_left_join` would push
/// an `OptJoin` whose ON references the SubPlan's derived-table alias `t{sp}`, but emit
/// renders `opts` BEFORE `subplan_joins`, so the ON references a table to its right — an
/// invalid-SQL CRASH at execution (`e7cb7e6` produced `Ok(plan)` that then blew up). The
/// `shared_reads_left_subplan` guard turns that into a SOUND 501 (ADR-0007: a 501 beats a
/// crash). Reverting the guard makes `tree()` return `Ok` again (this assertion fails)
/// and executing that plan is an invalid-SQL error — never a correct answer.
#[test]
fn item1d_plain_second_optional_over_subplan_var_stays_sound_501() {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?l ?nm ?p2 WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} \
         OPTIONAL {{ ?p2 ex:name ?nm }} }}"
    );
    let q = parse(&query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "a plain second OPTIONAL correlating on a LEFT-JOINed-SubPlan-bound variable \
         must be a SOUND 501 (its LEFT JOIN ON would reference a derived table emitted \
         to its right — an invalid-SQL crash): `{query}`"
    );
}

// ============================================================================
// Item 1d ROUND-5 REGRESSION LOCKS — consumer-side sweep of `subplan_joins`. The
// round-1..4 guards covered every OPTIONAL-decomposition / FILTER-EXISTS entry point
// (`build_left_join`, `inner_join_one`, `not_exists_cond_for`, `lower_iq_exists`) that
// correlates on a LEFT-JOINed-SubPlan-bound (nullable) variable — but NOT the plain
// InnerJoin / BGP-merge one (`IqNode::InnerJoin` → `join_branches` → `unfold::merge`).
// `merge` pushes a PLAIN (non-null-safe) equality per shared variable, so a mandatory
// pattern joined with a subplan-OPTIONAL group that shares the subplan-bound var
// silently DROPPED the subplan-no-match rows (SPARQL compatible-merge KEEPS them, binding
// the var from the mandatory side). Verified vs the INDEPENDENT spareval oracle: tree 3,
// oracle 6 (dept/30 "Empty" NULL-pads the subplan ⇒ ?nm unbound ⇒ the mandatory
// `?p2 ex:name ?nm` should re-bind it over every name). `unfold::merge` now returns a
// sound 501 for that shape (either merge orientation), mirroring `shared_reads_left_subplan`.
// NOTE: the flat path SUPPORTS this shape correctly (6 rows) via its own subquery
// lowering — this is a deliberate tree-side 501 boundary (ADR-0023 Item 1d), tested
// outside `diff` (whose flat/tree 501-parity rule does not apply to a tree-only capability
// boundary), exactly like `item1d_plain_second_optional_over_subplan_var_stays_sound_501`.
// ============================================================================

/// Primary repro (silent wrong answer → sound 501). Subplan-OPTIONAL group FIRST, then a
/// mandatory `?p2 ex:name ?nm` sharing the subplan-bound (nullable) ?nm. Reverting the
/// `merge_correlates_on_nullable_subplan` guard makes `tree()` return `Ok` and executing
/// it yields 3 rows where spareval yields 6 (the three dept/30 "Empty" rows — where ?nm is
/// unbound and must be re-bound from the mandatory pattern — are silently dropped).
#[test]
fn item1d_r5_innerjoin_after_subplan_optional_shared_var_stays_sound_501() {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?l ?nm WHERE {{ \
         {{ ?d ex:label ?l OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} }} \
         ?p2 ex:name ?nm }}"
    );
    let q = parse(&query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "an INNER JOIN correlating a mandatory pattern on a LEFT-JOINed-SubPlan-bound \
         (nullable) variable must be a SOUND 501 — `unfold::merge` pushes a plain \
         non-null-safe equality that silently drops the subplan-no-match rows: `{query}`"
    );
}

/// Mirror orientation (also a silent wrong answer → sound 501). The mandatory pattern
/// FIRST, then the subplan-OPTIONAL group — the subplan-carrying branch is the INCOMING
/// `right` operand of `join_branches`/`merge` (the guard must catch both orientations).
/// Same 3-vs-6 divergence when the guard is reverted.
#[test]
fn item1d_r5_innerjoin_before_subplan_optional_shared_var_stays_sound_501() {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?l ?nm WHERE {{ ?p2 ex:name ?nm . \
         {{ ?d ex:label ?l OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} }} }}"
    );
    let q = parse(&query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "the mirror orientation (subplan group as the incoming merge right operand) must \
         ALSO be a SOUND 501: `{query}`"
    );
}

/// No-overshoot lock. The InnerJoin's shared variable is the MANDATORY ?d (bound by
/// `?d ex:label ?l`, a core scan), NOT the subplan-bound ?nm — so the guard must NOT fire
/// and the query stays a correct answer. Proves the 501 is narrow (keyed on a shared var
/// actually reading a `left == true` subplan alias, not "any branch carrying a subplan").
/// dept/30 "Empty" NULL-pads ?nm but ?d is bound ⇒ the `?d ex:label ?l2` join keeps it;
/// diffed vs the independent spareval oracle.
#[test]
fn item1d_r5_innerjoin_sharing_mandatory_var_not_501_and_correct() {
    item1d(&format!(
        "{PFX} SELECT ?l ?l2 ?nm WHERE {{ \
         {{ ?d ex:label ?l OPTIONAL {{ SELECT DISTINCT ?d ?nm WHERE {{ ?p ex:dept ?d ; ex:name ?nm }} }} }} \
         ?d ex:label ?l2 }}"
    ));
}

// ============================================================================
// Item 1d ROUND-3 REGRESSION LOCKS — the `left_join_over_subplan` Branch shape (a
// `SubPlanJoin { left: true }`, introduced by `e7cb7e6`) composing with pre-existing
// merge / emit / cascade machinery that predates it. A third adversarial review found
// more silent-crash / silent-wrong-answer paths of the SAME root shape as Rounds 1-2.
// Each is diffed against the INDEPENDENT `spareval` oracle (correct-answer cases) or
// asserted as a sound 501 (ADR-0007: a 501 or a correct bag, never a crash / wrong bag).
// ============================================================================

/// Defect 1 (crash → sound 501). A nested subplan-OPTIONAL `{ ?a ex:name ?nm
/// OPTIONAL { <subSELECT> } }` lowers (via `left_join_over_subplan`) to a branch
/// carrying BOTH a core scan AND a `subplan_joins` entry. When THAT branch is the
/// RIGHT side of an OUTER OPTIONAL, `left_join_branches`' single-scan fast path
/// (`right[0].core.len() == 1`) routed it to `build_left_join`, which pushed the
/// core scan as an `OptJoin` but SILENTLY DROPPED `right.subplan_joins` — the
/// derived-table alias `t{sp}` stayed referenced in the SELECT (`?c`) with no FROM
/// entry ever introducing it → `no such column t{sp}.c1` at exec. `build_left_join`
/// now declines (sound 501) when the right carries `subplan_joins`, matching
/// `not_exists_cond_for`'s existing same-condition boundary. Reverting the guard
/// makes `tree()` return `Ok` again and executing it is an invalid-SQL crash.
#[test]
fn item1d_r3_nested_subplan_optional_as_outer_optional_right_stays_sound_501() {
    let conn = sqlite::load(I1D_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(I1D_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?l ?nm ?c WHERE {{ ?d ex:label ?l \
         OPTIONAL {{ ?a ex:name ?nm \
         OPTIONAL {{ SELECT ?nm (COUNT(?p) AS ?c) WHERE {{ ?p ex:name ?nm }} GROUP BY ?nm }} }} }}"
    );
    let q = parse(&query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "a subplan-carrying OPTIONAL-right routed through build_left_join's fast path \
         must be a SOUND 501 (dropping right.subplan_joins references a derived table \
         never introduced in FROM — an invalid-SQL crash): `{query}`"
    );
}

/// Defect 2 (wrong answer → correct). A LEADING bare `OPTIONAL {?p ex:name ?nm}`
/// makes the enclosing branch core-EMPTY (its left is the empty-BGP identity) with
/// the person scan in `opts`; the following subplan-OPTIONAL correlates on ?nm
/// (bound by that opt). `render_from`'s core-empty path made the FIRST subplan the
/// FROM anchor and emitted it with NO ON clause — silently DROPPING the
/// `person.name = t.c0` correlation → an uncorrelated cross join (9 rows, not 3).
/// `render_from` now uses a synthetic `(SELECT 1)` anchor and renders opts BEFORE
/// subplans (the SAME order as the core-bearing path), so the correlated subplan
/// keeps its ON and references the opt already emitted to its left. spareval = 3.
#[test]
fn item1d_r3_two_optionals_empty_left_subplan_correlates_on_prior_opt() {
    item1d(&format!(
        "{PFX} SELECT ?nm ?c WHERE {{ OPTIONAL {{ ?p ex:name ?nm }} \
         OPTIONAL {{ SELECT ?nm (COUNT(?p2) AS ?c) WHERE {{ ?p2 ex:name ?nm }} GROUP BY ?nm }} }}"
    ));
}

/// Defect 3 (crash → sound 501). A property-path LEFT side of a subplan-OPTIONAL:
/// `left_join_over_subplan` pushed a `SubPlanJoin` onto the path branch, producing a
/// branch with BOTH `path: Some(_)` AND `subplan_joins` — a combination
/// `emit_branch_with` routes to `emit_path_branch`, which renders ONLY the path's own
/// recursive CTE and IGNORES `subplan_joins` entirely (`?c` referencing `t{sp}` then
/// had no FROM entry → crash). `build_left_join` already guards the analogous
/// path-left + plain-scan-right case (`path_as_optional_left_via_single_scan_fast_
/// path_is_a_sound_501`); `left_join_over_subplan` now guards it too. Reverting the
/// guard makes `tree()` return `Ok` and exec crashes.
#[test]
fn item1d_r3_path_left_with_subplan_optional_right_stays_sound_501() {
    let conn = sqlite::load(PE_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(PE_R2RML).expect("R2RML parses");
    let query = format!(
        "{PFX} SELECT ?o ?c WHERE {{ ?s ex:reaches+ ?o \
         OPTIONAL {{ SELECT ?o (COUNT(?x) AS ?c) WHERE {{ ?x ex:reaches ?o }} GROUP BY ?o }} }}"
    );
    let q = parse(&query);
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "a property-path LEFT side of a subplan-OPTIONAL must be a SOUND 501 \
         (emit_path_branch ignores subplan_joins → the subplan's derived table is never \
         emitted, an invalid-SQL crash): `{query}`"
    );
}

/// Defect 4 (crash → correct). A subplan-OPTIONAL correlating on a variable bound by a
/// PRIOR plain OPTIONAL that is itself a PK self-LEFT-JOIN. The cascade's
/// `self_left_join_elimination` collapsed the redundant self-join and `rewrite_alias`'d
/// every reference of the dropped opt alias onto the kept core scan — but it did NOT
/// rewrite `subplan_joins[_].on`, so the subplan's correlation kept referencing the
/// vanished alias → `no such column` at exec. The cascade now SKIPS the constraint-driven
/// passes for any `subplan_joins`-carrying branch (mirroring its existing `path` /
/// `NotExists` bail), closing the whole class of "an optimizer pass drops / merges a scan
/// a subplan's ON still needs". spareval = 3 (?nm2 = ?nm by the PK self-join; each name's
/// COUNT = 1). Reverting the skip makes the cascade dangle the ON → an invalid-SQL crash.
#[test]
fn item1d_r3_cascade_self_lj_elim_keeps_subplan_correlation() {
    item1d(&format!(
        "{PFX} SELECT ?nm ?nm2 ?c WHERE {{ ?p ex:name ?nm \
         OPTIONAL {{ ?p ex:name ?nm2 }} \
         OPTIONAL {{ SELECT ?nm2 (COUNT(?p3) AS ?c) WHERE {{ ?p3 ex:name ?nm2 }} GROUP BY ?nm2 }} }}"
    ));
}

// ============================================================================
// SOUND-501 BOUNDARIES — ADR-0023 parity backlog items assessed as MUST-STAY-501,
// each with a precise architectural reason. Locked here via `diff` (BOTH flat and
// tree must return `Err(Unsupported)` — the identical-501-set arm), so a future
// change that silently turns any of these into a WRONG answer is caught (ADR-0007:
// a sound 501 beats a possibly-wrong answer). These are the "here is exactly what is
// architecturally missing" proofs, not "too hard" hand-waves.
// ============================================================================

/// Item 4 — a property-path inner inside EXISTS / NOT EXISTS / MINUS. SUPERSEDED 2026-07-07
/// by ADR-0025 Tier-2 gap 1: the TREE now computes it via a new `SqlCond::PathExists` variant
/// (a correlated `[NOT] EXISTS` over the path's recursive-CTE derived table `t{alias}` —
/// exactly the "new SqlCond variant carrying a CTE-backed sub-evaluation" this comment
/// predicted). Tree matches spareval for P+ and length-1 composites; FLAT still soundly 501s
/// (its `unfold::lower_exists` keeps the `r.path.is_some()` guard — tree-exceeds-flat). Full
/// coverage: `adr0025_tier2_gap1_path_in_exists_notexists_minus` (+ the reflexive-501 test).
#[test]
fn item4_property_path_inner_in_exists_minus_now_tree_superset_of_flat() {
    let conn = sqlite::load(PE_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(PE_R2RML).unwrap();
    for q in [
        format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches ?o FILTER EXISTS {{ ?s ex:reaches+ ?x }} }}"),
        format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches ?o FILTER NOT EXISTS {{ ?s ex:reaches+ ?x }} }}"),
        format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches ?o MINUS {{ ?s ex:reaches+ ?x }} }}"),
    ] {
        let parsed = parse(&q);
        assert_vs_spareval(PE_TTL, &q, &tree(&maps, &parsed, &schema).expect("tree computes it"), &conn);
        assert!(matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
            "flat still soundly 501s a path inside EXISTS/NOT EXISTS/MINUS: {q}");
    }
}

/// Item 7 — GROUP BY over a property-path closure. SUPERSEDED 2026-07-07 by ADR-0025 Tier-2
/// gap 4: the TREE now computes it by routing the path aggregation to the Rust group path
/// (`rust_group_execute` runs the path branch's own SQL and groups the solutions by variable
/// name — no base-column access needed, so no path-as-SubPlan machinery was required after
/// all). Tree matches spareval; FLAT still soundly 501s (its `unfold` group() lacks the same
/// routing — the documented tree-exceeds-flat limitation). Full coverage: the
/// `adr0025_tier2_gap4_group_by_over_path_closure` variants (both group directions, implicit
/// group, P*, cyclic).
#[test]
fn item7_group_by_over_property_path_now_tree_superset_of_flat() {
    let conn = sqlite::load(PE_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(PE_R2RML).unwrap();
    let q = format!("{PFX} SELECT ?s (COUNT(?o) AS ?c) WHERE {{ ?s ex:reaches+ ?o }} GROUP BY ?s");
    let parsed = parse(&q);
    assert_vs_spareval(
        PE_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree computes it"),
        &conn,
    );
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "flat still soundly 501s GROUP BY over a path closure"
    );
}

/// Item 5 — `COUNT(DISTINCT *)`. SUPERSEDED 2026-07-07 by ADR-0025 Tier-2 gap 3: the TREE
/// now IMPLEMENTS it (counts DISTINCT whole solutions via `rust_agg` whole-row dedup, rather
/// than the non-portable SQL `COUNT(DISTINCT c1,…,cn)`), so it is no longer a must-stay-501.
/// Tree computes it (matching spareval); FLAT still soundly 501s — its Rust-group path cannot
/// bind an aggregate result var (tree exceeds flat). Full coverage: the
/// `adr0025_tier2_count_distinct_star_*` tests below (single-branch, UNION, GROUP BY).
#[test]
fn item5_count_distinct_star_now_tree_superset_of_flat() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let q = format!("{PFX} SELECT (COUNT(DISTINCT *) AS ?c) WHERE {{ ?p ex:name ?n }}");
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree computes it"),
        &conn,
    );
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "flat still soundly 501s COUNT(DISTINCT *)"
    );
}

/// Item 8 — a post-GROUP-BY EXPRESSION over a multi-branch (UNION) aggregate. MUST STAY
/// 501: a UNION aggregate lowers to a `Plan::rust_group` (buffer-and-group in Rust); the
/// aggregate outputs are NOT columns of the pre-group union branches, so the outer
/// `Construction`'s `(expr AS ?v)` cannot fold into the branch bindings. The rust_group
/// path can only RENAME an aggregate output (`rename_rust_group_outputs`), never COMPUTE
/// an expression over it — that would require post-group expression evaluation in the
/// Rust executor (`exec_core::rust_group_execute`), a new capability. (The single-branch
/// SQL GROUP BY path DOES support post-aggregate expressions; only the multi-branch Rust
/// path is deferred.) Flat 501s identically (its own agg-over-UNION limitation). `diff`
/// locks it.
#[test]
fn item8_post_group_by_expr_over_union_aggregate_stays_501() {
    diff(
        AGG_SQL,
        AGG_R2RML,
        None,
        &format!(
            "{PFX} SELECT ?g (COUNT(?v) AS ?c) (STR(COUNT(?v)) AS ?cs) WHERE {{ ?s ex:grp ?g . \
         {{ ?s ex:p1 ?v }} UNION {{ ?s ex:p2 ?v }} }} GROUP BY ?g"
        ),
    );
}

/// Item 2 / 1b — a MULTI-branch modifier sub-SELECT (a UNION inside a LIMIT, or an
/// OPTIONAL whose right is a subselect containing a UNION/nested-OPTIONAL) as a join or
/// OPTIONAL input. MUST STAY 501: `lower_as_subplan` remaps each projected variable's
/// `TermDef` against ONE inner branch's column projection; a multi-branch (`UNION ALL`)
/// derived table would need every arm to agree on each variable's term STRUCTURE + type
/// (the same cross-arm compatibility `try_sql_group_over_union` proves), which is not
/// checked here — so it is a sound 501 rather than a possibly-mismatched remap. Tree-
/// only assertion (the flat path translates this shape but emits an un-introduced
/// derived-table alias — a separate pre-existing flat limitation, so `diff` is not
/// usable and flat is not exec'd).
#[test]
fn item2_multi_branch_subplan_stays_501() {
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    // A subselect containing a UNION, used as a join input.
    let q = parse(&format!(
        "{PFX} SELECT ?p ?e WHERE {{ ?p ex:name ?n . \
         {{ SELECT ?e WHERE {{ {{ ?p ex:email ?e }} UNION {{ ?p ex:name ?e }} }} LIMIT 5 }} }}"
    ));
    assert!(
        matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(_))),
        "a multi-branch (UNION) SubPlan must stay a sound 501 (cross-arm term-structure \
         agreement is not proven)"
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

// ============================================================================
// COMPOSITION SWEEP (ADR-0023 optimizer-residue) — closes the demonstrated
// blind spot: the normalize.rs rewrites were each tested in isolation, but a
// rewrite COMPOSED with a query modifier (the partial-fold + bare-LIMIT
// row-order bug, commit 84365ff) slipped every green gate because compositions
// were never exercised. This drives every normalize rewrite through the full
// modifier matrix { none, DISTINCT, ORDER BY, LIMIT, OFFSET, LIMIT+OFFSET,
// ORDER BY+LIMIT, DISTINCT+ORDER BY+LIMIT } against the =_bag oracle.
//
// Routing rationale (mirrors this file's established conventions):
//   * Non-slice variants (none / DISTINCT / ORDER BY) produce a deterministic
//     row SET → `diff_p` (flat-vs-tree =_bag AND vs the independent spareval
//     oracle over the set-faithful fixture-P graph).
//   * Any LIMIT/OFFSET variant has an implementation-defined tie-break WITHOUT a
//     total ORDER BY (and even with one, at the window boundary under a
//     collation spareval and this engine needn't share) → `diff_p_bag`
//     (flat-vs-tree =_bag only — always sound, both are the same engine).
//   * ORDER BY sequence (which `solutions_bag_eq` does NOT check) is verified
//     separately by `assert_ordered_ft`: flat vs tree row-sequence equality
//     (collation-neutral — both sides are this engine), so a rewrite that
//     reorders rows relative to the flat oracle fails even when the bag matches.
// ============================================================================

mod composition_sweep {
    use super::*;

    /// Ordered SEQUENCE parity, flat vs tree, for an ORDER BY query. The harness's
    /// `solutions_bag_eq` compares MULTISETS and is blind to row order; this asserts
    /// the tree's rewrites emit the flat oracle's rows in the SAME position-for-
    /// position sequence (unbound cells kept in place, so a mis-slotted NULL also
    /// fails). Compares only when BOTH translate (a divergence in Ok-ness is the
    /// `diff` helpers' job); both being 501 leaves nothing to compare.
    fn assert_ordered_ft(query: &str) {
        let conn = sqlite::load(P_SQL).expect("fixture loads");
        let schema = sqlite::introspect_all(&conn).expect("introspect");
        let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
        let q = parse(query);
        if let (Ok(fp), Ok(tp)) = (flat(&maps, &q, &schema), tree(&maps, &q, &schema)) {
            let fr = exec::select(&fp, &conn).expect("flat select");
            let tr = exec::select(&tp, &conn).expect("tree select");
            assert_eq!(
                (&fr.vars, &fr.rows),
                (&tr.vars, &tr.rows),
                "flat vs tree ORDER BY sequence divergence on `{query}`"
            );
        }
    }

    /// Drive one WHERE `body` (projecting the single var `proj`, e.g. "?v") through
    /// the whole modifier matrix. Every variant is asserted =_bag; ORDER BY also gets
    /// a flat-vs-tree sequence check.
    fn sweep(body: &str, proj: &str) {
        let sel = |m: &str| format!("{PFX} SELECT {proj} WHERE {{ {body} }} {m}");
        let seld = |m: &str| format!("{PFX} SELECT DISTINCT {proj} WHERE {{ {body} }} {m}");
        let ob = format!("ORDER BY {proj}");

        // deterministic row SET → flat-vs-tree AND vs spareval.
        diff_p(&sel(""));
        diff_p(&seld(""));
        diff_p(&sel(&ob));
        // ORDER BY row SEQUENCE (bag-blind) → flat vs tree.
        assert_ordered_ft(&sel(&ob));

        // slice variants → flat-vs-tree =_bag only (impl-defined tie-break).
        diff_p_bag(&sel("LIMIT 2"));
        diff_p_bag(&sel("OFFSET 1"));
        diff_p_bag(&sel("LIMIT 2 OFFSET 1"));
        diff_p_bag(&sel(&format!("{ob} LIMIT 2")));
        diff_p_bag(&seld(&format!("{ob} LIMIT 2")));
    }

    /// `try_fold_constant_union` (normalize.rs) — a Union whose arms are ALL bare
    /// constants folds to one `Values` leaf. Composed with every modifier.
    #[test]
    fn fold_constant_union_x_modifiers() {
        sweep(
            "{ BIND(\"mp1\" AS ?v) } UNION { BIND(\"mp2\" AS ?v) } UNION { BIND(\"mp3\" AS ?v) }",
            "?v",
        );
    }

    /// `try_partial_fold_constant_union` — a DATA arm plus 2+ contiguous constant
    /// arms: the constants fold to one `Values` arm kept AT their own position (the
    /// exact rule whose partial-fold + bare-LIMIT reorder bug this sweep exists for).
    /// Data arm FIRST (left-associative parse) so the partial fold — not the full
    /// `try_fold_constant_union` — is what fires.
    #[test]
    fn partial_fold_constant_union_x_modifiers() {
        sweep(
            "{ ?p ex:name ?v } UNION { BIND(\"Xx\" AS ?v) } UNION { BIND(\"Yy\" AS ?v) }",
            "?v",
        );
    }

    /// `try_slice_over_union` — a Slice over `Union[Values.., data-arm]` drops/
    /// truncates leading known-cardinality arms. Only the LIMIT/OFFSET variants of
    /// the matrix actually build the Slice that triggers this rule; the rest exercise
    /// the underlying plain Union.
    #[test]
    fn slice_over_union_x_modifiers() {
        sweep(
            "{ VALUES ?v { \"va\" \"vb\" \"vc\" } } UNION { ?d ex:label ?v }",
            "?v",
        );
    }

    /// `normalize_slice` — a Slice directly over a `Values` leaf truncates the row
    /// list in place. Triggered by the LIMIT/OFFSET variants.
    #[test]
    fn slice_over_values_x_modifiers() {
        sweep("VALUES ?v { \"sa\" \"sb\" \"sc\" \"sd\" }", "?v");
    }

    /// `normalize_distinct` — a Distinct over a `Values` leaf dedups in place. The
    /// duplicate rows also exercise the LIMIT-over-duplicates tie-break path.
    #[test]
    fn distinct_over_values_x_modifiers() {
        sweep("VALUES ?v { \"da\" \"da\" \"db\" \"dc\" \"dc\" }", "?v");
    }

    /// `normalize_union` identity pruning — an unmapped-predicate arm (`ex:nope`)
    /// becomes `Empty` and is pruned, keeping the surviving arms, under every
    /// modifier.
    #[test]
    fn union_empty_arm_prune_x_modifiers() {
        sweep(
            "{ ?p ex:name ?v } UNION { ?x ex:nope ?v } UNION { ?d ex:label ?v }",
            "?v",
        );
    }

    /// `normalize_left_join` (right operand preserved) — a plain single-scan
    /// OPTIONAL, the common LeftJoin shape, under every modifier.
    #[test]
    fn left_join_plain_optional_x_modifiers() {
        sweep("?p ex:name ?v OPTIONAL { ?p ex:dept ?dd }", "?v");
    }

    /// `normalize_left_join` (LEFT-union distribution) — `(A ∪ B) ⟕ C` distributes
    /// to `(A⟕C) ∪ (B⟕C)`. A self-union LEFT deliberately doubles the bag so a
    /// multiplicity-losing distribution would be caught.
    #[test]
    fn left_join_over_left_union_x_modifiers() {
        sweep(
            "{ { ?p ex:name ?v } UNION { ?p ex:name ?v } } OPTIONAL { ?p ex:dept ?dd }",
            "?v",
        );
    }

    /// `normalize_inner_join` (join-over-union distribution) — `A ⋈ (B ∪ C)`
    /// distributes to `(A⋈B) ∪ (A⋈C)`, under every modifier.
    #[test]
    fn inner_join_over_union_x_modifiers() {
        sweep(
            "?p ex:name ?nm . { ?p ex:name ?v } UNION { ?p ex:dept ?v }",
            "?v",
        );
    }

    /// `normalize_filter` (filter-over-union distribution) — `FILTER(σ)(B ∪ C)`
    /// clones the symbolic condition into each arm, under every modifier.
    #[test]
    fn filter_over_union_x_modifiers() {
        sweep(
            "{ { ?p ex:name ?v } UNION { ?d ex:label ?v } } FILTER(?v != \"Bob\")",
            "?v",
        );
    }
}

/// Item 1b (ADR-0023 optimizer-residue): GROUP BY over a MULTI-branch (UNION)
/// OPTIONAL right side, then aggregated — `?e ex:name ?n OPTIONAL { R1 ∪ R2 }`
/// grouped by `?e` with `COUNT(?v)`. An unmerged flat-path-era commit (git
/// 2015846) closed two 501s for this shape on the OLD flat executor that ADR-0024
/// has since refactored away; verified empirically here that today's tree-IR path
/// already handles it CORRECTLY (=_bag the independent spareval oracle), while the
/// FLAT oracle honestly DEFERS (its inherent agg-over-UNION 501 — the same
/// synthetic-unbound limitation `count_over_bind_only_union_*` documents). The tree
/// is thus a strict capability SUPERSET here, not a wrong answer — so commit
/// 2015846 is OBSOLETE (do NOT merge that stale pre-refactor code). Gated vs
/// spareval directly (not `diff`, whose "both must 501" rule would misread
/// tree-exceeds-flat as a mismatch), mirroring `agg_union`.
///
/// emp1: dept d10 + tags {x,y} ⇒ COUNT(?v)=3; emp2: dept d10, no tag ⇒ 1;
/// emp3: dept d20 + tag z ⇒ 2.
#[test]
fn group_by_over_multibranch_optional_is_tree_superset_of_flat() {
    let q = format!(
        "{PFX} SELECT ?e (COUNT(?v) AS ?c) WHERE {{ ?e ex:name ?n \
         OPTIONAL {{ {{ ?e ex:dept ?v }} UNION {{ ?e ex:tag ?v }} }} }} GROUP BY ?e"
    );
    let conn = sqlite::load(STRESS_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(STRESS_R2RML).expect("R2RML parses");
    let parsed = parse(&q);
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "flat is expected to honestly 501 (its inherent agg-over-UNION limitation) \
         -- if this now succeeds, flat gained the capability and this test's premise \
         needs revisiting, not silently relaxing"
    );
    let tp = tree(&maps, &parsed, &schema)
        .expect("tree must handle GROUP BY over multi-branch OPTIONAL");
    assert_vs_spareval(STRESS_TTL, &q, &tp, &conn);
}

/// Item 3a (ADR-0023 optimizer-residue): `MINUS` whose LEFT operand is a
/// property-path pattern (`?s ex:reaches+ ?o MINUS { ?s ex:reaches ?o }`). The
/// FLAT path honestly DEFERS this shape ("MINUS over a property-path left side is
/// deferred → 501"); the tree-IR path computes it CORRECTLY — verified here =_bag
/// the independent spareval oracle. This is a pure capability DIFFERENCE (tree is
/// a strict superset of flat), NOT a wrong answer, so it is documented here rather
/// than "fixed": flat's 501 is asserted explicitly so a future flat-side
/// capability gain doesn't silently invalidate this test's premise. Gated vs
/// spareval directly (not `diff`, whose "both must 501" rule would misread
/// tree-exceeds-flat as a mismatch), mirroring `agg_union` / `count_over_bind_only_union`.
///
/// Transitive closure of {1→2,2→3,3→4,1→5} minus the direct hops leaves the
/// 3 strictly-transitive pairs (1,3), (2,4), (1,4).
#[test]
fn minus_over_path_left_operand_is_tree_superset_of_flat() {
    let q =
        format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:reaches+ ?o MINUS {{ ?s ex:reaches ?o }} }}");
    let conn = sqlite::load(PE_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(PE_R2RML).expect("R2RML parses");
    let parsed = parse(&q);
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "flat is expected to honestly 501 (property-path MINUS left side) -- if this \
         now succeeds, flat gained the capability and this test's premise needs \
         revisiting, not silently relaxing"
    );
    let tp = tree(&maps, &parsed, &schema).expect("tree must handle MINUS over a path left side");
    assert_vs_spareval(PE_TTL, &q, &tp, &conn);
}

// ============================================================================
// ADR-0025 Tier-1 bug #1 (opts-nullability) — RED reproduction.
// An OPTIONAL-left-unbound variable (?x, unbound for Bob whose email is NULL)
// is re-joined via a DIFFERENT anchor (?q) in a later MANDATORY pattern.
// SPARQL §18.5: an unbound shared var is vacuously compatible, so Bob must MERGE
// with every ?q-email row (ADD rows). If sf treats the unbound ?x as SQL NULL and
// equi-joins, Bob is dropped. Expected spareval: 4 rows
// {(Ann,ann@x),(Zed,zed@x),(Bob,ann@x),(Bob,zed@x)}. Buggy sf: 2 rows.
#[test]
fn adr0025_tier1_opts_nullability_cross_anchor_rejoin() {
    let q = format!(
        "{PFX} SELECT ?name ?x WHERE {{ \
           ?p ex:name ?name . \
           OPTIONAL {{ ?p ex:email ?x }} \
           ?q ex:email ?x . \
         }}"
    );
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let parsed = parse(&q);
    let tp = tree(&maps, &parsed, &schema).expect("tree translates");
    assert_vs_spareval(P_TTL, &q, &tp, &conn);
}

// ADR-0025 Tier-1 (opts-nullability) — FLAT-path mirror of the tree repro above.
// The flat `merge` (unfold.rs) had the identical plain-equality drop; same expected
// 4 rows vs spareval. Asserts the flat plan directly (flat is the differential oracle).
#[test]
fn adr0025_tier1_opts_nullability_cross_anchor_rejoin_flat() {
    let q = format!(
        "{PFX} SELECT ?name ?x WHERE {{ \
           ?p ex:name ?name . \
           OPTIONAL {{ ?p ex:email ?x }} \
           ?q ex:email ?x . \
         }}"
    );
    let conn = sqlite::load(P_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(P_R2RML).expect("R2RML parses");
    let parsed = parse(&q);
    let fp = flat(&maps, &parsed, &schema).expect("flat translates");
    assert_vs_spareval(P_TTL, &q, &fp, &conn);
}

// ============================================================================
// ADR-0025 Tier-1 (opts-nullability) — regression set for the adversarial-review
// findings on the compatible-merge fix (insert_or_unify / merge R1 null-safe + R2).
// ============================================================================

/// Bug A (adversarial review): the OPTIONAL-bearing group as the RIGHT join operand.
/// Flat `merge` built its nullable-alias set from `left` only, missing `right`'s OPTIONAL,
/// so it fell back to plain equality and dropped the unbound row (order-dependent). Fixed
/// by unioning both branches' `nullable_aliases`. Assert BOTH paths vs spareval.
#[test]
fn adr0025_tier1_optional_as_right_join_operand() {
    let q = format!(
        "{PFX} SELECT ?name ?qname ?x WHERE {{ \
           {{ ?q ex:name ?qname . ?q ex:email ?x }} \
           {{ ?p ex:name ?name . OPTIONAL {{ ?p ex:email ?x }} }} }}"
    );
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
    assert_vs_spareval(
        P_TTL,
        &q,
        &flat(&maps, &parsed, &schema).expect("flat"),
        &conn,
    );
}

/// Bug B (adversarial review): `SELECT DISTINCT` over the merged shared var. The single-
/// nullable case must NOT introduce a non-injective COALESCE (SQL DISTINCT dedups raw
/// columns before term reconstruction), so the merged value uses the mandatory side's raw
/// column and DISTINCT collapses correctly. spareval = {ann@x, zed@x}.
#[test]
fn adr0025_tier1_distinct_over_merged_var_single_nullable() {
    let q = format!(
        "{PFX} SELECT DISTINCT ?x WHERE {{ ?p ex:name ?name . \
           OPTIONAL {{ ?p ex:email ?x }} ?q ex:email ?x . }}"
    );
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
    assert_vs_spareval(
        P_TTL,
        &q,
        &flat(&maps, &parsed, &schema).expect("flat"),
        &conn,
    );
}

/// Both-sides-nullable: the shared var is bound by TWO OPTIONALs, so the merged value would
/// need a non-injective COALESCE (unsafe under DISTINCT/dedup — adversarial-review Bug B
/// residual). Sound 501 on BOTH paths (ADR-0007: a 501 beats a shape that is wrong under
/// DISTINCT). Exactly-one-nullable (the common opts-nullability bug) stays correct above.
#[test]
fn adr0025_tier1_both_sides_nullable_join_sound_501() {
    let q = format!(
        "{PFX} SELECT ?name ?x WHERE {{ ?p ex:name ?name . \
           {{ OPTIONAL {{ ?p ex:email ?x }} }} \
           {{ ?q ex:name ?qn . OPTIONAL {{ ?q ex:email ?x }} }} }}"
    );
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert!(
        matches!(tree(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "both-nullable correlated join must sound-501 (tree)"
    );
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "both-nullable correlated join must sound-501 (flat)"
    );
}

// ============================================================================
// ADR-0025 Tier-1 bug #2: a sub-SELECT with a SLICE (LIMIT/OFFSET) as a join operand used
// to SILENTLY DROP the slice (tree `lower_as_subplan` derived table; flat Join/Union/Minus
// dropping the operand's limit) → the join saw the full unsliced set (wrong answer). The
// slice can't be emitted soundly (SQL collation ≠ SPARQL ORDER BY), so the fix is a sound
// 501 (ADR-0007), matching the OPTIONAL/`left_join_over_subplan` boundary.
// ============================================================================

/// Slice as a join input must sound-501 on BOTH paths (was: tree 3 rows vs oracle 1).
#[test]
fn adr0025_tier1_subplan_slice_as_join_input_sound_501() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    for q in [
        format!(
            "{PFX} SELECT ?name WHERE {{ \
               {{ SELECT ?p WHERE {{ ?p ex:name ?pn }} ORDER BY ?pn LIMIT 1 }} \
               ?p ex:name ?name . }}"
        ),
        format!(
            "{PFX} SELECT ?name WHERE {{ \
               {{ SELECT ?p WHERE {{ ?p ex:name ?pn }} LIMIT 2 }} ?p ex:name ?name . }}"
        ),
        format!(
            "{PFX} SELECT ?name WHERE {{ \
               {{ SELECT ?p WHERE {{ ?p ex:name ?pn }} OFFSET 1 }} ?p ex:name ?name . }}"
        ),
    ] {
        let parsed = parse(&q);
        assert!(
            matches!(tree(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
            "SubPlan slice as join input must sound-501 (tree): {q}"
        );
        assert!(
            matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
            "SubPlan slice as join input must sound-501 (flat): {q}"
        );
    }
}

/// Companion: ORDER BY with NO slice as a join input is a no-op for a bag-valued operand,
/// so it is safely dropped and still answers correctly (proves no over-501). Both paths.
#[test]
fn adr0025_tier1_subplan_orderby_only_as_join_input_ok() {
    let q = format!(
        "{PFX} SELECT ?name WHERE {{ \
           {{ SELECT ?p WHERE {{ ?p ex:name ?pn }} ORDER BY ?pn }} ?p ex:name ?name . }}"
    );
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
    assert_vs_spareval(
        P_TTL,
        &q,
        &flat(&maps, &parsed, &schema).expect("flat"),
        &conn,
    );
}

// ============================================================================
// ADR-0025 Tier-2 gap 3: COUNT(DISTINCT *) — count DISTINCT whole solutions. Was 501 on
// both paths; now deduped in rust_agg. Ontop itself has a live bug here (drops DISTINCT),
// so sf is ahead. Gated vs the spareval oracle.
// ============================================================================

/// Assert the TREE path (production) matches spareval for COUNT(DISTINCT *). The flat path
/// cannot bind an aggregate result var over its Rust-group path, so it soundly 501s every
/// COUNT(DISTINCT *) (the documented tree-exceeds-flat limitation) — only the tree is checked.
fn count_distinct_star_case(q: &str) {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(q);
    assert_vs_spareval(
        P_TTL,
        q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
    // Flat soundly 501s (agg result var over UNION/rust-group is unbindable — tree exceeds flat).
    assert!(
        matches!(flat(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "flat should sound-501 COUNT(DISTINCT *): {q}"
    );
}

#[test] // 2 identical UNION arms over 3 persons => 6 solutions; DISTINCT * => 3.
fn adr0025_tier2_count_distinct_star_over_union() {
    count_distinct_star_case(&format!(
        "{PFX} SELECT (COUNT(DISTINCT *) AS ?c) WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:name ?n }} }}"
    ));
}

#[test] // single-branch, all solutions already distinct => COUNT(DISTINCT *) == COUNT(*) == 3.
fn adr0025_tier2_count_distinct_star_single_branch() {
    count_distinct_star_case(&format!(
        "{PFX} SELECT (COUNT(DISTINCT *) AS ?c) WHERE {{ ?p ex:name ?n }}"
    ));
}

#[test] // control: COUNT(*) must NOT dedup => 6 over the duplicated union.
fn adr0025_tier2_count_star_not_deduped_control() {
    count_distinct_star_case(&format!(
        "{PFX} SELECT (COUNT(*) AS ?c) WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:name ?n }} }}"
    ));
}

#[test] // COUNT(DISTINCT *) with GROUP BY: per-group distinct whole-solution count.
fn adr0025_tier2_count_distinct_star_grouped() {
    count_distinct_star_case(&format!(
        "{PFX} SELECT ?d (COUNT(DISTINCT *) AS ?c) WHERE {{ {{ ?p ex:dept ?d }} UNION {{ ?p ex:dept ?d }} }} GROUP BY ?d"
    ));
}
#[test] // edge: unbound var in some solutions (OPTIONAL) — dedup must treat absent != bound
fn adr0025_tier2_count_distinct_star_with_unbound_var() {
    // arm1 binds ?e for Ann/Zed only (email); union with itself => dups; DISTINCT * over (?p,?e)
    // where Bob has ?e unbound. spareval counts distinct (?p,?e) incl the unbound-e Bob row.
    let q = format!("{PFX} SELECT (COUNT(DISTINCT *) AS ?c) WHERE {{ {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} }} UNION {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} }} }}");
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}
#[test] // edge: COUNT(DISTINCT *) alongside a non-distinct agg in the same group
fn adr0025_tier2_count_distinct_star_mixed_with_count_star() {
    let q = format!("{PFX} SELECT (COUNT(DISTINCT *) AS ?cd) (COUNT(*) AS ?c) WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:name ?n }} }}");
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}

// ============================================================================
// ADR-0025 Tier-2 gap 2: a multi-branch (UNION) SubPlan as a join input. Was 501; now the
// tree pools the arms into ONE UNION-ALL/UNION derived table when every projected var
// reconstructs identically across all arms (else sound 501). Per the Ontop dossier this is
// the primitive that also unblocks gaps 4/5.
// ============================================================================

#[test] // DISTINCT sub-SELECT over a UNION, joined with a triple => distinct ?p then join.
fn adr0025_tier2_gap2_multibranch_distinct_subplan_join() {
    let q = format!(
        "{PFX} SELECT ?p ?label WHERE {{ \
           {{ SELECT DISTINCT ?p WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:email ?e }} }} }} \
           ?p ex:name ?label . }}"
    );
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree pools it"),
        &conn,
    );
}

#[test] // cross-arm INCOMPATIBLE: ?x is a literal (name) in one arm, an IRI (dept) in the
        // other => must sound-501 (not silently pool a type-mismatched UNION).
fn adr0025_tier2_gap2_incompatible_arms_sound_501() {
    let q = format!(
        "{PFX} SELECT ?p ?x WHERE {{ \
           {{ SELECT DISTINCT ?p ?x WHERE {{ {{ ?p ex:name ?x }} UNION {{ ?p ex:dept ?x }} }} }} \
           ?p ex:name ?nm . }}"
    );
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert!(
        matches!(tree(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "type-incompatible cross-arm SubPlan must sound-501"
    );
}

#[test] // non-distinct multi-branch SubPlan (ORDER BY-only over UNION) => UNION ALL bag, joined.
        // Boundary: a non-DISTINCT OrderBy SubPlan over a UNION whose arms bind DIFFERENT internal
        // vars (?n vs ?e). Without the DISTINCT narrowing, those arm-local vars stay in the pooled
        // projection and reconstruct differently across arms (one bound, one absent), needing UNION
        // column-padding the pooling does not emit — so it soundly 501s (never a wrong answer). The
        // DISTINCT-over-shared-var shape above is the gap-2 win; padding is future work.
fn adr0025_tier2_gap2_nondistinct_disjoint_arm_vars_sound_501() {
    let q = format!(
        "{PFX} SELECT ?p WHERE {{ \
           {{ SELECT ?p WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:email ?e }} }} ORDER BY ?p }} \
           ?p ex:name ?label . }}"
    );
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let parsed = parse(&q);
    assert!(
        matches!(tree(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "non-distinct arms binding disjoint internal vars must sound-501 (needs UNION padding)"
    );
}

// ADR-0025 gap-2 injectivity gate (adversarial review): a multi-branch DISTINCT SubPlan
// whose projected var is a NON-INJECTIVE IRI template (distinct raw tuples → the same term)
// must stay a sound 501 — the pooled UNION dedups raw columns, which would NOT match SPARQL
// DISTINCT on the reconstructed term. Was 501 pre-gap-2; the pooling must not regress it.
// (The analogous SINGLE-branch `SELECT DISTINCT ?s` over a non-injective template is a
// separate, PRE-EXISTING soundness gap in DISTINCT emission — tracked as its own item.)
const NIT_SQL: &str = r#"
CREATE TABLE pair (a TEXT NOT NULL, b TEXT NOT NULL, val TEXT NOT NULL);
INSERT INTO pair VALUES ('1','23','X');
INSERT INTO pair VALUES ('12','3','Y');
"#;
const NIT_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P> rr:logicalTable [ rr:tableName "pair" ] ;
    rr:subjectMap [ rr:template "http://ex/{a}{b}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:val ; rr:objectMap [ rr:column "val" ] ] .
"#;
#[test]
fn adr0025_tier2_gap2_multibranch_distinct_noninjective_sound_501() {
    let conn = sqlite::load(NIT_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(NIT_R2RML).unwrap();
    let q = format!(
        "{PFX} SELECT ?s WHERE {{ \
           {{ SELECT DISTINCT ?s WHERE {{ {{ ?s ex:val ?v }} UNION {{ ?s ex:val ?w }} }} }} \
           ?s ex:val ?any . }}"
    );
    let parsed = parse(&q);
    assert!(
        matches!(tree(&maps, &parsed, &schema), Err(Error::Unsupported(_))),
        "multi-branch DISTINCT over a non-injective template must sound-501 (not pool)"
    );
}

// ADR-0025 C.3: SELECT DISTINCT over a NON-INJECTIVE IRI template must sound-501 (the query
// translates, but emission refuses: SQL DISTINCT dedups raw cols, which would NOT match
// SPARQL DISTINCT on the reconstructed term). Pre-existing bug found by the gap-2 refuter.
const C3_SQL: &str = "CREATE TABLE pair (a TEXT NOT NULL, b TEXT NOT NULL, val TEXT NOT NULL);\nINSERT INTO pair VALUES ('1','23','X');\nINSERT INTO pair VALUES ('12','3','Y');";
const C3_R2RML: &str = "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n@prefix ex: <http://ex/> .\n<#P> rr:logicalTable [ rr:tableName \"pair\" ] ; rr:subjectMap [ rr:template \"http://ex/{a}{b}\" ] ; rr:predicateObjectMap [ rr:predicate ex:val ; rr:objectMap [ rr:column \"val\" ] ] .";
#[test]
fn adr0025_c3_distinct_over_noninjective_template_sound_501() {
    let conn = sqlite::load(C3_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(C3_R2RML).unwrap();
    let q = format!("{PFX} SELECT DISTINCT ?s WHERE {{ ?s ex:val ?v }}");
    let parsed = parse(&q);
    // Both paths: translation succeeds, EMISSION soundly 501s (no silent duplicate rows).
    let tp = tree(&maps, &parsed, &schema).expect("translates");
    assert!(
        tp.emitted().is_err(),
        "non-injective DISTINCT must sound-501 at emit (tree)"
    );
    let fp = flat(&maps, &parsed, &schema).expect("translates");
    assert!(
        fp.emitted().is_err(),
        "non-injective DISTINCT must sound-501 at emit (flat)"
    );
}
#[test] // control: an INJECTIVE template DISTINCT (fixture P's http://ex/person/{id}) still works.
fn adr0025_c3_distinct_over_injective_template_ok() {
    let q = format!("{PFX} SELECT DISTINCT ?p WHERE {{ ?p ex:name ?n }}");
    diff_p(&q);
}

// ADR-0025 Tier-2 gap 5: a post-GROUP-BY arithmetic EXPRESSION over a UNION aggregate (e.g.
// COUNT(?p) * 2). Forced onto the rust_group path and computed via eval_expr. Was 501.
// Tree-only (flat 501s aggregate-over-UNION).
#[test]
fn adr0025_tier2_gap5_post_group_expr_over_union_aggregate() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    for q in [
        format!("{PFX} SELECT ?d ((COUNT(?p) * 2) AS ?c) WHERE {{ {{ ?p ex:dept ?d }} UNION {{ ?p ex:dept ?d }} }} GROUP BY ?d"),
        format!("{PFX} SELECT ((COUNT(?p) + 1) AS ?c) WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:email ?e }} }}"),
    ] {
        let parsed = parse(&q);
        assert_vs_spareval(P_TTL, &q, &tree(&maps, &parsed, &schema).expect("tree computes it"), &conn);
    }
}
#[test] // single-branch post-group arithmetic (forced to rust_group) still matches spareval.
fn adr0025_tier2_gap5_single_branch_post_group_arith() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let q = format!(
        "{PFX} SELECT ?d (((COUNT(?p) * 2) + 1) AS ?c) WHERE {{ ?p ex:dept ?d }} GROUP BY ?d"
    );
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}
#[test] // boundary: a NON-arithmetic post-group expr (STR) over a UNION aggregate stays sound-501.
fn adr0025_tier2_gap5_non_arith_post_group_stays_501() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let q = format!("{PFX} SELECT ?d (STR(COUNT(?p)) AS ?c) WHERE {{ {{ ?p ex:dept ?d }} UNION {{ ?p ex:dept ?d }} }} GROUP BY ?d");
    let parsed = parse(&q);
    assert!(
        matches!(
            tree(&maps, &parsed, &schema).and_then(|p| p.emitted().map(|_| ())),
            Err(Error::Unsupported(_))
        ),
        "non-arithmetic post-group expr over UNION aggregate must sound-501"
    );
}
#[test] // gap-5 boundary (adversarial review): arithmetic over a DECIMAL aggregate (AVG/SUM)
        // or a DIVISION must sound-501 — eval_expr's f64 arithmetic emits int/double, never
        // xsd:decimal, so it would mistype/round the result. Integer COUNT arithmetic is safe.
fn adr0025_tier2_gap5_decimal_or_division_stays_501() {
    let conn = sqlite::load(AGG_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(AGG_R2RML).unwrap();
    for q in [
        format!("{PFX} SELECT ((AVG(?v) * 2) AS ?c) WHERE {{ {{ ?x ex:p1 ?v }} UNION {{ ?x ex:p2 ?v }} }}"),
        format!("{PFX} SELECT ((SUM(?v) / COUNT(?v)) AS ?c) WHERE {{ {{ ?x ex:p1 ?v }} UNION {{ ?x ex:p2 ?v }} }}"),
    ] {
        let parsed = parse(&q);
        assert!(matches!(tree(&maps,&parsed,&schema).and_then(|p| p.emitted().map(|_|())), Err(Error::Unsupported(_))),
            "decimal/division post-group arithmetic must sound-501: {q}");
    }
}

// ADR-0025 Tier-2 gap 4: GROUP BY over a property-path CLOSURE. Was 501 (path vars live in
// the recursive CTE, not base columns); now routed to the Rust group path, which runs the
// path branch's own SQL and groups the solutions by variable name. Gated vs spareval.
fn gap4_case(sql: &str, r2rml: &str, ttl: &str, q: &str) {
    let conn = sqlite::load(sql).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(r2rml).unwrap();
    let parsed = parse(q);
    assert_vs_spareval(
        ttl,
        q,
        &tree(&maps, &parsed, &schema).expect("tree groups the path"),
        &conn,
    );
}
#[test]
fn adr0025_tier2_gap4_group_by_over_path_closure() {
    // group by reachable target, count sources
    gap4_case(
        PE_SQL,
        PE_R2RML,
        PE_TTL,
        &format!("{PFX} SELECT ?o (COUNT(?s) AS ?c) WHERE {{ ?s ex:reaches+ ?o }} GROUP BY ?o"),
    );
    // group by source, count reachable targets
    gap4_case(
        PE_SQL,
        PE_R2RML,
        PE_TTL,
        &format!("{PFX} SELECT ?s (COUNT(?o) AS ?c) WHERE {{ ?s ex:reaches+ ?o }} GROUP BY ?s"),
    );
    // implicit group (one COUNT over the whole closure)
    gap4_case(
        PE_SQL,
        PE_R2RML,
        PE_TTL,
        &format!("{PFX} SELECT (COUNT(?o) AS ?c) WHERE {{ ?s ex:reaches+ ?o }}"),
    );
    // reflexive-transitive closure (P*) grouped
    gap4_case(
        PE_SQL,
        PE_R2RML,
        PE_TTL,
        &format!("{PFX} SELECT ?o (COUNT(?s) AS ?c) WHERE {{ ?s ex:reaches* ?o }} GROUP BY ?o"),
    );
    // cyclic graph, grouped — must terminate + count each reachable pair once
    gap4_case(
        PC_SQL,
        PE_R2RML,
        PC_TTL,
        &format!("{PFX} SELECT ?o (COUNT(?s) AS ?c) WHERE {{ ?s ex:reaches+ ?o }} GROUP BY ?o"),
    );
}
#[test] // gap-4 edges: COUNT(DISTINCT) over a path, and a path joined with a bound pattern, grouped.
fn adr0025_tier2_gap4_count_distinct_over_path() {
    // COUNT(DISTINCT) over a path closure, grouped. (A path JOINED with another pattern is a
    // SEPARATE pre-existing 501 -- "joining a path closure with another pattern" -- orthogonal
    // to gap 4's GROUP-BY-over-path, so not exercised here.)
    gap4_case(
        PE_SQL,
        PE_R2RML,
        PE_TTL,
        &format!(
            "{PFX} SELECT ?o (COUNT(DISTINCT ?s) AS ?c) WHERE {{ ?s ex:reaches+ ?o }} GROUP BY ?o"
        ),
    );
}

// ADR-0025 C.4: AVG over a group whose operand var is UNBOUND in every row (rows>0) must be
// UNBOUND, not 0. A genuinely EMPTY group (0 rows) stays 0. (SUM stays 0 in both cases;
// MIN/MAX already return unbound uniformly.) Found by the gap-5 adversarial review.
#[test] // tree-only: aggregate-over-UNION is a documented flat 501.
fn adr0025_c4_avg_unbound_operand_is_unbound() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let q = format!("{PFX} SELECT (AVG(?missing) AS ?c) WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:email ?e }} }}");
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}
#[test] // control (rust_group path, the one my fix touches): AVG over a genuinely EMPTY group
        // — a UNION whose arms match nothing, 0 rows — stays "0"^^xsd:integer. Confirms the
        // fix's `rows.is_empty()` branch. Tree-only (aggregate-over-UNION is a flat 501).
fn adr0025_c4_avg_empty_group_stays_zero() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let q = format!(
        "{PFX} SELECT (AVG(?x) AS ?c) WHERE {{ {{ ?p ex:none1 ?x }} UNION {{ ?p ex:none2 ?x }} }}"
    );
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}

// ADR-0025 Tier-2 gap 1: a property-path CLOSURE inside FILTER EXISTS / NOT EXISTS / MINUS,
// lowered to a correlated `PathExists` (recursive-CTE derived table). P+ and length-1
// composites (p/q, ^p, p|q) work; the REFLEXIVE kinds (P*, P?) sound-501 (fallible prelude).
fn gap1_case(q: &str) {
    let conn = sqlite::load(PE_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(PE_R2RML).unwrap();
    let parsed = parse(q);
    assert_vs_spareval(
        PE_TTL,
        q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}
#[test]
fn adr0025_tier2_gap1_path_in_exists_notexists_minus() {
    // FILTER EXISTS: targets that themselves have an outgoing P+ path (filters out sinks)
    gap1_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s FILTER EXISTS {{ ?s ex:reaches+ ?y }} }}"
    ));
    // FILTER NOT EXISTS: targets with NO outgoing path (the sinks)
    gap1_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s FILTER NOT EXISTS {{ ?s ex:reaches+ ?y }} }}"
    ));
    // MINUS a path (anti-join)
    gap1_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s MINUS {{ ?s ex:reaches+ ?y }} }}"
    ));
    // a length-1 SEQUENCE composite path inside EXISTS (2-hop)
    gap1_case(&format!("{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s FILTER EXISTS {{ ?s ex:reaches/ex:reaches ?y }} }}"));
    // both endpoints outer-bound: correlate on sf_s AND sf_o
    gap1_case(&format!(
        "{PFX} SELECT ?a ?b WHERE {{ ?a ex:reaches ?b FILTER EXISTS {{ ?a ex:reaches+ ?b }} }}"
    ));
}
const MP_SQL: &str = r#"
CREATE TABLE edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO edge VALUES (1, 2);
CREATE TABLE tagged (id INTEGER PRIMARY KEY, tag TEXT NOT NULL);
INSERT INTO tagged VALUES (2, 'b');
INSERT INTO tagged VALUES (9, 'isolated');
"#;
const MP_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge> rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
<#Tag> rr:logicalTable [ rr:tableName "tagged" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:tag ; rr:objectMap [ rr:column "tag" ] ] .
"#;

// ADR-0025 gap 1 (reflexive) SOUNDNESS BOUNDARY: reflexive P*/P? inside EXISTS over a
// MULTI-predicate graph must sound-501 (never wrong-answer). sf scopes the reflexive (x,x)
// ZeroLengthPath to the hop predicate's node set, but SPARQL §18.4 covers ALL graph nodes;
// over a multi-predicate mapping these diverge, so reflexive_sql already 501s the graph-node
// enumeration (ADR-0007). The Result threading added for gap-1 propagates that 501 through
// render_cond into the EXISTS instead of a crash-or-wrong-answer. Node 9 is tagged but in NO
// reaches edge -- the exact case where a naive reflexive would wrongly make EXISTS true.
#[test]
fn adr0025_tier2_gap1_reflexive_in_exists_multipred_sound_501() {
    let conn = sqlite::load(MP_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(MP_R2RML).unwrap();
    let q =
        format!("{PFX} SELECT ?s WHERE {{ ?s ex:tag ?t FILTER EXISTS {{ ?s ex:reaches* ?y }} }}");
    let parsed = parse(&q);
    assert!(
        tree(&maps, &parsed, &schema).is_err(),
        "multi-predicate reflexive-in-EXISTS must sound-501, not return a (possibly wrong) answer"
    );
}

#[test] // reflexive P* / P? inside EXISTS/NOT EXISTS/MINUS now COMPUTE (was sound-501): the
        // render_cond chain threads the live catalog + returns Result, so the reflexive
        // prelude's fallible reflexive_sql resolves + propagates at emit. P* is reflexive, so
        // `EXISTS { ?s :p* ?y }` is always true (?s reaches itself) — spareval agrees.
fn adr0025_tier2_gap1_reflexive_path_in_exists_now_computes() {
    for q in [
        format!("{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s FILTER EXISTS {{ ?s ex:reaches* ?y }} }}"),
        format!("{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s FILTER NOT EXISTS {{ ?s ex:reaches* ?y }} }}"),
        format!("{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s FILTER EXISTS {{ ?s ex:reaches? ?y }} }}"),
        format!("{PFX} SELECT ?s WHERE {{ ?x ex:reaches ?s MINUS {{ ?s ex:reaches* ?y }} }}"),
    ] {
        gap1_case(&q);
    }
}

// ADR-0025 Tier-2 gap 1 (composite coverage): the gap-1 commit's docstring claimed p/q, ^p,
// and p|q composites are ALL covered inside EXISTS/NOT EXISTS/MINUS, but only the p/q
// SEQUENCE composite (`adr0025_tier2_gap1_path_in_exists_notexists_minus`, the 2-hop case)
// was actually exercised. `path.rs::compile_path`'s `Reverse` and `Alternative` arms build a
// `HopExpr` the SAME way `Sequence` does (any composite compiles to `PathKind::One` and rides
// the identical `bridge_branch` -> `IqNode::Path` -> `lower_iq_exists`'s `SqlCond::PathExists`
// machinery), so this closes the untested ^p / p|q cases directly against spareval.
#[test]
fn adr0025_tier2_gap1_inverse_path_in_exists_notexists_minus() {
    // FILTER EXISTS: subjects with an outgoing edge that ALSO have an INCOMING edge (via the
    // inverse path `^ex:reaches`, i.e. some other node reaches them). Node 1 is source-only
    // (no incoming edge) -> excluded; nodes 2 and 3 both have an incoming edge -> included.
    gap1_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?s ex:reaches ?o FILTER EXISTS {{ ?s ^ex:reaches ?y }} }}"
    ));
    // FILTER NOT EXISTS: the complement — only node 1 passes, once per its two outgoing
    // edges (row multiplicity preserved: two rows, both s=1).
    gap1_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?s ex:reaches ?o FILTER NOT EXISTS {{ ?s ^ex:reaches ?y }} }}"
    ));
    // MINUS: the same anti-join via the inverse path.
    gap1_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?s ex:reaches ?o MINUS {{ ?s ^ex:reaches ?y }} }}"
    ));
}

/// `gap1_case`'s twin over the two-predicate PA fixture (`ex:p`/`ex:q`), needed for the
/// ALTERNATIVE composite `p|q` (the PE fixture is single-predicate, so `p|q` there would
/// degenerate to just `p`).
fn gap1_pa_case(q: &str) {
    let conn = sqlite::load(PA_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(PA_R2RML).unwrap();
    let parsed = parse(q);
    assert_vs_spareval(
        PA_TTL,
        q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}

#[test]
fn adr0025_tier2_gap1_alternative_path_in_exists_notexists_minus() {
    // PA fixture: 1-p->2-p->3, 2-q->20, 3-q->30.
    // FILTER EXISTS: targets of `p|q` that themselves have a FURTHER outgoing `p|q` edge —
    // 2 (has both an outgoing p and an outgoing q) and 3 (has an outgoing q) qualify; the
    // sinks 20 and 30 (no outgoing edges at all) do not.
    gap1_pa_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?a ex:p|ex:q ?s FILTER EXISTS {{ ?s ex:p|ex:q ?y }} }}"
    ));
    // FILTER NOT EXISTS: the complement — the sinks 20 and 30.
    gap1_pa_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?a ex:p|ex:q ?s FILTER NOT EXISTS {{ ?s ex:p|ex:q ?y }} }}"
    ));
    // MINUS: the same anti-join via the alternative path.
    gap1_pa_case(&format!(
        "{PFX} SELECT ?s WHERE {{ ?a ex:p|ex:q ?s MINUS {{ ?s ex:p|ex:q ?y }} }}"
    ));
}

// ADR-0025 gap 2: MIN/MAX over a group whose operand var is UNBOUND in every row (rows>0)
// — C.4 only regression-tested AVG for this `rust_group` shape (aggregate over a UNION where
// the operand var is never bound in either arm). `rust_agg`'s `Min`/`Max` arms already return
// UNBOUND unconditionally whenever no row supplies a bound value (`exec_core.rs`'s
// `AggKind::Min | AggKind::Max` never had a `rows.is_empty()` special case at all, unlike
// C.4's `AggKind::Avg` fix) — this locks that in against the independent spareval oracle.
#[test]
fn adr0025_gap2_min_max_unbound_operand_is_unbound() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    for agg in ["MIN", "MAX"] {
        let q = format!(
            "{PFX} SELECT ({agg}(?missing) AS ?c) WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:email ?e }} }}"
        );
        let parsed = parse(&q);
        assert_vs_spareval(
            P_TTL,
            &q,
            &tree(&maps, &parsed, &schema).expect("tree"),
            &conn,
        );
    }
}

// ADR-0025 C.5 (regression): SUM over a NON-empty group whose operand is unbound in EVERY
// row must be UNBOUND, not "0" (SPARQL §11 error-propagation; only COUNT filters). C.4 gave
// AVG this discrimination; C.5 extends it to SUM/MIN/MAX. Now spareval-gated (passes post-fix).
#[test]
fn adr0025_gap2_sum_unbound_operand_is_unbound() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let q = format!(
        "{PFX} SELECT (SUM(?missing) AS ?c) WHERE {{ {{ ?p ex:name ?n }} UNION {{ ?p ex:email ?e }} }}"
    );
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}

// ADR-0025 C.5 (regression): a MIXED group — some rows bind the aggregate operand, others
// leave it unbound — makes SUM/MIN/MAX/AVG UNBOUND for that whole group (SPARQL §11 error
// propagation; only COUNT filters). Ann/Zed have an unbound ?x row (email arm) so ALL four
// aggregates are unbound for them; Bob's group is all-bound so it computes (SUM=7 etc.).
// Now spareval-gated (passes post-fix); previously rust_agg wrongly skipped the unbound row.
#[test]
fn adr0025_gap2_mixed_bound_unbound_group_all_aggregates() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let q = format!(
        "{PFX} SELECT ?p (SUM(?x) AS ?s) (MIN(?x) AS ?mn) (MAX(?x) AS ?mx) (AVG(?x) AS ?av) WHERE {{
            {{ ?p ex:name ?n BIND(2 AS ?x) }}
            UNION {{ ?p ex:dept ?d BIND(5 AS ?x) }}
            UNION {{ ?p ex:email ?e }}
        }} GROUP BY ?p"
    );
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}

// ADR-0025 Tier-3: the =_bag-meaningful content of the cosmetic SQL-shape backlog is CLOSED
// (Wave C + Group C, commits d313f26..487b4fb / 45ae36c). This regression test locks in that
// each shape computes the CORRECT =_bag result on current main. What remains UNimplemented in
// Tier-3 is pure SQL-*signature*-shape (binding-lift wrapper-hoist: not =_bag, architecturally
// N/A — sf's IqNode::Union explodes into independent branches, no shared wrapper to hoist; and
// the test10/11 Slice·Distinct·Union fold: a correct-but-unoptimized boundary). Neither affects
// =_bag correctness — this test proves the results are already right regardless of SQL shape.
#[test]
fn adr0025_tier3_bag_content_closed() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    let cases = [
        // right-nested OPTIONAL — L OPT (R1 OPT R2) — Group C (was flat-501, now tree Ok)
        format!("{PFX} SELECT ?p ?e ?d WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e OPTIONAL {{ ?p ex:dept ?d }} }} }}"),
        // MergeLJs-right-nested — OPT chain — Group C
        format!("{PFX} SELECT * WHERE {{ ?p ex:name ?n OPTIONAL {{ ?p ex:email ?e }} OPTIONAL {{ ?p ex:dept ?d }} }}"),
        // Slice-over-Values (Wave 6 test1); Distinct-over-Values (test3); constant-Union fold (test14)
        format!("{PFX} SELECT ?x WHERE {{ VALUES ?x {{ 1 2 3 }} }} LIMIT 1"),
        format!("{PFX} SELECT DISTINCT ?x WHERE {{ VALUES ?x {{ 1 1 2 }} }}"),
        format!("{PFX} SELECT ?x WHERE {{ {{ BIND(1 AS ?x) }} UNION {{ BIND(2 AS ?x) }} }}"),
        // test10 shape WITHOUT the non-deterministic LIMIT — the DISTINCT-over-Union SET is correct
        // (the missing OPTIMIZATION is SQL-shape only; the =_bag result is already right).
        format!("{PFX} SELECT DISTINCT ?x WHERE {{ {{ VALUES ?x {{ 1 2 }} }} UNION {{ VALUES ?x {{ 2 3 }} }} }}"),
    ];
    for q in cases {
        let parsed = parse(&q);
        assert_vs_spareval(
            P_TTL,
            &q,
            &tree(&maps, &parsed, &schema).expect("tree computes it"),
            &conn,
        );
    }
}

// ADR-0025 Tier-3 (test10/11, Slice·Distinct·Union): the =_bag content composes correctly —
// a constant Union folds bottom-up (try_fold_constant_union in normalize_union) → Distinct
// dedups the Values → Slice truncates it, all verified against spareval. The SQL *shape*
// (sf emits N constant-row branches, ADR-0006, not Ontop's single VALUES clause) is a
// documented architectural difference with zero =_bag impact — locked here as the result set.
#[test]
fn adr0025_tier3_slice_distinct_union_folds_bag_correct() {
    let conn = sqlite::load(P_SQL).unwrap();
    let schema = sqlite::introspect_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(P_R2RML).unwrap();
    // deterministic (no LIMIT): DISTINCT over Union{Values{1,2},Values{2,3}} = {1,2,3}
    let q = format!("{PFX} SELECT DISTINCT ?x WHERE {{ {{ VALUES ?x {{ 1 2 }} }} UNION {{ VALUES ?x {{ 2 3 }} }} }}");
    let parsed = parse(&q);
    assert_vs_spareval(
        P_TTL,
        &q,
        &tree(&maps, &parsed, &schema).expect("tree"),
        &conn,
    );
}
