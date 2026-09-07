use std::sync::atomic::{AtomicUsize, Ordering};

use sf_core::{SourceId, SourceMapping};
use sf_sparql::Epoch;
use tokio::sync::{mpsc, Mutex};

use super::*;
use crate::{Backend, IntrospectedSource, RuntimeSnapshot, RuntimeSource};

const POLL: Duration = Duration::from_secs(10);
const OPERATION: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Event {
    Probe,
    Build(RuntimeReadiness),
}

#[derive(Clone, Copy)]
enum Action {
    Succeed,
    Fail(ReadinessCause),
}

struct ScriptedControl {
    events: mpsc::UnboundedSender<Event>,
    actions: Mutex<mpsc::UnboundedReceiver<Action>>,
    in_flight: AtomicUsize,
    maximum_in_flight: AtomicUsize,
}

struct ScriptHarness {
    control: Arc<ScriptedControl>,
    actions: mpsc::UnboundedSender<Action>,
    events: mpsc::UnboundedReceiver<Event>,
}

impl ScriptHarness {
    fn new() -> Self {
        let (event_tx, events) = mpsc::unbounded_channel();
        let (actions, action_rx) = mpsc::unbounded_channel();
        Self {
            control: Arc::new(ScriptedControl {
                events: event_tx,
                actions: Mutex::new(action_rx),
                in_flight: AtomicUsize::new(0),
                maximum_in_flight: AtomicUsize::new(0),
            }),
            actions,
            events,
        }
    }

    async fn next_event(&mut self) -> Event {
        for _ in 0..64 {
            match self.events.try_recv() {
                Ok(event) => return event,
                Err(mpsc::error::TryRecvError::Empty) => tokio::task::yield_now().await,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("coordinator event channel closed")
                }
            }
        }
        panic!("coordinator did not emit the expected event")
    }
}

struct InFlight<'a>(&'a AtomicUsize);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ScriptedControl {
    async fn action(&self, event: Event) -> Result<(), ReadinessCause> {
        let active = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        self.maximum_in_flight.fetch_max(active, Ordering::AcqRel);
        let _guard = InFlight(&self.in_flight);
        self.events.send(event).expect("test event receiver");
        match self.actions.lock().await.recv().await {
            Some(Action::Succeed) => Ok(()),
            Some(Action::Fail(cause)) => Err(cause),
            None => Err(ReadinessCause::SourceUnavailable),
        }
    }
}

impl LifecycleControl for Arc<ScriptedControl> {
    type Generation = u64;

    fn probe<'a>(
        &'a self,
        _generation: &'a Self::Generation,
    ) -> ControlFuture<'a, Result<(), ReadinessCause>> {
        Box::pin(async move { self.action(Event::Probe).await })
    }

    fn build<'a>(
        &'a self,
        expected: RuntimeReadiness,
    ) -> ControlFuture<'a, Result<(ValidatedRuntimeCandidate, Self::Generation), ReadinessCause>>
    {
        Box::pin(async move {
            self.action(Event::Build(expected)).await?;
            let next = expected.activation_id().get();
            Ok((
                crate::pg_direct_lifecycle::sealed_candidate_for_test(expected, snapshot(next)),
                next,
            ))
        })
    }
}

fn snapshot(epoch: u64) -> RuntimeSnapshot {
    let source_id = SourceId::new(0).unwrap();
    RuntimeSnapshot::single(
        Epoch(epoch),
        crate::test_support::empty_ontology(),
        RuntimeSource::new(
            IntrospectedSource::unchecked(
                Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
                Vec::new(),
            ),
            SourceMapping::new(source_id, Vec::new()),
        ),
    )
    .unwrap()
}

fn runtime() -> Arc<RuntimeManager> {
    Arc::new(RuntimeManager::new(snapshot(0)))
}

fn config() -> Arc<ServeConfig> {
    Arc::new(
        ServeConfig::new_with_unverified_source(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
            crate::test_support::empty_ontology(),
            Vec::new(),
        )
        .unwrap(),
    )
}

fn policy() -> PgDirectCoordinatorPolicy {
    PgDirectCoordinatorPolicy::new(POLL, OPERATION).unwrap()
}

fn spawn_loop(
    runtime: Arc<RuntimeManager>,
    control: Arc<ScriptedControl>,
) -> (oneshot::Sender<()>, JoinHandle<WorkerExit>) {
    spawn_loop_with_policy(runtime, control, policy())
}

fn spawn_loop_with_policy(
    runtime: Arc<RuntimeManager>,
    control: Arc<ScriptedControl>,
    timing: PgDirectCoordinatorPolicy,
) -> (oneshot::Sender<()>, JoinHandle<WorkerExit>) {
    let authority = Arc::new(RuntimeTransitionAuthority::new());
    let (stop, stop_rx) = oneshot::channel();
    let worker = tokio::spawn(run_coordinator(
        control, 0, runtime, authority, timing, stop_rx,
    ));
    (stop, worker)
}

async fn start_tick() {
    tokio::task::yield_now().await;
    tokio::time::advance(POLL).await;
    tokio::task::yield_now().await;
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

#[test]
fn production_api_requires_both_sealed_candidate_and_transition_authority() {
    let activation = include_str!("../../activation.rs");
    assert!(activation.contains("candidate: ValidatedRuntimeCandidate"));
    assert!(activation.contains("_authority: &RuntimeTransitionAuthority"));
    assert!(activation.contains("\n    fn publish("));
    assert!(!activation.contains("pub(crate) fn publish("));
    assert!(!activation.contains("pub fn publish("));
    let authored = include_str!("../../activation_reload.rs");
    assert!(authored.contains("candidate: crate::reload::AuthoredCandidate"));
    assert!(authored.contains("_authority: &crate::reload::ReloadAuthority"));

    for request_path in [
        include_str!("../../request_generation.rs"),
        include_str!("../../http.rs"),
        include_str!("../../pg_response.rs"),
    ] {
        assert!(!request_path.contains("transition_not_ready"));
        assert!(!request_path.contains("RuntimeTransitionAuthority"));
        assert!(!request_path.contains("ReloadAuthority"));
    }
}

#[test]
fn timing_policy_rejects_zero_and_unrepresentable_bounds() {
    for (poll, operation) in [
        (Duration::ZERO, OPERATION),
        (POLL, Duration::ZERO),
        (Duration::MAX, OPERATION),
        (POLL, Duration::MAX),
    ] {
        assert!(PgDirectCoordinatorPolicy::new(poll, operation).is_err());
    }
}

#[test]
fn production_policy_is_bound_to_the_specs_immutable_operation_deadline() {
    fn spec(operation: Duration) -> PgDirectLifecycleSpec {
        let mut config = tokio_postgres::Config::new();
        config.options(crate::source::POSTGRES_RELATION_SCOPE_OPTIONS);
        PgDirectLifecycleSpec::from_resolved_config(
            config,
            2,
            Duration::from_secs(1),
            crate::test_support::empty_ontology(),
            "https://example.test/direct/",
            operation,
        )
        .unwrap()
    }

    let first = spec(Duration::from_secs(3));
    let different = spec(Duration::from_secs(4));
    let policy = PgDirectCoordinatorPolicy::for_spec(POLL, &first).unwrap();
    assert!(policy.matches(&first));
    assert!(!policy.matches(&different));
}

#[tokio::test(start_paused = true)]
async fn completed_ready_probe_failures_fence_each_closed_class() {
    for cause in [
        ReadinessCause::SourceUnavailable,
        ReadinessCause::SchemaDrift,
        ReadinessCause::CapabilityDrift,
    ] {
        let runtime = runtime();
        let mut script = ScriptHarness::new();
        let (stop, worker) = spawn_loop(Arc::clone(&runtime), Arc::clone(&script.control));
        start_tick().await;
        assert_eq!(script.next_event().await, Event::Probe);
        script.actions.send(Action::Fail(cause)).unwrap();
        settle().await;
        assert!(matches!(
            runtime.readiness().unwrap(),
            RuntimeReadiness::NotReady { cause: actual, .. } if actual == cause
        ));
        stop.send(()).unwrap();
        assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
    }
}

#[tokio::test(start_paused = true)]
async fn not_ready_failures_retry_and_only_a_complete_candidate_heals() {
    let runtime = runtime();
    let initial = runtime.readiness().unwrap();
    let first = runtime
        .mark_not_ready(initial, ReadinessCause::SchemaDrift)
        .unwrap();
    let mut script = ScriptHarness::new();
    let (stop, worker) = spawn_loop(Arc::clone(&runtime), Arc::clone(&script.control));

    start_tick().await;
    assert_eq!(script.next_event().await, Event::Build(first));
    script
        .actions
        .send(Action::Fail(ReadinessCause::SourceUnavailable))
        .unwrap();
    settle().await;
    let second = runtime.readiness().unwrap();
    assert!(second.state_revision() > first.state_revision());

    tokio::time::advance(POLL).await;
    assert_eq!(script.next_event().await, Event::Build(second));
    script
        .actions
        .send(Action::Fail(ReadinessCause::CapabilityDrift))
        .unwrap();
    settle().await;
    let third = runtime.readiness().unwrap();
    assert!(third.state_revision() > second.state_revision());

    tokio::time::advance(POLL).await;
    assert_eq!(script.next_event().await, Event::Build(third));
    script.actions.send(Action::Succeed).unwrap();
    settle().await;
    assert!(matches!(
        runtime.readiness().unwrap(),
        RuntimeReadiness::Ready { .. }
    ));
    assert_eq!(script.control.maximum_in_flight.load(Ordering::Acquire), 1);
    stop.send(()).unwrap();
    assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
}

#[tokio::test(start_paused = true)]
async fn stale_probe_and_candidate_results_cannot_overwrite_newer_state() {
    let runtime = runtime();
    let mut script = ScriptHarness::new();
    let (stop, worker) = spawn_loop(Arc::clone(&runtime), Arc::clone(&script.control));
    start_tick().await;
    assert_eq!(script.next_event().await, Event::Probe);
    let observed = runtime.readiness().unwrap();
    let newer = runtime
        .mark_not_ready(observed, ReadinessCause::CapabilityDrift)
        .unwrap();
    script
        .actions
        .send(Action::Fail(ReadinessCause::SchemaDrift))
        .unwrap();
    settle().await;
    assert_eq!(runtime.readiness().unwrap(), newer);

    tokio::time::advance(POLL).await;
    assert_eq!(script.next_event().await, Event::Build(newer));
    let newest = runtime
        .mark_not_ready(newer, ReadinessCause::SourceUnavailable)
        .unwrap();
    script.actions.send(Action::Succeed).unwrap();
    settle().await;
    assert_eq!(runtime.readiness().unwrap(), newest);
    stop.send(()).unwrap();
    assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
}

#[tokio::test(start_paused = true)]
async fn blocked_work_is_serial_and_missed_ticks_are_skipped() {
    let runtime = runtime();
    let mut script = ScriptHarness::new();
    let long_operation = PgDirectCoordinatorPolicy::new(POLL, POLL * 200).unwrap();
    let (stop, worker) =
        spawn_loop_with_policy(runtime, Arc::clone(&script.control), long_operation);
    start_tick().await;
    assert_eq!(script.next_event().await, Event::Probe);
    tokio::time::advance(POLL * 100).await;
    settle().await;
    assert_eq!(script.control.in_flight.load(Ordering::Acquire), 1);
    assert_eq!(script.control.maximum_in_flight.load(Ordering::Acquire), 1);
    assert!(matches!(
        script.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    script.actions.send(Action::Succeed).unwrap();
    assert_eq!(script.next_event().await, Event::Probe);
    settle().await;
    assert!(matches!(
        script.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    script.actions.send(Action::Succeed).unwrap();
    settle().await;
    tokio::time::advance(POLL).await;
    assert_eq!(script.next_event().await, Event::Probe);
    script.actions.send(Action::Succeed).unwrap();
    stop.send(()).unwrap();
    assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
}

#[tokio::test(start_paused = true)]
async fn a_control_timeout_fences_source_unavailable() {
    let runtime = runtime();
    let mut script = ScriptHarness::new();
    let (stop, worker) = spawn_loop(Arc::clone(&runtime), Arc::clone(&script.control));
    start_tick().await;
    assert_eq!(script.next_event().await, Event::Probe);
    tokio::time::advance(OPERATION).await;
    settle().await;
    assert!(matches!(
        runtime.readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::SourceUnavailable,
            ..
        }
    ));
    stop.send(()).unwrap();
    assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
}

fn supervised_worker(worker: JoinHandle<WorkerExit>) -> PgDirectLifecycleSupervisor {
    let config = config();
    let runtime = config.lifecycle_runtime();
    let authority = Arc::new(RuntimeTransitionAuthority::new());
    let (stop, _stop_rx) = oneshot::channel();
    PgDirectLifecycleSupervisor::supervise(config, runtime, authority, stop, worker)
}

#[tokio::test]
async fn early_return_panic_and_cancellation_each_fail_closed() {
    let early = supervised_worker(tokio::spawn(async { WorkerExit::Unexpected }));
    let early_runtime = Arc::clone(&early.runtime);
    early.wait_for_failure().await;
    assert!(matches!(
        early_runtime.readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::SourceUnavailable,
            ..
        }
    ));

    let panic = supervised_worker(tokio::spawn(async { panic!("worker fault") }));
    let panic_runtime = Arc::clone(&panic.runtime);
    panic.wait_for_failure().await;
    assert!(matches!(
        panic_runtime.readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::SourceUnavailable,
            ..
        }
    ));

    let cancelled = supervised_worker(tokio::spawn(std::future::pending::<WorkerExit>()));
    let cancelled_runtime = Arc::clone(&cancelled.runtime);
    cancelled.abort_worker();
    cancelled.wait_for_failure().await;
    assert!(matches!(
        cancelled_runtime.readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::SourceUnavailable,
            ..
        }
    ));
}

#[tokio::test]
async fn planned_shutdown_fences_then_stops_and_joins() {
    let config = config();
    let runtime = config.lifecycle_runtime();
    let authority = Arc::new(RuntimeTransitionAuthority::new());
    let (stop, stop_rx) = oneshot::channel();
    let worker = tokio::spawn(async move {
        if stop_rx.await.is_ok() {
            WorkerExit::Planned
        } else {
            WorkerExit::Unexpected
        }
    });
    let supervisor = PgDirectLifecycleSupervisor::supervise(
        config,
        Arc::clone(&runtime),
        authority,
        stop,
        worker,
    );
    supervisor.shutdown().await.unwrap();
    assert!(matches!(
        runtime.readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::Administrative,
            ..
        }
    ));
}

#[tokio::test]
async fn dropping_the_supervisor_aborts_and_fences_immediately() {
    let supervisor = supervised_worker(tokio::spawn(std::future::pending::<WorkerExit>()));
    let runtime = Arc::clone(&supervisor.runtime);
    drop(supervisor);
    assert!(matches!(
        runtime.readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::SourceUnavailable,
            ..
        }
    ));
}
