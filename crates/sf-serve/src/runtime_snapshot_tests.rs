use sf_core::ir::{LogicalSource, SubjectMap, TermMap, TriplesMap};
use sf_core::query_control::UncontrolledQueryControl;
use sf_core::{NamedNode, SourceId, SourceMapping, Term};
use sf_sparql::Epoch;
use sf_sql::TableSchema;

use crate::activation::{RuntimeManager, SnapshotUnavailable};
use crate::{
    ActivationError, Backend, IntrospectedSource, ReadinessCause, RuntimeReadiness,
    RuntimeSnapshot, RuntimeSource, SnapshotError,
};

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

fn snapshot(table: &str) -> RuntimeSnapshot {
    RuntimeSnapshot::single(
        Epoch(0),
        crate::test_support::empty_ontology(),
        runtime_source(0, table),
    )
    .unwrap()
}

#[test]
fn registry_is_source_keyed_and_accepts_multiple_runtime_sources() {
    let snapshot = RuntimeSnapshot::new(
        Epoch(4),
        crate::test_support::empty_ontology(),
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
        RuntimeSnapshot::new(Epoch(0), crate::test_support::empty_ontology(), Vec::new(),),
        Err(SnapshotError::EmptyRegistry)
    ));
    assert!(matches!(
        RuntimeSnapshot::new(
            Epoch(0),
            crate::test_support::empty_ontology(),
            vec![runtime_source(1, "first"), runtime_source(1, "second")],
        ),
        Err(SnapshotError::DuplicateSource { source_id })
            if source_id == SourceId::new(1).unwrap()
    ));
}

#[test]
fn compile_provenance_is_checked_against_the_execution_snapshot() {
    let source_id = SourceId::new(0).unwrap();
    let first = RuntimeSnapshot::new(
        Epoch(0),
        crate::test_support::empty_ontology(),
        vec![runtime_source(0, "items")],
    )
    .unwrap();
    let replacement = RuntimeSnapshot::new(
        Epoch(0),
        crate::test_support::empty_ontology(),
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
    let snapshot =
        RuntimeSnapshot::single(Epoch(0), crate::test_support::empty_ontology(), source).unwrap();

    assert_eq!(snapshot.registry().len(), 1);
    assert!(snapshot
        .registry()
        .contains_source(SourceId::new(5).unwrap()));
}

#[test]
fn activation_is_monotonic_pins_old_requests_and_rejects_a_stale_candidate() {
    let manager = RuntimeManager::new(snapshot("first"));
    let old_lease = manager.lease().unwrap();
    let old_state = manager.readiness().unwrap();
    let old_id = old_lease.activation_id();
    let old_snapshot = old_lease.weak_snapshot();

    let next_id = manager.activate(old_state, snapshot("second")).unwrap();
    assert!(next_id > old_id);
    assert_eq!(
        old_lease
            .snapshot()
            .registry()
            .schema(SourceId::new(0).unwrap())
            .unwrap()[0]
            .name,
        "first"
    );
    assert!(matches!(
        manager.activate(old_state, snapshot("stale")),
        Err(ActivationError::StaleState {
            expected,
            actual: RuntimeReadiness::Ready { activation_id },
        }) if expected == old_state && activation_id == next_id
    ));
    assert_eq!(
        manager
            .lease()
            .unwrap()
            .snapshot()
            .registry()
            .schema(SourceId::new(0).unwrap())
            .unwrap()[0]
            .name,
        "second"
    );

    assert!(old_snapshot.upgrade().is_some());
    drop(old_lease);
    assert!(old_snapshot.upgrade().is_none());

    let third_id = manager
        .activate(manager.readiness().unwrap(), snapshot("first"))
        .unwrap();
    assert!(third_id > next_id, "A-B-A publication must not reuse an ID");
}

#[test]
fn rejected_candidate_construction_leaves_the_active_snapshot_unchanged() {
    let manager = RuntimeManager::new(snapshot("active"));
    let before = manager.lease().unwrap();
    let before_identity = before.weak_snapshot();

    assert!(matches!(
        RuntimeSnapshot::new(Epoch(1), crate::test_support::empty_ontology(), Vec::new(),),
        Err(SnapshotError::EmptyRegistry)
    ));

    let after = manager.lease().unwrap();
    assert_eq!(after.activation_id(), before.activation_id());
    assert!(std::sync::Weak::ptr_eq(
        &before_identity,
        &after.weak_snapshot()
    ));
}

#[test]
fn drift_blocks_new_leases_until_a_new_generation_activates() {
    let manager = RuntimeManager::new(snapshot("active"));
    let in_flight = manager.lease().unwrap();
    let original_id = in_flight.activation_id();

    manager
        .mark_not_ready(original_id, ReadinessCause::SchemaDrift)
        .unwrap();
    assert_eq!(
        manager.readiness().unwrap(),
        RuntimeReadiness::NotReady {
            activation_id: original_id,
            cause: ReadinessCause::SchemaDrift,
        }
    );
    assert!(matches!(
        manager.lease(),
        Err(SnapshotUnavailable::NotReady {
            activation_id,
            cause: ReadinessCause::SchemaDrift,
        }) if activation_id == original_id
    ));
    assert!(matches!(
        manager.mark_not_ready(original_id, ReadinessCause::SourceUnavailable),
        Err(ActivationError::AlreadyNotReady { activation_id })
            if activation_id == original_id
    ));

    let not_ready = manager.readiness().unwrap();
    let replacement_id = manager
        .activate(not_ready, snapshot("replacement"))
        .unwrap();
    assert!(replacement_id > original_id);
    assert!(matches!(
        manager.readiness().unwrap(),
        RuntimeReadiness::Ready { activation_id } if activation_id == replacement_id
    ));
    assert_eq!(in_flight.activation_id(), original_id);
    assert!(matches!(
        manager.mark_not_ready(original_id, ReadinessCause::SchemaDrift),
        Err(ActivationError::StaleGeneration { expected, actual })
            if expected == original_id && actual == replacement_id
    ));
}

#[test]
fn slow_candidate_cannot_overwrite_a_faster_successor() {
    let manager = std::sync::Arc::new(RuntimeManager::new(snapshot("initial")));
    let expected = manager.readiness().unwrap();
    let (release_slow, wait_for_fast) = std::sync::mpsc::sync_channel(0);
    let slow_manager = manager.clone();
    let slow = std::thread::spawn(move || {
        let candidate = snapshot("slow");
        wait_for_fast.recv().unwrap();
        slow_manager.activate(expected, candidate)
    });

    let fast_id = manager.activate(expected, snapshot("fast")).unwrap();
    release_slow.send(()).unwrap();
    assert!(matches!(
        slow.join().unwrap(),
        Err(ActivationError::StaleState {
            expected: stale,
            actual: RuntimeReadiness::Ready { activation_id },
        }) if stale == expected && activation_id == fast_id
    ));
    assert_eq!(
        manager
            .lease()
            .unwrap()
            .snapshot()
            .registry()
            .schema(SourceId::new(0).unwrap())
            .unwrap()[0]
            .name,
        "fast"
    );
}

#[test]
fn candidate_built_before_drift_cannot_heal_the_not_ready_state() {
    let manager = std::sync::Arc::new(RuntimeManager::new(snapshot("initial")));
    let expected = manager.readiness().unwrap();
    let activation_id = expected.activation_id();
    let (release_slow, wait_for_drift) = std::sync::mpsc::sync_channel(0);
    let slow_manager = manager.clone();
    let slow = std::thread::spawn(move || {
        let candidate = snapshot("pre-drift");
        wait_for_drift.recv().unwrap();
        slow_manager.activate(expected, candidate)
    });

    manager
        .mark_not_ready(activation_id, ReadinessCause::SchemaDrift)
        .unwrap();
    let drifted = manager.readiness().unwrap();
    release_slow.send(()).unwrap();
    assert!(matches!(
        slow.join().unwrap(),
        Err(ActivationError::StaleState { expected: stale, actual })
            if stale == expected && actual == drifted
    ));
    assert!(matches!(
        manager.lease(),
        Err(SnapshotUnavailable::NotReady {
            activation_id: actual,
            cause: ReadinessCause::SchemaDrift,
        }) if actual == activation_id
    ));

    let healed = manager.activate(drifted, snapshot("post-drift")).unwrap();
    assert!(healed > activation_id);
}
