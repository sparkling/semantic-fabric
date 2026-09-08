//! Actual public query results against only owned, pinned, TLS-enabled providers.
use super::*;
use std::collections::BTreeSet;

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
    // Authored regular column references fold to the live native names. A path
    // joined to an ordinary pattern must not freeze quoted SRC/DST before probe.
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

fn assert_collated_paths(fixture: &Fixture, database: &Database, postgres: bool) -> Server {
    if postgres {
        sql(database, "CREATE COLLATION path_ci (provider = icu, locale = 'und-u-ks-level1', deterministic = false); ALTER TABLE items ALTER COLUMN src TYPE VARCHAR(32) COLLATE path_ci; ALTER TABLE items ALTER COLUMN dst TYPE VARCHAR(32) COLLATE path_ci");
    } else {
        sql(database, "ALTER TABLE items MODIFY src VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci, MODIFY dst VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci");
    }
    sql(database, "DELETE FROM items; INSERT INTO items(id,src,dst,value) VALUES (0,'a','B','same'),(1,'b','c','same'),(2,'A','B','same'),(3,'s','a ','same'),(4,'a','z','same')");
    let (server, address) = start(fixture, database);
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

#[test]
#[ignore = "requires Docker and pinned owned PostgreSQL/MySQL images; required in CI"]
fn native_describe_and_recursive_paths_are_exact() {
    for postgres in [true, false] {
        let fixture = Fixture::new();
        fixture.write("first.ttl", MAPPING);
        fixture.write(
            "ontology.ttl",
            &format!("<{EDGE}> a <http://www.w3.org/2002/07/owl#ObjectProperty> ."),
        );
        let database = Database::start(&fixture, postgres);
        sql(&database, "ALTER TABLE items ADD COLUMN id INTEGER PRIMARY KEY DEFAULT 0; ALTER TABLE items ADD COLUMN src VARCHAR(32); ALTER TABLE items ADD COLUMN dst VARCHAR(32)");
        seed(&database, &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 0), (0, 1)]);
        let (server, address) = start(&fixture, &database);
        assert_describe(address, &fixture, &database);
        assert_paths(address, &fixture, &database);
        drop(server);
        seed(&database, &[(0, 1), (0, 2), (1, 3), (2, 3), (3, 0), (0, 1)]);
        fixture.write(
            "first.ttl",
            &MAPPING.replace("{src}", "{SRC}").replace("{dst}", "{DST}"),
        );
        let (server, address) = start(&fixture, &database);
        assert_joined_paths(address, &fixture);
        drop(server);
        let _server = assert_collated_paths(&fixture, &database, postgres);
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
