//! Cross-backend `=_bag` differential (ADR-0006 / ADR-0012): the SAME OBDA query
//! run through the **sync SQLite** executor ([`sf_sparql::exec::select`] / `ask`)
//! and the **async PostgreSQL** executor ([`sf_sparql::exec_pg::select_pg`] /
//! `ask_pg`) over the SAME hand-built dataset must produce IDENTICAL binding bags
//! / boolean answers. Two independent executors (blocking `rusqlite` cursor vs
//! `tokio-postgres` server-side `query_raw` cursor), one rewriter, one sf-core
//! term-gen reconstruction (ADR-0003 R3) — a divergence pinpoints a PG-path bug.
//!
//! The OBDA SELECT exercises a BGP + JOIN + FILTER + OPTIONAL (the NULL-safe LEFT
//! JOIN), plus an ASK case. Values stay bound parameters (ADR-0010); both paths
//! introspect the live schema so the identifier case-folding resolution
//! (`build_catalog` / `build_catalog_pg`) applies on each side.
//!
//! **Graceful skip (CI):** like the live-PG conformance suite, this probes the
//! connection and returns early (the test passes as a no-op) when no PostgreSQL
//! server is reachable — point it at one with `SF_PG_URL` (host/user, no dbname).

use rusqlite::Connection;
use sf_conformance::oracle::{engine_bag, solutions_bag_eq};
use sf_conformance::sqlite;
use sf_sparql::{exec, exec_mysql, exec_pg, parse_and_translate_with, Tbox};
use sf_sql::introspect::introspect_postgres;
use sf_sql::{Dialect, TableSchema};
use tokio_postgres::{Client, NoTls};

/// Schema-/dialect-neutral fixture: `dept`(PK `id`) ⟕ `person`(PK `id`, NOT-NULL
/// FK `dept_id`, nullable `email`). All identifiers lowercase/unquoted so they
/// load identically into SQLite and PostgreSQL. Bob's NULL email exercises the
/// OPTIONAL's NULL-safe LEFT JOIN; Zed is removed by the FILTER.
const CREATE_SQL: &str = r#"
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

const R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name  ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:email ; rr:objectMap [ rr:column "email" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:deptId ; rr:objectMap [ rr:column "dept_id" ] ] ;
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

/// BGP + JOIN + FILTER + OPTIONAL — the supported-surface shape (ADR-0007).
const SELECT_Q: &str = r#"
    PREFIX ex: <http://ex/>
    SELECT ?name ?label ?email WHERE {
        ?p ex:name ?name .
        ?p ex:dept ?d .
        ?d ex:label ?label .
        OPTIONAL { ?p ex:email ?email }
        FILTER (?name != "Zed")
    }"#;

/// ASK over the same BGP/JOIN/FILTER (Ann is in Sales, name != "Zed") ⇒ true.
const ASK_TRUE_Q: &str = r#"
    PREFIX ex: <http://ex/>
    ASK { ?p ex:name ?name . ?p ex:dept ?d . ?d ex:label "Sales" . FILTER (?name = "Ann") }"#;

/// ASK that matches nothing (no person named "Nobody") ⇒ false.
const ASK_FALSE_Q: &str = r#"
    PREFIX ex: <http://ex/>
    ASK { ?p ex:name "Nobody" }"#;

/// PG-path regression guard (ADR-0023): each previously **SQLite-green but
/// PostgreSQL-broken** feature class, as one `=_bag` query over the SAME fixture.
/// The bug pattern was a blind spot — the SQLite differential passed while the live
/// PG execution/lowering path aborted, silently emptied, or dropped DISTINCT. Here
/// the always-run SQLite arm is the oracle and the live-PG arm must reproduce its
/// bag exactly, so a recurrence fails the gate. Each entry: (name, SPARQL, expected
/// SQLite/PG bag cardinality). Bob's NULL email (⇒ no `ex:email` triple) is what
/// makes EXISTS/MINUS/agg-over-UNION non-trivial.
///
/// * **agg-over-union** — COUNT over a UNION + GROUP BY. Was SF-501 (q9): the PG
///   core loop hard-errored on `rust_group`, aborting the response mid-stream.
///   name(3) + email(2) over dept "Sales" ⇒ one group, COUNT 5.
/// * **filter-exists** — FILTER EXISTS over a plain (text) column. Was SF-501 (q12):
///   the correlated sub-plan aborted on the PG bind path. Ann + Zed have email ⇒ 2.
/// * **typed-filter** — the *actual* q12 root cause: a FILTER constant compared to a
///   **typed INT4 column** (`FILTER(?di = 10)` over `dept_id`) lowers to a bare `$1`
///   placeholder PostgreSQL infers as INT4; binding the raw Rust `String` aborts the
///   PG stream mid-body (the `filter-exists` arm alone does NOT reproduce this — its
///   EXISTS is over text — so `LexicalParam` needs a typed-column arm to be guarded).
///   All 3 persons have dept_id 10 ⇒ 3.
/// * **minus** — MINUS removing the emailed persons. Was SF-EMPTY (q11): the PG
///   anti-join removed everything. Only Bob (NULL email) survives ⇒ 1.
/// * **sequence-path** — property path `ex:dept/ex:label`. Was SF-EMPTY (q10): the
///   sequence lowered to nothing on PG. All 3 persons reach "Sales" ⇒ 3.
/// * **distinct-join** — DISTINCT over a duplicate-producing join. Was MISMATCH
///   (q15): DISTINCT was dropped on PG, leaking duplicates. 3 persons, 1 dept ⇒ 1.
/// * **agg-over-union-count-distinct** — ADR-0023 optimizer-residue (q9
///   agg-pushdown, Wave A.1): `COUNT(DISTINCT ?v)` over a SELF-union (`ex:name`
///   unioned with itself) must SQL-pushdown `COUNT(DISTINCT col)` over the live
///   PG/MySQL `UNION ALL` — one group ("Sales"), row count 1 (the aggregate
///   VALUE, 3 not 6, is pinned by the live-PG value guard below).
/// * **agg-over-union-compound-key** — ADR-0023 optimizer-residue (Wave A.1): a
///   TWO-variable `GROUP BY ?label ?email` over a UNION must SQL-pushdown a
///   multi-column `GROUP BY` on the live PG/MySQL `UNION ALL`. Only Ann/Zed have
///   an email (Bob's NULL drops him from this pattern) ⇒ 2 groups (one per
///   person), each COUNT 2 (name + email).
/// * **group-d-fd-on-right** — ADR-0023 optimizer-residue (Wave B, closing the
///   SQLite-only adversarial-review caveat): DISTINCT over a right-nested
///   OPTIONAL (`L OPT (R1 OPT R2)`, Ontop's FDOnRight shape) — Group C's
///   decomposition must correctly close this on the LIVE PG/MySQL executors
///   (not just SQLite), including the correlated `NOT EXISTS` subquery the
///   right-nested decomposition emits. All 3 persons share dept "Sales" ⇒ 3
///   distinct (?name,?label) rows.
/// * **group-d-fd-simplification** — ADR-0023 optimizer-residue (Wave B, same
///   closing purpose): a right-nested OPTIONAL with a FILTER INSIDE the
///   OPTIONAL right (on the FD-determined `?label`) PLUS an outer ancestor
///   FILTER (Ontop's FDSimplification shape) — proves the live PG/MySQL
///   correlated-subquery FILTER placement (inside vs outside the `NOT EXISTS`)
///   is correct, not just the SQLite embedded engine's. No dept is labelled
///   "X" (inner FILTER a no-op); Bob is dropped by the outer FILTER ⇒ 2 rows
///   (Ann/Sales, Zed/Sales).
const FEATURE_QUERIES: &[(&str, &str, usize)] = &[
    (
        "agg-over-union",
        r#"PREFIX ex: <http://ex/>
           SELECT ?label (COUNT(?v) AS ?c) WHERE {
             ?p ex:dept ?d . ?d ex:label ?label .
             { ?p ex:name ?v } UNION { ?p ex:email ?v }
           } GROUP BY ?label"#,
        1,
    ),
    (
        "filter-exists",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name WHERE {
             ?p ex:name ?name .
             FILTER EXISTS { ?p ex:email ?e }
           }"#,
        2,
    ),
    (
        "typed-filter",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name WHERE {
             ?p ex:name ?name ; ex:deptId ?di .
             FILTER(?di = 10)
           }"#,
        3,
    ),
    (
        "minus",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name WHERE {
             ?p ex:name ?name .
             MINUS { ?p ex:email ?e }
           }"#,
        1,
    ),
    (
        "sequence-path",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name ?label WHERE {
             ?p ex:name ?name .
             ?p ex:dept/ex:label ?label
           }"#,
        3,
    ),
    (
        "distinct-join",
        r#"PREFIX ex: <http://ex/>
           SELECT DISTINCT ?label WHERE {
             ?p ex:dept ?d . ?d ex:label ?label
           }"#,
        1,
    ),
    (
        "agg-over-union-count-distinct",
        r#"PREFIX ex: <http://ex/>
           SELECT ?label (COUNT(DISTINCT ?v) AS ?c) WHERE {
             ?p ex:dept ?d . ?d ex:label ?label .
             { ?p ex:name ?v } UNION { ?p ex:name ?v }
           } GROUP BY ?label"#,
        1,
    ),
    (
        "agg-over-union-compound-key",
        r#"PREFIX ex: <http://ex/>
           SELECT ?label ?email (COUNT(?v) AS ?c) WHERE {
             ?p ex:dept ?d . ?d ex:label ?label . ?p ex:email ?email .
             { ?p ex:name ?v } UNION { ?p ex:email ?v }
           } GROUP BY ?label ?email"#,
        2,
    ),
    (
        "group-d-fd-on-right",
        r#"PREFIX ex: <http://ex/>
           SELECT DISTINCT ?name ?label WHERE {
             ?p ex:name ?name
             OPTIONAL {
               ?p ex:dept ?d . ?d ex:label ?label
               OPTIONAL { ?p ex:email ?email }
             }
           }"#,
        3,
    ),
    (
        "group-d-fd-simplification",
        r#"PREFIX ex: <http://ex/>
           SELECT ?name ?label WHERE {
             ?p ex:name ?name
             OPTIONAL { ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != "X") }
             FILTER(?name != "Bob")
           }"#,
        2,
    ),
];

/// Datatype-agnostic lexical form of a term (value only — no `^^<iri>`), so the
/// ADR-0024 M3 guards pin actual computed VALUES without brittleness to the exact
/// datatype IRI.
fn term_lex(t: &sf_core::Term) -> String {
    use sf_core::Term::*;
    match t {
        NamedNode(n) => n.as_str().to_owned(),
        BlankNode(b) => b.as_str().to_owned(),
        Literal(l) => l.value().to_owned(),
        // The rdf-star `Term::Triple` variant — not produced by these fixtures.
        _ => "<<triple>>".to_owned(),
    }
}

/// One arm's bag → sorted `Vec` of sorted `(var, value)` rows (order-independent
/// bag equality for the value-level guards).
fn bag_lex(
    bag: &[std::collections::BTreeMap<String, sf_core::Term>],
) -> Vec<Vec<(String, String)>> {
    let mut rows: Vec<Vec<(String, String)>> = bag
        .iter()
        .map(|r| {
            let mut v: Vec<_> = r.iter().map(|(k, t)| (k.clone(), term_lex(t))).collect();
            v.sort();
            v
        })
        .collect();
    rows.sort();
    rows
}

/// Base connection params (host/port/user, **no** dbname): `SF_PG_URL` if set,
/// else a local trust-auth default keyed on `$USER` (matches `pg.rs`).
fn base_conn() -> String {
    std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    })
}

/// Connect and spawn the driver task, returning the live client.
async fn connect(conn_str: &str) -> Result<Client, String> {
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

/// Introspect every base table in `public` (name order) over the live PG client —
/// the same schema set the SQLite side gets, so translation is symmetric.
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

/// SQLite side: load the fixture, introspect, translate (Sqlite), run SELECT/ASK,
/// plus each [`FEATURE_QUERIES`] entry as an `=_bag`-ready binding bag (aligned).
#[allow(clippy::type_complexity)]
fn sqlite_side() -> (
    Vec<Vec<Option<sf_core::Term>>>,
    Vec<String>,
    bool,
    bool,
    Vec<Vec<std::collections::BTreeMap<String, sf_core::Term>>>,
) {
    let conn: Connection = sqlite::load(CREATE_SQL).expect("sqlite fixture loads");
    let maps = sf_mapping::parse_r2rml(R2RML).expect("R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("sqlite introspection");

    let sel_plan =
        parse_and_translate_with(SELECT_Q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
            .expect("translate SELECT (sqlite)");
    let sols = exec::select(&sel_plan, &conn).expect("sqlite select");

    let ask_t = exec::ask(
        &parse_and_translate_with(
            ASK_TRUE_Q,
            &maps,
            Dialect::Sqlite,
            &Tbox::default(),
            &schema,
        )
        .expect("translate ASK-true (sqlite)"),
        &conn,
    )
    .expect("sqlite ask-true");
    let ask_f = exec::ask(
        &parse_and_translate_with(
            ASK_FALSE_Q,
            &maps,
            Dialect::Sqlite,
            &Tbox::default(),
            &schema,
        )
        .expect("translate ASK-false (sqlite)"),
        &conn,
    )
    .expect("sqlite ask-false");

    // Feature-class arms — the ADR-0023 PG-path regression guard (SQLite oracle).
    let features = FEATURE_QUERIES
        .iter()
        .map(|(name, q, _)| {
            let plan =
                parse_and_translate_with(q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
                    .unwrap_or_else(|e| panic!("translate {name} (sqlite): {e}"));
            let sols =
                exec::select(&plan, &conn).unwrap_or_else(|e| panic!("{name} (sqlite): {e}"));
            engine_bag(&sols)
        })
        .collect();

    (sols.rows, sols.vars, ask_t, ask_f, features)
}

/// PG side: recreate a clean `public` schema, load the SAME rows, introspect,
/// translate (Postgres), run SELECT/ASK over the live client via `select_pg` /
/// `ask_pg`, plus each [`FEATURE_QUERIES`] entry (tree path + Postgres lowering +
/// live `exec_pg` — the exact production surface sf-serve uses).
#[allow(clippy::type_complexity)]
async fn pg_side(
    client: &std::sync::Arc<Client>,
) -> (
    Vec<Vec<Option<sf_core::Term>>>,
    Vec<String>,
    bool,
    bool,
    Vec<Vec<std::collections::BTreeMap<String, sf_core::Term>>>,
) {
    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public; SET search_path TO public;",
        )
        .await
        .expect("reset public schema");
    client
        .batch_execute(CREATE_SQL)
        .await
        .expect("pg fixture loads");

    let maps = sf_mapping::parse_r2rml(R2RML).expect("R2RML parses");
    let schema = introspect_all_pg(client).await.expect("pg introspection");

    let sel_plan = parse_and_translate_with(
        SELECT_Q,
        &maps,
        Dialect::Postgres,
        &Tbox::default(),
        &schema,
    )
    .expect("translate SELECT (pg)");
    let sols = exec_pg::select_pg(&sel_plan, client)
        .await
        .expect("pg select");

    let ask_t = exec_pg::ask_pg(
        &parse_and_translate_with(
            ASK_TRUE_Q,
            &maps,
            Dialect::Postgres,
            &Tbox::default(),
            &schema,
        )
        .expect("translate ASK-true (pg)"),
        std::sync::Arc::clone(client),
    )
    .await
    .expect("pg ask-true");
    let ask_f = exec_pg::ask_pg(
        &parse_and_translate_with(
            ASK_FALSE_Q,
            &maps,
            Dialect::Postgres,
            &Tbox::default(),
            &schema,
        )
        .expect("translate ASK-false (pg)"),
        std::sync::Arc::clone(client),
    )
    .await
    .expect("pg ask-false");

    // Feature-class arms over the LIVE PostgreSQL cursor — the previously-broken set.
    let mut features = Vec::with_capacity(FEATURE_QUERIES.len());
    for (name, q, _) in FEATURE_QUERIES {
        let plan = parse_and_translate_with(q, &maps, Dialect::Postgres, &Tbox::default(), &schema)
            .unwrap_or_else(|e| panic!("translate {name} (pg): {e}"));
        let sols = exec_pg::select_pg(&plan, client)
            .await
            .unwrap_or_else(|e| panic!("{name} (pg): {e}"));
        features.push(engine_bag(&sols));
    }

    (sols.rows, sols.vars, ask_t, ask_f, features)
}

#[test]
fn select_and_ask_agree_across_sqlite_and_pg() {
    // SQLite arm (sync) — always runs.
    let (s_rows, s_vars, s_ask_t, s_ask_f, s_features) = sqlite_side();
    let s_sel = engine_bag(&exec::Solutions {
        vars: s_vars,
        rows: s_rows,
    });
    // Sanity on the SQLite arm before any cross-check (independent of PG).
    assert_eq!(s_sel.len(), 2, "Ann + Bob survive the FILTER (Zed removed)");
    assert!(s_ask_t, "ASK Ann-in-Sales is true on sqlite");
    assert!(!s_ask_f, "ASK Nobody is false on sqlite");
    // The SQLite oracle must itself match the hand-computed cardinality per class,
    // so a symmetric-but-wrong PG match cannot slip through.
    for ((name, _, want), bag) in FEATURE_QUERIES.iter().zip(&s_features) {
        assert_eq!(
            bag.len(),
            *want,
            "sqlite oracle cardinality wrong for {name}: {bag:#?}"
        );
    }

    // PG arm (async) — graceful skip if no server is reachable (CI stays green).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let base = base_conn();
        // Probe via the maintenance database; absence ⇒ graceful skip.
        let admin = match connect(&format!("{base} dbname=postgres")).await {
            Ok(c) => c,
            Err(_) => {
                eprintln!("no PostgreSQL server reachable — skipping PG differential");
                return;
            }
        };
        let dbname = format!("sf_diff_{}", std::process::id());
        admin
            .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
            .await
            .expect("drop pre-existing throwaway db");
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create throwaway db");

        let work = std::sync::Arc::new(
            connect(&format!("{base} dbname={dbname}"))
                .await
                .expect("connect work db"),
        );
        let (p_rows, p_vars, p_ask_t, p_ask_f, p_features) = pg_side(&work).await;
        drop(work);
        let _ = admin
            .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
            .await;

        let p_sel = engine_bag(&exec::Solutions {
            vars: p_vars,
            rows: p_rows,
        });

        // The differential: identical binding bags across the two executors.
        assert!(
            solutions_bag_eq(&s_sel, &p_sel),
            "SELECT diverges sqlite vs pg:\n sqlite={s_sel:#?}\n pg={p_sel:#?}"
        );
        // And identical ASK answers.
        assert_eq!(s_ask_t, p_ask_t, "ASK-true diverges sqlite vs pg");
        assert_eq!(s_ask_f, p_ask_f, "ASK-false diverges sqlite vs pg");

        // ADR-0023 PG-path regression guard: every previously SQLite-green /
        // PG-broken feature class must now be `=_bag` on the live PG path.
        for (((name, _, want), s_bag), p_bag) in
            FEATURE_QUERIES.iter().zip(&s_features).zip(&p_features)
        {
            assert!(
                solutions_bag_eq(s_bag, p_bag),
                "{name} diverges sqlite vs pg (PG-path regression):\n sqlite={s_bag:#?}\n pg={p_bag:#?}"
            );
            assert_eq!(
                p_bag.len(),
                *want,
                "{name} PG cardinality wrong (expected {want}): {p_bag:#?}"
            );
        }

        // --- ADR-0024 M3: hard live-PG value guards (revert-sensitive, oracle-independent) ---
        // Each pins the EXACT computed rows on the LIVE PG path (not just cardinality,
        // not just agreement with the SQLite oracle), so a regression to wrong/empty
        // output FAILS even if the SQLite oracle co-regressed identically. These run
        // only when a live PG server is reachable (the enclosing `rt.block_on` returns
        // early otherwise), matching the q12 `typed-filter` arm's live-only nature.
        let pg_bag = |name: &str| -> Vec<Vec<(String, String)>> {
            let i = FEATURE_QUERIES
                .iter()
                .position(|(n, _, _)| *n == name)
                .expect("arm exists");
            bag_lex(&p_features[i])
        };
        // q9 agg-over-UNION: COUNT(name ∪ email) over the single "Sales" group = 5
        // (3 names + 2 emails; Bob's NULL email drops). Reverting the PG rust_group
        // route (the deleted exec_pg loop) empties/errors this ⇒ guard fails.
        assert_eq!(
            pg_bag("agg-over-union"),
            vec![vec![
                ("c".to_owned(), "5".to_owned()),
                ("label".to_owned(), "Sales".to_owned())
            ]],
            "q9 agg-over-union live-PG value regression"
        );
        // q10 sequence path ex:dept/ex:label: all 3 persons reach "Sales". Reverting
        // the q10 lowering ⇒ empty bag.
        assert_eq!(
            pg_bag("sequence-path"),
            vec![
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Ann".to_owned())
                ],
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Bob".to_owned())
                ],
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Zed".to_owned())
                ],
            ],
            "q10 sequence-path live-PG value regression"
        );
        // q11 MINUS: only Bob (NULL email) survives. Reverting the PG anti-join ⇒
        // 0 rows (or all 3).
        assert_eq!(
            pg_bag("minus"),
            vec![vec![("name".to_owned(), "Bob".to_owned())]],
            "q11 minus live-PG value regression"
        );
        // q15 DISTINCT-over-join: 3 persons, 1 dept ⇒ one "Sales". Reverting the
        // DISTINCT dedup (now in exec_core, before the slice) ⇒ 3 duplicate rows.
        assert_eq!(
            pg_bag("distinct-join"),
            vec![vec![("label".to_owned(), "Sales".to_owned())]],
            "q15 distinct-join live-PG value regression"
        );
        // ADR-0023 optimizer-residue Wave A.1: COUNT(DISTINCT ?v) over a self-union
        // must dedupe Ann/Bob/Zed to 3, NOT double-count to 6 — pins the pushdown's
        // live-PG `COUNT(DISTINCT col)` SQL, not just that some row came back.
        assert_eq!(
            pg_bag("agg-over-union-count-distinct"),
            vec![vec![
                ("c".to_owned(), "3".to_owned()),
                ("label".to_owned(), "Sales".to_owned())
            ]],
            "agg-over-union-count-distinct live-PG value regression"
        );
        // ADR-0023 optimizer-residue Wave A.1: the 2-column GROUP BY ?label ?email
        // must keep Ann's and Zed's groups SEPARATE (COUNT 2 each: name+email), not
        // collapse them under the shared ?label — pins the pushdown's live-PG
        // multi-column `GROUP BY` SQL.
        assert_eq!(
            pg_bag("agg-over-union-compound-key"),
            vec![
                vec![
                    ("c".to_owned(), "2".to_owned()),
                    ("email".to_owned(), "ann@x".to_owned()),
                    ("label".to_owned(), "Sales".to_owned())
                ],
                vec![
                    ("c".to_owned(), "2".to_owned()),
                    ("email".to_owned(), "zed@x".to_owned()),
                    ("label".to_owned(), "Sales".to_owned())
                ],
            ],
            "agg-over-union-compound-key live-PG value regression"
        );
        // ADR-0023 optimizer-residue Wave B: FDOnRight on live PG. All 3 persons
        // share dept "Sales" -- a regression in the live NOT-EXISTS correlated
        // subquery (right-nested-OPTIONAL decomposition) would drop rows or
        // duplicate them under DISTINCT, not just return the right cardinality.
        assert_eq!(
            pg_bag("group-d-fd-on-right"),
            vec![
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Ann".to_owned())
                ],
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Bob".to_owned())
                ],
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Zed".to_owned())
                ],
            ],
            "group-d-fd-on-right live-PG value regression"
        );
        // ADR-0023 optimizer-residue Wave B: FDSimplification on live PG. Bob is
        // dropped by the ANCESTOR (outer) FILTER; the per-right (inner) FILTER on
        // ?label is a no-op (no dept named "X") -- a regression that misplaces
        // either FILTER (inside vs outside the correlated NOT-EXISTS) would drop
        // Ann/Zed too, or fail to drop Bob.
        assert_eq!(
            pg_bag("group-d-fd-simplification"),
            vec![
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Ann".to_owned())
                ],
                vec![
                    ("label".to_owned(), "Sales".to_owned()),
                    ("name".to_owned(), "Zed".to_owned())
                ],
            ],
            "group-d-fd-simplification live-PG value regression"
        );
    });
}

// ============================================================================
// ADR-0024 M4 — MySQL differential arm (graceful skip; live-only A1 guard).
//
// Mirrors the PG arm: the always-run SQLite side is the oracle; the live MySQL side
// (probe → throwaway database → load the SAME fixture → run via `exec_mysql` →
// `=_bag` vs the oracle) must reproduce every bag, including the q9/q15/sequence/etc.
// feature classes MySQL now GAINS from the shared `exec_core` (the old buffered
// `for_each_solution_mysql` never checked `rust_group`, DISTINCT-over-union, or
// ORDER-expression keys). Plus the design §5-M4 A1 guard: an INTEGER, a DATETIME,
// and a NON-UTF-8 VARBINARY column proving `mysql_value_to_string` semantics
// (int → "42", DATETIME → T-separated, non-UTF-8 bytes → UNBOUND, never
// `from_utf8_lossy` replacement chars).
// ============================================================================

use mysql_async::prelude::Queryable;

/// Base MySQL URL: `SF_MYSQL_URL` if set, else the `mysql_e2e` default. Includes a
/// default database; the throwaway db is created/USE-d on the same connection.
fn mysql_url() -> String {
    std::env::var("SF_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:sftest@127.0.0.1:13306/sftest".to_owned())
}

/// Probe; `None` ⇒ graceful skip (design §5 M4; mirrors `mysql_e2e.rs`).
async fn try_connect_mysql() -> Option<mysql_async::Conn> {
    let opts = mysql_async::Opts::from_url(&mysql_url()).ok()?;
    mysql_async::Conn::new(opts).await.ok()
}

/// Introspect every base table in the current database (name order) over the live
/// MySQL connection — the same schema set the SQLite side gets, so translation is
/// symmetric.
async fn introspect_all_mysql(conn: &mut mysql_async::Conn) -> Result<Vec<TableSchema>, String> {
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

/// Run each `;`-separated statement of a multi-statement fixture in turn (the fixture
/// has no `;` inside any statement). `mysql_async` runs one statement per call.
async fn run_stmts(conn: &mut mysql_async::Conn, sql: &str) {
    for stmt in sql.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            conn.query_drop(s).await.expect("mysql fixture statement");
        }
    }
}

/// MySQL side: create a throwaway database, load the SAME fixture rows, introspect,
/// translate (MySql), run SELECT/ASK over the live connection via `select_mysql` /
/// `ask_mysql`, plus each [`FEATURE_QUERIES`] entry (tree path + MySQL lowering +
/// live `exec_mysql` — the exact production surface sf-serve uses). Returns the same
/// 5-tuple shape as [`pg_side`].
#[allow(clippy::type_complexity)]
async fn mysql_side(
    conn: &mut mysql_async::Conn,
) -> (
    Vec<Vec<Option<sf_core::Term>>>,
    Vec<String>,
    bool,
    bool,
    Vec<Vec<std::collections::BTreeMap<String, sf_core::Term>>>,
) {
    let db = format!("sf_diff_my_{}", std::process::id());
    conn.query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await
        .expect("drop pre-existing throwaway db");
    conn.query_drop(format!("CREATE DATABASE {db}"))
        .await
        .expect("create throwaway db");
    conn.query_drop(format!("USE {db}"))
        .await
        .expect("use throwaway db");
    run_stmts(conn, CREATE_SQL).await;

    let maps = sf_mapping::parse_r2rml(R2RML).expect("R2RML parses");
    let schema = introspect_all_mysql(conn)
        .await
        .expect("mysql introspection");

    let sel_plan =
        parse_and_translate_with(SELECT_Q, &maps, Dialect::MySql, &Tbox::default(), &schema)
            .expect("translate SELECT (mysql)");
    let sols = exec_mysql::select_mysql(&sel_plan, conn)
        .await
        .expect("mysql select");

    let ask_t = exec_mysql::ask_mysql(
        &parse_and_translate_with(ASK_TRUE_Q, &maps, Dialect::MySql, &Tbox::default(), &schema)
            .expect("translate ASK-true (mysql)"),
        conn,
    )
    .await
    .expect("mysql ask-true");
    let ask_f = exec_mysql::ask_mysql(
        &parse_and_translate_with(
            ASK_FALSE_Q,
            &maps,
            Dialect::MySql,
            &Tbox::default(),
            &schema,
        )
        .expect("translate ASK-false (mysql)"),
        conn,
    )
    .await
    .expect("mysql ask-false");

    // Feature-class arms over the LIVE MySQL cursor — the classes MySQL now GAINS
    // from the shared core (q9 rust_group, q15 DISTINCT-over-union, sequence path,
    // ORDER-expression keys).
    let mut features = Vec::with_capacity(FEATURE_QUERIES.len());
    for (name, q, _) in FEATURE_QUERIES {
        let plan = parse_and_translate_with(q, &maps, Dialect::MySql, &Tbox::default(), &schema)
            .unwrap_or_else(|e| panic!("translate {name} (mysql): {e}"));
        let sols = exec_mysql::select_mysql(&plan, conn)
            .await
            .unwrap_or_else(|e| panic!("{name} (mysql): {e}"));
        features.push(engine_bag(&sols));
    }

    let _ = conn
        .query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await;
    (sols.rows, sols.vars, ask_t, ask_f, features)
}

/// R2RML for the A1 typed-values table: an INTEGER, a DATETIME, and a NON-UTF-8
/// VARBINARY column, each a plain-literal object map.
const A1_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#A1>
    rr:logicalTable [ rr:tableName "sf_a1" ] ;
    rr:subjectMap [ rr:template "http://ex/a1/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:intval  ; rr:objectMap [ rr:column "i" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:dtval   ; rr:objectMap [ rr:column "dt" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:blobval ; rr:objectMap [ rr:column "b" ] ] .
"#;

/// Anchor on the non-null INTEGER; the DATETIME and VARBINARY are OPTIONAL so the
/// row still surfaces with the NULL-producing (non-UTF-8) blob UNBOUND.
const A1_Q: &str = r#"
    PREFIX ex: <http://ex/>
    SELECT ?intval ?dtval ?blobval WHERE {
        ?s ex:intval ?intval .
        OPTIONAL { ?s ex:dtval ?dtval }
        OPTIONAL { ?s ex:blobval ?blobval }
    }"#;

/// The design §5-M4 A1 guard: exact computed values (oracle-independent), proving the
/// adapter's `mysql_value_to_string` semantics — INTEGER→`"42"`, DATETIME→T-separated
/// `"2021-03-04T05:06:07"`, and a NON-UTF-8 VARBINARY→UNBOUND (NOT `from_utf8_lossy`
/// replacement chars). A recurrence of the `mysql_for_each` decode fails this.
///
/// Runs on its OWN fresh connection (not the shared one from `mysql_side`): the
/// `MYSQL_COLUMNS_SQL` introspection statement selects `WHERE TABLE_SCHEMA =
/// DATABASE()`, and a `mysql_async`-cached prepared copy of it binds `DATABASE()` to
/// the schema live at prepare time. `mysql_side` primes that cache against its own
/// throwaway db and then drops it, so reusing the connection here would resolve the
/// cached statement against a now-dropped schema and see zero columns. The
/// production serve path introspects ONCE on a fresh connection, so it never hits
/// this — the fresh connection keeps the guard faithful to that path.
async fn mysql_a1_typed_values() {
    let mut conn = match try_connect_mysql().await {
        Some(c) => c,
        None => {
            eprintln!("skipping MySQL A1 guard (no server)");
            return;
        }
    };
    let conn = &mut conn;
    let db = format!("sf_a1_{}", std::process::id());
    conn.query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await
        .expect("drop pre-existing a1 db");
    conn.query_drop(format!("CREATE DATABASE {db}"))
        .await
        .expect("create a1 db");
    conn.query_drop(format!("USE {db}"))
        .await
        .expect("use a1 db");
    // Regular (non-TEMPORARY) table so it appears in information_schema for
    // introspection; the whole db is dropped at the end.
    conn.query_drop("CREATE TABLE sf_a1 (id INT PRIMARY KEY, i INT, dt DATETIME, b VARBINARY(16))")
        .await
        .expect("create sf_a1");
    // Non-UTF-8 bytes (0xFF 0xFE) bound as a parameter; DATETIME non-midnight to dodge
    // the documented DATE/DATETIME midnight ambiguity.
    conn.exec_drop(
        "INSERT INTO sf_a1 (id, i, dt, b) VALUES (?, ?, ?, ?)",
        (1i32, 42i32, "2021-03-04 05:06:07", vec![0xFFu8, 0xFE]),
    )
    .await
    .expect("insert sf_a1 row");

    let maps = sf_mapping::parse_r2rml(A1_R2RML).expect("A1 R2RML parses");
    let schema = vec![sf_sql::introspect::introspect_mysql(conn, "sf_a1")
        .await
        .expect("introspect sf_a1")];
    let plan = parse_and_translate_with(A1_Q, &maps, Dialect::MySql, &Tbox::default(), &schema)
        .expect("translate A1 (mysql)");
    let sols = exec_mysql::select_mysql(&plan, conn)
        .await
        .expect("A1 select");
    let bag = engine_bag(&sols);

    assert_eq!(bag.len(), 1, "A1 exactly one row: {bag:#?}");
    let row = &bag[0];
    assert_eq!(
        row.get("intval").map(term_lex),
        Some("42".to_owned()),
        "A1 INTEGER: Value::Int → \"42\""
    );
    assert_eq!(
        row.get("dtval").map(term_lex),
        Some("2021-03-04T05:06:07".to_owned()),
        "A1 DATETIME: non-midnight Date branch → T-separated"
    );
    assert!(
        row.get("blobval").is_none(),
        "A1 non-UTF-8 VARBINARY must be UNBOUND (from_utf8 None), NOT from_utf8_lossy: {row:#?}"
    );

    let _ = conn
        .query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await;
}

#[test]
fn select_and_ask_agree_across_sqlite_and_mysql() {
    // SQLite arm (sync) — always runs; the oracle.
    let (s_rows, s_vars, s_ask_t, s_ask_f, s_features) = sqlite_side();
    let s_sel = engine_bag(&exec::Solutions {
        vars: s_vars,
        rows: s_rows,
    });

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let mut conn = match try_connect_mysql().await {
            Some(c) => c,
            None => {
                eprintln!("skipping MySQL differential");
                return;
            }
        };
        let (m_rows, m_vars, m_ask_t, m_ask_f, m_features) = mysql_side(&mut conn).await;
        let m_sel = engine_bag(&exec::Solutions {
            vars: m_vars,
            rows: m_rows,
        });
        assert!(
            solutions_bag_eq(&s_sel, &m_sel),
            "SELECT diverges sqlite vs mysql:\n sqlite={s_sel:#?}\n mysql={m_sel:#?}"
        );
        assert_eq!(s_ask_t, m_ask_t, "ASK-true diverges sqlite vs mysql");
        assert_eq!(s_ask_f, m_ask_f, "ASK-false diverges sqlite vs mysql");

        // MySQL now GAINS q9/q15/sequence/etc. from the shared core — each must be
        // `=_bag` with the SQLite oracle at the hand-computed cardinality.
        for (((name, _, want), s_bag), m_bag) in
            FEATURE_QUERIES.iter().zip(&s_features).zip(&m_features)
        {
            assert!(
                solutions_bag_eq(s_bag, m_bag),
                "{name} diverges sqlite vs mysql:\n sqlite={s_bag:#?}\n mysql={m_bag:#?}"
            );
            assert_eq!(
                m_bag.len(),
                *want,
                "{name} mysql cardinality wrong (expected {want}): {m_bag:#?}"
            );
        }

        // A1 exact-value guard (INTEGER / DATETIME / non-UTF-8 VARBINARY) — on its
        // own fresh connection (see `mysql_a1_typed_values` docs: cached-statement
        // `DATABASE()` staleness across the shared conn's dropped db).
        drop(conn);
        mysql_a1_typed_values().await;
    });
}
/// PostgreSQL DATE/TIME/TIMESTAMP column read, end-to-end through the real R2RML
/// pipeline (translate -> emit -> `select_pg` -> `pg_value`/`pg_xsd_code` ->
/// reconstruction/canonicalisation) -- not just the raw driver-level decode (see
/// `sf_sql::backend::pg::tests::pg_value_reads_date_time_timestamp_columns` for
/// that unit-level coverage). Before this fix `pg_value` had no extraction arm
/// for these PostgreSQL types, so `select_pg` hard-501'd on ANY DATE/TIME/
/// TIMESTAMP column -- a real functionality gap on the primary production
/// dialect. Confirms the space-separated PG TIMESTAMP lexical form normalises to
/// ISO 'T'-separated xsd:dateTime through the SAME `normalize_timestamp` path
/// every other backend already uses. Live-PG only; gracefully skips when no
/// server is reachable.
#[tokio::test]
async fn pg_date_time_columns_readable_end_to_end() {
    let base = base_conn();
    let conn_str = format!("{base} dbname=postgres");
    let Ok(admin) = connect(&conn_str).await else {
        eprintln!(
            "skipping pg_date_time_columns_readable_end_to_end: no live PostgreSQL reachable"
        );
        return;
    };
    let db = format!("sf_pgdt_e2e_{}", std::process::id());
    let _ = admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db}"))
        .await;
    admin
        .batch_execute(&format!("CREATE DATABASE {db}"))
        .await
        .expect("create test db");
    let conn_str2 = format!("{base} dbname={db}");
    let client = connect(&conn_str2).await.expect("connect to test db");
    client
        .batch_execute(
            "CREATE TABLE ev (id INTEGER PRIMARY KEY, d DATE, ts TIMESTAMP);
             INSERT INTO ev VALUES (1, '2024-03-15', '2024-03-15 13:45:30');",
        )
        .await
        .expect("seed table");

    let r2rml = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#E> rr:logicalTable [ rr:tableName "ev" ] ;
    rr:subjectMap [ rr:template "http://ex/e/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:d ; rr:objectMap [ rr:column "d" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:ts ; rr:objectMap [ rr:column "ts" ] ] .
"#;
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let schema = introspect_all_pg(&client).await.expect("pg introspection");
    let q = "PREFIX ex: <http://ex/> SELECT ?d ?ts WHERE { <http://ex/e/1> ex:d ?d ; ex:ts ?ts }";
    let tp = parse_and_translate_with(q, &maps, Dialect::Postgres, &Tbox::default(), &schema)
        .expect("translate SELECT (pg)");
    let sol = exec_pg::select_pg(&tp, &client)
        .await
        .expect("select_pg — previously hard-501'd on DATE/TIMESTAMP columns");

    assert_eq!(sol.rows.len(), 1);
    let d = sol.rows[0][0].as_ref().expect("?d bound");
    let ts = sol.rows[0][1].as_ref().expect("?ts bound");
    assert_eq!(
        d.to_string(),
        "\"2024-03-15\"^^<http://www.w3.org/2001/XMLSchema#date>"
    );
    assert_eq!(
        ts.to_string(),
        "\"2024-03-15T13:45:30\"^^<http://www.w3.org/2001/XMLSchema#dateTime>"
    );

    let _ = admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db}"))
        .await;
}

/// PostgreSQL NUMERIC column read, end-to-end through the real R2RML pipeline
/// (translate -> emit -> `select_pg` -> `pg_value`/`pg_xsd_code` ->
/// reconstruction/canonicalisation) -- not just the raw wire-decode (see
/// `sf_sql::backend::pg::tests` for that unit-level coverage). Before this fix
/// `pg_value` had NO extraction arm for `Type::NUMERIC` (the TRACKED RESIDUE), so
/// `select_pg` hard-501'd on ANY NUMERIC column -- a real functionality gap on
/// the primary production dialect. The four values are all "non-even" (not
/// exactly representable in binary floating point), proving the decode never
/// touches `f64` -- an exact PostgreSQL decimal in, an exact `xsd:decimal`
/// lexical form out. Live-PG only; gracefully skips when no server is reachable.
#[tokio::test]
async fn pg_numeric_columns_readable_end_to_end() {
    let base = base_conn();
    let conn_str = format!("{base} dbname=postgres");
    let Ok(admin) = connect(&conn_str).await else {
        eprintln!("skipping pg_numeric_columns_readable_end_to_end: no live PostgreSQL reachable");
        return;
    };
    let db = format!("sf_pgnum_e2e_{}", std::process::id());
    let _ = admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db}"))
        .await;
    admin
        .batch_execute(&format!("CREATE DATABASE {db}"))
        .await
        .expect("create test db");
    let conn_str2 = format!("{base} dbname={db}");
    let client = connect(&conn_str2).await.expect("connect to test db");
    client
        .batch_execute(
            "CREATE TABLE amt (id INTEGER PRIMARY KEY, p1 NUMERIC, p2 NUMERIC, p3 NUMERIC, p4 NUMERIC);
             INSERT INTO amt VALUES (1, 12345.678, 0.0001, -42, 100000000);",
        )
        .await
        .expect("seed table");

    let r2rml = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#A> rr:logicalTable [ rr:tableName "amt" ] ;
    rr:subjectMap [ rr:template "http://ex/a/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p1 ; rr:objectMap [ rr:column "p1" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p2 ; rr:objectMap [ rr:column "p2" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p3 ; rr:objectMap [ rr:column "p3" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:p4 ; rr:objectMap [ rr:column "p4" ] ] .
"#;
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let schema = introspect_all_pg(&client).await.expect("pg introspection");
    let q = "PREFIX ex: <http://ex/> SELECT ?p1 ?p2 ?p3 ?p4 WHERE { \
             <http://ex/a/1> ex:p1 ?p1 ; ex:p2 ?p2 ; ex:p3 ?p3 ; ex:p4 ?p4 }";
    let tp = parse_and_translate_with(q, &maps, Dialect::Postgres, &Tbox::default(), &schema)
        .expect("translate SELECT (pg)");
    let sol = exec_pg::select_pg(&tp, &client)
        .await
        .expect("select_pg — previously hard-501'd on NUMERIC columns");

    assert_eq!(sol.rows.len(), 1);
    const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
    let want = ["12345.678", "0.0001", "-42", "100000000"];
    for (idx, expected) in want.iter().enumerate() {
        let t = sol.rows[0][idx]
            .as_ref()
            .unwrap_or_else(|| panic!("?p{} bound", idx + 1));
        assert_eq!(t.to_string(), format!("\"{expected}\"^^<{XSD_DECIMAL}>"));
    }

    let _ = admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db}"))
        .await;
}

/// PostgreSQL NUMERIC NaN/+Infinity/-Infinity refusal, end-to-end through the real
/// R2RML pipeline (translate -> emit -> `select_pg` -> `pg_value`/`pg_xsd_code` ->
/// `decode_pg_numeric` -> `exec_core::map_sql_err`) -- not just the raw wire-decode
/// unit tests (see `sf_sql::backend::pg::tests::decode_pg_numeric_*_is_unsupported`)
/// or the `pg_value`/`next_row`-level live test (`sf_sql::backend::pg::tests::
/// pg_value_numeric_nan_and_infinity_surface_as_unsupported`) for those narrower
/// layers. `xsd:decimal` has no NaN/Infinity representation, so refusing these
/// three PG NUMERIC special values is CORRECT -- but it must surface as
/// `sf_sparql::Error::Unsupported` (-> HTTP 501 via sf-serve's `status_for`), never
/// a generic `Error::Sql` (-> HTTP 500): a 500 would misclassify a KNOWN, documented
/// non-representable value as an unexpected internal failure. `map_sql_err` only
/// pattern-matches the top-level `sf_sql::Error::Unsupported` variant, so this also
/// guards the layer below (`pg_value`) staying honest about that variant, not just
/// wrapping it as `Error::Postgres` through `tokio_postgres::Row::try_get`'s
/// `FromSql`-failure re-wrap. Live-PG only; gracefully skips when no server is
/// reachable.
#[tokio::test]
async fn pg_numeric_nonfinite_values_refused_as_unsupported() {
    let base = base_conn();
    let conn_str = format!("{base} dbname=postgres");
    let Ok(admin) = connect(&conn_str).await else {
        eprintln!(
            "skipping pg_numeric_nonfinite_values_refused_as_unsupported: no live PostgreSQL reachable"
        );
        return;
    };
    let db = format!("sf_pgnan_e2e_{}", std::process::id());
    let _ = admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db}"))
        .await;
    admin
        .batch_execute(&format!("CREATE DATABASE {db}"))
        .await
        .expect("create test db");
    let conn_str2 = format!("{base} dbname={db}");
    let client = connect(&conn_str2).await.expect("connect to test db");
    client
        .batch_execute(
            "CREATE TABLE bad_amt (id INTEGER PRIMARY KEY, p NUMERIC);
             INSERT INTO bad_amt VALUES (1, 'NaN'), (2, 'Infinity'), (3, '-Infinity');",
        )
        .await
        .expect("seed table");

    let r2rml = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#A> rr:logicalTable [ rr:tableName "bad_amt" ] ;
    rr:subjectMap [ rr:template "http://ex/a/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "p" ] ] .
"#;
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let schema = introspect_all_pg(&client).await.expect("pg introspection");

    // Each non-finite class in its own query, so a regression pinpoints exactly
    // which sign value broke (mirrors decode_pg_numeric's own 3-way unit split).
    for (id, class, needle) in [
        (1, "NaN", "NaN"),
        (2, "+Infinity", "+Infinity"),
        (3, "-Infinity", "-Infinity"),
    ] {
        let q = format!("PREFIX ex: <http://ex/> SELECT ?p WHERE {{ <http://ex/a/{id}> ex:p ?p }}");
        let tp = parse_and_translate_with(&q, &maps, Dialect::Postgres, &Tbox::default(), &schema)
            .unwrap_or_else(|e| panic!("translate {class} query: {e}"));
        let err = match exec_pg::select_pg(&tp, &client).await {
            Ok(sol) => panic!(
                "PG NUMERIC {class} must be refused, not decoded: {} row(s)",
                sol.rows.len()
            ),
            Err(e) => e,
        };
        assert!(
            matches!(err, sf_sparql::Error::Unsupported(_)),
            "PG NUMERIC {class} must classify as Error::Unsupported (-> HTTP 501), got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains(needle),
            "PG NUMERIC {class} error message should name the value class, got: {msg}"
        );
    }

    let _ = admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db}"))
        .await;
}
