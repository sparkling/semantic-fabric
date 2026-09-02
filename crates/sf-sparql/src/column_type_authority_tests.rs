//! Adversarial tests for column-type authority at the compiler boundary.

use std::collections::HashSet;

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::{SourceId, SourceMapping};
use sf_sql::{Column, Dialect, TableSchema};

use crate::cascade::{
    group_can_fallback_to_shared_term_dedup, group_pool_type_safety, PoolTypeSafety,
};
use crate::compiler_schema::ColumnTypeUse;
use crate::iq::{Branch, Scan, TermDef};
use crate::{
    parse_and_translate_flat_with, parse_and_translate_with, ColumnTypeAuthority, CompilerBinding,
    CompilerSchema, Plan, PlanForm, Tbox,
};

fn column_branch(alias: usize, table: &str, column: &str) -> Branch {
    let mut branch = Branch::single(Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    });
    branch.bindings.insert(
        "value".to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column(column.into(), TermSpec::plain_literal()),
            alias,
        },
    );
    branch
}

fn table(name: &str, column: &str, sql_type: &str) -> TableSchema {
    let mut table = TableSchema::new(name);
    table.columns = vec![Column::new(column, sql_type, false)];
    table
}

#[test]
fn should_treat_different_unverified_physical_columns_as_unproven() {
    let left = column_branch(0, "left_values", "value");
    let right = column_branch(1, "right_values", "value");
    let schema = vec![
        table("left_values", "value", "text"),
        table("right_values", "value", "text"),
    ];

    assert_eq!(
        group_pool_type_safety(
            &[&left, &right],
            &schema,
            Dialect::Postgres,
            ColumnTypeUse::Unverified,
        ),
        PoolTypeSafety::Unproven
    );
}

#[test]
fn should_allow_the_same_physical_column_without_type_authority() {
    let left = column_branch(0, "values", "value");
    let right = column_branch(1, "values", "value");

    assert_eq!(
        group_pool_type_safety(
            &[&left, &right],
            &[table("values", "value", "text")],
            Dialect::Postgres,
            ColumnTypeUse::Unverified,
        ),
        PoolTypeSafety::ProvenSafe
    );
}

#[test]
fn should_fail_closed_when_frozen_type_metadata_is_missing() {
    let left = column_branch(0, "left_values", "value");
    let right = column_branch(1, "right_values", "value");

    assert_eq!(
        group_pool_type_safety(
            &[&left, &right],
            &[],
            Dialect::Postgres,
            ColumnTypeUse::CallerAuthorizedFrozen,
        ),
        PoolTypeSafety::Unproven
    );
}

#[test]
fn should_use_term_dedup_for_an_injective_standalone_group_without_type_proof() {
    let left = column_branch(0, "left_query", "value");
    let right = column_branch(1, "right_query", "value");
    let keep = HashSet::from(["value".to_owned()]);

    assert!(group_can_fallback_to_shared_term_dedup(
        &[left, right],
        &keep,
    ));
}

#[test]
fn should_reject_a_shared_dedup_arm_missing_part_of_the_pattern_key() {
    let left = column_branch(0, "left_query", "value");
    let mut right = column_branch(1, "right_query", "value");
    right.bindings.clear();

    assert!(!group_can_fallback_to_shared_term_dedup(
        &[left, right],
        &HashSet::from(["value".to_owned()]),
    ));
}

#[test]
fn should_reject_frozen_float_text_pooling() {
    let left = column_branch(0, "left_values", "value");
    let right = column_branch(1, "right_values", "value");
    let schema = vec![
        table("left_values", "value", "double precision"),
        table("right_values", "value", "text"),
    ];

    assert_eq!(
        group_pool_type_safety(
            &[&left, &right],
            &schema,
            Dialect::Postgres,
            ColumnTypeUse::CallerAuthorizedFrozen,
        ),
        PoolTypeSafety::Unproven
    );
}

#[test]
fn should_preserve_sqlite_dynamic_type_pooling() {
    let left = column_branch(0, "left_values", "value");
    let right = column_branch(1, "right_values", "value");

    assert_eq!(
        group_pool_type_safety(
            &[&left, &right],
            &[],
            Dialect::Sqlite,
            ColumnTypeUse::Unverified,
        ),
        PoolTypeSafety::ProvenSafe
    );
}

const TYPE_AUTHORITY_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
<#Left>
    rr:logicalTable [ rr:tableName "left_values" ] ;
    rr:subjectMap [ rr:template "{name}_{value}" ; rr:termType rr:BlankNode ] ;
    rr:predicateObjectMap [ rr:predicate rdf:type ; rr:object ex:Thing ] .
<#Right>
    rr:logicalTable [ rr:tableName "right_values" ] ;
    rr:subjectMap [ rr:template "{name}_{value}" ; rr:termType rr:BlankNode ] ;
    rr:predicateObjectMap [ rr:predicate rdf:type ; rr:object ex:Thing ] .
"#;

const PROJECTED_KEY_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Left>
    rr:logicalTable [ rr:tableName "left_values" ] ;
    rr:subjectMap [ rr:template "{name}" ; rr:termType rr:BlankNode ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "value" ] ] .
<#Right>
    rr:logicalTable [ rr:tableName "right_values" ] ;
    rr:subjectMap [ rr:template "{name}" ; rr:termType rr:BlankNode ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:column "value" ] ] .
"#;

const GRAPH_KEY_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#DynamicGraphs>
    rr:logicalTable [ rr:tableName "dynamic_graphs" ] ;
    rr:subjectMap [
        rr:template "http://example.test/id/{id}" ;
        rr:graphMap [ rr:column "g1" ; rr:termType rr:IRI ] ;
        rr:graphMap [ rr:column "g2" ; rr:termType rr:IRI ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:p ;
        rr:objectMap [ rr:column "value" ]
    ] .
"#;

const GROUND_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#GroundLeft> rr:logicalTable [ rr:tableName "ground_left" ] ;
    rr:subjectMap [ rr:constant ex:s ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:object ex:o ] .
<#GroundRight> rr:logicalTable [ rr:tableName "ground_right" ] ;
    rr:subjectMap [ rr:constant ex:s ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:object ex:o ] .
<#Rows> rr:logicalTable [ rr:tableName "facts" ] ;
    rr:subjectMap [ rr:template "http://example.test/f/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:column "value" ] ] .
"#;

fn columns_table(name: &str, columns: &[&str]) -> TableSchema {
    TableSchema {
        columns: columns
            .iter()
            .map(|column| Column::new(*column, "text", false))
            .collect(),
        ..TableSchema::new(name)
    }
}

fn assert_full_pattern_key_is_hidden(plan: &Plan) {
    let PlanForm::Select { vars } = &plan.form else {
        panic!("expected a SELECT plan");
    };
    assert_eq!(vars, &["s"]);
    let scopes = plan.dedup_scopes.iter().flatten().collect::<Vec<_>>();
    assert_eq!(scopes.len(), 2);
    assert!(scopes
        .iter()
        .all(|scope| { scope.key_bindings.keys().map(String::as_str).eq(["o", "s"]) }));
    assert!(plan
        .branches
        .iter()
        .all(|branch| !branch.bindings.contains_key("o")));
}

#[test]
fn should_preserve_the_bgp_dedup_key_past_outer_projection() {
    let maps = sf_mapping::parse_r2rml(PROJECTED_KEY_R2RML).unwrap();
    let schema = vec![
        TableSchema {
            columns: vec![
                Column::new("name", "text", false),
                Column::new("value", "double precision", false),
            ],
            ..TableSchema::new("left_values")
        },
        TableSchema {
            columns: vec![
                Column::new("name", "text", false),
                Column::new("value", "text", false),
            ],
            ..TableSchema::new("right_values")
        },
    ];
    let query = "PREFIX ex: <http://example.test/> SELECT ?s WHERE { ?s ex:p ?o }";

    let flat =
        parse_and_translate_flat_with(query, &maps, Dialect::Postgres, &Tbox::default(), &schema)
            .unwrap();
    assert_full_pattern_key_is_hidden(&flat);

    let binding = CompilerBinding::new(
        SourceMapping::new(SourceId::new(0).unwrap(), maps),
        Dialect::Postgres,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(schema),
        8,
    );
    let tree = binding.compile(query).unwrap();
    assert_full_pattern_key_is_hidden(&tree);
}

#[test]
fn should_keep_the_dynamic_graph_variable_in_flat_tree_and_fallback_keys() {
    let maps = sf_mapping::parse_r2rml(GRAPH_KEY_R2RML).unwrap();
    let schema = vec![columns_table(
        "dynamic_graphs",
        &["id", "value", "g1", "g2"],
    )];
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE dynamic_graphs (id TEXT, value TEXT, g1 TEXT, g2 TEXT);\
             INSERT INTO dynamic_graphs VALUES \
             ('one', 'same', 'http://example.test/g1', 'http://example.test/g2');",
        )
        .unwrap();
    for (query, expected) in [
        (
            "SELECT ?g ?s WHERE { GRAPH ?g { ?s <http://example.test/p> ?o } }",
            2,
        ),
        (
            "SELECT ?s WHERE { GRAPH ?g { ?s <http://example.test/p> ?o } }",
            2,
        ),
        (
            "SELECT DISTINCT ?s WHERE { GRAPH ?g { ?s <http://example.test/p> ?o } }",
            1,
        ),
    ] {
        for plan in [
            parse_and_translate_flat_with(query, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
                .unwrap(),
            parse_and_translate_with(query, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
                .unwrap(),
        ] {
            assert_eq!(
                crate::exec::select(&plan, &connection).unwrap().rows.len(),
                expected
            );
        }
    }

    let binding = CompilerBinding::new(
        SourceMapping::new(SourceId::new(0).unwrap(), maps),
        Dialect::Postgres,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(schema),
        8,
    );
    let plan = binding
        .compile("SELECT ?s WHERE { GRAPH ?g { ?s <http://example.test/p> ?o } }")
        .unwrap();
    assert!(plan.dedup_scopes.iter().flatten().all(|scope| {
        scope
            .key_bindings
            .keys()
            .map(String::as_str)
            .eq(["g", "o", "s"])
    }));
    assert_eq!(plan.dedup_scopes.iter().flatten().count(), 2);
}

#[test]
fn should_pool_overlapping_ground_patterns_as_unit_relations() {
    let maps = sf_mapping::parse_r2rml(GROUND_R2RML).unwrap();
    let schema = vec![
        columns_table("ground_left", &["present"]),
        columns_table("ground_right", &["present"]),
        columns_table("facts", &["id", "value"]),
    ];
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE ground_left (present TEXT); INSERT INTO ground_left VALUES ('yes');\
             CREATE TABLE ground_right (present TEXT); INSERT INTO ground_right VALUES ('yes');\
             CREATE TABLE facts (id TEXT, value TEXT);\
             INSERT INTO facts VALUES ('1', 'a'), ('2', 'b');",
        )
        .unwrap();
    for (query, expected) in [
        ("SELECT * WHERE { <http://example.test/s> <http://example.test/p> <http://example.test/o> }", 1),
        ("SELECT * WHERE { <http://example.test/s> <http://example.test/p> <http://example.test/absent> }", 0),
        ("SELECT ?x WHERE { <http://example.test/s> <http://example.test/p> <http://example.test/o> . ?x <http://example.test/q> ?v }", 2),
    ] {
        for plan in [
            parse_and_translate_flat_with(query, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
                .unwrap(),
            parse_and_translate_with(query, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
                .unwrap(),
        ] {
            assert_eq!(crate::exec::select(&plan, &connection).unwrap().rows.len(), expected);
        }
    }
}

#[test]
fn should_quarantine_startup_types_on_the_cached_compiler_path() {
    let source_id = SourceId::new(0).unwrap();
    let maps = sf_mapping::parse_r2rml(TYPE_AUTHORITY_R2RML).unwrap();
    let mapping = SourceMapping::new(source_id, maps);
    let schema = vec![
        TableSchema {
            columns: vec![
                Column::new("name", "text", false),
                Column::new("value", "text", false),
            ],
            ..TableSchema::new("left_values")
        },
        TableSchema {
            columns: vec![
                Column::new("name", "text", false),
                Column::new("value", "text", false),
            ],
            ..TableSchema::new("right_values")
        },
    ];
    let query = "PREFIX ex: <http://example.test/> SELECT ?s WHERE { ?s a ex:Thing }";
    let frozen = parse_and_translate_with(
        query,
        mapping.triples_maps(),
        Dialect::Postgres,
        &Tbox::default(),
        &schema,
    )
    .unwrap();
    assert!(
        frozen.dedup_scopes.is_empty(),
        "caller-authorized equal frozen types may retain the SQL pool"
    );

    let binding = CompilerBinding::new(
        mapping,
        Dialect::Postgres,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(schema),
        8,
    );
    let quarantined = binding.compile(query).unwrap();
    assert_eq!(
        binding.column_type_authority(),
        ColumnTypeAuthority::Unverified
    );
    assert_eq!(
        binding.scope().column_type_authority(),
        ColumnTypeAuthority::Unverified
    );
    assert!(
        quarantined.dedup_scopes.iter().any(Option::is_some),
        "cached serving compilation must avoid a pool authorized by mutable startup types"
    );
}

#[test]
fn should_fail_closed_when_values_duplicates_a_shared_dedup_arm() {
    let source_id = SourceId::new(0).unwrap();
    let maps = sf_mapping::parse_r2rml(TYPE_AUTHORITY_R2RML).unwrap();
    let mapping = SourceMapping::new(source_id, maps);
    let schema = vec![
        TableSchema {
            columns: vec![
                Column::new("name", "text", false),
                Column::new("value", "text", false),
            ],
            ..TableSchema::new("left_values")
        },
        TableSchema {
            columns: vec![
                Column::new("name", "text", false),
                Column::new("value", "text", false),
            ],
            ..TableSchema::new("right_values")
        },
    ];
    let binding = CompilerBinding::new(
        mapping,
        Dialect::Postgres,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(schema),
        8,
    );
    let query = "PREFIX ex: <http://example.test/> SELECT ?s WHERE { \
                 VALUES ?copy { 1 2 } ?s a ex:Thing }";

    assert!(matches!(
        binding.compile(query),
        Err(crate::Error::Unsupported(_))
    ));
}
