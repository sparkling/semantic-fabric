//! ADVERSARIAL REFUTE-ONLY review of the ADR-0034 "virtual-graph set
//! semantics" implementation (commit 6bcacf9): BGP-level D1 (within-branch
//! duplicate-row DISTINCT) + D2 (cross-branch same-triple UNION-dedup) with
//! key/disjointness elision, wired at `unfold::bgp` (flat) / `iq::resolve`'s
//! `Intensional` arm (tree), the `cascade::force_distinct_for_dup_safety` /
//! `table_key_covered_by_bindings` / `binding_is_injective` proofs, the
//! `unfold::pool_pattern_relation` D2 pooling gate, and `cascade::
//! dedup_before_aggregate`.
//!
//! Every fixture targets an attack surface NOT already covered by the ADR-0034
//! cells landed in `differential_star.rs`/`differential_tree.rs`. Standalone
//! probe: a Cargo integration test is its own crate, so the harness plumbing is
//! duplicated from `adversarial_run4b_refute.rs`/`differential_star.rs` rather
//! than imported. The oracle is the mapping's OWN materialization
//! (`exec::dump_quads` → `graph::quads_to_dataset`, a genuine SET) evaluated by
//! the independent `spareval` engine — so materialization is the reference and
//! the SQL-rewrite (OBDA) path is the system-under-test.
//!
//! Attack surfaces (verdicts in the accompanying report, not here):
//!  S1 — D2 cross-arm injective-but-DIFFERENT reconstruction (shared literal
//!       prefix, different template shape / different object spec): the corner
//!       the phase-1 non-injective 501 pin may not cover.
//!  S2 — the union-composite-key coverage proof: non-injective binding must not
//!       count; nullable UNIQUE must not elide; per-alias (never cross-alias).
//!  S3 — dedup must NOT over-fire: UNION of identical BGPs, join-projection
//!       duplicates, view (`rr:sqlQuery`) sources, user DISTINCT idempotence.
//!  S4 — exemption boundaries: EXISTS/MINUS bodies exempt, same pattern in/out
//!       of an EXISTS, a projected subquery must NOT inherit the exemption.
//!  S5 — NPS × D1: `!p` legitimate bag multiplicity joined with an unkeyed
//!       duplicate-carrying table; and plain `!p` over duplicate ROWS (a set —
//!       the triple exists once — must yield ONE solution).
//!  S6 — CONSTRUCT over duplicate-carrying sources: §16.2 output is a SET; a
//!       template that projects a distinguishing variable away.
//!
//! A test carrying a `BUG:` marker is a REFUTED verdict left deliberately red
//! (an exact repro), never a lock. Every other test is a SURVIVED regression
//! lock.

use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, Tbox};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};
use std::collections::BTreeMap;

use oxrdf::{Dataset, Term, Triple};

const PFX: &str = "PREFIX ex: <http://ex/> ";

// ============================================================================
// Harness plumbing — duplicated from `adversarial_run4b_refute.rs` (separate
// test binary; no `pub` cross-file surface to import). Semantics identical.
// ============================================================================

fn parse(q: &str) -> Query {
    SparqlParser::new().parse_query(q).expect("query parses")
}

fn introspect(create: &str) -> Vec<TableSchema> {
    let conn = sqlite::load(create).expect("fixture loads");
    sqlite::introspect_all(&conn).expect("introspection")
}

/// Translate a query through BOTH engines. Loads the fixture only for its schema
/// (translation needs the catalog, not a live row source).
fn translate_both(
    create: &str,
    r2rml: &str,
    query: &str,
) -> (sf_sparql::Result<Plan>, sf_sparql::Result<Plan>) {
    let schema = introspect(create);
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);
    let f = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    let t = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    (f, t)
}

/// A "sound 501" is either a translate-time `Unsupported` OR a successful
/// translation whose emission refuses (`plan.emitted().is_err()`) — mirrors
/// `differential_star.rs`'s own `adr0034_d2_..._sound_501` two-stage shape.
fn is_sound_501(r: &sf_sparql::Result<Plan>) -> bool {
    match r {
        Err(Error::Unsupported(_)) => true,
        Ok(p) => p.emitted().is_err(),
        Err(_) => false,
    }
}

/// The emitted SQL statements of a plan (one per branch). Panics if emission
/// itself refuses — callers that want to assert on shape have already
/// established the plan emits.
fn engine_sqls(plan: &Plan) -> Vec<String> {
    plan.emitted()
        .expect("plan emits")
        .into_iter()
        .map(|e| e.sql)
        .collect()
}

fn run_select(plan: &Plan, conn: &Connection) -> Vec<BTreeMap<String, Term>> {
    oracle::engine_bag(&exec::select(plan, conn).expect("select exec"))
}

/// Both translators must both succeed with the SAME row bag, or both refuse.
/// Returns the tree engine's rows (empty on a shared 501). Mirrors
/// `differential_star.rs::diff`.
fn diff(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);

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

/// The mapping's own materialization as an in-memory RDF dataset — the oracle
/// graph (a genuine SET: `dump_quads`'s bag is collapsed by `quads_to_dataset`).
fn materialized(create: &str, r2rml: &str) -> Dataset {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    graph::quads_to_dataset(&quads)
}

/// The spareval oracle's SELECT row bag over the materialized (set) graph.
fn oracle_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    match oracle::evaluate(&materialized(create, r2rml), query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected Solutions, got {other:?}"),
    }
}

/// Engine (tree∧flat-agreed) vs oracle — the acceptance bar. Returns the agreed
/// rows for the caller's own additional assertions.
fn assert_oracle_agrees(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let engine = diff(create, r2rml, query);
    let oracle_rows = oracle_bag(create, r2rml, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "engine vs oracle divergence on `{query}`:\n \
         engine (SQL-rewritten) = {engine:#?}\n \
         oracle (materialized graph, spareval) = {oracle_rows:#?}"
    );
    engine
}

/// Both translators' CONSTRUCT triple BAGS (multiplicity preserved), asserted
/// flat==tree, returned as the tree bag.
fn construct_bag(create: &str, r2rml: &str, query: &str) -> Vec<Triple> {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);
    let fp = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("flat translates");
    let tp = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("tree translates");
    let mut ft = exec::construct_triples(&fp, &conn).expect("flat construct");
    let mut tt = exec::construct_triples(&tp, &conn).expect("tree construct");
    ft.sort_by_key(ToString::to_string);
    tt.sort_by_key(ToString::to_string);
    assert_eq!(ft, tt, "flat vs tree CONSTRUCT bag divergence on `{query}`");
    tt
}

/// The spareval oracle's CONSTRUCT output graph (a SET) over the materialized graph.
fn oracle_construct(create: &str, r2rml: &str, query: &str) -> Dataset {
    match oracle::evaluate(&materialized(create, r2rml), query).expect("oracle eval") {
        OracleAnswer::Graph(g) => *g,
        other => panic!("expected Graph, got {other:?}"),
    }
}

/// Flat engine's own SELECT bag (not gated on flat==tree). Used by the REFUTED
/// repros below, where the FLAT engine is the wrong side and `diff`'s flat==tree
/// gate would mask the specific wrong count.
fn flat_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = parse(query);
    let fp = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("flat translates");
    run_select(&fp, &conn)
}

// ############################################################################
// S1 — D2 cross-arm injective-but-DIFFERENT reconstruction
// ############################################################################
//
// Two candidate maps for ONE pattern, each individually injective, subject
// templates NOT provably disjoint (shared literal prefix `http://ex/a`), but a
// DIFFERENT shape (`http://ex/a/{x}` vs `http://ex/a{y}`). A raw-column UNION
// would dedup on the raw key value, which is NOT term equality. The claim: the
// `pool_pattern_relation` cross-arm reconstruction-agreement gate refuses (sound
// 501) because the remapped templates keep their (different) literal segments.

const S1_SHAPE_SQL: &str = r#"
CREATE TABLE s1a (k TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO s1a VALUES ('5', 'X');
CREATE TABLE s1b (k TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO s1b VALUES ('5', 'Y');
"#;

const S1_SHAPE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#A>
    rr:logicalTable [ rr:tableName "s1a" ] ;
    rr:subjectMap [ rr:template "http://ex/a/{k}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
<#B>
    rr:logicalTable [ rr:tableName "s1b" ] ;
    rr:subjectMap [ rr:template "http://ex/a{k}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// Raw keys COLLIDE (both `'5'`) but the arm subjects render to DIFFERENT terms
/// (`http://ex/a/5` vs `http://ex/a5`). A raw UNION-dedup would UNDER-count
/// (collapse two distinct subjects). The agreement gate must sound-501 on both
/// engines rather than pool a raw UNION.
#[test]
fn s1_different_shape_raw_collision_sound_501() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:p ?o }}");
    let (f, t) = translate_both(S1_SHAPE_SQL, S1_SHAPE_R2RML, &q);
    assert!(is_sound_501(&f), "flat must sound-501, got {f:?}");
    assert!(is_sound_501(&t), "tree must sound-501, got {t:?}");
}

const S1_SAMETERM_SQL: &str = r#"
CREATE TABLE s1a (k TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO s1a VALUES ('5', 'X');
CREATE TABLE s1b (k TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO s1b VALUES ('/5', 'X');
"#;

/// The OTHER direction: DIFFERENT raw keys (`'5'` vs `'/5'`) render the SAME
/// term (`http://ex/a/5` via `http://ex/a/{k}` and via `http://ex/a{k}`) with
/// the SAME object — one genuine triple. A raw UNION-dedup would OVER-count
/// (keep two rows for one triple). Same agreement-gate refusal expected.
#[test]
fn s1_different_shape_same_term_sound_501() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:p ?o }}");
    let (f, t) = translate_both(S1_SAMETERM_SQL, S1_SAMETERM_R2RML, &q);
    assert!(is_sound_501(&f), "flat must sound-501, got {f:?}");
    assert!(is_sound_501(&t), "tree must sound-501, got {t:?}");
}

const S1_SAMETERM_R2RML: &str = S1_SHAPE_R2RML;

const S1_DTYPE_SQL: &str = r#"
CREATE TABLE s1p (id TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO s1p VALUES ('1', '5');
CREATE TABLE s1q (id TEXT NOT NULL, v TEXT NOT NULL);
INSERT INTO s1q VALUES ('1', '5');
"#;

/// Object-position spec mismatch: SAME subject template (arms NOT disjoint),
/// same raw object lexical `'5'`, but arm A's object is a plain literal and arm
/// B's is `xsd:integer` — `"5"` and `"5"^^xsd:integer` are DIFFERENT RDF terms.
/// A raw-column UNION would dedup them; the reconstruction-agreement gate must
/// see the differing `TermSpec` and sound-501.
#[test]
fn s1_object_datatype_mismatch_sound_501() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:p ?o }}");
    let (f, t) = translate_both(S1_DTYPE_SQL, S1_DTYPE_R2RML, &q);
    assert!(is_sound_501(&f), "flat must sound-501, got {f:?}");
    assert!(is_sound_501(&t), "tree must sound-501, got {t:?}");
}

const S1_DTYPE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<#A>
    rr:logicalTable [ rr:tableName "s1p" ] ;
    rr:subjectMap [ rr:template "http://ex/s/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
<#B>
    rr:logicalTable [ rr:tableName "s1q" ] ;
    rr:subjectMap [ rr:template "http://ex/s/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ;
        rr:objectMap [ rr:column "v" ; rr:datatype xsd:integer ] ] .
"#;

// ############################################################################
// S2 — the union-composite-key coverage proof
// ############################################################################

const S2_NONINJ_SQL: &str = r#"
CREATE TABLE s2t (a TEXT NOT NULL, b TEXT NOT NULL, v TEXT NOT NULL, PRIMARY KEY (a, b));
INSERT INTO s2t VALUES ('1', '23', 'X');
INSERT INTO s2t VALUES ('12', '3', 'X');
"#;

const S2_NONINJ_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#T>
    rr:logicalTable [ rr:tableName "s2t" ] ;
    rr:subjectMap [ rr:template "http://ex/{a}{b}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// (a) A NON-injective subject template (`{a}{b}`, adjacent slots) whose columns
/// WOULD complete the composite PK `(a,b)` must NOT count toward coverage
/// (`table_key_covered_by_bindings` filters by `binding_is_injective`). The two
/// PK-distinct rows `(1,23)`/`(12,3)` both render `http://ex/123` with object
/// `'X'` — ONE genuine triple that the graph-set holds once. D1 therefore must
/// force a DISTINCT; but a DISTINCT over a non-injective term cannot be pushed
/// to SQL soundly (ADR-0025 C.3), so the honest outcome is a sound 501 on both
/// engines. If elision WRONGLY counted the non-injective binding, no DISTINCT
/// would fire and the engine would return 2 rows (the oracle says 1) — a silent
/// wrong answer this test would catch as a NON-501.
#[test]
fn s2a_noninjective_binding_does_not_cover_key_sound_501() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:p ?o }}");
    let (f, t) = translate_both(S2_NONINJ_SQL, S2_NONINJ_R2RML, &q);
    assert!(is_sound_501(&f), "flat must sound-501, got {f:?}");
    assert!(is_sound_501(&t), "tree must sound-501, got {t:?}");
    // The oracle proves dedup was genuinely required (the collision is real): a
    // single triple, not two.
    let oracle = oracle_bag(S2_NONINJ_SQL, S2_NONINJ_R2RML, &q);
    assert_eq!(
        oracle.len(),
        1,
        "collision must be a single triple: {oracle:#?}"
    );
}

const S2_NULLABLE_UNIQUE_SQL: &str = r#"
CREATE TABLE s2u (uk INTEGER UNIQUE, v TEXT NOT NULL);
INSERT INTO s2u VALUES (1, 'a');
INSERT INTO s2u VALUES (2, 'b');
"#;

const S2_NULLABLE_UNIQUE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#U>
    rr:logicalTable [ rr:tableName "s2u" ] ;
    rr:subjectMap [ rr:template "http://ex/u/{uk}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// (b) The ONLY declared key is a NULLABLE `UNIQUE` column (`uk`). SQLite lets
/// several NULL rows coexist under `UNIQUE`, so `uk` is not a true key —
/// `key_is_non_null` must drop it from the elision key set, leaving D1 to force
/// a DISTINCT. Asserted on the emitted SQL shape (the answer alone can't tell:
/// the sample data has no NULLs, so the DISTINCT is a no-op here). If the guard
/// were absent, `uk` would count and the SQL would elide the DISTINCT.
#[test]
fn s2b_nullable_unique_does_not_elide_distinct() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:p ?o }}");
    let (f, t) = translate_both(S2_NULLABLE_UNIQUE_SQL, S2_NULLABLE_UNIQUE_R2RML, &q);
    for (label, r) in [("flat", &f), ("tree", &t)] {
        let plan = r.as_ref().expect("translates");
        let has_distinct = engine_sqls(plan)
            .iter()
            .any(|s| s.to_uppercase().contains("DISTINCT"));
        assert!(
            has_distinct,
            "{label}: a nullable-UNIQUE-only table must keep D1's DISTINCT (not elide it)"
        );
    }
    // And the answer is still correct (2 distinct subjects).
    let rows = assert_oracle_agrees(S2_NULLABLE_UNIQUE_SQL, S2_NULLABLE_UNIQUE_R2RML, &q);
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
}

const S2_PARTIAL_PK_SQL: &str = r#"
CREATE TABLE s2p (a TEXT NOT NULL, b TEXT NOT NULL, v TEXT NOT NULL, PRIMARY KEY (a, b));
INSERT INTO s2p VALUES ('1', '10', 'x');
INSERT INTO s2p VALUES ('1', '20', 'x');
"#;

const S2_PARTIAL_PK_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "s2p" ] ;
    rr:subjectMap [ rr:template "http://ex/p/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// (c-i) PARTIAL composite-key coverage: the subject reads only `a` of PK
/// `(a,b)`, the object reads `v`. `covered = {a,v}` does NOT include `b`, so the
/// key is NOT covered and D1 must fire. The two rows `(1,10,x)`/`(1,20,x)` both
/// render `http://ex/p/1 ex:q x` — ONE triple — because the distinguishing `b`
/// is not read; D1's dedup is genuinely required. Oracle says 1; a missed D1
/// would return 2.
#[test]
fn s2c_partial_composite_key_coverage_dedups() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:q ?o }}");
    let rows = assert_oracle_agrees(S2_PARTIAL_PK_SQL, S2_PARTIAL_PK_R2RML, &q);
    assert_eq!(rows.len(), 1, "partial-key coverage must dedup: {rows:#?}");
}

const S2_CROSS_ALIAS_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#M1>
    rr:logicalTable [ rr:tableName "s2p" ] ;
    rr:subjectMap [ rr:template "http://ex/p/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "b" ] ] .
<#M2>
    rr:logicalTable [ rr:tableName "s2p" ] ;
    rr:subjectMap [ rr:template "http://ex/p/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// (c-ii) CROSS-ALIAS non-combination: a join of two scans of the same table.
/// Alias-0 (`M1`, reads `a`,`b`) DOES cover PK `(a,b)`. Alias-1 (`M2`, reads
/// `a`,`v`) does NOT (missing `b`). The proof is per-alias, so alias-1's gap
/// forces D1 — alias-0's `b` must NOT be borrowed to "complete" alias-1's key.
/// Correctness bites: `M2`'s `?s ex:q ?v` over `(1,10,x)`/`(1,20,x)` maps both
/// rows to the single triple `http://ex/p/1 ex:q x`, so the join yields 2
/// solutions (`?b`∈{10,20}); a cross-alias mis-elision would leave `M2`
/// un-deduped and inflate to 4.
#[test]
fn s2c_cross_alias_key_columns_do_not_combine() {
    let q = format!("{PFX} SELECT ?s ?b ?v WHERE {{ ?s ex:p ?b . ?s ex:q ?v }}");
    let rows = assert_oracle_agrees(S2_PARTIAL_PK_SQL, S2_CROSS_ALIAS_R2RML, &q);
    assert_eq!(
        rows.len(),
        2,
        "cross-alias must not fake-cover the key: {rows:#?}"
    );
}

// ############################################################################
// S3 — dedup must NOT over-fire
// ############################################################################

const S3_KEYED_SQL: &str = r#"
CREATE TABLE s3e (id INTEGER PRIMARY KEY, v TEXT NOT NULL);
INSERT INTO s3e VALUES (1, 'a');
INSERT INTO s3e VALUES (2, 'b');
"#;

const S3_KEYED_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#E>
    rr:logicalTable [ rr:tableName "s3e" ] ;
    rr:subjectMap [ rr:template "http://ex/e/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "v" ] ] .
"#;

/// (a) A UNION of two IDENTICAL BGPs: D1 dedups WITHIN each arm, but the algebra
/// above (bag UNION) must preserve BOTH arms — the result is 2× the rows, never
/// collapsed to one. D1 must not reach across the union seam.
#[test]
fn s3a_union_of_identical_bgps_keeps_both_arms() {
    let q = format!("{PFX} SELECT ?x ?y WHERE {{ {{ ?x ex:p ?y }} UNION {{ ?x ex:p ?y }} }}");
    let rows = assert_oracle_agrees(S3_KEYED_SQL, S3_KEYED_R2RML, &q);
    assert_eq!(rows.len(), 4, "2 rows × 2 identical arms = 4: {rows:#?}");
}

const S3_JOIN_SQL: &str = r#"
CREATE TABLE s3p (s INTEGER NOT NULL, y TEXT NOT NULL);
INSERT INTO s3p VALUES (1, 'y1');
INSERT INTO s3p VALUES (1, 'y2');
CREATE TABLE s3q (s INTEGER NOT NULL, z TEXT NOT NULL);
INSERT INTO s3q VALUES (1, 'z1');
INSERT INTO s3q VALUES (1, 'z2');
"#;

const S3_JOIN_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "s3p" ] ;
    rr:subjectMap [ rr:template "http://ex/s/{s}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "y" ] ] .
<#Q>
    rr:logicalTable [ rr:tableName "s3q" ] ;
    rr:subjectMap [ rr:template "http://ex/s/{s}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column "z" ] ] .
"#;

/// (b) Join-produced LEGITIMATE duplicates: `?x` joins to 2 `?y` × 2 `?z` = 4
/// distinct solution mappings; projecting to just `?x` yields 4 identical `?x`
/// rows that MUST survive (SPARQL projects a bag; dedup is not at the final
/// result). D1 (below-projection, over the full bindings) must not collapse them.
///
/// BUG (REFUTED, flat engine): flat returns 1, oracle+tree return 4. Root cause
/// in the emitted flat SQL:
///   `SELECT DISTINCT t0."s" AS c0 FROM "s3p" t0 CROSS JOIN "s3q" t1 WHERE ...`
/// Both unkeyed patterns get D1's `Branch::distinct = true`; `unfold::merge`
/// OR-folds the flags onto the joined branch; then `cascade::run`'s projection
/// shrinking (pass 7) narrows the branch's bindings to just `?x` — so the single
/// branch-level DISTINCT emits as a DISTINCT over ONLY the projected `?x`, i.e.
/// exactly the "final result" dedup the ADR's Decision forbids ("projection/UNION
/// above the BGP create *legitimate* duplicates that must survive"). The flat
/// engine's coarse per-branch DISTINCT cannot express D1's required per-relation
/// dedup; the tree engine emits `SELECT DISTINCT` per derived relation and is
/// correct (4). A silent wrong SELECT count on a very common shape
/// (`SELECT ?x WHERE { ?x :p ?y . ?x :q ?z }` over multi-valued unkeyed sources).
#[test]
fn s3b_join_projection_duplicates_survive() {
    let q = format!("{PFX} SELECT ?x WHERE {{ ?x ex:p ?y . ?x ex:q ?z }}");
    let flat = flat_bag(S3_JOIN_SQL, S3_JOIN_R2RML, &q);
    let oracle = oracle_bag(S3_JOIN_SQL, S3_JOIN_R2RML, &q);
    assert_eq!(oracle.len(), 4, "oracle sanity: bag union of the join = 4");
    assert_eq!(
        flat.len(),
        oracle.len(),
        "BUG: flat D1 DISTINCT collapses legitimate projected duplicates \
         (flat={}, oracle/tree=4)",
        flat.len()
    );
}

const S3_VIEW_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#V>
    rr:logicalTable [ rr:sqlQuery "SELECT v FROM s3e" ] ;
    rr:subjectMap [ rr:template "http://ex/v/{v}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:has ; rr:objectMap [ rr:constant "yes" ] ] .
"#;

const S3_VIEW_SQL: &str = r#"
CREATE TABLE s3e (id INTEGER PRIMARY KEY, v TEXT NOT NULL);
INSERT INTO s3e VALUES (1, 'dup');
INSERT INTO s3e VALUES (2, 'dup');
"#;

/// (c) An `rr:sqlQuery` VIEW source has no `TableSchema` entry, so D1 can prove
/// no key: it is conservatively treated as never-covered → always DISTINCT. Two
/// `v='dup'` rows collapse to the single subject `http://ex/v/dup`. Must be
/// correct vs the oracle (and either engine may sound-501, in which case both
/// must — `diff` enforces that).
#[test]
fn s3c_view_source_always_distinct_correct() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s ex:has ?o }}");
    let rows = assert_oracle_agrees(S3_VIEW_SQL, S3_VIEW_R2RML, &q);
    assert_eq!(rows.len(), 1, "view rows dedup to one subject: {rows:#?}");
}

const S3_DUP_SQL: &str = r#"
CREATE TABLE s3d (x TEXT NOT NULL, y TEXT NOT NULL);
INSERT INTO s3d VALUES ('1', 'a');
INSERT INTO s3d VALUES ('1', 'a');
INSERT INTO s3d VALUES ('2', 'b');
"#;

const S3_DUP_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#D>
    rr:logicalTable [ rr:tableName "s3d" ] ;
    rr:subjectMap [ rr:template "http://ex/x/{x}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "y" ] ] .
"#;

/// (d) A user-level `SELECT DISTINCT` over a duplicate-carrying, unkeyed source:
/// D1 already dedups the BGP; the outer DISTINCT is idempotent and must not
/// double-dedup into a WRONG answer (2 distinct rows either way). A plain
/// non-DISTINCT companion pins the same 2-row deduped answer.
#[test]
fn s3d_user_distinct_over_dup_source_idempotent() {
    let d = format!("{PFX} SELECT DISTINCT ?x ?y WHERE {{ ?x ex:p ?y }}");
    let p = format!("{PFX} SELECT ?x ?y WHERE {{ ?x ex:p ?y }}");
    let dr = assert_oracle_agrees(S3_DUP_SQL, S3_DUP_R2RML, &d);
    let pr = assert_oracle_agrees(S3_DUP_SQL, S3_DUP_R2RML, &p);
    assert_eq!(dr.len(), 2, "DISTINCT rows={dr:#?}");
    assert_eq!(pr.len(), 2, "plain (D1-deduped) rows={pr:#?}");
}

// ############################################################################
// S4 — exemption boundaries (EXISTS / MINUS / subquery)
// ############################################################################

const S4_SQL: &str = r#"
CREATE TABLE s4a (s TEXT NOT NULL, y TEXT NOT NULL);
INSERT INTO s4a VALUES ('1', 'a');
INSERT INTO s4a VALUES ('2', 'c');
CREATE TABLE s4b (s TEXT NOT NULL, z TEXT NOT NULL);
INSERT INTO s4b VALUES ('1', 'b');
INSERT INTO s4b VALUES ('1', 'b');
"#;

const S4_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#A>
    rr:logicalTable [ rr:tableName "s4a" ] ;
    rr:subjectMap [ rr:template "http://ex/s/{s}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "y" ] ] .
<#B>
    rr:logicalTable [ rr:tableName "s4b" ] ;
    rr:subjectMap [ rr:template "http://ex/s/{s}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column "z" ] ] .
"#;

/// (a) `FILTER EXISTS { <duplicate-carrying pattern> }`: `s4b` has a duplicate
/// row, but existence is duplicate-insensitive (§8.3). The answer must equal the
/// oracle whether or not the body deduped — and flat/tree must agree (the tree
/// engine exempts the body via `in_existential`; if the flat engine forces D1
/// there and trips the SubPlan-in-correlated-subquery 501 boundary, `diff`
/// catches the divergence).
#[test]
fn s4a_exists_body_duplicate_pattern_existence_unchanged() {
    let q = format!("{PFX} SELECT ?x ?y WHERE {{ ?x ex:p ?y . FILTER EXISTS {{ ?x ex:q ?z }} }}");
    let rows = assert_oracle_agrees(S4_SQL, S4_R2RML, &q);
    assert_eq!(rows.len(), 1, "only s=1 has a q-edge: {rows:#?}");
}

/// (b) `MINUS { <duplicate-carrying pattern> }`: the anti-join is
/// duplicate-insensitive (§18.4) — s=1 is removed regardless of `s4b`'s
/// duplicate. Must match the oracle; flat/tree must agree.
#[test]
fn s4b_minus_right_duplicates_antijoin_unchanged() {
    let q = format!("{PFX} SELECT ?x ?y WHERE {{ ?x ex:p ?y . MINUS {{ ?x ex:q ?z }} }}");
    let rows = assert_oracle_agrees(S4_SQL, S4_R2RML, &q);
    assert_eq!(rows.len(), 1, "s=1 removed, only s=2 remains: {rows:#?}");
}

/// (c) The SAME duplicate-carrying pattern (`?x ex:q ?z`) both PROJECTED
/// (outer) and inside an EXISTS: D1 must dedup the projected occurrence while
/// the EXISTS body stays exempt — no cross-contamination of the exemption. `s4b`
/// has one distinct triple (s=1, z=b) despite the duplicate row.
#[test]
fn s4c_same_pattern_inside_and_outside_exists() {
    let q = format!("{PFX} SELECT ?x ?z WHERE {{ ?x ex:q ?z . FILTER EXISTS {{ ?x ex:q ?z2 }} }}");
    let rows = assert_oracle_agrees(S4_SQL, S4_R2RML, &q);
    assert_eq!(rows.len(), 1, "the projected q-edge dedups to 1: {rows:#?}");
}

/// (d) A projected SUBQUERY (`{ SELECT ?x ?z WHERE { <dup pattern> } }`) is NOT
/// an existence context — its result IS projected, so D1 must fire on it. The
/// exemption must not leak from the EXISTS machinery to an ordinary nested
/// SELECT. The duplicate `s4b` row must dedup to 1.
#[test]
fn s4d_projected_subquery_dup_bgp_not_exempt() {
    let q = format!("{PFX} SELECT ?x ?z WHERE {{ {{ SELECT ?x ?z WHERE {{ ?x ex:q ?z }} }} }}");
    let rows = assert_oracle_agrees(S4_SQL, S4_R2RML, &q);
    assert_eq!(
        rows.len(),
        1,
        "the subquery's dup row must dedup: {rows:#?}"
    );
}

// ############################################################################
// S5 — NPS (negated property set) × D1
// ############################################################################

const S5_PLAIN_SQL: &str = r#"
CREATE TABLE s5e (s TEXT NOT NULL, o TEXT NOT NULL);
INSERT INTO s5e VALUES ('1', '2');
INSERT INTO s5e VALUES ('1', '2');
"#;

const S5_PLAIN_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#E>
    rr:logicalTable [ rr:tableName "s5e" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{s}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:a ; rr:objectMap [ rr:template "http://ex/n/{o}" ] ] .
"#;

/// A PLAIN NPS `?s !ex:other ?o` (the complement is `{ex:a}`) over a source with
/// a DUPLICATE ROW. The materialized graph is a SET: the triple `n/1 ex:a n/2`
/// exists ONCE, so §18.2.2 ("one solution per matching triple") yields ONE
/// solution. If the NPS realization scans the duplicate rows without deduping
/// the underlying triple, it returns 2 — a wrong answer. Run vs oracle.
#[test]
fn s5_plain_nps_over_duplicate_rows_yields_one() {
    let q = format!("{PFX} SELECT ?s ?o WHERE {{ ?s !ex:other ?o }}");
    let rows = assert_oracle_agrees(S5_PLAIN_SQL, S5_PLAIN_R2RML, &q);
    assert_eq!(
        rows.len(),
        1,
        "the set-graph holds the triple once: {rows:#?}"
    );
}

const S5_JOIN_SQL: &str = r#"
CREATE TABLE s5edge (s TEXT NOT NULL, o TEXT NOT NULL);
INSERT INTO s5edge VALUES ('1', '2');
CREATE TABLE s5edge2 (s TEXT NOT NULL, o TEXT NOT NULL);
INSERT INTO s5edge2 VALUES ('1', '2');
CREATE TABLE s5name (id TEXT NOT NULL, nm TEXT NOT NULL);
INSERT INTO s5name VALUES ('2', 'Bob');
INSERT INTO s5name VALUES ('2', 'Bob');
"#;

const S5_JOIN_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#E1>
    rr:logicalTable [ rr:tableName "s5edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{s}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:a ; rr:objectMap [ rr:template "http://ex/n/{o}" ] ] .
<#E2>
    rr:logicalTable [ rr:tableName "s5edge2" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{s}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:b ; rr:objectMap [ rr:template "http://ex/n/{o}" ] ] .
<#N>
    rr:logicalTable [ rr:tableName "s5name" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "nm" ] ] .
"#;

/// NPS legitimate bag multiplicity × D1. `n/1` reaches `n/2` via TWO different
/// predicates (`ex:a`, `ex:b`), so `?s !ex:name ?o` matches TWO distinct triples
/// → 2 solutions (a genuine NPS bag, protected by `Branch.nps`). Joined with
/// `?o ex:name ?nm` from an UNKEYED, duplicate-carrying `s5name` table, whose D1
/// dedup must collapse its own duplicate to one. Correct product = 2 (2 NPS
/// hops × 1 deduped name).
///
/// BUG (REFUTED, flat engine): flat returns 4, oracle+tree return 2. The emitted
/// flat SQL cross-joins `s5name` RAW (no dedup):
///   `... t0 CROSS JOIN "s5name" t1 WHERE t1."nm" IS NOT NULL AND t0."sf_o" = t1."id"`
/// The name branch DID earn `Branch::distinct = true` from its own
/// `force_distinct_for_dup_safety`, but `unfold::merge`'s NPS guard
/// (`if !left.nps && !right.nps { left.distinct |= right.distinct }`) DROPS it
/// when folding into the NPS-carrying accumulator — so the sibling table's
/// LEGITIMATE D1 dedup is lost, contradicting the ADR's own "D1 still applies to
/// its underlying scans". The tree engine dedups `s5name` as its own derived
/// table (`SELECT DISTINCT t1."nm", t1."id" FROM "s5name" ...`) and is correct
/// (2). The flat single-DISTINCT-per-branch model cannot dedup one join input
/// while preserving another's (NPS) bag multiplicity; the flag-drop yields the
/// raw over-counted bag. (The answer is also order-dependent: swapping the two
/// patterns instead lets the DISTINCT survive and collapse the NPS bag to 1 —
/// wrong the other way — a second symptom of the same coarse-flag defect.)
#[test]
fn s5_nps_bag_multiplicity_joined_with_unkeyed_dup_table() {
    let q = format!("{PFX} SELECT ?s ?o ?nm WHERE {{ ?s !ex:name ?o . ?o ex:name ?nm }}");
    let flat = flat_bag(S5_JOIN_SQL, S5_JOIN_R2RML, &q);
    let oracle = oracle_bag(S5_JOIN_SQL, S5_JOIN_R2RML, &q);
    assert_eq!(
        oracle.len(),
        2,
        "oracle sanity: 2 NPS hops × 1 deduped name = 2"
    );
    assert_eq!(
        flat.len(),
        oracle.len(),
        "BUG: flat drops the joined table's D1 dedup across an NPS merge \
         (flat={}, oracle/tree=2)",
        flat.len()
    );
}

// ############################################################################
// S6 — CONSTRUCT over duplicate-carrying sources (§16.2: output is a SET)
// ############################################################################

const S6_SQL: &str = r#"
CREATE TABLE s6p (pid TEXT NOT NULL, age TEXT NOT NULL);
INSERT INTO s6p VALUES ('1', '30');
INSERT INTO s6p VALUES ('1', '40');
INSERT INTO s6p VALUES ('2', '50');
"#;

const S6_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "s6p" ] ;
    rr:subjectMap [ rr:template "http://ex/p/{pid}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rr:column "age" ] ] .
"#;

/// A CONSTRUCT template that PROJECTS AWAY the distinguishing `?a`: person 1 has
/// two distinct ages, so the WHERE BGP yields 2 distinct solutions for `?p=p/1`
/// (D1 keeps them — the pair `(p,a)` is distinct), but the template
/// `{ ?p ex:type ex:Person }` maps both to the SAME triple. §16.2 says the
/// output graph is a SET → `http://ex/p/1 ex:type ex:Person` appears ONCE.
///
/// BUG (REFUTED, §16.2 — both engines, low severity): the produced triple BAG
/// over-counts (engine bag = 3: `p/1 type Person` TWICE + `p/2 type Person`;
/// oracle set = 2). The set-isomorphism assertion PASSES (the RDF graph is
/// correct as a set) — only the §16.2 "output is a SET" bag-count fails, because
/// `exec_core::construct` streams one triple per solution with no set-union
/// dedup. D1/D2 dedup the WHERE BGP, but a template that projects a
/// distinguishing variable away is a SEPARATE §16.2 concern the ADR's own
/// construct cell (`duplicate_source_row_construct_round_trip_bag_multiplicity`,
/// all-vars-kept) never exercised. Likely PRE-EXISTING (construct never
/// deduped), not introduced by this commit — but a real spec nonconformance: an
/// N-Triples serialization of this bag emits the duplicate line. Fix: term-level
/// dedup of the produced triple stream (or at the serialization boundary).
#[test]
fn s6_construct_template_drops_var_output_is_a_set() {
    let q = format!("{PFX} CONSTRUCT {{ ?p ex:type ex:Person }} WHERE {{ ?p ex:age ?a }}");
    let engine = construct_bag(S6_SQL, S6_R2RML, &q);
    let oracle_graph = oracle_construct(S6_SQL, S6_R2RML, &q);
    // Set-correctness: the deduped engine graph is isomorphic to the oracle.
    let engine_graph = graph::triples_to_dataset(&engine);
    assert!(
        graph::isomorphic(&engine_graph, &oracle_graph),
        "CONSTRUCT set divergence:\n engine set={engine_graph:?}\n oracle={oracle_graph:?}"
    );
    // §16.2 bag-count claim (the ADR's own `engine.len() == oracle_graph.len()`
    // bar): p/1 type Person + p/2 type Person = 2 triples in the SET.
    assert_eq!(
        engine.len(),
        oracle_graph.len(),
        "§16.2: CONSTRUCT output is a SET — the engine's triple bag must not \
         carry the projected-away duplicate:\n engine bag={engine:#?}\n oracle set len={}",
        oracle_graph.len()
    );
}

/// Positive control: a CONSTRUCT template that KEEPS both BGP variables. Here
/// D1's BGP-level dedup already suffices (the literal duplicate row collapses),
/// so the bag and the set agree — confirming the harness distinguishes the two
/// mechanisms rather than always passing/failing.
#[test]
fn s6_construct_template_keeps_vars_control() {
    let sql = r#"
CREATE TABLE s6c (pid TEXT NOT NULL, age TEXT NOT NULL);
INSERT INTO s6c VALUES ('1', '30');
INSERT INTO s6c VALUES ('1', '30');
INSERT INTO s6c VALUES ('2', '50');
"#;
    let r2rml = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "s6c" ] ;
    rr:subjectMap [ rr:template "http://ex/p/{pid}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rr:column "age" ] ] .
"#;
    let q = format!("{PFX} CONSTRUCT {{ ?p ex:age ?a }} WHERE {{ ?p ex:age ?a }}");
    let engine = construct_bag(sql, r2rml, &q);
    let oracle_graph = oracle_construct(sql, r2rml, &q);
    let engine_graph = graph::triples_to_dataset(&engine);
    assert!(
        graph::isomorphic(&engine_graph, &oracle_graph),
        "control CONSTRUCT set divergence"
    );
    assert_eq!(
        engine.len(),
        oracle_graph.len(),
        "control bag==set (D1 suffices)"
    );
}
