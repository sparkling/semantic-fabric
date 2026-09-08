use super::*;

fn policy_configured(kind: &str, object_kind: &str, data: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!("CREATE TABLE edges(s {kind},o {object_kind},tenant TEXT COLLATE NOCASE); CREATE TABLE outer_nodes(s TEXT,tenant TEXT); INSERT INTO outer_nodes VALUES('a','reader'); INSERT INTO edges VALUES {data};")).unwrap();
    with_policy(support::serve_config(Backend::sqlite(conn), MAP))
}

fn with_policy(mut cfg: ServeConfig) -> ServeConfig {
    use sf_serve::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    };
    let policy = PortableRowPolicy::new(vec![
        PortableRowRule::new(0, "edges", "tenant", "reader").unwrap(),
        PortableRowRule::new(0, "outer_nodes", "tenant", "reader").unwrap(),
    ])
    .unwrap();
    let subject = ProvisionedBearerSubject::portable_rows("reader", TOKEN, policy).unwrap();
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![subject]).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn ordinary_policy_retains_case_distinct_rdf_terms() {
    for query in [
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let json = answer(
            policy_configured(
                "TEXT COLLATE NOCASE",
                "TEXT COLLATE NOCASE",
                "('a','B','reader'),('A','B','reader'),('denied','secret','other')",
            ),
            query,
        )
        .await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "2");
        } else {
            let mut subjects = rows
                .iter()
                .map(|r| r["s"]["value"].as_str().unwrap())
                .collect::<Vec<_>>();
            subjects.sort();
            assert_eq!(subjects, vec!["http://ex/n/A", "http://ex/n/a"]);
        }
    }
}

#[tokio::test]
async fn ordinary_policy_only_witnesses_do_not_duplicate_rdf_triples() {
    for (kind, data, expected) in [
        (
            "TEXT",
            "('a','b','other'),('a','b','reader'),('a','b','READER'),('denied','secret','other')",
            "http://ex/n/a",
        ),
        (
            "CHARACTER(4)",
            "('a','b','reader'),('a ','b ','READER'),('denied','secret','other')",
            "http://ex/n/a%20%20%20",
        ),
    ] {
        for query in [
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
            "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
        ] {
            let json = answer(policy_configured(kind, kind, data), query).await;
            let rows = json["results"]["bindings"].as_array().unwrap();
            if query.contains("COUNT") {
                assert_eq!(rows[0]["n"]["value"], "1", "{kind}");
            } else {
                assert_eq!(rows.len(), 1, "{kind}");
                assert_eq!(rows[0]["s"]["value"], expected);
            }
        }
    }
}

#[tokio::test]
async fn ordinary_policy_mixed_keys_preserve_signed_zero_iris() {
    // SQLite canonicalizes stored floating zero. A virtual expression preserves
    // its sign; assert that premise before using it as an RDF identity witness.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE edges(s CHARACTER(2), o GENERATED ALWAYS AS (CASE WHEN s='a' THEN 0.0 ELSE -0.0 END) VIRTUAL, tenant TEXT); CREATE TABLE outer_nodes(s TEXT,tenant TEXT); INSERT INTO edges(s,tenant) VALUES('a','reader'),('a ','reader');").unwrap();
    let native = conn
        .prepare("SELECT o FROM edges ORDER BY length(s)")
        .unwrap()
        .query_map([], |row| Ok(row.get::<_, f64>(0)?.to_string()))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(native, vec!["0", "-0"]);
    let cfg = with_policy(support::serve_config(Backend::sqlite(conn), MAP));
    let json = answer(cfg, "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }").await;
    let rows = json["results"]["bindings"].as_array().unwrap();
    let mut objects = rows
        .iter()
        .map(|r| {
            assert_eq!(r["s"]["value"], "http://ex/n/a%20");
            r["o"]["value"].as_str().unwrap()
        })
        .collect::<Vec<_>>();
    objects.sort();
    assert_eq!(objects, vec!["http://ex/n/-0", "http://ex/n/0"]);
}

#[tokio::test]
async fn ordinary_policy_preserves_projection_bags_and_correlations() {
    for (pattern, expected) in [
        ("?s <http://ex/p> ?o", 2),
        ("?s <http://ex/p> ?o . ?other <http://ex/p> ?o", 4),
        ("?s <http://ex/mark> ?m OPTIONAL { ?s <http://ex/p> ?o }", 1),
        (
            "?s <http://ex/mark> ?m FILTER EXISTS { ?s <http://ex/p> ?o }",
            1,
        ),
        (
            "?s <http://ex/mark> ?m FILTER NOT EXISTS { ?s <http://ex/p> ?o }",
            0,
        ),
        ("?s <http://ex/mark> ?m MINUS { ?s <http://ex/p> ?o }", 0),
        ("{ ?s <http://ex/p> ?o } UNION { ?s <http://ex/p> ?o }", 4),
        ("?s <http://ex/p> <http://ex/n/target>", 2),
        ("?s <http://ex/p> <http://ex/n/TARGET>", 0),
    ] {
        let cfg = policy_configured("TEXT COLLATE NOCASE", "TEXT COLLATE NOCASE",
            "('a','target','other'),('a','target','reader'),('a','target','READER'),('b','target','reader'),('b','target','READER'),('denied','secret','other')");
        let query = format!("SELECT ?o WHERE {{ {pattern} }}");
        let json = answer(cfg, &query).await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            expected,
            "{query}"
        );
        assert!(!json.to_string().contains("secret"), "{query}");
    }
}
