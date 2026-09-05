use sf_core::ir::{LogicalSource, SubjectMap, TermMap, TriplesMap};
use sf_core::{NamedNode, SourceId, SourceMapping, Term};
use sf_sql::{Column, Dialect, TableSchema};

use crate::{CompilerBinding, Epoch, Tbox};

fn mapping(source_id: SourceId, table: &str) -> SourceMapping {
    SourceMapping::new(
        source_id,
        vec![TriplesMap {
            id: "http://example.test/map/items".to_owned(),
            source: LogicalSource::Table(table.to_owned()),
            subject: SubjectMap {
                term: TermMap::Constant(Term::NamedNode(NamedNode::new_unchecked(
                    "http://example.test/item",
                ))),
                classes: Vec::new(),
                graphs: Vec::new(),
            },
            predicate_object_maps: Vec::new(),
        }],
    )
}

fn schema(table: &str, sql_type: &str, rows: u64) -> Vec<TableSchema> {
    let mut table = TableSchema::new(table);
    table.columns = vec![Column {
        name: "id".to_owned(),
        sql_type: sql_type.to_owned(),
        not_null: true,
        distinct_estimate: Some(rows),
    }];
    table.primary_key = vec!["id".to_owned()];
    table.row_estimate = Some(rows);
    vec![table]
}

fn binding(
    mapping: SourceMapping,
    dialect: Dialect,
    tbox: Tbox,
    schema: Vec<TableSchema>,
) -> CompilerBinding {
    CompilerBinding::from_unverified_observation(mapping, dialect, tbox, schema, Epoch(7), 8)
}

#[test]
fn identical_inputs_have_identical_deterministic_provenance() {
    let source_id = SourceId::new(3).unwrap();
    let first = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "integer", 10),
    );
    let second = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "integer", 10),
    );

    assert_eq!(first.scope(), second.scope());
    assert_eq!(first.digests(), second.digests());
}

#[test]
fn ontology_identity_is_independent_of_hash_map_insertion_order() {
    let source_id = SourceId::new(3).unwrap();
    let mut first_tbox = Tbox::default();
    first_tbox.add_subclass("http://example.test/A", "http://example.test/Parent");
    first_tbox.add_subclass("http://example.test/B", "http://example.test/Parent");
    let mut second_tbox = Tbox::default();
    second_tbox.add_subclass("http://example.test/B", "http://example.test/Parent");
    second_tbox.add_subclass("http://example.test/A", "http://example.test/Parent");

    let first = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        first_tbox,
        schema("items", "integer", 10),
    );
    let second = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        second_tbox,
        schema("items", "integer", 10),
    );

    assert_eq!(first.digests().ontology(), second.digests().ontology());
}

#[test]
fn ontology_mapping_and_epoch_each_partition_the_compile_scope() {
    let source_id = SourceId::new(3).unwrap();
    let baseline = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "integer", 10),
    );

    let mut changed_tbox = Tbox::default();
    changed_tbox.add_subclass("http://example.test/Child", "http://example.test/Parent");
    let ontology_changed = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        changed_tbox,
        schema("items", "integer", 10),
    );
    let mapping_changed = binding(
        mapping(source_id, "other_items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "integer", 10),
    );

    assert_ne!(
        baseline.digests().ontology(),
        ontology_changed.digests().ontology()
    );
    assert_eq!(
        baseline.digests().mapping(),
        ontology_changed.digests().mapping()
    );
    assert_ne!(
        baseline.digests().mapping(),
        mapping_changed.digests().mapping()
    );
    assert_ne!(baseline.scope(), ontology_changed.scope());
    assert_ne!(baseline.scope(), mapping_changed.scope());

    let later = CompilerBinding::from_unverified_observation(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "integer", 10),
        Epoch(8),
        8,
    );
    assert_eq!(baseline.digests(), later.digests());
    assert_ne!(baseline.scope(), later.scope());
}

#[test]
fn structural_type_and_capabilities_are_identity_but_statistics_are_not() {
    let source_id = SourceId::new(3).unwrap();
    let baseline = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "integer", 10),
    );
    let structure_changed = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("renamed_items", "integer", 10),
    );
    let type_changed = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "bigint", 10),
    );
    let statistics_changed = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        schema("items", "integer", 11),
    );
    let capabilities_changed = binding(
        mapping(source_id, "items"),
        Dialect::Postgres,
        Tbox::default(),
        schema("items", "integer", 10),
    );

    assert_ne!(
        baseline.digests().structural_schema(),
        structure_changed.digests().structural_schema()
    );
    assert_ne!(
        baseline.digests().type_schema(),
        type_changed.digests().type_schema()
    );
    assert_eq!(
        baseline.digests().structural_schema(),
        type_changed.digests().structural_schema()
    );
    assert_eq!(
        baseline.digests().schema(),
        statistics_changed.digests().schema()
    );
    assert_eq!(
        baseline.digests().structural_schema(),
        statistics_changed.digests().structural_schema()
    );
    assert_eq!(
        baseline.digests().type_schema(),
        statistics_changed.digests().type_schema()
    );
    assert_ne!(
        baseline.digests().capability(),
        capabilities_changed.digests().capability()
    );
    assert_eq!(
        baseline.digests().constraint_policy(),
        capabilities_changed.digests().constraint_policy()
    );
}

#[test]
fn raw_constraint_changes_invalidate_schema_identity_without_granting_authority() {
    let source_id = SourceId::new(3).unwrap();
    let with_primary_key = schema("items", "integer", 10);
    let mut without_primary_key = with_primary_key.clone();
    without_primary_key[0].primary_key.clear();
    let first = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        with_primary_key,
    );
    let second = binding(
        mapping(source_id, "items"),
        Dialect::Sqlite,
        Tbox::default(),
        without_primary_key,
    );

    assert_ne!(
        first.digests().structural_schema(),
        second.digests().structural_schema()
    );
    assert_ne!(first.digests().schema(), second.digests().schema());
    assert_eq!(
        first.digests().constraint_policy(),
        second.digests().constraint_policy()
    );
    assert_eq!(
        first.constraint_authority(),
        crate::ConstraintAuthority::Unverified
    );
    assert_eq!(
        second.constraint_authority(),
        crate::ConstraintAuthority::Unverified
    );
}
