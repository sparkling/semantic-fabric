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
use sf_sparql::{exec, exec_pg, parse_and_translate_with, Tbox};
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
/// * **filter-exists** — FILTER EXISTS over a typed/plain column. Was SF-501 (q12):
///   the correlated sub-plan aborted on the PG bind path. Ann + Zed have email ⇒ 2.
/// * **minus** — MINUS removing the emailed persons. Was SF-EMPTY (q11): the PG
///   anti-join removed everything. Only Bob (NULL email) survives ⇒ 1.
/// * **sequence-path** — property path `ex:dept/ex:label`. Was SF-EMPTY (q10): the
///   sequence lowered to nothing on PG. All 3 persons reach "Sales" ⇒ 3.
/// * **distinct-join** — DISTINCT over a duplicate-producing join. Was MISMATCH
///   (q15): DISTINCT was dropped on PG, leaking duplicates. 3 persons, 1 dept ⇒ 1.
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
];

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
    client: &Client,
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
        client,
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
        client,
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

        let work = connect(&format!("{base} dbname={dbname}"))
            .await
            .expect("connect work db");
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
    });
}
