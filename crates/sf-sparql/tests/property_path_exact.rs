//! Adversarial exactness tests for recursive property paths (ADR-0049 R1).
//!
//! A supported closure must reach a true finite-pair fixed point. A numeric walk
//! limit may fail explicitly, but must never be returned as a successful prefix.

use rusqlite::{params, Connection};
use sf_core::ir::{
    LogicalSource, ObjectMap, PredicateObjectMap, Segment, SubjectMap, Template, TermMap, TermSpec,
    TriplesMap,
};
use sf_core::{Column, NamedNode, TableSchema};
use sf_sparql::{exec, parse_and_translate};
use sf_sql::Dialect;

const REACHES: &str = "http://ex/reaches";

#[test]
#[ignore = "known ADR-0049 RDF-key collation defect; required explicit release check remains failing"]
fn path_keys_do_not_inherit_case_insensitive_source_collation() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE edge(parent TEXT COLLATE NOCASE, child TEXT COLLATE NOCASE); INSERT INTO edge VALUES ('a','B'),('b','c'),('A','B');").unwrap();
    for operator in ["+", "*", "?", "|<http://ex/reaches>"] {
        let query = format!("SELECT ?s ?o WHERE {{ ?s (<{REACHES}>{operator}) ?o }}");
        let plan = parse_and_translate(&query, &edge_mapping(), Dialect::Sqlite).unwrap();
        let rows = exec::select(&plan, &conn).unwrap().rows;
        let actual: std::collections::BTreeSet<_> = rows
            .iter()
            .map(|row| {
                (
                    row[0].as_ref().unwrap().to_string(),
                    row[1].as_ref().unwrap().to_string(),
                )
            })
            .collect();
        let mut expected: std::collections::BTreeSet<_> = [("a", "B"), ("b", "c"), ("A", "B")]
            .map(|(s, o)| (format!("<http://ex/n/{s}>"), format!("<http://ex/n/{o}>")))
            .into_iter()
            .collect();
        if matches!(operator, "*" | "?") {
            expected.extend(
                ["a", "A", "b", "B", "c"]
                    .map(|n| (format!("<http://ex/n/{n}>"), format!("<http://ex/n/{n}>"))),
            );
        }
        assert_eq!(actual, expected, "RDF key identity for {operator}");
        assert_eq!(rows.len(), expected.len());
    }
}

fn edge_mapping() -> Vec<TriplesMap> {
    vec![TriplesMap {
        id: "EDGE".to_owned(),
        source: LogicalSource::Table("edge".to_owned()),
        subject: SubjectMap {
            term: TermMap::Template(
                Template::parse("http://ex/n/{parent}").unwrap(),
                TermSpec::iri(),
            ),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![PredicateObjectMap {
            predicates: vec![TermMap::Constant(NamedNode::new_unchecked(REACHES).into())],
            objects: vec![ObjectMap::Term(TermMap::Template(
                Template::parse("http://ex/n/{child}").unwrap(),
                TermSpec::iri(),
            ))],
            graphs: vec![],
        }],
    }]
}

fn rowid_reflexive_mapping() -> Vec<TriplesMap> {
    let row_node = TermMap::Template(
        Template::from_segments(vec![
            Segment::Literal("edge_".into()),
            Segment::Column("rowid".into()),
        ])
        .expect("non-empty row template"),
        TermSpec::blank_node(),
    );
    vec![TriplesMap {
        id: "ROWID_EDGE".to_owned(),
        source: LogicalSource::Table("edge".to_owned()),
        subject: SubjectMap {
            term: row_node.clone(),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![PredicateObjectMap {
            predicates: vec![TermMap::Constant(NamedNode::new_unchecked(REACHES).into())],
            objects: vec![ObjectMap::Term(row_node)],
            graphs: vec![],
        }],
    }]
}

fn chain(edge_count: u32) -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE edge(parent INTEGER, child INTEGER);")
        .unwrap();
    let tx = conn.transaction().unwrap();
    {
        let mut insert = tx
            .prepare("INSERT INTO edge(parent, child) VALUES (?, ?)")
            .unwrap();
        for parent in 0..edge_count {
            insert.execute(params![parent, parent + 1]).unwrap();
        }
    }
    tx.commit().unwrap();
    conn
}

fn cyclic_graph() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE edge(parent INTEGER, child INTEGER);
         INSERT INTO edge VALUES (1, 2), (2, 3), (3, 1), (1, 3);",
    )
    .unwrap();
    conn
}

fn iri(node: u32) -> String {
    format!("<http://ex/n/{node}>")
}

#[test]
fn one_or_more_is_exact_at_and_beyond_the_old_256_hop_boundary() {
    const EDGE_COUNT: u32 = 258;
    let maps = edge_mapping();
    let conn = chain(EDGE_COUNT);
    let query = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>+ ?o }}");
    let plan = parse_and_translate(&query, &maps, Dialect::Sqlite).unwrap();

    let solutions = exec::select(&plan, &conn).unwrap();
    let pairs: std::collections::BTreeSet<(String, String)> = solutions
        .rows
        .iter()
        .map(|row| {
            (
                row[0].as_ref().unwrap().to_string(),
                row[1].as_ref().unwrap().to_string(),
            )
        })
        .collect();

    let expected_pairs = usize::try_from(EDGE_COUNT * (EDGE_COUNT + 1) / 2).unwrap();
    assert_eq!(pairs.len(), expected_pairs, "closure returned a prefix");
    assert!(pairs.contains(&(iri(0), iri(256))), "256-hop pair missing");
    assert!(pairs.contains(&(iri(0), iri(257))), "257-hop pair missing");
    assert!(pairs.contains(&(iri(0), iri(258))), "258-hop pair missing");
}

#[test]
fn evidenced_target_dialects_deduplicate_on_pairs_not_walk_depth() {
    let maps = edge_mapping();
    let mut depth_keyed = Vec::new();

    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        for operator in ['+', '*'] {
            let query = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>{operator} ?o }}");
            let plan = parse_and_translate(&query, &maps, dialect).unwrap();
            let sql = &plan.emitted().unwrap()[0].sql;
            if sql.contains("sf_d") || sql.contains("256") {
                depth_keyed.push(format!("{dialect:?}:{operator}"));
            }
            assert!(
                !sql.to_uppercase().contains("UNION ALL"),
                "recursive pair fixed point must discard revisits: {dialect:?}:{operator}: {sql}"
            );
        }
    }

    assert!(
        depth_keyed.is_empty(),
        "closure identity still includes walk depth: {depth_keyed:?}"
    );
}

#[test]
fn pair_fixed_point_terminates_and_is_exact_on_a_cycle() {
    let maps = edge_mapping();

    for operator in ['+', '*'] {
        let conn = cyclic_graph();
        let query = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>{operator} ?o }}");
        let plan = parse_and_translate(&query, &maps, Dialect::Sqlite).unwrap();
        let solutions = exec::select(&plan, &conn).unwrap();
        let pairs: std::collections::BTreeSet<(String, String)> = solutions
            .rows
            .iter()
            .map(|row| {
                (
                    row[0].as_ref().unwrap().to_string(),
                    row[1].as_ref().unwrap().to_string(),
                )
            })
            .collect();

        let expected: std::collections::BTreeSet<(String, String)> = (1..=3)
            .flat_map(|subject| (1..=3).map(move |object| (iri(subject), iri(object))))
            .collect();
        assert_eq!(pairs, expected, "cyclic reaches{operator} was not exact");
        assert_eq!(
            solutions.rows.len(),
            9,
            "cyclic reaches{operator} duplicated pairs"
        );
    }
}

#[test]
fn path_key_comparison_preserves_native_character_padding() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE edge(parent CHARACTER(4), child CHARACTER(4)); INSERT INTO edge VALUES ('a','b');").unwrap();
    let mut maps = edge_mapping();
    maps[0].source = LogicalSource::Query(
        "SELECT parent COLLATE BINARY AS parent, child COLLATE BINARY AS child FROM edge".into(),
    );
    let ordinary = parse_and_translate(
        &format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}> ?o }}"),
        &maps,
        Dialect::Sqlite,
    )
    .unwrap();
    let closure = parse_and_translate(
        &format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>+ ?o }}"),
        &maps,
        Dialect::Sqlite,
    )
    .unwrap();
    let expected = exec::select(&ordinary, &conn).unwrap().rows;
    assert_eq!(
        expected[0][0].as_ref().unwrap().to_string(),
        "<http://ex/n/a%20%20%20>"
    );
    assert_eq!(exec::select(&closure, &conn).unwrap().rows, expected);
}

#[test]
fn path_key_comparison_preserves_natural_date_type() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE edge(parent TEXT, child DATE); INSERT INTO edge VALUES ('a','2026-09-08');",
    )
    .unwrap();
    let mut maps = edge_mapping();
    maps[0].source = LogicalSource::Query(
        "SELECT parent COLLATE BINARY AS parent, child COLLATE BINARY AS child FROM edge".into(),
    );
    maps[0].predicate_object_maps[0].objects = vec![ObjectMap::Term(TermMap::Column(
        "child".into(),
        TermSpec::plain_literal(),
    ))];
    let ordinary = parse_and_translate(
        &format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}> ?o }}"),
        &maps,
        Dialect::Sqlite,
    )
    .unwrap();
    let alternative = parse_and_translate(
        &format!("SELECT ?s ?o WHERE {{ ?s (<{REACHES}>|<{REACHES}>) ?o }}"),
        &maps,
        Dialect::Sqlite,
    )
    .unwrap();
    let expected = exec::select(&ordinary, &conn).unwrap().rows;
    assert_eq!(
        expected[0][1].as_ref().unwrap().to_string(),
        "\"2026-09-08\"^^<http://www.w3.org/2001/XMLSchema#date>"
    );
    assert_eq!(exec::select(&alternative, &conn).unwrap().rows, expected);
}

#[test]
fn unproven_dialects_reject_recursive_paths_before_emission() {
    let maps = edge_mapping();
    let unproven = [
        Dialect::Redshift,
        Dialect::DuckDb,
        Dialect::SqlServer,
        Dialect::Oracle,
        Dialect::SapHana,
        Dialect::MonetDb,
        Dialect::Snowflake,
        Dialect::BigQuery,
        Dialect::Athena,
        Dialect::Databricks,
        Dialect::Trino,
        Dialect::PrestoDB,
        Dialect::Db2,
        Dialect::H2,
        Dialect::Spark,
        Dialect::Dremio,
        Dialect::Denodo,
        Dialect::Teiid,
    ];

    for dialect in unproven {
        for operator in ['+', '*'] {
            let query = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>{operator} ?o }}");
            let error = parse_and_translate(&query, &maps, dialect).unwrap_err();
            assert!(
                matches!(error, sf_sparql::Error::Unsupported(ref message)
                    if message.contains("proven finite-pair fixed point")),
                "{dialect:?}:{operator} did not fail with the typed path capability error: {error}"
            );
        }
    }
}

#[test]
fn no_pk_direct_mapping_paths_use_each_dialects_physical_row_identifier() {
    let mut table = TableSchema::new("edge");
    table.columns = vec![Column::new("child", "text", false)];
    let maps = sf_mapping::direct_mapping(std::slice::from_ref(&table), "http://ex/")
        .expect("generate direct mapping");
    let predicate = "http://ex/edge#child";

    let query = format!("SELECT ?s ?o WHERE {{ ?s (<{predicate}>|<{predicate}>) ?o }}");

    let postgres = parse_and_translate(&query, &maps, Dialect::Postgres)
        .expect("translate PostgreSQL path")
        .emitted()
        .expect("emit PostgreSQL path")[0]
        .sql
        .clone();
    assert!(postgres.contains("(h0.ctid)::TEXT"), "{postgres}");
    assert!(!postgres.contains("h0.\"rowid\""), "{postgres}");

    let sqlite = parse_and_translate(&query, &maps, Dialect::Sqlite)
        .expect("translate SQLite path")
        .emitted()
        .expect("emit SQLite path")[0]
        .sql
        .clone();
    assert!(sqlite.contains("h0.\"rowid\""), "{sqlite}");
    assert!(!sqlite.contains("ctid"), "{sqlite}");
}

#[test]
fn reflexive_rowid_paths_use_postgresql_ctid_and_sqlite_rowid() {
    let maps = rowid_reflexive_mapping();
    let query = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>? ?o }}");

    let postgres = parse_and_translate(&query, &maps, Dialect::Postgres)
        .expect("translate PostgreSQL reflexive path")
        .emitted()
        .expect("emit PostgreSQL reflexive path")[0]
        .sql
        .clone();
    assert!(postgres.contains("(h0.ctid)::TEXT"), "{postgres}");
    assert!(!postgres.contains("h0.\"rowid\""), "{postgres}");

    let sqlite = parse_and_translate(&query, &maps, Dialect::Sqlite)
        .expect("translate SQLite reflexive path")
        .emitted()
        .expect("emit SQLite reflexive path")[0]
        .sql
        .clone();
    assert!(sqlite.contains("h0.\"rowid\""), "{sqlite}");
    assert!(!sqlite.contains("ctid"), "{sqlite}");
}

#[test]
fn query_backed_rowid_path_endpoints_remain_ordinary_postgresql_columns() {
    let mut maps = rowid_reflexive_mapping();
    maps[0].source = LogicalSource::Query("SELECT 7 AS rowid".to_owned());
    let query = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>? ?o }}");

    let postgres = parse_and_translate(&query, &maps, Dialect::Postgres)
        .expect("translate query-backed PostgreSQL path")
        .emitted()
        .expect("emit query-backed PostgreSQL path")[0]
        .sql
        .clone();

    assert!(postgres.contains("h0.\"rowid\""), "{postgres}");
    assert!(!postgres.contains("h0.ctid"), "{postgres}");
}
