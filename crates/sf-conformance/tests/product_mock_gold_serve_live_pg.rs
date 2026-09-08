//! Optional live Product Mock proof through the governed `sf-serve` endpoint.
//!
//! The success case compares every ordered HTTP binding with a direct SQL read
//! of the same finite Style window. The rejection case uses the public offline
//! plan resource profile with a deliberately unopened pool, because safe serving
//! construction now requires a real coherent source observation.

#[path = "product_mock_gold/support.rs"]
mod support;

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use http_body_util::BodyExt;
use sf_serve::{router, IntrospectedSource, SemanticOntology, ServeConfig};
use sf_sparql::resource_profile::SourceSizedState;
use sf_sql::introspect::introspect_postgres;
use sf_sql::{Dialect, TableSchema};
use tokio_postgres::config::Host;
use tokio_postgres::{Config, NoTls};
use tower::ServiceExt;

const RESULTS_JSON: &str = "application/sparql-results+json";
const SEARCH_PATH_OPTIONS: &str = "-csearch_path=pg_catalog,public,pg_temp";
const SEARCH_PATH_RECYCLE: &str =
    "SELECT pg_catalog.set_config('search_path', 'pg_catalog,public,pg_temp', false)";

fn required_path(name: &str) -> Result<PathBuf, &'static str> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or("required product-mock path variable is missing")
}

fn external_gold() -> Result<support::GoldVertical, &'static str> {
    let gold_root = required_path(support::GOLD_ROOT_ENV)?;
    let source_root = required_path(support::SOURCE_ROOT_ENV)?;
    support::load_external(&gold_root, &source_root)
}

fn validated_loopback_config(value: &str) -> Result<Config, &'static str> {
    let config: Config = value
        .parse()
        .map_err(|_| "PostgreSQL connection configuration is invalid")?;
    let literal_loopback = match config.get_hosts() {
        [Host::Tcp(host)] => host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        _ => false,
    };
    let hostaddr_loopback = match config.get_hostaddrs() {
        [] => true,
        [address] => address.is_loopback(),
        _ => false,
    };
    if !literal_loopback || !hostaddr_loopback {
        return Err("PostgreSQL endpoint is not a literal loopback address");
    }
    if config.get_user() != Some("product_design") || config.get_dbname() != Some("product_design")
    {
        return Err("PostgreSQL connection identity mismatch");
    }
    Ok(config)
}

fn endpoint_pool(mut config: Config, wait: Duration) -> Pool {
    config.options(SEARCH_PATH_OPTIONS);
    let manager = Manager::from_config(
        config,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Custom(SEARCH_PATH_RECYCLE.to_owned()),
        },
    );
    Pool::builder(manager)
        .max_size(1)
        .wait_timeout(Some(wait))
        .runtime(Runtime::Tokio1)
        .build()
        .expect("bounded Product Mock endpoint pool")
}

fn request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, RESULTS_JSON)
        .body(Body::from(query.to_owned()))
        .expect("static Product Mock request")
}

async fn response(
    config: ServeConfig,
    query: &str,
) -> Result<(StatusCode, String, Vec<u8>), &'static str> {
    let response = router(Arc::new(config))
        .oneshot(request(query))
        .await
        .map_err(|_| "Product Mock request routing failed")?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|_| "Product Mock response stream failed")?
        .to_bytes()
        .to_vec();
    Ok((status, content_type, body))
}

fn assert_style_contract(actual: &TableSchema, expected: &TableSchema) {
    assert_eq!(actual.name, expected.name);
    let catalog_columns: Vec<_> = actual
        .columns
        .iter()
        .map(|column| (&column.name, &column.sql_type, column.not_null))
        .collect();
    let sealed_columns: Vec<_> = expected
        .columns
        .iter()
        .map(|column| (&column.name, &column.sql_type, column.not_null))
        .collect();
    assert_eq!(catalog_columns, sealed_columns);
    assert_eq!(actual.primary_key, expected.primary_key);
    assert_eq!(actual.foreign_keys, expected.foreign_keys);
}

fn decode_rows(body: &[u8]) -> Vec<(String, i32)> {
    let json: serde_json::Value = serde_json::from_slice(body).expect("SPARQL results JSON");
    assert_eq!(
        json["head"]["vars"],
        serde_json::json!(["styleNumber", "version"])
    );
    json["results"]["bindings"]
        .as_array()
        .expect("bindings array")
        .iter()
        .map(|binding| {
            let style = &binding["styleNumber"];
            assert_eq!(style["type"], "literal");
            assert!(style.get("xml:lang").is_none());
            if let Some(datatype) = style.get("datatype") {
                assert_eq!(datatype, "http://www.w3.org/2001/XMLSchema#string");
            }
            let version = &binding["version"];
            assert_eq!(version["type"], "literal");
            assert_eq!(
                version["datatype"],
                "http://www.w3.org/2001/XMLSchema#integer"
            );
            (
                style["value"].as_str().expect("Style number").to_owned(),
                version["value"]
                    .as_str()
                    .expect("Style version")
                    .parse()
                    .expect("integer Style version"),
            )
        })
        .collect()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires exact external gold/source roots and loopback Product Mock PostgreSQL"]
async fn exact_live_style_window_is_http_200_and_matches_ordered_sql_rows() {
    let gold = external_gold().expect("sealed Product Mock inputs");
    let source = std::env::var(support::PG_URL_ENV)
        .map_err(|_| "SF_PRODUCT_MOCK_PG_URL is required and must be UTF-8")
        .and_then(|value| validated_loopback_config(&value))
        .expect("admitted Product Mock PostgreSQL source");
    let endpoint_source = source.clone();
    let (client, connection) = source
        .connect(NoTls)
        .await
        .expect("connect to Product Mock PostgreSQL");
    let driver = tokio::spawn(async move {
        let _ = connection.await;
    });

    let actual_schema = introspect_postgres(&client, "style")
        .await
        .expect("introspect live Style");
    let sealed_schema = support::style_table_schema(&gold.style);
    assert_style_contract(&actual_schema, &sealed_schema);
    let count: i64 = client
        .query_one("SELECT count(*)::bigint FROM public.style", &[])
        .await
        .expect("count live Style rows")
        .get(0);
    assert!(
        (1..=10_000).contains(&count),
        "live Style row count={count}"
    );
    let expected: Vec<(String, i32)> = client
        .query(support::STYLE_DIRECT_SQL, &[])
        .await
        .expect("read direct ordered Style window")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(expected.len(), count as usize);

    let observed = IntrospectedSource::observe_postgres(endpoint_pool(
        endpoint_source,
        Duration::from_secs(2),
    ))
    .await
    .expect("observe the serving pool and complete public catalogue");
    let ontology = SemanticOntology::from_turtle(&gold.ontology_turtle)
        .expect("parse sealed canonical ontology");
    let mut serve = ServeConfig::from_authored_r2rml(observed, &gold.r2rml, ontology)
        .expect("admit authored Product Mock mapping against ontology and source");
    serve.set_query_admission(sf_serve::QueryAdmission::UnrestrictedDevelopment);
    serve.set_parser_runtime(
        sf_sparql::ParserRuntime::prepare(std::path::Path::new(env!(
            "CARGO_BIN_EXE_conformance-parser-host"
        )))
        .expect("explicit conformance parser host"),
    );
    serve.set_max_order_rows(support::STYLE_WINDOW_LIMIT);
    let (status, content_type, body) = response(serve, support::STYLE_SPARQL_QUERY)
        .await
        .expect("complete governed Product Mock response");
    assert_eq!(
        status,
        StatusCode::OK,
        "body={}",
        String::from_utf8_lossy(&body)
    );
    assert!(content_type.starts_with(RESULTS_JSON));
    assert_eq!(decode_rows(&body), expected);

    drop(client);
    driver.abort();
}

#[test]
#[ignore = "requires exact external gold/source roots"]
fn over_cap_style_window_is_identified_before_poison_postgres_io() {
    let gold = external_gold().expect("sealed Product Mock inputs");
    let maps = sf_mapping::parse_r2rml(&gold.r2rml).expect("parse sealed Style mapping");
    let mut poison: Config = "host=127.0.0.1 port=1 user=product_design dbname=product_design"
        .parse()
        .expect("static poison source");
    poison.connect_timeout(Duration::from_millis(20));
    let pool = endpoint_pool(poison, Duration::from_millis(20));
    let query = support::STYLE_SPARQL_QUERY.replace("LIMIT 10001", "LIMIT 10002");
    assert_ne!(query, support::STYLE_SPARQL_QUERY);
    let admitted =
        sf_sparql::parse_and_translate(support::STYLE_SPARQL_QUERY, &maps, Dialect::Postgres)
            .expect("compile admitted finite Style window");
    assert!(!admitted
        .source_sized_states_with_order_window(support::STYLE_WINDOW_LIMIT)
        .contains(&SourceSizedState::GlobalOrder));
    let over_cap = sf_sparql::parse_and_translate(&query, &maps, Dialect::Postgres)
        .expect("compile over-cap Style window for structural preflight");
    assert!(over_cap
        .source_sized_states_with_order_window(support::STYLE_WINDOW_LIMIT)
        .contains(&SourceSizedState::GlobalOrder));
    assert_eq!(pool.status().size, 0, "poison pool must remain unopened");
}
