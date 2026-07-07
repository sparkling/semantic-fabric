//! ADVERSARIAL REFUTE-ONLY review of the OPTIONAL anti-join FILTER fix
//! (`crates/sf-sparql/src/leftjoin.rs::not_exists_cond_for`, threaded through its
//! three call sites: `leftjoin.rs::left_join_branches`,
//! `iq/lower.rs::left_join_decomposed`, `iq/lower.rs::left_join_as_subplan`).
//!
//! Every fixture/query here targets an angle NOT already covered by
//! `differential_tree.rs`'s `optional_anti_join_filter_*` tests (base repro,
//! no-op guard, nested-OPTIONAL variant, nullable-determinant variant). This
//! file is a standalone probe: separate test binary (Cargo integration tests are
//! independent crates), so harness plumbing is duplicated rather than imported.
//!
//! Angles covered:
//! 1. `left_join_as_subplan` (LeftJoinJoinLimit) reachability probe.
//! 2. Multi-BRANCH (UNION) OPTIONAL right, filter above the union, asymmetric
//!    per-arm removal.
//! 3. A 3VL/type-error-producing FILTER (not just a clean `false`).
//! 4. Chained sibling OPTIONALs where the filtered one is NOT last.
//! 5. A FILTER referencing MULTIPLE right-only variables at once.
//! 6. DISTINCT wrapping the whole query.
//! 7. Live PostgreSQL / MySQL smoke of the base repro shape (graceful skip).
//!
//! Plus two bonus angles: CONSTRUCT-form and ASK-form discriminators.

use rusqlite::Connection;
use sf_conformance::graph::{isomorphic, parse_turtle, triples_to_dataset};
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::sqlite;
use sf_sparql::{exec, translate_tree, translate_with_flat, Error, Plan, PlanForm, Tbox};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};

const BASE: &str = "http://ex/";
const PFX: &str = "PREFIX ex: <http://ex/>";

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
    translate_tree(q, maps, &Tbox::default(), Dialect::Sqlite, schema)
}

#[derive(Debug)]
enum Answer {
    Select(Vec<std::collections::BTreeMap<String, oxrdf::Term>>),
    Construct(Vec<String>),
    Ask(bool),
}

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

fn answers_eq(a: &Answer, b: &Answer) -> bool {
    match (a, b) {
        (Answer::Select(x), Answer::Select(y)) => oracle::solutions_bag_eq(x, y),
        (Answer::Construct(x), Answer::Construct(y)) => x == y,
        (Answer::Ask(x), Answer::Ask(y)) => x == y,
        _ => false,
    }
}

/// Core differential: flat/tree row-bag parity + identical 501 set, plus (when
/// `ttl` is `Some`) the tree answer vs the independent `spareval` oracle.
fn diff(create: &str, r2rml: &str, ttl: Option<&str>, query: &str) {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);
    let f = flat(&maps, &q, &schema);
    let t = tree(&maps, &q, &schema);

    match (&f, &t) {
        (Err(Error::Unsupported(_)), Err(Error::Unsupported(_))) => {}
        (Ok(fp), Ok(tp)) => {
            let fa = run(fp, &conn);
            let ta = run(tp, &conn);
            assert!(
                answers_eq(&fa, &ta),
                "flat vs tree row-bag divergence on `{query}`:\n flat={fa:#?}\n tree={ta:#?}"
            );
            if let Some(ttl) = ttl {
                assert_vs_spareval(ttl, query, tp, &conn);
            }
        }
        _ => panic!(
            "501-set mismatch on `{query}` (flat and tree must agree on Unsupported):\n flat={f:?}\n tree={t:?}"
        ),
    }
}

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

// ============================================================================
// Fixture OD — IDENTICAL to `differential_tree.rs`'s OD fixture (person ⟕ dept
// multi-scan [core.len()>1 ⇒ decomposition, not the single-scan LEFT JOIN
// shortcut] + person ⟕ person nullable self-join `mentor`). Duplicated here
// (separate test binary) as the base for angles 3/4/6/7/bonus.
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

// ============================================================================
// ANGLE 1 — `left_join_as_subplan` (LeftJoinJoinLimit, ADR-0023 M5 Wave 2) call
// site, PLUS `left_join_decomposed`'s own loop (the 2nd call site) via nested
// OPTIONAL. CONFIRMED EMPIRICALLY (revert-tested — see
// `angle1_followup_isolate_subplan_join_drop_bug` and the manual stash/pop
// cross-check performed while authoring this file):
//
// * Candidates A ("mentor OUTER, dept/label+filter INNER", 2 levels) and B (3
//   levels) DO reach and exercise `left_join_decomposed`'s own decomposition
//   loop (2nd call site) with a non-`None` `expr` — genuinely NEW coverage
//   (nested, not sibling, unlike the existing `optional_anti_join_filter_
//   nested_optional` test). Candidate A is REVERT-SENSITIVE (confirmed via a
//   manual stash/pop of the fix): reverting it makes A fail with a real
//   `=_bag` divergence — Bob's row comes back as `{name: "Bob"}` (BOTH `?m`
//   AND `?label` dropped, not just `?label` — worse than the flat base-repro
//   symptom, since the *mentor* binding, established BEFORE the inner
//   filtered OPTIONAL, gets collaterally destroyed too) where spareval
//   correctly expects `{name: "Bob", m: <person/1>}`. Candidate B did NOT
//   reproduce a divergence on this fixture even reverted (the extra nesting
//   level happens not to matter for this data/shape) — reported for honesty,
//   not claimed as a second confirmed kill. Under the FIXED code both A and B
//   pass cleanly.
// * Candidates C/D add a LIMIT-subselect on the LEFT of the OPTIONAL (the
//   literal Ontop `testLeftJoinJoinLimit` shape). At the time this comment was
//   first written, both translated successfully but CRASHED at SQL-execution
//   time (`no such column: tN.c0`) — confirmed via
//   `angle1_followup_isolate_subplan_join_drop_bug` to be a PRE-EXISTING,
//   FIX-INDEPENDENT defect relative to the anti-join-FILTER fix under review
//   here (reproduced with no FILTER at all; did NOT reproduce for the same
//   LIMIT-subselect joined via a plain INNER JOIN instead of OPTIONAL) — so
//   neither C nor D exercised THIS review's fix, and neither was a
//   counterexample to it.
//
//   **UPDATE (2026-07-03, subplan-drop fix, `crates/sf-sparql/src/leftjoin.rs`):**
//   root-caused and closed, in code this review's own diff never touched. Two
//   distinct defects: (1) `inner_join_one` built its output `Branch` with
//   `subplan_joins: Vec::new()`, unconditionally discarding `left.subplan_joins`
//   (and any `right.subplan_joins`) instead of carrying them forward — fixed by
//   extending both sides through, mirroring `unfold::merge`'s InnerJoin idiom.
//   That alone fixes candidate C (and the `i`/`ii` shapes in
//   `angle1_followup_isolate_subplan_join_drop_bug`). (2) Candidate D goes one
//   level deeper: the OUTER decomposition's anti-join branch calls
//   `not_exists_cond_for(l_outer, r=<inner LeftJoin's own decomposed match
//   branch>, ..)`, and that inner match branch itself carries a SubPlan (the
//   LIMIT-subselect) — `SqlCond::NotExists::scans` is a plain `Vec<Scan>` with
//   no room for a nested SubPlan, so the correlated reference into it would
//   still be unrepresentable. Rather than extend the `SqlCond` enum (a bigger,
//   riskier change touching emit.rs + cascade + iq.rs traversal for a shape no
//   test yet needs), `not_exists_cond_for` now detects `right.subplan_joins`
//   non-empty and returns a sound `Unsupported` (501, ADR-0007) instead of
//   building unrenderable SQL. D now fails honestly at translate time instead
//   of crashing at execution time; C is unaffected by this second, narrower
//   guard (its right side carries no subplan). See
//   `angle1_followup_isolate_subplan_join_drop_bug` for the revert-proof.
// * `left_join_as_subplan` specifically (as opposed to `left_join_decomposed`'s
//   own loop) still appears structurally unreachable in a way that completes
//   successfully: it is only invoked when some right branch carries non-empty
//   `opts`, but `opts` only ever becomes non-empty via `build_left_join`, which
//   is only invoked when `decompose == false` — so within the `decompose ==
//   true` subtree that is the ONLY context in which `left_join_as_subplan` is
//   ever called, no branch can carry non-empty `opts` by construction. No
//   candidate here (nor any considered) reaches its `inner_join_one`/
//   `not_exists_cond_for` loop specifically; it remains untested (not "proven
//   correct", not "proven broken" — simply unreached by any query found;
//   unaffected by the 2026-07-03 fix above since it's never actually called).
// ============================================================================

#[test]
fn angle1_left_join_as_subplan_probe() {
    let conn = sqlite::load(OD_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(OD_R2RML).expect("R2RML parses");

    // Candidate A: OPTIONAL nested INSIDE another OPTIONAL's group (mentor
    // OUTER, dept/label+filter INNER) — the shape that forces `decompose=true`
    // onto the mentor-optional's own LeftJoin dispatch (§5.3 nested-right
    // closure), which is the only way to reach `left_join_decomposed` at all.
    let q_a = format!(
        "{PFX} SELECT ?name ?label ?m WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:mentor ?m \
         OPTIONAL {{ ?m ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }} }}"
    );
    // Candidate B: THREE levels deep (mentor / dept / label-filter), in case a
    // single level of nesting isn't enough to leave a branch opts-carrying.
    let q_b = format!(
        "{PFX} SELECT ?name ?m ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:mentor ?m \
           OPTIONAL {{ ?m ex:dept ?d \
             OPTIONAL {{ ?d ex:label ?label FILTER(?label != \"Eng\") }} }} }} }}"
    );
    // Candidate C: the literal Ontop `testLeftJoinJoinLimit` shape — a SUBSELECT
    // with LIMIT joined on the LEFT of the OPTIONAL, multi-scan+filtered R.
    let q_c = format!(
        "{PFX} SELECT ?name ?label WHERE {{ \
         {{ SELECT ?p ?name WHERE {{ ?p ex:name ?name }} ORDER BY ?name LIMIT 10 }} \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }}"
    );
    // Candidate D: the LIMIT-subselect nested INSIDE a further enclosing
    // OPTIONAL, so the whole "(L ⋈ SUBSELECT) OPT R" is itself right-nested,
    // forcing decompose=true onto it directly (closer to the literal ADR shape
    // than candidate C, which is top-level / decompose=false there).
    let q_d = format!(
        "{PFX} SELECT ?name ?m ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ \
           {{ SELECT ?p ?m WHERE {{ ?p ex:mentor ?m }} ORDER BY ?m LIMIT 10 }} \
           OPTIONAL {{ ?m ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }} }}"
    );

    // A/B: nested-OPTIONAL shapes (no slice) — must translate AND match spareval.
    for (name, q_str) in [("A-nested2", &q_a), ("B-nested3", &q_b)] {
        let q = parse(q_str);
        let tp = tree(&maps, &q, &schema)
            .unwrap_or_else(|e| panic!("[angle1:{name}] tree translation should succeed: {e:?}"));
        assert_vs_spareval(OD_TTL, q_str, &tp, &conn);
    }
    // C: a LIMIT-subselect as a join operand. Since ADR-0025 Tier-1 bug #2 this soundly
    // 501s — the derived table cannot carry the slice, and dropping it is a wrong answer
    // whenever the LIMIT bites (here `LIMIT 10` on ≤10 rows was harmless, so C used to pass
    // coincidentally). The inner_join_one subplan-carrying coverage C once exercised is
    // preserved by `subplan_aggregate_as_join_operand` / `subplan_distinct_as_join_operand`
    // (aggregate/DISTINCT subplans have no slice, so they still translate).
    {
        let q = parse(&q_c);
        assert!(
            matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(msg)) if msg.contains("SubPlan")),
            "[angle1:C-subselect-limit] LIMIT-subselect as a join operand must sound-501 (ADR-0025 bug #2)"
        );
    }

    // D: the OUTER anti-join branch's right side itself carries a SubPlan (the
    // LIMIT-subselect one level down) — an ADR-0023 M5 boundary `not_exists_
    // cond_for` cannot represent (see UPDATE above). Must fail honestly at
    // translate time (501), not crash at execution time.
    let q = parse(&q_d);
    match tree(&maps, &q, &schema) {
        Err(Error::Unsupported(msg)) => {
            assert!(
                msg.contains("SubPlan"),
                "[angle1:D-subselect-limit-nested] expected the SubPlan-boundary 501, got: {msg}"
            );
        }
        Ok(_) => panic!(
            "[angle1:D-subselect-limit-nested] now translates OK — if this is a genuine capability \
             gain, replace this arm with an `assert_vs_spareval` call (matching A/B/C above) and \
             update the doc comment; a silent behavior change here should not go unnoticed"
        ),
        Err(other) => panic!("[angle1:D-subselect-limit-nested] unexpected error: {other:?}"),
    }
}

/// FOLLOW-UP to angle 1: candidate C (LIMIT-subselect LEFT-joined, then OPTIONAL'd
/// with a multi-scan filtered right) crashed at SQL EXECUTION time with `no such
/// column: t4.c0` — the generated SQL's FROM clause never joins in the alias
/// (`t4`) that its own SELECT-list/WHERE-clause reference. This isolated WHETHER
/// that crash implicates the anti-join-filter fix under review, or is a
/// pre-existing, independent defect: `leftjoin.rs::inner_join_one` (NOT touched by
/// that fix's diff) built its output `Branch` with `subplan_joins: Vec::new()`,
/// unconditionally DISCARDING `left.subplan_joins` instead of carrying it forward
/// (contrast `opts: left.opts.clone()` on the very next field, which correctly
/// preserved the left's state). If `left` came from a LIMIT-subselect (a
/// `SubPlanJoin`-carrying branch), any `inner_join_one` call silently dropped the
/// FROM-clause join while its correlated `where_conds` (cloned from
/// `left.where_conds`, which DOES survive) kept referencing the now-unjoined
/// alias — a `core`/`subplan_joins` inconsistency, independent of whether a
/// FILTER or even an OPTIONAL-anti-join path was involved at all.
///
/// FIXED (2026-07-03, `crates/sf-sparql/src/leftjoin.rs::inner_join_one`):
/// `subplan_joins` now extends `left.subplan_joins` + `right.subplan_joins`
/// (mirrors `unfold::merge`'s InnerJoin idiom) instead of being zeroed.
///
/// SUPERSEDED (2026-07-07, ADR-0025 Tier-1 bug #2): shapes (i)/(ii)/(iv) use a
/// LIMIT-subselect as a join operand, which now soundly 501s — the derived table
/// cannot carry the slice, and silently dropping it is a wrong answer whenever the
/// LIMIT bites (these passed before only because `LIMIT 10` was a no-op on the ≤10-row
/// fixture). They therefore now assert the sound 501, not a spareval match. The
/// inner_join_one subplan-carrying fix above stays covered by the aggregate/DISTINCT
/// subplan-as-join-operand tests (no slice). (iii) — a plain BGP left, no slice — is the
/// unaffected control that still translates and matches spareval.
#[test]
fn angle1_followup_isolate_subplan_join_drop_bug() {
    let conn = sqlite::load(OD_SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(OD_R2RML).expect("R2RML parses");

    // (i) Same LEFT (LIMIT-subselect) + multi-scan right, but NO FILTER at all —
    // isolates whether the FILTER (this review's subject) is required to trigger
    // the crash, or whether plain `inner_join_one` (unconditional, filter-free)
    // already drops the subplan join on its own.
    let q_no_filter = format!(
        "{PFX} SELECT ?name ?label WHERE {{ \
         {{ SELECT ?p ?name WHERE {{ ?p ex:name ?name }} ORDER BY ?name LIMIT 10 }} \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label }} }}"
    );
    // (ii) Same LEFT (LIMIT-subselect), but a SINGLE-scan right (no dept join, just
    // a direct column) — isolates whether the decomposition path specifically is
    // required, or whether the plain `build_left_join` LEFT JOIN shortcut path
    // ALSO mishandles a `subplan_joins`-carrying left (a completely different
    // function from `inner_join_one`).
    let q_single_scan_right = format!(
        "{PFX} SELECT ?name ?m WHERE {{ \
         {{ SELECT ?p ?name WHERE {{ ?p ex:name ?name }} ORDER BY ?name LIMIT 10 }} \
         OPTIONAL {{ ?p ex:mentor ?m }} }}"
    );
    // (iii) No LIMIT-subselect at all (plain BGP left) — same multi-scan+filter
    // right as candidate C. This is the ALREADY-PASSING base repro shape
    // (`optional_anti_join_filter_match_removing`) restated here to confirm the
    // LIMIT-subselect specifically (not the filter, not the multi-scan-right
    // decomposition alone) is the differentiator.
    let q_plain_left = format!(
        "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }}"
    );

    // (iv) LIMIT-subselect joined via a PLAIN INNER JOIN (no OPTIONAL at all) with
    // a multi-scan pattern — isolates whether LIMIT-subselects-as-join-operand are
    // unsupported in GENERAL (vs. the proven-working `subplan_aggregate_as_join_
    // operand`/`subplan_distinct_as_join_operand` shapes, which use aggregate/
    // DISTINCT subqueries, not a bare LIMIT), independent of OPTIONAL entirely.
    let q_inner_join_no_optional = format!(
        "{PFX} SELECT ?name ?label WHERE {{ \
         {{ SELECT ?p ?name WHERE {{ ?p ex:name ?name }} ORDER BY ?name LIMIT 10 }} \
         ?p ex:dept ?d . ?d ex:label ?label }}"
    );

    // (iii) plain BGP left (NO slice) — still translates + matches spareval (control).
    {
        let q = parse(&q_plain_left);
        let tp = tree(&maps, &q, &schema)
            .unwrap_or_else(|e| panic!("[followup:iii-plain-left-control] should succeed: {e:?}"));
        assert_vs_spareval(OD_TTL, &q_plain_left, &tp, &conn);
    }
    // (i)/(ii)/(iv) all use a LIMIT-subselect as a join operand → sound-501 since ADR-0025
    // Tier-1 bug #2 (they used to pass only because `LIMIT 10` didn't bite on ≤10 rows; the
    // slice was silently dropped). The inner_join_one subplan-carrying fix these once
    // exercised stays covered by the aggregate/DISTINCT subplan-as-join-operand tests.
    for (name, q_str) in [
        ("i-no-filter", &q_no_filter),
        ("ii-single-scan-right", &q_single_scan_right),
        ("iv-inner-join-no-optional", &q_inner_join_no_optional),
    ] {
        let q = parse(q_str);
        assert!(
            matches!(tree(&maps, &q, &schema), Err(Error::Unsupported(msg)) if msg.contains("SubPlan")),
            "[followup:{name}] LIMIT-subselect as join operand must sound-501 (ADR-0025 bug #2)"
        );
    }
}

// ============================================================================
// ANGLE 2 — multi-BRANCH (UNION) OPTIONAL right, filter ABOVE the union but
// INSIDE the OPTIONAL. Per the SPARQL 1.1 translation algorithm (18.2.2.8), a
// FILTER directly in a GroupGraphPattern is collected and wrapped around the
// WHOLE group's algebra at the end — so `{ {A} UNION {B} FILTER(f) }` becomes
// `Filter(f, Union(A,B))`, and since this sits as an OPTIONAL's body, the
// enclosing OPTIONAL processing unwraps that top Filter into the LeftJoin's
// OWN condition argument: `LeftJoin(outer_left, Union(A,B), f)` — i.e. the
// SAME shared `expr` parameter applies to EACH union arm's OWN combined
// bindings (verified structurally, not assumed). `right.len() == 2` here (one
// per union arm), so this exercises the `for r in &right { not_exists_cond_for
// (l, r, expr, ..) }` loop over MULTIPLE branches, not the single
// multi-SCAN-but-one-branch shape the existing base repro uses.
//
// Fixture: person has a PRIMARY dept (mandatory FK) and a nullable BACKUP
// dept (a second, independent FK to the same dept table). The filter removes
// "Eng"-labelled candidates from EITHER arm. Data is built so the filter's
// pass/fail differs ACROSS the two arms for the same left row:
//   Ann: primary=Eng (filtered out) but backup=Sales (passes)  ⇒ must MATCH
//        (via the backup arm), NOT be null-padded.
//   Bob: primary=Eng (filtered out), NO backup at all           ⇒ BOTH arms
//        fail (one by filter, one by having zero rows)          ⇒ must be
//        NULL-PADDED (the exact bug shape, via a union instead of a single
//        multi-scan branch).
//   Zed: primary=Sales (passes), no backup                      ⇒ matches via
//        primary (control case, unaffected).
// ============================================================================

const UD_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    dept_id INTEGER NOT NULL,
    backup_dept_id INTEGER,
    FOREIGN KEY (dept_id) REFERENCES dept(id),
    FOREIGN KEY (backup_dept_id) REFERENCES dept(id)
);
INSERT INTO dept VALUES (10, 'Sales');
INSERT INTO dept VALUES (20, 'Eng');
INSERT INTO person VALUES (1, 'Ann', 20, 10);
INSERT INTO person VALUES (2, 'Bob', 20, NULL);
INSERT INTO person VALUES (3, 'Zed', 10, NULL);
"#;

const UD_R2RML: &str = r#"
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
        rr:predicate ex:backupDept ;
        rr:objectMap [
            rr:parentTriplesMap <#Dept> ;
            rr:joinCondition [ rr:child "backup_dept_id" ; rr:parent "id" ]
        ]
    ] .
<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const UD_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/person/1> ex:name "Ann" ; ex:dept <http://ex/dept/20> ; ex:backupDept <http://ex/dept/10> .
<http://ex/person/2> ex:name "Bob" ; ex:dept <http://ex/dept/20> .
<http://ex/person/3> ex:name "Zed" ; ex:dept <http://ex/dept/10> .
<http://ex/dept/10> ex:label "Sales" .
<http://ex/dept/20> ex:label "Eng" .
"#;

fn diff_ud(query: &str) {
    diff(UD_SQL, UD_R2RML, Some(UD_TTL), query);
}

#[test]
fn angle2_union_in_optional_asymmetric_filter() {
    // THE REPRO (union variant): Bob's ONLY candidates across BOTH arms fail
    // (primary filtered by label, backup doesn't exist) ⇒ must NULL-pad, not
    // vanish. Ann matches via the SURVIVING arm (backup); Zed via primary.
    diff_ud(&format!(
        "{PFX} SELECT ?name ?d ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ {{ ?p ex:dept ?d . ?d ex:label ?label }} \
                     UNION {{ ?p ex:backupDept ?d . ?d ex:label ?label }} \
                     FILTER(?label != \"Eng\") }} }}"
    ));
    // No-op-guard companion: a filter that never excludes anything (no dept is
    // "ZZZ") — both arms can independently match for Ann (primary=Eng passes,
    // backup=Sales passes), which per UNION bag semantics legitimately
    // produces TWO solution rows for Ann (once per matching arm) — guards
    // against an over-correction that would incorrectly suppress one.
    diff_ud(&format!(
        "{PFX} SELECT ?name ?d ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ {{ ?p ex:dept ?d . ?d ex:label ?label }} \
                     UNION {{ ?p ex:backupDept ?d . ?d ex:label ?label }} \
                     FILTER(?label != \"ZZZ\") }} }}"
    ));
}

// ============================================================================
// ANGLE 5 — FILTER referencing MULTIPLE right-only variables at once
// (`?label` AND `?budget`, both sourced from the SAME multi-scan OPTIONAL
// right, neither redundant with the other — the AND's removal outcome cannot
// be replicated by either conjunct alone).
// ============================================================================

const MV_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL, budget INTEGER NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    dept_id INTEGER NOT NULL,
    FOREIGN KEY (dept_id) REFERENCES dept(id)
);
INSERT INTO dept VALUES (10, 'Sales', 50);
INSERT INTO dept VALUES (20, 'Eng', 200);
INSERT INTO dept VALUES (30, 'Ops', 200);
INSERT INTO person VALUES (1, 'Ann', 20);
INSERT INTO person VALUES (2, 'Bob', 10);
INSERT INTO person VALUES (3, 'Zed', 30);
"#;

const MV_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
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
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:budget ; rr:objectMap [ rr:column "budget" ; rr:datatype xsd:integer ] ] .
"#;

const MV_TTL: &str = r#"
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<http://ex/person/1> ex:name "Ann" ; ex:dept <http://ex/dept/20> .
<http://ex/person/2> ex:name "Bob" ; ex:dept <http://ex/dept/10> .
<http://ex/person/3> ex:name "Zed" ; ex:dept <http://ex/dept/30> .
<http://ex/dept/10> ex:label "Sales" ; ex:budget "50"^^xsd:integer .
<http://ex/dept/20> ex:label "Eng" ; ex:budget "200"^^xsd:integer .
<http://ex/dept/30> ex:label "Ops" ; ex:budget "200"^^xsd:integer .
"#;

fn diff_mv(query: &str) {
    diff(MV_SQL, MV_R2RML, Some(MV_TTL), query);
}

#[test]
fn angle5_multi_var_filter_two_right_only_vars() {
    // Ann: label fails (Eng) -- removed by the FIRST conjunct.
    // Bob: label passes (Sales) but budget fails (50 !> 100) -- removed by the
    //      SECOND conjunct alone (label alone would have kept Bob: proves the
    //      fix threads the WHOLE expr, not just a first-var approximation).
    // Zed: both pass -- matches normally.
    // All three must still appear (NULL-padded for Ann/Bob), 3 rows total.
    diff_mv(&format!(
        "{PFX} SELECT ?name ?label ?budget WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label . ?d ex:budget ?budget \
         FILTER(?label != \"Eng\" && ?budget > 100) }} }}"
    ));
}

// ============================================================================
// ANGLE 3 — a FILTER whose truth value is a type-error / non-boolean
// comparison (RDF term-equality between an `xsd:string`-valued column and an
// `xsd:integer` literal `1`), not a clean SPARQL `false`. Per SPARQL 1.1
// §11.4.1 a type error's effective boolean value is "not true" — same
// FILTER-exclusion effect as `false`, but via a different code path in a
// standards-faithful evaluator (and a DIFFERENT SQL rendering here, since the
// constant's literal datatype differs from the base repro's plain-string
// comparison). Every dept label ("Sales"/"Eng") is a string, never equal to
// the integer `1`, so EVERY person's only candidate fails ⇒ ALL THREE must be
// NULL-padded (a stronger shape than the base repro, where only Ann is
// affected) -- if the anti-join's filter rendering diverges from the
// inner-join's for this expression form, this should surface it as either a
// spareval mismatch or an outright SQL execution error/panic.
// ============================================================================

#[test]
fn angle3_type_error_filter_treated_as_no_match_for_all() {
    diff_od(&format!(
        "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label = 1) }} }}"
    ));
}

// ============================================================================
// ANGLE 4 — chained SIBLING OPTIONALs (NOT nested one inside the other's
// group -- that shape is already covered by `optional_anti_join_filter_
// nested_optional`), where the FILTERED optional is FIRST, not last, and a
// SUBSEQUENT OPTIONAL depends on a variable whose bindability the first one
// gates. `{ A OPTIONAL{B} OPTIONAL{C} }` parses as
// `LeftJoin(LeftJoin(A,B,filterB), C, true)` -- two SEPARATE LeftJoin nodes,
// where the decomposition's OUTPUT (Vec<Branch>: one inner-join branch + one
// null-padded no-match branch) becomes the INPUT `left: Vec<Branch>` to the
// second OPTIONAL's own lowering. This stresses whether the NULL-padded
// branch the fix produces is a clean, uncorrupted `Branch` for the SECOND
// OPTIONAL to build on top of.
// ============================================================================

#[test]
fn angle4_chained_optionals_filtered_first_not_last() {
    // (a) second OPTIONAL depends on ?p only (independent of the filtered
    // one) -- checks the null-padded branch doesn't leak a stale ?p/alias.
    diff_od(&format!(
        "{PFX} SELECT ?name ?label ?m WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} \
         OPTIONAL {{ ?p ex:mentor ?m }} }}"
    ));
    // (b) second OPTIONAL depends on ?d -- the EXACT variable the first
    // OPTIONAL'S FILTER may leave unbound (Ann). If the fix's null-padded
    // branch accidentally retained a phantom/stale ?d binding instead of
    // truly leaving it unbound, this second OPTIONAL would wrongly match for
    // Ann instead of also staying unbound on ?label2.
    diff_od(&format!(
        "{PFX} SELECT ?name ?label ?label2 WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} \
         OPTIONAL {{ ?d ex:label ?label2 }} }}"
    ));
}

// ============================================================================
// ANGLE 6 — DISTINCT wrapping the whole query, interacting with the anti-join
// -filter shape AND the cascade's `distinct_prune_unused_opts` pass (a
// DISTINCT-driven optimization that can prune an OPTIONAL whose own variables
// are never projected -- soundness there relies on OPTIONAL never removing
// left rows AND any multiplicity collapsing under the outer DISTINCT anyway).
// Projecting ?label (an OPTIONAL-internal variable) specifically keeps that
// prune from firing, so the actual decomposition + fix must run, THEN get
// deduplicated. Bob/Zed's real dedup pressure (both -> "Sales") plus Ann's
// NULL-padded row collapsing correctly under DISTINCT is the target.
// ============================================================================

#[test]
fn angle6_distinct_wrapping_anti_join_filter() {
    // Real collapsing: Bob and Zed both end up label="Sales" (2 underlying
    // rows -> 1 distinct); Ann NULL-padded (1 distinct UNBOUND row). Expect 2
    // distinct rows total, not 3 and not 1.
    diff_od(&format!(
        "{PFX} SELECT DISTINCT ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }}"
    ));
    // Guard: projecting ?name alongside ?label means every row is already
    // distinguishable (3 distinct names) -- DISTINCT must NOT additionally
    // collapse or drop Ann's NULL-padded row here.
    diff_od(&format!(
        "{PFX} SELECT DISTINCT ?name ?label WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }}"
    ));
}

// ============================================================================
// BONUS — CONSTRUCT-form and ASK-form discriminators. Both rely on projecting/
// testing a variable bound by the MANDATORY (non-OPTIONAL) part alone, so
// "entire row silently vanished" (the bug) is externally distinguishable from
// "OPTIONAL var correctly NULL-padded" (correct) -- a CONSTRUCT/ASK over only
// the OPTIONAL's OWN variables cannot tell the two apart (both yield zero
// triples / unaffected truth value for the affected row either way).
// ============================================================================

#[test]
fn bonus_construct_form_whole_row_survives() {
    // ?name is bound unconditionally; the bug drops Ann's WHOLE solution (not
    // just ?label), so her `ex:seen "Ann"` triple would be MISSING under the
    // bug (2 triples instead of 3).
    diff_od(&format!(
        "{PFX} CONSTRUCT {{ ?p ex:seen ?name }} WHERE {{ ?p ex:name ?name \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }}"
    ));
}

#[test]
fn bonus_ask_form_whole_row_survives() {
    // Ann's solution exists via the mandatory ?p ex:name "Ann" triple alone;
    // the OPTIONAL's outcome (matched-Sales / NULL-padded) must never affect
    // whether this ASK is true. Under the bug Ann's whole row vanishes ⇒ ASK
    // would wrongly be false.
    diff_od(&format!(
        "{PFX} ASK {{ ?p ex:name \"Ann\" \
         OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"Eng\") }} }}"
    ));
}

// ============================================================================
// ANGLE 7 — live PostgreSQL / MySQL smoke of the base repro shape (graceful
// skip when no server is reachable, mirroring `differential_pg_sqlite.rs`'s
// established probe/throwaway-db convention). A subtle dialect-specific SQL
// rendering bug in how `filter_cond` renders inside a correlated `NOT EXISTS`
// (alias scoping, parameter binding/typing) could exist even if the logic
// itself is right -- this exercises that on REAL servers, not just SQLite.
// ============================================================================

mod live_db {
    use super::*;
    use sf_sparql::{exec_mysql, exec_pg, parse_and_translate_with};
    use sf_sql::introspect::introspect_postgres;
    use tokio_postgres::{Client, NoTls};

    /// The base repro query (angle: match-removing filter on a multi-scan
    /// OPTIONAL right) -- identical shape to `optional_anti_join_filter_
    /// match_removing`, run over the OD fixture (schema-neutral: lowercase,
    /// unquoted identifiers; INTEGER/TEXT types load identically on all three
    /// backends, matching `differential_pg_sqlite.rs`'s `CREATE_SQL` style).
    const LIVE_Q: &str = r#"
        PREFIX ex: <http://ex/>
        SELECT ?name ?label WHERE {
            ?p ex:name ?name
            OPTIONAL { ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != "Eng") }
        }"#;

    fn sqlite_oracle_bag() -> Vec<Vec<std::collections::BTreeMap<String, sf_core::Term>>> {
        let conn = sqlite::load(OD_SQL).expect("sqlite fixture loads");
        let maps = sf_mapping::parse_r2rml(OD_R2RML).expect("R2RML parses");
        let schema = sqlite::introspect_all(&conn).expect("sqlite introspection");
        let plan =
            parse_and_translate_with(LIVE_Q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
                .expect("translate (sqlite)");
        let sols = exec::select(&plan, &conn).expect("sqlite select");
        vec![oracle::engine_bag(&sols)]
    }

    fn base_pg_conn() -> String {
        std::env::var("SF_PG_URL").unwrap_or_else(|_| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
            format!("host=localhost port=5432 user={user}")
        })
    }

    async fn connect_pg(conn_str: &str) -> Result<Client, String> {
        let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(client)
    }

    async fn introspect_all_pg(client: &Client) -> Result<Vec<TableSchema>, String> {
        let rows = client
            .query(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
                &[],
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut schemas = Vec::with_capacity(rows.len());
        for r in rows {
            let name: String = r.get(0);
            schemas.push(
                introspect_postgres(client, &name)
                    .await
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(schemas)
    }

    #[test]
    fn angle7_live_postgres_base_repro() {
        let sqlite_bag = sqlite_oracle_bag().remove(0);
        assert_eq!(
            sqlite_bag.len(),
            3,
            "sqlite oracle sanity: Ann NULL-padded + Bob + Zed = 3 rows"
        );

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            let base = base_pg_conn();
            let admin = match connect_pg(&format!("{base} dbname=postgres")).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("no PostgreSQL server reachable ({e}) — skipping angle7 PG smoke");
                    return;
                }
            };
            let dbname = format!("sf_adv_review_{}", std::process::id());
            admin
                .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
                .await
                .expect("drop pre-existing throwaway db");
            admin
                .batch_execute(&format!("CREATE DATABASE {dbname}"))
                .await
                .expect("create throwaway db");

            let work = connect_pg(&format!("{base} dbname={dbname}"))
                .await
                .expect("connect work db");
            work.batch_execute(OD_SQL).await.expect("pg fixture loads");

            let maps = sf_mapping::parse_r2rml(OD_R2RML).expect("R2RML parses");
            let schema = introspect_all_pg(&work).await.expect("pg introspection");
            let plan = parse_and_translate_with(LIVE_Q, &maps, Dialect::Postgres, &Tbox::default(), &schema)
                .expect("translate (pg)");
            let sols = exec_pg::select_pg(&plan, &work).await.expect("pg select");
            let pg_bag = oracle::engine_bag(&sols);

            let _ = admin
                .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
                .await;

            assert!(
                oracle::solutions_bag_eq(&sqlite_bag, &pg_bag),
                "angle7 PG SMOKE FAILED — live PostgreSQL diverges from the SQLite oracle:\n sqlite={sqlite_bag:#?}\n pg={pg_bag:#?}"
            );
            assert_eq!(pg_bag.len(), 3, "live PG: Ann NULL-padded + Bob + Zed = 3 rows");
        });
    }

    fn mysql_url() -> String {
        std::env::var("SF_MYSQL_URL")
            .unwrap_or_else(|_| "mysql://root:sftest@127.0.0.1:13306/sftest".to_owned())
    }

    async fn try_connect_mysql() -> Option<mysql_async::Conn> {
        let opts = mysql_async::Opts::from_url(&mysql_url()).ok()?;
        mysql_async::Conn::new(opts).await.ok()
    }

    async fn introspect_all_mysql(
        conn: &mut mysql_async::Conn,
    ) -> Result<Vec<TableSchema>, String> {
        use mysql_async::prelude::Queryable;
        let names: Vec<String> = conn
            .query(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' ORDER BY table_name",
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut schemas = Vec::with_capacity(names.len());
        for name in names {
            schemas.push(
                sf_sql::introspect::introspect_mysql(conn, &name)
                    .await
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(schemas)
    }

    async fn run_stmts(conn: &mut mysql_async::Conn, sql: &str) {
        use mysql_async::prelude::Queryable;
        for stmt in sql.split(';') {
            let s = stmt.trim();
            if !s.is_empty() {
                conn.query_drop(s).await.expect("mysql fixture statement");
            }
        }
    }

    #[test]
    fn angle7_live_mysql_base_repro() {
        use mysql_async::prelude::Queryable;
        let sqlite_bag = sqlite_oracle_bag().remove(0);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            let mut conn = match try_connect_mysql().await {
                Some(c) => c,
                None => {
                    eprintln!("no MySQL server reachable — skipping angle7 MySQL smoke");
                    return;
                }
            };
            let db = format!("sf_adv_review_my_{}", std::process::id());
            conn.query_drop(format!("DROP DATABASE IF EXISTS {db}"))
                .await
                .expect("drop pre-existing throwaway db");
            conn.query_drop(format!("CREATE DATABASE {db}"))
                .await
                .expect("create throwaway db");
            conn.query_drop(format!("USE {db}"))
                .await
                .expect("use throwaway db");
            run_stmts(&mut conn, OD_SQL).await;

            let maps = sf_mapping::parse_r2rml(OD_R2RML).expect("R2RML parses");
            let schema = introspect_all_mysql(&mut conn).await.expect("mysql introspection");
            let plan = parse_and_translate_with(LIVE_Q, &maps, Dialect::MySql, &Tbox::default(), &schema)
                .expect("translate (mysql)");
            let sols = exec_mysql::select_mysql(&plan, &mut conn).await.expect("mysql select");
            let mysql_bag = oracle::engine_bag(&sols);

            let _ = conn.query_drop(format!("DROP DATABASE IF EXISTS {db}")).await;

            assert!(
                oracle::solutions_bag_eq(&sqlite_bag, &mysql_bag),
                "angle7 MySQL SMOKE FAILED — live MySQL diverges from the SQLite oracle:\n sqlite={sqlite_bag:#?}\n mysql={mysql_bag:#?}"
            );
            assert_eq!(mysql_bag.len(), 3, "live MySQL: Ann NULL-padded + Bob + Zed = 3 rows");
        });
    }
}
