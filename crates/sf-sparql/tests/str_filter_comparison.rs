//! Real SQLite lexical comparisons for the exact successor cursor operand.
use rusqlite::Connection;
use sf_core::query_control::UncontrolledQueryControl;
use sf_core::{SourceId, SourceMapping, Term};
use sf_sparql::cache::generated::{ConstantCoverageError, ConstantOccurrence};
use sf_sparql::{exec, CompilerBinding, Epoch, Tbox};
use sf_sql::Dialect;

fn fixture() -> (CompilerBinding, Connection) {
    fixture_mapping(true)
}

fn fixture_mapping(explicit: bool) -> (CompilerBinding, Connection) {
    let mapping = r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        <urn:map> rr:logicalTable [ rr:tableName "items" ];
          rr:subjectMap [ rr:template "http://ex/{id}" ];
          rr:predicateObjectMap [ rr:predicate <http://ex/value>;
            rr:objectMap [ rr:column "v";
              rr:datatype <http://www.w3.org/2001/XMLSchema#string> ] ] .
    "#;
    let mapping = if explicit {
        mapping.to_owned()
    } else {
        mapping.replace("rr:datatype <http://www.w3.org/2001/XMLSchema#string>", "")
    };
    let maps = sf_mapping::parse_r2rml(&mapping).unwrap();
    let binding = CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        64,
    );
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items(id INTEGER, v TEXT COLLATE NOCASE);
        INSERT INTO items VALUES (1,'10'),(2,'2'),(3,'A'),(4,'a'),(5,'a '),
        (6,char(233)),(7,NULL),(8,'2');",
    )
    .unwrap();
    (binding, conn)
}

fn values(binding: &CompilerBinding, conn: &Connection, condition: &str) -> Vec<String> {
    let query = format!("SELECT ?v WHERE {{ ?s <http://ex/value> ?v FILTER({condition}) }}");
    let constant = |_: ConstantOccurrence<'_>| Ok::<_, ConstantCoverageError>(());
    let cold = binding
        .compile_shared_with_generated_admission(&query, &UncontrolledQueryControl, constant)
        .unwrap();
    let warm = binding
        .compile_shared_with_generated_admission(&query, &UncontrolledQueryControl, constant)
        .unwrap();
    let uncached = binding
        .compile_uncached_shared_with_generated_admission(
            &query,
            &UncontrolledQueryControl,
            constant,
        )
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&cold, &warm));
    let mut reference = None;
    for plan in [cold, warm, uncached] {
        let mut rows: Vec<_> = exec::select(&plan, conn)
            .unwrap()
            .rows
            .into_iter()
            .map(|r| {
                let Some(Term::Literal(value)) = &r[0] else {
                    panic!("expected literal: {r:?}")
                };
                value.value().to_owned()
            })
            .collect();
        rows.sort();
        if let Some(expected) = &reference {
            assert_eq!(&rows, expected);
        }
        reference = Some(rows);
    }
    reference.unwrap()
}

#[test]
fn lexical_order_flipped_operators_duplicates_and_collation_are_exact() {
    let (binding, conn) = fixture();
    for (left, flipped, expected) in [
        ("STR(?v) < \"2\"", "\"2\" > STR(?v)", vec!["10"]),
        ("STR(?v) <= \"2\"", "\"2\" >= STR(?v)", vec!["10", "2", "2"]),
        ("STR(?v) > \"a\"", "\"a\" < STR(?v)", vec!["a ", "é"]),
        ("STR(?v) >= \"a\"", "\"a\" <= STR(?v)", vec!["a", "a ", "é"]),
        ("STR(?v) = \"A\"", "\"A\" = STR(?v)", vec!["A"]),
    ] {
        assert_eq!(values(&binding, &conn, left), expected, "{left}");
        assert_eq!(values(&binding, &conn, flipped), expected, "{flipped}");
    }
}

#[test]
fn error_boolean_composition_and_null_elimination_are_preserved() {
    let (binding, conn) = fixture();
    assert_eq!(values(&binding, &conn, "!(STR(?v) >= \"2\")"), ["10"]);
    assert!(values(&binding, &conn, "!(STR(?missing) > \"2\")").is_empty());
    assert_eq!(
        values(&binding, &conn, "STR(?missing) > \"2\" || STR(?v) = \"2\""),
        ["2", "2"]
    );
    assert!(values(&binding, &conn, "STR(?missing) > \"2\" && STR(?v) = \"2\"").is_empty());
    assert_eq!(
        values(&binding, &conn, "STR(?v) > \"10\" && STR(?v) < \"A\""),
        ["2", "2"]
    );
}

#[test]
fn hostile_cursor_stays_a_bound_literal() {
    let (binding, conn) = fixture();
    let rows = values(&binding, &conn, "STR(?v) = \"' OR 1=1 --\"");
    assert!(rows.is_empty());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        8
    );
}

#[test]
fn nontext_source_cannot_silently_supply_numeric_ordering_to_str() {
    for explicit in [true, false] {
        let (binding, _) = fixture_mapping(explicit);
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items(id INTEGER, v INTEGER);
        INSERT INTO items VALUES (1,10),(2,2);",
        )
        .unwrap();
        let query = "SELECT ?v WHERE { ?s <http://ex/value> ?v FILTER(STR(?v) < \"2\") }";
        let plan = binding
            .compile_shared_with_generated_admission(
                query,
                &UncontrolledQueryControl,
                |_: ConstantOccurrence<'_>| Ok::<_, ConstantCoverageError>(()),
            )
            .unwrap();
        assert!(
            exec::select(&plan, &conn).is_err(),
            "unqualified numeric source must refuse, never use numeric ordering"
        );
    }
}

#[test]
fn mixed_text_blob_storage_refuses_before_raw_comparison() {
    for explicit in [true, false] {
        assert_mixed_text_blob_refused(explicit);
    }
}

fn assert_mixed_text_blob_refused(explicit: bool) {
    let queries = [
        "SELECT ?v WHERE { ?s <http://ex/value> ?v FILTER(STR(?v) < \"2\") }",
        "SELECT ?v WHERE { ?s <http://ex/value> ?v FILTER(\"2\" < STR(?v)) }",
        "SELECT (COUNT(DISTINCT ?v) AS ?n) WHERE { ?s <http://ex/value> ?v }",
    ];
    let (binding, _) = fixture_mapping(explicit);
    let conn = Connection::open_in_memory().unwrap();
    // Declared TEXT affinity keeps a BLOB cell; BINARY orders it after all text.
    conn.execute_batch(
        "CREATE TABLE items(id INTEGER, v TEXT COLLATE NOCASE);
        INSERT INTO items VALUES (1,X'3130'),(2,'2'),(3,'A'),(4,'a');",
    )
    .unwrap();
    for query in queries {
        for plan in plans(&binding, query) {
            assert!(
                exec::select(&plan, &conn).is_err(),
                "non-text cell must refuse, never storage-class ordering: {query}"
            );
        }
    }
    // Homogeneous text in the same declared column keeps exact lexical results.
    conn.execute("DELETE FROM items WHERE id = 1", []).unwrap();
    for plan in plans(&binding, queries[1]) {
        assert_eq!(exec::select(&plan, &conn).unwrap().rows.len(), 2);
    }
    for plan in plans(&binding, queries[2]) {
        let rows = exec::select(&plan, &conn).unwrap().rows;
        assert!(
            matches!(&rows[0][0], Some(Term::Literal(v)) if v.value() == "3"),
            "{rows:?}"
        );
    }
}

#[test]
fn natural_text_mapping_keeps_lexical_and_null_semantics() {
    let (binding, conn) = fixture_mapping(false);
    assert_eq!(values(&binding, &conn, "STR(?v) < \"2\""), ["10"]);
    assert_eq!(values(&binding, &conn, "STR(?v) = \"A\""), ["A"]);
    assert_eq!(values(&binding, &conn, "!(STR(?v) >= \"2\")"), ["10"]);
}

fn mapped_columns() -> CompilerBinding {
    let mapping = r#"
      @prefix rr: <http://www.w3.org/ns/r2rml#> .
      @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
      <urn:m> rr:logicalTable [ rr:tableName "items" ];
        rr:subjectMap [ rr:template "http://ex/{id}" ];
        rr:predicateObjectMap [ rr:predicate <http://ex/id>;
          rr:objectMap [ rr:column "id" ] ];
        rr:predicateObjectMap [ rr:predicate <http://ex/value>;
          rr:objectMap [ rr:column "v"; rr:datatype xsd:string ] ];
        rr:predicateObjectMap [ rr:predicate <http://ex/other>;
          rr:objectMap [ rr:column "w"; rr:datatype xsd:string ] ] .
    "#;
    CompilerBinding::from_unverified_observation(
        SourceMapping::new(
            SourceId::new(0).unwrap(),
            sf_mapping::parse_r2rml(mapping).unwrap(),
        ),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        64,
    )
}

fn plans(binding: &CompilerBinding, query: &str) -> Vec<std::sync::Arc<sf_sparql::Plan>> {
    let free = &UncontrolledQueryControl;
    let check = |_: ConstantOccurrence<'_>| Ok::<_, ConstantCoverageError>(());
    let cold = binding
        .compile_shared_with_generated_admission(query, free, check)
        .unwrap();
    let warm = binding
        .compile_shared_with_generated_admission(query, free, check)
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&cold, &warm));
    vec![
        cold,
        warm,
        binding
            .compile_uncached_shared_with_generated_admission(query, free, check)
            .unwrap(),
    ]
}

#[test]
fn explicit_string_column_pairs_require_text_storage_on_both_sides() {
    for (left, right, succeeds) in [
        ("INTEGER", "INTEGER", false),
        ("TEXT", "INTEGER", false),
        ("INTEGER", "TEXT", false),
        ("TEXT", "TEXT", true),
    ] {
        let binding = mapped_columns();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE items(id INTEGER,v {left},w {right});
            INSERT INTO items VALUES(1,'10','2');"
        ))
        .unwrap();
        for plan in plans(
            &binding,
            "SELECT ?v WHERE { ?s <http://ex/value> ?v; <http://ex/other> ?w FILTER(?v < ?w) }",
        ) {
            let result = exec::select(&plan, &conn);
            if succeeds {
                let rows = result.unwrap().rows;
                assert_eq!(rows.len(), 1);
                assert!(matches!(&rows[0][0], Some(Term::Literal(v)) if v.value() == "10"));
            } else {
                assert!(
                    result.is_err(),
                    "unqualified {left}/{right} column pair must refuse"
                );
            }
        }
    }
}

#[test]
fn optional_unbound_operand_preserves_not_and_or_truth_tables() {
    let binding = mapped_columns();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items(id INTEGER,v TEXT,w TEXT);
        INSERT INTO items VALUES(1,'10',NULL),(2,NULL,NULL),(3,'2',NULL);",
    )
    .unwrap();
    for (condition, expected) in [
        ("!(STR(?v) >= \"2\")", vec!["1"]),
        ("STR(?v) > \"10\" || ?id = 2", vec!["2", "3"]),
        ("STR(?v) > \"10\" && ?id = 2", vec![]),
    ] {
        let query = format!("SELECT ?id WHERE {{ ?s <http://ex/id> ?id OPTIONAL {{ ?s <http://ex/value> ?v }} FILTER({condition}) }}");
        for plan in plans(&binding, &query) {
            let mut actual: Vec<_> = exec::select(&plan, &conn)
                .unwrap()
                .rows
                .into_iter()
                .map(|row| {
                    let Some(Term::Literal(v)) = &row[0] else {
                        panic!("expected id")
                    };
                    v.value().to_owned()
                })
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "{condition}");
        }
    }
}

#[test]
fn empty_and_typed_cursors_count_distinct_subquery_pages() {
    let (binding, conn) = fixture();
    assert_eq!(
        values(&binding, &conn, "STR(?v) > \"\""),
        ["10", "2", "2", "A", "a", "a ", "é"]
    );
    assert_eq!(
        values(
            &binding,
            &conn,
            "STR(?v) < \"2\"^^<http://www.w3.org/2001/XMLSchema#string>"
        ),
        ["10"]
    );
    // Same aggregate-subquery plus outer cursor/order/slice shape as Query's
    // unchanged successor enumerator; no unsupported sliced join input.
    let query = "SELECT DISTINCT ?v ?count WHERE { { SELECT (COUNT(DISTINCT ?all) AS ?count) WHERE { ?item <http://ex/value> ?all } } ?s <http://ex/value> ?v FILTER(STR(?v) > \"10\") } ORDER BY ?v LIMIT 3";
    for plan in plans(&binding, query) {
        let rows = exec::select(&plan, &conn).unwrap().rows;
        assert_eq!(rows.len(), 3);
        for (row, expected) in rows.iter().zip(["2", "A", "a"]) {
            assert!(
                matches!(&row[0], Some(Term::Literal(v)) if v.value() == expected),
                "{rows:?}"
            );
            assert!(
                matches!(&row[1], Some(Term::Literal(v)) if v.value() == "6"),
                "{rows:?}"
            );
        }
    }
}

#[test]
fn count_variants_preserve_null_bag_and_numeric_identity() {
    let binding = mapped_columns();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items(id INTEGER,v TEXT COLLATE NOCASE,w TEXT);
      INSERT INTO items VALUES(1,'A',NULL),(1,'a',NULL),(2,'a',NULL),(3,NULL,NULL);",
    )
    .unwrap();
    for (predicate, aggregate, expected) in [
        ("value", "COUNT(?v)", "3"),
        ("value", "COUNT(DISTINCT ?v)", "2"),
        // Repeated id=1 has the same subject/predicate/object: one RDF triple.
        ("id", "COUNT(?v)", "3"),
        ("id", "COUNT(DISTINCT ?v)", "3"),
    ] {
        let query = format!("SELECT ({aggregate} AS ?n) WHERE {{ ?s <http://ex/{predicate}> ?v }}");
        for plan in plans(&binding, &query) {
            let rows = exec::select(&plan, &conn).unwrap().rows;
            assert_eq!(rows.len(), 1);
            assert!(
                matches!(&rows[0][0], Some(Term::Literal(v)) if v.value() == expected),
                "{rows:?}"
            );
        }
    }
}
