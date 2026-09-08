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

#[test]
fn lineage_origins_follow_actual_emitters_and_cost_order_not_unused_maps() {
    let binding_with = |index, predicate, estimate| {
        let original = binding(index, r#"rr:tableName "items""#, predicate);
        let mut maps = original.triples_maps().to_vec();
        maps[0].id = "urn:actual".into();
        let mut unused = maps[0].clone();
        unused.id = "urn:unused".into();
        unused.predicate_object_maps[0].predicates = vec![TermMap::Constant(
            oxrdf::NamedNode::new("http://example.test/unused")
                .unwrap()
                .into(),
        )];
        maps.insert(0, unused);
        let mut table = sf_sql::TableSchema::new("items");
        table.row_estimate = Some(estimate);
        CompilerBinding::from_unverified_observation(
            SourceMapping::new(SourceId::new(index).unwrap(), maps),
            Dialect::Sqlite,
            crate::Tbox::default(),
            vec![table],
            crate::Epoch::default(),
            8,
        )
    };
    let left = binding_with(0, "rr:predicate <http://example.test/left>", 1000);
    let right = binding_with(1, "rr:predicate <http://example.test/right>", 1);
    let result =
        compile_source_affine_union_lineage(JOIN, [&left, &right], &UncontrolledQueryControl)
            .unwrap();
    assert_eq!(
        result.fragments()[0].source_id().index(),
        1,
        "smaller source drives"
    );
    let origins = result.bounded_join().unwrap().mapping_origins().unwrap();
    assert_eq!(
        origins,
        &[
            (SourceId::new(1).unwrap(), "urn:actual".into()),
            (SourceId::new(0).unwrap(), "urn:actual".into())
        ]
    );
    assert!(result.fragments().iter().all(|f| f.lineage().is_none()));
    let ordinary =
        compile_source_affine_union(JOIN, [&left, &right], &UncontrolledQueryControl).unwrap();
    assert!(ordinary.bounded_join().unwrap().mapping_origins().is_none());
}

#[test]
fn join_lineage_rejects_unbounded_or_ambiguous_mapping_identity() {
    let right = binding(
        1,
        r#"rr:tableName "items""#,
        "rr:predicate <http://example.test/right>",
    );
    for (length, duplicate, accepted) in [
        (0, false, false),
        (1024, false, true),
        (1025, false, false),
        (16, true, false),
    ] {
        let original = binding(
            0,
            r#"rr:tableName "items""#,
            "rr:predicate <http://example.test/left>",
        );
        let mut maps = original.triples_maps().to_vec();
        maps[0].id = "m".repeat(length);
        if duplicate {
            let mut unused = maps[0].clone();
            unused.predicate_object_maps[0].predicates = vec![TermMap::Constant(
                oxrdf::NamedNode::new("http://example.test/unused")
                    .unwrap()
                    .into(),
            )];
            maps.push(unused);
        }
        let left = CompilerBinding::from_unverified_observation(
            SourceMapping::new(SourceId::new(0).unwrap(), maps),
            Dialect::Sqlite,
            crate::Tbox::default(),
            vec![sf_sql::TableSchema::new("items")],
            crate::Epoch::default(),
            8,
        );
        let result =
            compile_source_affine_union_lineage(JOIN, [&left, &right], &UncontrolledQueryControl);
        assert_eq!(
            result.is_ok(),
            accepted,
            "length={length}, duplicate={duplicate}: {result:?}"
        );
        if !accepted {
            assert!(matches!(result, Err(Error::Unsupported(_))));
        }
        assert_eq!(left.cache_len(), 0);
    }
}
