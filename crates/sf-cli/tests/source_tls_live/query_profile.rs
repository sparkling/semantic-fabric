//! Actual public query results against only owned, pinned, TLS-enabled providers.
use super::*;
use std::collections::BTreeSet;
#[path = "ordinary_identity.rs"]
mod ordinary_identity;

const EDGE: &str = "http://example.test/edge";
const MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#edges> a rr:TriplesMap ; rr:logicalTable [ rr:tableName "items" ] ;
rr:subjectMap [ rr:template "http://example.test/n/{src}" ] ;
rr:predicateObjectMap [ rr:predicate <http://example.test/edge> ;
rr:objectMap [ rr:template "http://example.test/n/{dst}" ; rr:termType rr:IRI ] ] ."#;

fn iri(node: u32) -> String {
    format!("http://example.test/n/{node}")
}

fn sql(database: &Database, statement: &str) {
    database.sql(&format!(
        "{}{statement}",
        if database.source.starts_with("mysql:") {
            "USE sf_tls; "
        } else {
            ""
        }
    ));
}

fn seed(database: &Database, edges: &[(u32, u32)]) {
    let values = edges
        .iter()
        .enumerate()
        .map(|(id, (src, dst))| format!("({id},'{src}','{dst}','same')"))
        .collect::<Vec<_>>()
        .join(",");
    sql(
        database,
        &format!("DELETE FROM items; INSERT INTO items(id,src,dst,value) VALUES {values}"),
    );
}

fn start(fixture: &Fixture, database: &Database) -> (Server, SocketAddr) {
    let (mut command, address) = command(fixture, database, None);
    command.args([
        "--pg-pool-size",
        "1",
        "--max-concurrent-requests",
        "1",
        "--timeout-secs",
        "2",
    ]);
    let log = fixture.root.join("query-profile.stderr");
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some((status, _)) = request(address, "ASK {}", None) {
            assert_eq!(status, 401);
            return (server, address);
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "query-profile startup: {}",
            std::fs::read_to_string(&log).unwrap()
        );
        assert!(Instant::now() < until, "query-profile startup deadline");
        thread::sleep(Duration::from_millis(25));
    }
}

fn graph(address: SocketAddr, fixture: &Fixture, query: &str, format: &str) -> BTreeSet<String> {
    let (status, body) = request_format(address, query, Some(&fixture.token), format).unwrap();
    assert_eq!(status, 200, "{query}: {}", String::from_utf8_lossy(&body));
    let triples = oxttl::TurtleParser::new()
        .for_slice(&body)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let set: BTreeSet<_> = triples.iter().map(ToString::to_string).collect();
    assert_eq!(set.len(), triples.len(), "duplicate graph triples: {query}");
    set
}

fn assert_describe(address: SocketAddr, fixture: &Fixture, database: &Database) {
    let expected: BTreeSet<_> = [1, 2]
        .into_iter()
        .map(|o| format!("<{}> <{EDGE}> <{}>", iri(0), iri(o)))
        .collect();
    for format in ["application/n-triples", "text/turtle"] {
        for query in [
            format!("DESCRIBE <{}>", iri(0)),
            format!(
                "DESCRIBE ?s WHERE {{ VALUES ?s {{ <{}> <{}> }} }}",
                iri(0),
                iri(0)
            ),
            format!("DESCRIBE ?s WHERE {{ ?s <{EDGE}> <{}> }}", iri(1)),
            format!("DESCRIBE * WHERE {{ ?s <{EDGE}> <{}> }}", iri(1)),
        ] {
            assert_eq!(graph(address, fixture, &query, format), expected, "{query}");
        }
    }
    // Unsupported shapes must reject before touching the locked source.
    let mut held = database.hold_table("items");
    for query in [
        format!("DESCRIBE <{}> <{}>", iri(0), iri(1)),
        format!("DESCRIBE ?s WHERE {{ ?x <{EDGE}> ?y }}"),
        format!("DESCRIBE ?s WHERE {{ ?s <{EDGE}> ?o }} LIMIT 1"),
        format!("DESCRIBE ?s WHERE {{ ?s <{EDGE}> ?o }} ORDER BY ?s"),
    ] {
        let before = Instant::now();
        let (status, _) =
            request_format(address, &query, Some(&fixture.token), "text/turtle").unwrap();
        assert_eq!(status, 501, "{query}");
        assert!(before.elapsed() < Duration::from_secs(1));
        held.assert_held();
    }
}

fn rows(address: SocketAddr, fixture: &Fixture, query: &str) -> Vec<serde_json::Value> {
    let (status, body) = request_format_bounded(
        address,
        query,
        Some(&fixture.token),
        "application/sparql-results+json",
        8 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(status, 200, "{query}: {}", String::from_utf8_lossy(&body));
    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["results"]["bindings"]
        .as_array()
        .unwrap()
        .clone()
}

fn assert_joined_paths(address: SocketAddr, fixture: &Fixture) {
    // Authored regular column references must resolve before a D1 DISTINCT
    // wrapper is prepared, not only when a typed property path is rendered.
    let ordinary = rows(
        address,
        fixture,
        &format!("SELECT ?s ?o WHERE {{ ?s <{EDGE}> ?o }}"),
    );
    let expected: BTreeSet<_> = [(0, 1), (0, 2), (1, 3), (2, 3), (3, 0)]
        .map(|(s, o)| (iri(s), iri(o)))
        .into_iter()
        .collect();
    let actual: BTreeSet<_> = ordinary
        .iter()
        .map(|row| {
            (
                row["s"]["value"].as_str().unwrap().to_owned(),
                row["o"]["value"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        actual, expected,
        "ordinary DISTINCT wrapper resolves live native names"
    );
    assert_eq!(ordinary.len(), expected.len());
    let joined = rows(
        address,
        fixture,
        &format!("SELECT DISTINCT ?s ?o WHERE {{ ?s <{EDGE}>+ ?o . ?s <{EDGE}>+ ?direct }}"),
    );
    let actual: BTreeSet<_> = joined
        .iter()
        .map(|row| {
            (
                row["s"]["value"].as_str().unwrap().to_owned(),
                row["o"]["value"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected: BTreeSet<_> = (0..4)
        .flat_map(|s| (0..4).map(move |o| (iri(s), iri(o))))
        .collect();
    assert_eq!(actual, expected, "joined path uses live native columns");
    assert_eq!(joined.len(), 16);
}

fn assert_paths(address: SocketAddr, fixture: &Fixture, database: &Database) {
    for operator in ['+', '*'] {
        let query = format!("SELECT ?s ?o WHERE {{ ?s <{EDGE}>{operator} ?o }}");
        let rows = rows(address, fixture, &query);
        let actual: BTreeSet<_> = rows
            .iter()
            .map(|row| {
                assert_eq!(row.as_object().unwrap().len(), 2);
                assert_eq!(row["s"]["type"], "uri");
                assert_eq!(row["o"]["type"], "uri");
                (
                    row["s"]["value"].as_str().unwrap().to_owned(),
                    row["o"]["value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let expected: BTreeSet<_> = (0..4)
            .flat_map(|s| (0..4).map(move |o| (iri(s), iri(o))))
            .collect();
        assert_eq!(actual, expected, "cycle/diamond closure {operator}");
        assert_eq!(rows.len(), 16, "duplicate closure pairs");
    }
    seed(database, &(0..258).map(|i| (i, i + 1)).collect::<Vec<_>>());
    for operator in ['+', '*'] {
        let query = format!("SELECT ?s ?o WHERE {{ ?s <{EDGE}>{operator} ?o }}");
        let rows = rows(address, fixture, &query);
        let actual: BTreeSet<_> = rows
            .iter()
            .map(|row| {
                assert_eq!(row.as_object().unwrap().len(), 2);
                assert_eq!(row["s"]["type"], "uri");
                assert_eq!(row["o"]["type"], "uri");
                (
                    row["s"]["value"].as_str().unwrap().to_owned(),
                    row["o"]["value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let expected: BTreeSet<_> = (0..=258)
            .flat_map(|s| ((s + u32::from(operator == '+'))..=258).map(move |o| (iri(s), iri(o))))
            .collect();
        assert_eq!(actual, expected, "successful closure must not stop at 256");
        assert_eq!(rows.len(), expected.len(), "duplicate chain endpoints");
    }
}

fn assert_null_terms(fixture: &Fixture, database: &Database) {
    let class = MAPPING.replace(
        "rr:subjectMap [ rr:template \"http://example.test/n/{src}\" ]",
        "rr:subjectMap [ rr:template \"http://example.test/n/{src}\"; rr:class <http://example.test/C> ]",
    );
    for (mapping, values, pattern) in [
        (MAPPING, "(0,NULL,'b','same')", format!("?s <{EDGE}> ?o")),
        (MAPPING, "(0,'a',NULL,'same')", format!("?s <{EDGE}> ?o")),
        (class.as_str(), "(0,NULL,'b','same')", "?s a ?o".to_owned()),
    ] {
        fixture.write("first.ttl", mapping);
        sql(
            database,
            &format!("DELETE FROM items; INSERT INTO items(id,src,dst,value) VALUES {values}"),
        );
        let (server, address) = start(fixture, database);
        assert!(
            rows(
                address,
                fixture,
                &format!("SELECT ?o WHERE {{ {pattern} }}")
            )
            .is_empty(),
            "{pattern}"
        );
        let (status, body) = request(
            address,
            &format!("ASK {{ {pattern} }}"),
            Some(&fixture.token),
        )
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
            false,
            "{pattern}"
        );
        let count = rows(
            address,
            fixture,
            &format!("SELECT (COUNT(*) AS ?n) WHERE {{ {pattern} }}"),
        );
        assert_eq!(count[0]["n"]["value"], "0", "{pattern}");
        sql(
            database,
            "UPDATE items SET src='present',dst='http://example.test/edge'",
        );
        assert_eq!(
            rows(
                address,
                fixture,
                &format!("SELECT ?o WHERE {{ {pattern} }}")
            )
            .len(),
            1,
            "non-NULL control: {pattern}"
        );
        database.assert_encrypted_sessions();
        drop(server);
    }
    fixture.write("first.ttl", MAPPING);
}

fn assert_collated_paths(fixture: &Fixture, database: &Database, postgres: bool) -> Server {
    if postgres {
        sql(database, "CREATE COLLATION path_ci (provider = icu, locale = 'und-u-ks-level1', deterministic = false); ALTER TABLE items ALTER COLUMN src TYPE VARCHAR(32) COLLATE path_ci; ALTER TABLE items ALTER COLUMN dst TYPE VARCHAR(32) COLLATE path_ci");
    } else {
        sql(database, "ALTER TABLE items MODIFY src VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci, MODIFY dst VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci");
    }
    sql(database, "DELETE FROM items; INSERT INTO items(id,src,dst,value) VALUES (0,'a','B','same'),(1,'b','c','same'),(2,'A','B','same'),(3,'s','a ','same'),(4,'a','z','same')");
    let (server, address) = start(fixture, database);
    ordinary_identity::assert_ordinary(address, fixture);
    for operator in ["+", "*", "?", "|<http://example.test/edge>"] {
        let query = format!("SELECT ?s ?o WHERE {{ ?s (<{EDGE}>{operator}) ?o }}");
        let result = rows(address, fixture, &query);
        let actual: BTreeSet<_> = result
            .iter()
            .map(|row| {
                (
                    row["s"]["value"].as_str().unwrap().to_owned(),
                    row["o"]["value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        let node = |s: &str| format!("http://example.test/n/{}", s.replace(' ', "%20"));
        let mut expected: BTreeSet<_> =
            [("a", "B"), ("b", "c"), ("A", "B"), ("s", "a "), ("a", "z")]
                .into_iter()
                .map(|(s, o)| (node(s), node(o)))
                .collect();
        if matches!(operator, "*" | "?") {
            expected.extend(["a", "B", "b", "c", "A", "s", "a ", "z"].map(|s| (node(s), node(s))));
        }
        assert_eq!(
            actual, expected,
            "native collation must not define RDF identity: {query}"
        );
        assert_eq!(
            result.len(),
            expected.len(),
            "no RDF-distinct pairs lost or duplicated"
        );
    }
    server
}

fn assert_character_paths(fixture: &Fixture, database: &Database, postgres: bool) -> Server {
    for (src_width, dst_width) in [(4, 2), (2, 4), (4, 4)] {
        sql(database, "DELETE FROM items");
        if postgres {
            sql(database, &format!("ALTER TABLE items ALTER COLUMN src TYPE CHAR({src_width}); ALTER TABLE items ALTER COLUMN dst TYPE CHAR({dst_width})"));
        } else {
            sql(database, &format!("ALTER TABLE items MODIFY src CHAR({src_width}) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci, MODIFY dst CHAR({dst_width}) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"));
        }
        sql(database, "INSERT INTO items(id,src,dst,value) VALUES (0,'a','b','same'),(1,'a ','b ','same'),(2,'b','c','same'),(3,'A','b','same')");
        let (server, address) = start(fixture, database);
        let direct = rows(
            address,
            fixture,
            &format!("SELECT ?s ?o WHERE {{ ?s <{EDGE}> ?o }}"),
        );
        let pairs = |rows: &[serde_json::Value]| -> BTreeSet<(String, String)> {
            rows.iter()
                .map(|r| {
                    (
                        r["s"]["value"].as_str().unwrap().to_owned(),
                        r["o"]["value"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };
        let actual_direct = pairs(&direct);
        let direct_len = direct.len();
        let node = |s: &str, width: usize| {
            format!(
                "http://example.test/n/{s}{}",
                if postgres {
                    "%20".repeat(width - 1)
                } else {
                    String::new()
                }
            )
        };
        let direct: BTreeSet<_> = [("a", "b"), ("b", "c"), ("A", "b")]
            .map(|(s, o)| (node(s, src_width), node(o, dst_width)))
            .into_iter()
            .collect();
        assert_eq!(
            actual_direct, direct,
            "ordinary native CHAR {src_width}/{dst_width}"
        );
        assert_eq!(
            direct_len,
            direct.len(),
            "duplicate ordinary decoded CHAR pairs"
        );
        let mut closure = direct.clone();
        loop {
            let more: Vec<_> = closure
                .iter()
                .flat_map(|(s, m)| {
                    direct
                        .iter()
                        .filter_map(move |(n, o)| (m == n).then_some((s.clone(), o.clone())))
                })
                .collect();
            let before = closure.len();
            closure.extend(more);
            if before == closure.len() {
                break;
            }
        }
        for op in ["+", "*", "?", "|<http://example.test/edge>"] {
            let mut expected = if matches!(op, "+" | "*") {
                closure.clone()
            } else {
                direct.clone()
            };
            if matches!(op, "*" | "?") {
                expected.extend(
                    direct
                        .iter()
                        .flat_map(|(s, o)| [(s.clone(), s.clone()), (o.clone(), o.clone())]),
                );
            }
            let result = rows(
                address,
                fixture,
                &format!("SELECT ?s ?o WHERE {{ ?s (<{EDGE}>{op}) ?o }}"),
            );
            assert_eq!(
                pairs(&result),
                expected,
                "native CHAR {src_width}/{dst_width} {op}"
            );
            assert_eq!(result.len(), expected.len(), "duplicate decoded CHAR pairs");
        }
        database.assert_encrypted_sessions();
        drop(server);
    }
    start(fixture, database).0
}

#[test]
#[ignore = "requires Docker and pinned owned PostgreSQL/MySQL images; required in CI"]
fn native_describe_and_recursive_paths_are_exact() {
    for postgres in [true, false] {
        let fixture = Fixture::new();
        fixture.write("first.ttl", MAPPING);
        fixture.write(
            "ontology.ttl",
            &format!("<{EDGE}> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/C> a <http://www.w3.org/2002/07/owl#Class> ."),
        );
        let database = Database::start(&fixture, postgres);
        sql(&database, "ALTER TABLE items ADD COLUMN id INTEGER PRIMARY KEY DEFAULT 0; ALTER TABLE items ADD COLUMN src VARCHAR(32); ALTER TABLE items ADD COLUMN dst VARCHAR(32)");
        seed(&database, &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 0), (0, 1)]);
        let (server, address) = start(&fixture, &database);
        assert_describe(address, &fixture, &database);
        assert_paths(address, &fixture, &database);
        drop(server);
        assert_null_terms(&fixture, &database);
        seed(&database, &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 0), (0, 1)]);
        fixture.write(
            "first.ttl",
            &MAPPING.replace("{src}", "{SRC}").replace("{dst}", "{DST}"),
        );
        let (server, address) = start(&fixture, &database);
        assert_joined_paths(address, &fixture);
        drop(server);
        drop(assert_collated_paths(&fixture, &database, postgres));
        ordinary_identity::assert_references(&fixture, &database, postgres);
        fixture.write("first.ttl", MAPPING);
        let _server = assert_character_paths(&fixture, &database, postgres);
        database.assert_encrypted_sessions();
        eprintln!(
            "exact native DESCRIBE/path profile: {}",
            database.sql(if postgres {
                "SHOW server_version"
            } else {
                "SELECT VERSION()"
            })
        );
    }
}
