use super::*;
use sf_core::query_control::UncontrolledQueryControl;
use sf_core::SourceMapping;
use sf_sql::Dialect;

fn binding(index: usize, logical: &str, predicate: &str) -> CompilerBinding {
    let mapping = format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap ; rr:logicalTable [ {logical} ] ;
rr:subjectMap [ rr:template "http://example.test/item/{{id}}" ] ;
rr:predicateObjectMap [ {predicate} ; rr:objectMap [ rr:column "value" ;
rr:datatype <http://www.w3.org/2001/XMLSchema#string> ] ] ."#
    );
    CompilerBinding::from_unverified_observation(
        SourceMapping::new(
            SourceId::new(index).unwrap(),
            sf_mapping::parse_r2rml(&mapping).unwrap(),
        ),
        Dialect::Sqlite,
        crate::Tbox::default(),
        vec![sf_sql::TableSchema::new("items")],
        crate::Epoch::default(),
        8,
    )
}

const JOIN: &str = "SELECT ?left ?right WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }";

#[test]
fn authored_sql_and_dynamic_predicates_cannot_inherit_base_scan_authority() {
    let right = binding(
        1,
        r#"rr:tableName "items""#,
        "rr:predicate <http://example.test/right>",
    );
    for (logical, predicate, accepted) in [
        (
            r#"rr:tableName "items""#,
            "rr:predicate <http://example.test/left>",
            true,
        ),
        (
            r#"rr:sqlQuery "SELECT id,value FROM items WHERE value='hidden'""#,
            "rr:predicate <http://example.test/left>",
            false,
        ),
        (
            r#"rr:sqlQuery "SELECT id,value FROM items""#,
            "rr:predicate <http://example.test/left>",
            false,
        ),
        (
            r#"rr:tableName "items""#,
            r#"rr:predicateMap [ rr:template "http://example.test/{id}" ]"#,
            false,
        ),
    ] {
        let left = binding(0, logical, predicate);
        let result = compile_source_affine_union(JOIN, [&left, &right], &UncontrolledQueryControl);
        if accepted {
            assert!(result.unwrap().bounded_join().is_some());
        } else {
            assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
        }
        assert_eq!(left.cache_len(), 0);
        assert_eq!(right.cache_len(), 0);
    }
}
