use sf_core::ir::{LogicalSource, SubjectMap, TermMap, TriplesMap};
use sf_core::query_control::UncontrolledQueryControl;
use sf_core::{NamedNode, SourceId, SourceMapping, Term};
use sf_sparql::{Epoch, Tbox};
use sf_sql::TableSchema;

use crate::{Backend, IntrospectedSource, RuntimeSnapshot, RuntimeSource, SnapshotError};

fn runtime_source(index: usize, table: &str) -> RuntimeSource {
    let source_id = SourceId::new(index).unwrap();
    let mapping = SourceMapping::new(
        source_id,
        vec![TriplesMap {
            id: format!("http://example.test/map/{table}"),
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
    );
    let source = IntrospectedSource::unchecked(
        Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
        vec![TableSchema::new(table)],
    );
    RuntimeSource::new(source, mapping)
}

#[test]
fn registry_is_source_keyed_and_accepts_multiple_runtime_sources() {
    let snapshot = RuntimeSnapshot::new(
        Epoch(4),
        Tbox::default(),
        vec![runtime_source(7, "seven"), runtime_source(2, "two")],
    )
    .unwrap();

    assert_eq!(snapshot.epoch(), Epoch(4));
    assert_eq!(
        snapshot.registry().source_ids().collect::<Vec<_>>(),
        vec![SourceId::new(2).unwrap(), SourceId::new(7).unwrap()]
    );
    assert_eq!(snapshot.registry().len(), 2);
    assert_eq!(
        snapshot
            .registry()
            .schema(SourceId::new(7).unwrap())
            .unwrap()[0]
            .name,
        "seven"
    );
}

#[test]
fn empty_and_duplicate_source_registries_fail_closed() {
    assert!(matches!(
        RuntimeSnapshot::new(Epoch(0), Tbox::default(), Vec::new()),
        Err(SnapshotError::EmptyRegistry)
    ));
    assert!(matches!(
        RuntimeSnapshot::new(
            Epoch(0),
            Tbox::default(),
            vec![runtime_source(1, "first"), runtime_source(1, "second")],
        ),
        Err(SnapshotError::DuplicateSource { source_id })
            if source_id == SourceId::new(1).unwrap()
    ));
}

#[test]
fn compile_provenance_is_checked_against_the_execution_snapshot() {
    let source_id = SourceId::new(0).unwrap();
    let first =
        RuntimeSnapshot::new(Epoch(0), Tbox::default(), vec![runtime_source(0, "items")]).unwrap();
    let replacement = RuntimeSnapshot::new(
        Epoch(0),
        Tbox::default(),
        vec![runtime_source(0, "replacement_items")],
    )
    .unwrap();
    let bound = first
        .compile(
            source_id,
            "SELECT * WHERE { ?s ?p ?o }",
            &UncontrolledQueryControl,
        )
        .unwrap();

    assert!(replacement.prepare_execution(bound).is_err());
}

#[test]
fn singleton_constructor_preserves_the_current_runtime_shape() {
    let source = runtime_source(5, "items");
    let snapshot = RuntimeSnapshot::single(Epoch(0), Tbox::default(), source);

    assert_eq!(snapshot.registry().len(), 1);
    assert!(snapshot
        .registry()
        .contains_source(SourceId::new(5).unwrap()));
}
