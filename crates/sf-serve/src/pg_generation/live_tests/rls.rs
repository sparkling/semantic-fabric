//! Public HTTP RLS evidence; reuses only the disposable provisioning fixture.
use super::*;
use crate::{BearerQueryAdmission, PostgresRlsClaims, QueryAdmission, ServeConfig};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::Response,
};
use http_body_util::BodyExt;
use std::collections::BTreeMap;
use tower::ServiceExt;

const TOKEN: &str = "test-only-rls-credential-0123456789";
const SELECT: &str = "SELECT ?name WHERE { ?s <http://ex/name> ?name }";
const GRAPH: &str = "CONSTRUCT { ?s <http://ex/name> ?name } WHERE { ?s <http://ex/name> ?name }";
const LINEAGE: &str = "application/vnd.semantic-fabric.lineage+json-seq";

#[path = "rls_subjects.rs"]
mod subjects;

fn mapping(table: &str, predicate: &str) -> String {
    format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#p> a rr:TriplesMap; rr:logicalTable [rr:tableName "{table}"];
      rr:subjectMap [rr:template "http://ex/{table}/{{id}}"];
      rr:predicateObjectMap [rr:predicate <{predicate}>; rr:objectMap [rr:column "label"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]]."#
    )
}

fn profile(tenant: &str) -> QueryAdmission {
    QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN)
            .unwrap()
            .with_postgres_rls(
                PostgresRlsClaims::new(BTreeMap::from([("app.tenant_id".into(), tenant.into())]))
                    .unwrap(),
            )
            .unwrap(),
    )
}

async fn config(pool: crate::PostgresPool, tenant: &str, mapping: &str) -> ServeConfig {
    let source = IntrospectedSource::observe_postgres(pool).await.unwrap();
    let ontology = crate::test_support::ontology(&[], &["http://ex/name"]);
    let mut cfg = ServeConfig::from_authored_r2rml(source, mapping, ontology)
        .unwrap_or_else(|error| panic!("RLS fixture admission: {}", error.internal_cause()));
    cfg.use_in_process_test_parser();
    cfg.set_query_admission(profile(tenant));
    cfg
}

async fn request(cfg: Arc<ServeConfig>, query: &str) -> Response {
    request_format(cfg, query, "*/*").await
}

async fn request_format(cfg: Arc<ServeConfig>, query: &str, accept: &str) -> Response {
    crate::router(cfg)
        .oneshot(
            Request::post("/sparql")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::CONTENT_TYPE, "application/sparql-query")
                .header(header::ACCEPT, accept)
                .body(Body::from(query.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn text(cfg: Arc<ServeConfig>, query: &str) -> String {
    let response = request(cfg, query).await;
    assert_eq!(response.status(), StatusCode::OK);
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

async fn clean(pool: &deadpool_postgres::Pool) -> i32 {
    let conn = pool.get().await.unwrap();
    let row = conn
        .query_one(
            "SELECT pg_backend_pid(), current_setting('app.tenant_id', true), \
        current_setting('transaction_read_only')",
            &[],
        )
        .await
        .unwrap();
    let claim: Option<String> = row.get(1);
    assert!(
        claim.is_none_or(|value| value.is_empty()),
        "identity leaked into pooled session"
    );
    assert_eq!(
        row.get::<_, String>(2),
        "off",
        "read-only request transaction leaked"
    );
    row.get(0)
}

async fn exercise(f: Arc<Fixture>) {
    f.admin.batch_execute("ALTER TABLE public.parent ADD COLUMN tenant text NOT NULL DEFAULT 'tenant-a'; \
        ALTER TABLE public.child ADD COLUMN tenant text NOT NULL DEFAULT 'tenant-a'; \
        UPDATE public.parent SET label='Alice'; UPDATE public.child SET label='Alice'; \
        INSERT INTO public.parent VALUES (2,'Bob','tenant-b'); \
        INSERT INTO public.child VALUES (2,2,NULL,'Bob','tenant-b'); \
        ALTER TABLE public.parent ENABLE ROW LEVEL SECURITY; ALTER TABLE public.parent FORCE ROW LEVEL SECURITY; \
        ALTER TABLE public.child ENABLE ROW LEVEL SECURITY; ALTER TABLE public.child FORCE ROW LEVEL SECURITY; \
        CREATE POLICY identity ON public.parent USING (tenant = current_setting('app.tenant_id', true)); \
        CREATE POLICY identity ON public.child USING (tenant = current_setting('app.tenant_id', true));").await.unwrap();
    let mapped = mapping("parent", "http://ex/name");
    let alice = Arc::new(config(f.pool.clone(), "tenant-a", &mapped).await);
    let bob = Arc::new(config(f.pool.clone(), "tenant-b", &mapped).await);
    let pid = clean(&f.pool).await;
    for (cfg, allowed, denied) in [
        (&alice, "Alice", "Bob"),
        (&bob, "Bob", "Alice"),
        (&alice, "Alice", "Bob"),
    ] {
        for query in [
            SELECT.to_owned(),
            "CONSTRUCT { ?s <http://ex/name> ?name } WHERE { ?s <http://ex/name> ?name }".into(),
        ] {
            let result = text(cfg.clone(), &query).await;
            assert!(
                result.contains(allowed) && !result.contains(denied),
                "{result}"
            );
            assert_eq!(
                clean(&f.pool).await,
                pid,
                "normal completion must reuse clean member"
            );
        }
        for (name, expected) in [(allowed, true), (denied, false)] {
            let result = text(
                cfg.clone(),
                &format!("ASK {{ ?s <http://ex/name> \"{name}\" }}"),
            )
            .await;
            let result: serde_json::Value = serde_json::from_str(&result).unwrap();
            assert_eq!(result["boolean"], expected);
            assert_eq!(clean(&f.pool).await, pid);
        }
    }
    // Quotes, SQL syntax and unicode are bound data, never executable SQL.
    let injection = Arc::new(config(f.pool.clone(), "tenant-a'; RESET ALL; -- λ", &mapped).await);
    let result = text(injection, SELECT).await;
    assert!(!result.contains("Alice") && !result.contains("Bob"));
    assert_eq!(clean(&f.pool).await, pid);

    // Concurrent identities use the same public path and distinct connections.
    let concurrent_pool = pool(f.reader_config(), 2);
    let ca = Arc::new(config(concurrent_pool.clone(), "tenant-a", &mapped).await);
    let cb = Arc::new(config(concurrent_pool.clone(), "tenant-b", &mapped).await);
    let (a, b) = tokio::join!(text(ca, SELECT), text(cb, SELECT));
    assert!(a.contains("Alice") && !a.contains("Bob"));
    assert!(b.contains("Bob") && !b.contains("Alice"));
    drop(concurrent_pool);

    exercise_union(&f).await;
    subjects::exercise(&f).await;
    exercise_cleanup(&f, &alice, &bob).await;
    exercise_deadline(&f, &bob).await;

    f.admin
        .batch_execute("BEGIN; LOCK TABLE public.parent IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let contended = request(alice.clone(), SELECT).await;
    f.admin.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(contended.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(contended.headers().contains_key(header::RETRY_AFTER));
    clean(&f.pool).await;

    // Table/view and role admission failures return before a success body.
    f.admin
        .batch_execute("ALTER TABLE public.parent DISABLE ROW LEVEL SECURITY")
        .await
        .unwrap();
    assert_eq!(
        request(alice.clone(), SELECT).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request_format(alice.clone(), SELECT, LINEAGE)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    clean(&f.pool).await;
    f.admin
        .batch_execute("ALTER TABLE public.parent ENABLE ROW LEVEL SECURITY")
        .await
        .unwrap();
    let sql_mapping = mapped.replace(
        "rr:tableName \"parent\"",
        "rr:sqlQuery \"SELECT * FROM parent\"",
    );
    let sql = Arc::new(config(f.pool.clone(), "tenant-a", &sql_mapping).await);
    assert_eq!(request(sql, SELECT).await.status(), StatusCode::FORBIDDEN);
    f.admin
        .batch_execute(&format!(
            "CREATE VIEW public.person_view AS SELECT * FROM public.parent; \
        GRANT SELECT ON public.person_view TO {}",
            f.role
        ))
        .await
        .unwrap();
    let view = Arc::new(
        config(
            f.pool.clone(),
            "tenant-a",
            &mapping("person_view", "http://ex/name"),
        )
        .await,
    );
    assert_eq!(request(view, SELECT).await.status(), StatusCode::FORBIDDEN);
    f.admin
        .batch_execute("DROP VIEW public.person_view")
        .await
        .unwrap();
    f.admin
        .batch_execute(&format!("ALTER ROLE {} SUPERUSER", f.role))
        .await
        .unwrap();
    assert_eq!(
        request(alice.clone(), SELECT).await.status(),
        StatusCode::FORBIDDEN
    );
    f.admin
        .batch_execute(&format!("ALTER ROLE {} NOSUPERUSER", f.role))
        .await
        .unwrap();
    f.admin
        .batch_execute(&format!("ALTER ROLE {} BYPASSRLS", f.role))
        .await
        .unwrap();
    assert_eq!(
        request(alice.clone(), SELECT).await.status(),
        StatusCode::FORBIDDEN
    );
    f.admin
        .batch_execute(&format!("ALTER ROLE {} NOBYPASSRLS", f.role))
        .await
        .unwrap();
    f.admin
        .batch_execute(&format!("ALTER TABLE public.parent OWNER TO {}", f.role))
        .await
        .unwrap();
    assert_eq!(request(alice, SELECT).await.status(), StatusCode::FORBIDDEN);
}

fn pool(config: Config, size: usize) -> crate::PostgresPool {
    let manager = deadpool_postgres::Manager::from_config(
        config,
        NoTls,
        deadpool_postgres::ManagerConfig {
            recycling_method: deadpool_postgres::RecyclingMethod::Custom(
                crate::source::POSTGRES_RELATION_SCOPE_RECYCLE_SQL.into(),
            ),
        },
    );
    deadpool_postgres::Pool::builder(manager)
        .max_size(size)
        .wait_timeout(Some(Duration::from_secs(2)))
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .build()
        .unwrap()
        .into()
}

async fn exercise_union(f: &Fixture) {
    use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
    let ontology = crate::test_support::ontology(&[], &["http://ex/left", "http://ex/right"]);
    let second_pool = pool(f.reader_config(), 1);
    let mut sources = Vec::new();
    for (id, table, predicate, pool) in [
        (0, "parent", "http://ex/left", f.pool.clone()),
        (1, "child", "http://ex/right", second_pool.clone()),
    ] {
        let source = IntrospectedSource::observe_postgres(pool).await.unwrap();
        let mapping = sf_core::SourceMapping::new(
            SourceId::new(id).unwrap(),
            sf_mapping::parse_r2rml(&mapping(table, predicate)).unwrap(),
        );
        let mapping =
            ValidatedMapping::validate(mapping, MappingOrigin::Authored, &ontology, &source)
                .unwrap();
        sources.push(crate::RuntimeSource::admitted(source, mapping).unwrap());
    }
    let mut cfg = ServeConfig::new_federated(sources.try_into().ok().unwrap(), ontology).unwrap();
    cfg.set_query_admission(profile("tenant-a"));
    let cfg = Arc::new(cfg);
    let query =
        "SELECT ?name WHERE { { ?s <http://ex/left> ?name } UNION { ?s <http://ex/right> ?name } }";
    let result: serde_json::Value = serde_json::from_str(&text(cfg.clone(), query).await).unwrap();
    assert_eq!(result["results"]["bindings"].as_array().unwrap().len(), 2);
    assert!(!result.to_string().contains("Bob"));
    clean(&f.pool).await;
    clean(&second_pool).await;
    f.admin
        .batch_execute("ALTER TABLE public.child DISABLE ROW LEVEL SECURITY")
        .await
        .unwrap();
    assert_eq!(request(cfg, query).await.status(), StatusCode::FORBIDDEN);
    clean(&f.pool).await;
    clean(&second_pool).await;
    f.admin
        .batch_execute("ALTER TABLE public.child ENABLE ROW LEVEL SECURITY")
        .await
        .unwrap();
}

async fn exercise_cleanup(f: &Fixture, alice: &Arc<ServeConfig>, bob: &Arc<ServeConfig>) {
    // Deterministic ownership cancellation: an unclosed lease is never recycled.
    let admission = profile("tenant-a");
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let mut budget = budget();
    budget
        .retain_security(admission.authenticate(&headers).unwrap().unwrap())
        .unwrap();
    budget
        .retain_postgres_rls(admission.postgres_rls())
        .unwrap();
    let tables: Arc<[String]> = vec!["parent".into()].into();
    let old = clean(&f.pool).await;
    f.admin.batch_execute(&format!("CREATE TABLE public.pg_settings(id integer, label text, tenant text); \
        ALTER TABLE public.pg_settings ENABLE ROW LEVEL SECURITY; \
        CREATE POLICY identity ON public.pg_settings USING (tenant=current_setting('app.tenant_id',true)); \
        GRANT SELECT ON public.pg_settings TO {};", f.role)).await.unwrap();
    let shadowed: Arc<[String]> = vec!["pg_settings".into()].into();
    let rejection = crate::pg_rls::PgRlsLease::acquire(&f.pool, Some(shadowed), &budget)
        .await
        .err()
        .unwrap();
    assert_eq!(
        rejection.status(),
        StatusCode::FORBIDDEN,
        "guarded public table must be actual executed relation"
    );
    f.admin
        .batch_execute("DROP TABLE public.pg_settings")
        .await
        .unwrap();
    let lease = crate::pg_rls::PgRlsLease::acquire(&f.pool, Some(tables.clone()), &budget)
        .await
        .unwrap();
    drop(lease);
    assert_ne!(
        clean(&f.pool).await,
        old,
        "unacknowledged lease must be detached"
    );
    let missing: Arc<[String]> = vec!["missing_table".into()].into();
    assert!(
        crate::pg_rls::PgRlsLease::acquire(&f.pool, Some(missing), &budget)
            .await
            .is_err()
    );
    clean(&f.pool).await;
    let lease = crate::pg_rls::PgRlsLease::acquire(&f.pool, Some(tables), &budget)
        .await
        .unwrap();
    assert!(lease.client().batch_execute("SELECT 1/0").await.is_err());
    lease.finish().await.unwrap();
    clean(&f.pool).await;
    // Active server work can survive a dropped driver future. A rollback stuck
    // behind it must time out and discard, never recycle the claimed session.
    let tables: Arc<[String]> = vec!["parent".into()].into();
    let lease = crate::pg_rls::PgRlsLease::acquire(&f.pool, Some(tables), &budget)
        .await
        .unwrap();
    let client = lease.client();
    let pid = client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let sleeping = tokio::spawn(async move { client.batch_execute("SELECT pg_sleep(10)").await });
    wait_for_backend(f, pid, true).await;
    sleeping.abort();
    let _ = sleeping.await;
    let started = tokio::time::Instant::now();
    assert!(lease.finish().await.is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_ne!(clean(&f.pool).await, pid);
    // Backpressure ensures there is active work when the HTTP body is dropped.
    f.admin.batch_execute("INSERT INTO public.parent SELECT n, repeat('x',512), 'tenant-a' FROM generate_series(3,20000) n").await.unwrap();
    for (query, accept) in [(SELECT, "*/*"), (SELECT, LINEAGE), (GRAPH, LINEAGE)] {
        let response = request_format(alice.clone(), query, accept).await;
        assert_eq!(response.status(), StatusCode::OK);
        drop(response);
        let result = text(bob.clone(), SELECT).await;
        assert!(result.contains("Bob") && !result.contains("Alice"));
        clean(&f.pool).await;
    }
    f.admin
        .batch_execute("DELETE FROM public.parent WHERE id > 2")
        .await
        .unwrap();
}

async fn exercise_deadline(f: &Fixture, bob: &Arc<ServeConfig>) {
    f.admin.batch_execute("CREATE FUNCTION public.rls_test_visible(row_tenant text) RETURNS boolean \
        LANGUAGE plpgsql VOLATILE AS $$ BEGIN \
        IF current_setting('app.tenant_id',true)='slow' THEN PERFORM pg_sleep(10); END IF; \
        IF current_setting('app.tenant_id',true)='error' THEN RAISE EXCEPTION 'test-only-policy-error'; END IF; \
        RETURN row_tenant=current_setting('app.tenant_id',true); END $$; \
        ALTER POLICY identity ON public.parent USING (public.rls_test_visible(tenant));").await.unwrap();
    for tenant in ["slow", "error"] {
        for (query, accept) in [(SELECT, "*/*"), (SELECT, LINEAGE), (GRAPH, LINEAGE)] {
            let mut cfg =
                config(f.pool.clone(), tenant, &mapping("parent", "http://ex/name")).await;
            cfg.timeout = Duration::from_millis(250);
            let started = tokio::time::Instant::now();
            let response = request_format(Arc::new(cfg), query, accept).await;
            if response.status() == StatusCode::OK {
                assert!(
                    response.into_body().collect().await.is_err(),
                    "active source failure must error the body"
                );
            } else {
                assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            clean(&f.pool).await;
            assert!(text(bob.clone(), SELECT).await.contains("Bob"));
        }
    }
    f.admin.batch_execute("ALTER POLICY identity ON public.parent USING (tenant=current_setting('app.tenant_id',true)); \
        DROP FUNCTION public.rls_test_visible(text);").await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires SF_PG_RLS_TEST_URL naming a disposable administrator endpoint"]
async fn public_row_security_is_isolated_and_cleans_pool() {
    let spec =
        std::env::var("SF_PG_RLS_TEST_URL").expect("explicit disposable RLS endpoint required");
    let fixture = Arc::new(Fixture::create(spec.parse().expect("valid RLS test URL")).await);
    let work = fixture.clone();
    let outcome = tokio::spawn(exercise(work)).await;
    let fixture = Arc::try_unwrap(fixture).unwrap_or_else(|_| panic!("fixture retained"));
    fixture.cleanup().await;
    match outcome {
        Ok(()) => {}
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(error) => panic!("RLS test task failed: {error}"),
    }
}
