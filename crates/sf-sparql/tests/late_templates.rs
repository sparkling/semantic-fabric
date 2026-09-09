//! Late-template identity is a whole generated IRI, independent of raw slot equality.
use sf_mapping::{parse_r2rml_with_options, R2rmlOptions};
use sf_sparql::{exec, parse_and_translate_with, Tbox};
use sf_sql::Dialect;

#[test]
fn different_processor_bases_compare_absolute_and_relative_expansions_differently() {
    let mut maps = vec![];
    for (name, base, predicate) in [
        ("left_items", "http://left/", "urn:p"),
        ("right_items", "http://right/", "urn:q"),
    ] {
        let mapping = format!(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<urn:{name}> rr:logicalTable [rr:tableName "{name}"]; rr:subjectMap [rr:template "{{v}}:x"];
rr:predicateObjectMap [rr:predicate <{predicate}>; rr:object "same"]."#
        );
        maps.extend(
            parse_r2rml_with_options(
                &mapping,
                R2rmlOptions {
                    processor_base_iri: base,
                    ..R2rmlOptions::default()
                },
            )
            .unwrap(),
        );
    }
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE left_items(v TEXT); CREATE TABLE right_items(v TEXT); INSERT INTO left_items VALUES('urn'),('1'); INSERT INTO right_items VALUES('urn'),('1');").unwrap();
    for query in [
        "SELECT ?s WHERE { ?s <urn:p> ?o . ?s <urn:q> ?other }",
        "SELECT ?s WHERE { ?s <urn:p> ?o . ?t <urn:q> ?other FILTER(?s = ?t) }",
        "SELECT ?s WHERE { ?s <urn:p> ?o . ?t <urn:q> ?other FILTER(sameTerm(?s, ?t)) }",
    ] {
        let plan =
            parse_and_translate_with(query, &maps, Dialect::Sqlite, &Tbox::default(), &[]).unwrap();
        let rows = exec::select(&plan, &conn).unwrap().rows;
        assert_eq!(rows.len(), 1, "{query}: {rows:?}");
        assert_eq!(rows[0][0].as_ref().unwrap().to_string(), "<urn:x>");
    }
}

#[test]
fn late_predicates_match_the_requested_value_and_reject_unproved_entailment() {
    let maps = parse_r2rml_with_options(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<urn:m> rr:logicalTable [rr:tableName "items"]; rr:subject <urn:s>;
rr:predicateObjectMap [rr:predicateMap [rr:template "{v}:q"]; rr:object "same"]."#,
        R2rmlOptions::default(),
    )
    .unwrap();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE items(v TEXT); INSERT INTO items VALUES('urn'),(NULL);")
        .unwrap();
    for (predicate, expected) in [("urn:q", 1), ("urn:other", 0)] {
        let plan = parse_and_translate_with(
            &format!("SELECT ?s WHERE {{ ?s <{predicate}> ?o }}"),
            &maps,
            Dialect::Sqlite,
            &Tbox::default(),
            &[],
        )
        .unwrap();
        assert_eq!(exec::select(&plan, &conn).unwrap().rows.len(), expected);
    }
    for inverse in [false, true] {
        let mut tbox = Tbox::default();
        if inverse {
            tbox.add_inverse("urn:q", "urn:p");
        } else {
            tbox.add_subproperty("urn:q", "urn:p");
        }
        assert!(parse_and_translate_with(
            "SELECT ?s WHERE { ?s <urn:p> ?o }",
            &maps,
            Dialect::Sqlite,
            &tbox,
            &[]
        )
        .is_err());
    }
}

#[test]
fn unqualified_native_and_mixed_atom_shapes_reject_before_execution() {
    let mapping = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<urn:m> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "{v}:x"];
rr:predicateObjectMap [rr:predicate <urn:p>; rr:object "same"]."#;
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        let maps = sf_mapping::parse_r2rml(mapping).unwrap();
        assert!(parse_and_translate_with(
            "SELECT ?s WHERE { ?s <urn:p> ?o }",
            &maps,
            dialect,
            &Tbox::default(),
            &[]
        )
        .is_err());
    }
    let maps = sf_mapping::parse_r2rml(
        &mapping.replace("rr:object \"same\"", "rr:objectMap [rr:column \"o\"]"),
    )
    .unwrap();
    assert!(parse_and_translate_with(
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <urn:p> ?o }",
        &maps,
        Dialect::Sqlite,
        &Tbox::default(),
        &[]
    )
    .is_err());
}
