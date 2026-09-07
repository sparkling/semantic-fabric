//! Fixed-K ORDER BY heap proof across a 100x file-backed source-cardinality sweep.

use rusqlite::Connection;
use sf_sparql::{exec, parse_and_translate_with, Tbox};
use sf_sql::Dialect;

#[global_allocator]
static GLOBAL: sf_bench::mem::Tracking = sf_bench::mem::Tracking;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ; rr:objectMap [ rr:column "value" ]
  ] .
"#;

const QUERY: &str = "SELECT ?value WHERE { ?item <http://example.test/value> ?value } \
                     ORDER BY ?value OFFSET 20 LIMIT 80";

fn source(path: &std::path::Path, rows: u32) -> Connection {
    let connection = Connection::open(path).expect("open file-backed source");
    connection
        .execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT NOT NULL);")
        .expect("create source table");
    let transaction = connection.unchecked_transaction().expect("begin fixture");
    {
        let mut insert = transaction
            .prepare("INSERT INTO items (id, value) VALUES (?1, ?2)")
            .expect("prepare fixture insert");
        for index in 0..rows {
            insert
                .execute(rusqlite::params![index, format!("value-{index:09}")])
                .expect("insert fixture row");
        }
    }
    transaction.commit().expect("commit fixture");
    connection
}

fn measure(directory: &std::path::Path, rows: u32) -> (usize, i64) {
    let connection = source(&directory.join(format!("order-{rows}.db")), rows);
    let mapping = sf_mapping::parse_r2rml(MAPPING).expect("parse mapping");
    let schema =
        sf_sql::introspect::introspect_sqlite(&connection, "items").expect("introspect source");
    let plan = parse_and_translate_with(
        QUERY,
        &mapping,
        Dialect::Sqlite,
        &Tbox::default(),
        &[schema],
    )
    .expect("compile fixed-window ORDER query");

    let baseline = sf_bench::mem::reset_peak();
    let rows = exec::select(&plan, &connection).expect("execute fixed-window ORDER");
    (rows.rows.len(), sf_bench::mem::window_peak(baseline))
}

#[test]
fn fixed_order_window_heap_is_source_cardinality_independent() {
    let directory = tempfile::TempDir::new().expect("temporary fixture directory");
    // Initialize process-wide parallel execution before every measured window.
    let _ = measure(directory.path(), 2_048);
    let scales = [1_000_u32, 10_000, 100_000];
    let mut peaks = Vec::new();
    for scale in scales {
        let (result_rows, peak) = measure(directory.path(), scale);
        assert_eq!(result_rows, 80, "fixed LIMIT at scale {scale}");
        eprintln!("bounded ORDER: source_rows={scale} result_rows={result_rows} peak_B={peak}");
        peaks.push(peak);
    }

    let ten_x = u128::try_from(peaks[1]).expect("non-negative 10x peak");
    let hundred_x = u128::try_from(peaks[2]).expect("non-negative 100x peak");
    assert!(
        hundred_x * 100 <= ten_x * 110,
        "fixed-K heap growth from 10x to 100x must be at most 10%: {peaks:?}"
    );
}
