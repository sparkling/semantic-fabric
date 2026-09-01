//! Adversarial exactness tests for recursive property paths (ADR-0038 R1).
//!
//! A supported closure must reach a true finite-pair fixed point. A numeric walk
//! limit may fail explicitly, but must never be returned as a successful prefix.

use rusqlite::{params, Connection};
use sf_core::ir::{
    LogicalSource, ObjectMap, PredicateObjectMap, SubjectMap, Template, TermMap, TermSpec,
    TriplesMap,
};
use sf_core::NamedNode;
use sf_sparql::{exec, parse_and_translate};
use sf_sql::Dialect;

const REACHES: &str = "http://ex/reaches";

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
fn admitted_dialects_deduplicate_on_pairs_not_walk_depth() {
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
