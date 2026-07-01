//! ADR-0024 §4.2 serve-lane requirement: a MySQL streaming request owns a
//! **dedicated pooled connection** for the stream's lifetime and RELEASES it (return
//! or dispose) on early drop (LIMIT / deadline / client-gone) — it is NOT pinned
//! non-recyclable forever. This is the M4/M5 gate that distinguishes the accepted
//! connection-holding tradeoff (§4.2) from a connection leak.
//!
//! The test drives the real HTTP endpoint (`Backend::Mysql(pool)`) over a size-1
//! pool with a large result set, reads ONE body chunk, then DROPS the response body
//! (a slow / early-gone consumer). If the dedicated connection is released, a
//! subsequent `pool.get_conn()` succeeds within a short timeout; a leak makes the
//! size-1 pool starve and the `get_conn` time out ⇒ the test fails.
//!
//! Gated on `SF_MYSQL_URL` (skips cleanly when no server is reachable, like the
//! differential's MySQL arm). Run serially with the other live-MySQL suites.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts};
use sf_serve::{router, Backend, ServeConfig};
use sf_sparql::Tbox;
use tower::ServiceExt;

/// Base MySQL URL: `SF_MYSQL_URL` if set, else the `mysql_e2e` container default.
/// Includes a default database; the throwaway db is created/USE-d over it.
fn mysql_base_url() -> String {
    std::env::var("SF_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:sftest@127.0.0.1:13306/sftest".to_owned())
}

/// Rebuild a URL's authority against a specific database name (last path segment).
fn url_with_db(base: &str, db: &str) -> String {
    // `mysql://user:pass@host:port/<db>` — replace the trailing `/<db>` segment.
    match base.rsplit_once('/') {
        Some((authority, _db)) => format!("{authority}/{db}"),
        None => format!("{base}/{db}"),
    }
}

const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mysql_stream_releases_connection_on_early_drop() {
    // Admin connection: create a throwaway db, load a large fixture, introspect.
    let base = mysql_base_url();
    let Ok(opts) = Opts::from_url(&base) else {
        eprintln!("SKIP mysql_release: bad SF_MYSQL_URL");
        return;
    };
    let Ok(mut admin) = Conn::new(opts).await else {
        eprintln!("SKIP mysql_release: no MySQL reachable");
        return;
    };

    let db = format!("sf_release_{}", std::process::id());
    admin
        .query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await
        .expect("drop pre-existing db");
    admin
        .query_drop(format!("CREATE DATABASE {db}"))
        .await
        .expect("create throwaway db");
    admin.query_drop(format!("USE {db}")).await.expect("use db");
    admin
        .query_drop("CREATE TABLE `People` (id INT PRIMARY KEY, name VARCHAR(64), age INT)")
        .await
        .expect("create table");

    // Enough rows that the serialised SELECT body exceeds the channel window
    // (CHANNEL_CAP × CHUNK = 128 KiB) several times over: the serve task is still
    // streaming — holding the dedicated connection — when we drop the body after one
    // chunk. Generated server-side (one INSERT…SELECT over a recursive CTE) so
    // fixture load is a single round trip, not N.
    let n: i64 = 8_000;
    admin
        .query_drop("SET SESSION cte_max_recursion_depth = 100000")
        .await
        .expect("raise cte recursion depth");
    admin
        .query_drop(format!(
            "INSERT INTO `People` \
             WITH RECURSIVE seq(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM seq WHERE x < {n}) \
             SELECT x, CONCAT('Person', x), x % 100 FROM seq"
        ))
        .await
        .expect("insert rows");

    let schema = vec![sf_sql::introspect::introspect_mysql(&mut admin, "People")
        .await
        .expect("introspect People")];
    let maps = sf_mapping::parse_r2rml(MAPPING_TTL).expect("parse mapping");
    drop(admin);

    // A size-1 pool scoped to the throwaway db: the whole pool is exactly ONE
    // connection, so a leaked stream connection would make `get_conn` starve.
    let constraints = PoolConstraints::new(1, 1).expect("valid 1,1 constraints");
    let pool_opts = PoolOpts::default().with_constraints(constraints);
    let db_opts: Opts = OptsBuilder::from_opts(Opts::from_url(&url_with_db(&base, &db)).unwrap())
        .pool_opts(pool_opts)
        .into();
    let pool = Pool::new(db_opts);

    let cfg = Arc::new(ServeConfig::new(
        Backend::Mysql(pool.clone()),
        maps,
        Tbox::default(),
        schema,
    ));

    // Fire a large streaming SELECT at the endpoint (draws the pool's one conn).
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(
            "SELECT ?name WHERE { ?s <http://ex/name> ?name }".to_owned(),
        ))
        .unwrap();
    let resp = router(cfg).oneshot(req).await.expect("endpoint responds");
    assert_eq!(resp.status(), StatusCode::OK);

    // Read exactly ONE chunk, then DROP the body — the early-gone slow consumer.
    let mut body = resp.into_body();
    let first = body.frame().await;
    assert!(
        first.is_some(),
        "expected at least one streamed chunk before drop"
    );
    drop(body); // receiver gone ⇒ the serve task's next send fails ⇒ conn released.

    // The dedicated connection must return to the size-1 pool (or be disposed and
    // replaced) — never pinned forever. A leak starves the pool ⇒ this times out.
    let acquired = tokio::time::timeout(Duration::from_secs(10), pool.get_conn()).await;
    assert!(
        matches!(&acquired, Ok(Ok(_))),
        "dedicated MySQL stream connection was not released on early drop (§4.2): {acquired:?}"
    );

    // Cleanup: drop the throwaway db over the reclaimed connection.
    let mut conn = acquired.unwrap().unwrap();
    let _ = conn
        .query_drop(format!("DROP DATABASE IF EXISTS {db}"))
        .await;
    drop(conn);
    let _ = pool.disconnect().await;
}
