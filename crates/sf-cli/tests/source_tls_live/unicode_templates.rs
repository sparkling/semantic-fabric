//! Authenticated public equality renders both different-shape templates in SQL.
use super::*;

pub(super) fn assert_native_encoding(fixture: &Fixture, database: &Database, postgres: bool) {
    fixture.write(
        "first.ttl",
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://example.test/id/{id}"];
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>;
   rr:objectMap [rr:template "http://example.test/n/{src}-suffix"]];
 rr:predicateObjectMap [rr:predicate <http://example.test/other>;
   rr:objectMap [rr:template "http://example.test/n/{dst}"]]."#,
    );
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/other> a <http://www.w3.org/2002/07/owl#ObjectProperty> .");
    sql(database, "DELETE FROM items");
    // Earlier profile slices deliberately narrow these columns to CHAR(4).
    // This separate fixture slice needs space for a scalar plus its suffix.
    sql(
        database,
        if postgres {
            "ALTER TABLE items ALTER COLUMN src TYPE VARCHAR(64); ALTER TABLE items ALTER COLUMN dst TYPE VARCHAR(64)"
        } else {
            "ALTER TABLE items MODIFY src VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci, MODIFY dst VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
        },
    );
    let cases = [
        ("C280", "%C2%80"),
        ("EE8080", "%EE%80%80"),
        ("EFB790", "%EF%B7%90"),
        ("F09FBFBE", "%F0%9F%BF%BE"),
        ("F3A08080", "%F3%A0%80%80"),
        ("F3B08080", "%F3%B0%80%80"),
        ("E4BDA0E5A5BD", "你好"),
        ("F3A18080", "\u{e1000}"),
    ];
    let text = |hex: &str| {
        if postgres {
            format!("convert_from(decode('{hex}', 'hex'), 'UTF8')")
        } else {
            format!("CONVERT(UNHEX('{hex}') USING utf8mb4)")
        }
    };
    for (id, (hex, _)) in cases.iter().enumerate() {
        sql(database, &format!("INSERT INTO items(id,src,dst,value) VALUES ({id}, {}, CONCAT({},'-suffix'),'same')", text(hex), text(hex)));
    }
    let (server, address) = start(fixture, database);
    // Raw term reconstruction and SQL TemplateEq must agree independently.
    for filter in ["", "FILTER(?a = ?b)", "FILTER(sameTerm(?a, ?b))"] {
        let query = format!(
            "SELECT ?a WHERE {{ ?s <{EDGE}> ?a . ?s <http://example.test/other> ?b {filter} }}"
        );
        let result = rows(address, fixture, &query);
        let actual: BTreeSet<_> = result
            .iter()
            .map(|row| row["a"]["value"].as_str().unwrap().to_owned())
            .collect();
        let expected: BTreeSet<_> = cases
            .iter()
            .map(|(_, encoded)| format!("http://example.test/n/{encoded}-suffix"))
            .collect();
        assert_eq!(actual, expected, "{query}");
        assert_eq!(result.len(), cases.len());
    }
    let query = format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> ?a . ?s <http://example.test/other> ?b FILTER(?a != ?b) }}");
    assert_eq!(rows(address, fixture, &query)[0]["n"]["value"], "0");
    database.assert_encrypted_sessions();
    drop(server);
}
