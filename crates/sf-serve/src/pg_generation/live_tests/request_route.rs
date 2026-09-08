//! Internal request-route evidence over one verified PostgreSQL generation.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sf_core::query_control::{QueryCharge, QueryLimits};
use sf_core::{Literal, NamedNode, Term, Triple};

use super::*;
use crate::request_compile::BoundQuery;
use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
use crate::{RuntimeSource, ServeConfig};

const BASE: &str = "http://example.test/base/";
const LABEL: &str = "http://example.test/base/parent#label";

pub(super) async fn exercise(fixture: &Arc<Fixture>) {
    let source_id = SourceId::new(0).expect("source zero");
    let source = IntrospectedSource::observe_postgres(fixture.pool.clone())
        .await
        .expect("observe request-route PostgreSQL");
    let (source, mapping) = build_and_bind_direct_candidate(source, BASE, source_id, &budget())
        .await
        .expect("build request-route Direct Mapping candidate");
    let ontology = direct_ontology();
    let mapping = ValidatedMapping::validate(mapping, MappingOrigin::Direct, &ontology, &source)
        .expect("admit request-route Direct Mapping");
    let source = RuntimeSource::admitted(source, mapping)
        .expect("bind request-route mapping to verified generation");
    let mut cfg = ServeConfig::from_runtime_source(source, ontology)
        .expect("build private request-route runtime");
    cfg.set_query_admission(crate::QueryAdmission::UnrestrictedDevelopment);
    let cfg = Arc::new(cfg);
    exercise_config(fixture, &cfg).await;
}

pub(super) async fn exercise_config(fixture: &Arc<Fixture>, cfg: &Arc<ServeConfig>) {
    let snapshot = cfg.runtime_lease().expect("lease request-route snapshot");

    assert_exact_metadata_reservation(fixture, cfg, &snapshot).await;

    let permits = cfg.compiler_permits();
    let spare = permits
        .available_permits()
        .checked_sub(1)
        .expect("runtime has at least one compiler permit");
    let held = permits
        .clone()
        .acquire_many_owned(u32::try_from(spare).expect("compiler permit count fits u32"))
        .await
        .expect("retain every compiler permit except one");
    assert_eq!(permits.available_permits(), 1);

    let select = format!("SELECT ?label WHERE {{ ?s <{LABEL}> ?label }}");
    let (plan, lease, request, identity) =
        acquire_compile(fixture, cfg, &snapshot, &select, &permits).await;
    let rows = Arc::new(Mutex::new(Vec::new()));
    let sink_rows = Arc::clone(&rows);
    lease
        .select_each(&plan, &request, move |row| {
            let sink_rows = Arc::clone(&sink_rows);
            async move {
                sink_rows
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(row);
                Ok(())
            }
        })
        .await
        .expect("close SELECT generation")
        .expect("execute mapped SELECT");
    assert_eq!(
        *rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![vec![Some(Term::Literal(Literal::new_simple_literal(
            "parent"
        )))]]
    );
    assert_clean_rollback(fixture, &identity).await;

    let ask = format!("ASK WHERE {{ <{BASE}parent/id=1> <{LABEL}> \"parent\" }}");
    let (plan, lease, request, ask_identity) =
        acquire_compile(fixture, cfg, &snapshot, &ask, &permits).await;
    assert_eq!(
        ask_identity, identity,
        "clean rollback must recycle the member"
    );
    assert!(lease
        .ask(&plan, &request)
        .await
        .expect("close ASK generation")
        .expect("execute mapped ASK"));
    assert_clean_rollback(fixture, &ask_identity).await;

    let construct = format!(
        "CONSTRUCT {{ ?s <http://example.test/copied> ?label }} \
         WHERE {{ ?s <{LABEL}> ?label }}"
    );
    let (plan, lease, request, construct_identity) =
        acquire_compile(fixture, cfg, &snapshot, &construct, &permits).await;
    assert_eq!(
        construct_identity, identity,
        "every request must reuse only the acknowledged-clean member"
    );
    let triples = Arc::new(Mutex::new(Vec::new()));
    let sink_triples = Arc::clone(&triples);
    lease
        .construct_each(&plan, &request, move |batch| {
            let sink_triples = Arc::clone(&sink_triples);
            async move {
                sink_triples
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(batch);
                Ok(())
            }
        })
        .await
        .expect("close CONSTRUCT generation")
        .expect("execute mapped CONSTRUCT");
    assert_eq!(
        *triples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![Triple::new(
            NamedNode::new(format!("{BASE}parent/id=1")).expect("valid expected subject"),
            NamedNode::new("http://example.test/copied").expect("valid expected predicate"),
            Literal::new_simple_literal("parent"),
        )]
    );
    assert_clean_rollback(fixture, &construct_identity).await;

    assert_eq!(permits.available_permits(), 1);
    drop(held);
}

async fn assert_exact_metadata_reservation(
    fixture: &Fixture,
    cfg: &Arc<ServeConfig>,
    snapshot: &crate::activation::RuntimeSnapshotLease,
) {
    let permits = cfg.compiler_permits();
    let available = permits.available_permits();
    let query = format!("SELECT ?label WHERE {{ ?s <{LABEL}> ?label }}");
    let request = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(
            10_000,
            GENERATION_METADATA_PROBE_RESERVATION,
            10_000,
            1_000_000,
        ),
    );

    let admission = crate::request_generation::acquire(cfg.clone(), snapshot, &query, &request)
        .await
        .unwrap_or_else(|response| {
            panic!(
                "exact metadata reservation admission failed: {}",
                response.status()
            )
        });
    assert_eq!(
        request.consumed(QueryCharge::SourceWork),
        GENERATION_METADATA_PROBE_RESERVATION
    );
    let (mut generations, reservation) = admission.into_parts();
    assert!(reservation.is_some());
    assert_eq!(permits.available_permits(), available - 1);
    let bound = crate::request_compile::compile(
        cfg.clone(),
        snapshot.clone(),
        query,
        request.clone(),
        reservation,
        false,
    )
    .await
    .unwrap_or_else(|response| {
        panic!(
            "exact metadata reservation compile failed: {}",
            response.status()
        )
    });
    assert_eq!(permits.available_permits(), available);
    let BoundQuery::Single(bound) = bound else {
        panic!("exact metadata reservation must compile a single-source plan")
    };
    let executable = snapshot
        .prepare_execution(*bound)
        .expect("prepare exact-reservation execution binding");
    let (source_id, binding, _, verified, _) = executable.into_parts();
    assert!(generations.matches(source_id, &binding, verified));
    let lease = generations
        .take(source_id, &binding)
        .expect("take exact-reservation generation");
    assert!(generations.is_empty());
    let client = lease.execution_client();
    let identity = backend_identity(&client).await;
    drop(client);
    assert_generation_transaction(fixture, &identity).await;
    lease
        .finish_bounded(&request)
        .await
        .expect("complete exact-reservation metadata lifecycle");
    assert_clean_rollback(fixture, &identity).await;
    assert_eq!(
        request.consumed(QueryCharge::SourceWork),
        GENERATION_METADATA_PROBE_RESERVATION
    );
}

pub(super) fn direct_ontology() -> crate::SemanticOntology {
    crate::test_support::ontology(
        &[
            "http://example.test/base/child",
            "http://example.test/base/parent",
        ],
        &[
            "http://example.test/base/child#alternate_parent_id",
            "http://example.test/base/child#id",
            "http://example.test/base/child#label",
            "http://example.test/base/child#parent_id",
            "http://example.test/base/child#ref-parent_id",
            "http://example.test/base/parent#id",
            LABEL,
        ],
    )
}

async fn acquire_compile(
    fixture: &Fixture,
    cfg: &Arc<ServeConfig>,
    snapshot: &crate::activation::RuntimeSnapshotLease,
    query: &str,
    permits: &tokio::sync::Semaphore,
) -> (
    Arc<sf_sparql::Plan>,
    VerifiedPostgresGenerationLease,
    RequestBudget,
    BackendIdentity,
) {
    let request = RequestBudget::after(
        Duration::from_secs(5),
        QueryLimits::new(10_000, 10_000, 10_000, 1_000_000),
    );
    let admission = crate::request_generation::acquire(cfg.clone(), snapshot, query, &request)
        .await
        .unwrap_or_else(|response| {
            panic!("request generation admission failed: {}", response.status())
        });
    let (mut generations, reservation) = admission.into_parts();
    assert!(
        reservation.is_some(),
        "verified generation reserves compiler"
    );
    assert_eq!(
        permits.available_permits(),
        0,
        "preflight must retain the only free compiler permit across source acquisition"
    );
    let bound = crate::request_compile::compile(
        cfg.clone(),
        snapshot.clone(),
        query.to_owned(),
        request.clone(),
        reservation,
        false,
    )
    .await
    .unwrap_or_else(|response| panic!("authoritative compile failed: {}", response.status()));
    assert_eq!(
        permits.available_permits(),
        1,
        "authoritative compile must consume the retained permit without requeueing"
    );
    let BoundQuery::Single(bound) = bound else {
        panic!("request-route fixture must compile a single-source plan")
    };
    crate::admission::admit(bound.plan(), cfg.max_order_rows())
        .expect("request-route plan is admitted");
    let executable = snapshot
        .prepare_execution(*bound)
        .expect("prepare binding-owned request-route execution");
    let (source_id, binding, backend, verified, plan) = executable.into_parts();
    assert!(generations.matches(source_id, &binding, verified));
    assert!(matches!(backend, Backend::Pg(_)));
    let lease = generations
        .take(source_id, &binding)
        .expect("take exact binding-matched generation");
    assert!(generations.is_empty());
    let client = lease.execution_client();
    let identity = backend_identity(&client).await;
    drop(client);
    assert_generation_transaction(fixture, &identity).await;
    (plan, lease, request, identity)
}

async fn assert_generation_transaction(fixture: &Fixture, identity: &BackendIdentity) {
    let row = fixture
        .admin
        .query_one(
            "SELECT state::text AS state, xact_start IS NOT NULL AS in_transaction, \
             backend_xmin IS NOT NULL AS has_snapshot FROM pg_catalog.pg_stat_activity \
             WHERE pid = $1 AND backend_start::text = $2",
            &[&identity.pid, &identity.started_at],
        )
        .await
        .expect("inspect request-route generation transaction");
    assert_eq!(row.get::<_, String>("state"), "idle in transaction");
    assert!(row.get::<_, bool>("in_transaction"));
    assert!(row.get::<_, bool>("has_snapshot"));
}

async fn assert_clean_rollback(fixture: &Fixture, identity: &BackendIdentity) {
    let row = fixture
        .admin
        .query_one(
            "SELECT state::text AS state, xact_start IS NULL AS no_transaction, \
             backend_xmin IS NULL AS no_snapshot FROM pg_catalog.pg_stat_activity \
             WHERE pid = $1 AND backend_start::text = $2",
            &[&identity.pid, &identity.started_at],
        )
        .await
        .expect("inspect request-route generation rollback");
    assert_eq!(row.get::<_, String>("state"), "idle");
    assert!(row.get::<_, bool>("no_transaction"));
    assert!(row.get::<_, bool>("no_snapshot"));
}
