//! Regression: a deadline bounds acceptance, not the lifetime of native work.

use super::*;

fn unavailable(runtime: &RuntimeManager) -> RuntimeReadiness {
    let state = runtime.readiness().unwrap();
    assert!(matches!(
        state,
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::SourceUnavailable,
            ..
        }
    ));
    state
}

#[tokio::test(start_paused = true)]
async fn timed_out_build_stays_owned_and_discards_late_success() {
    let runtime = runtime();
    let expected = runtime
        .mark_not_ready(runtime.readiness().unwrap(), ReadinessCause::SchemaDrift)
        .unwrap();
    let mut script = ScriptHarness::new();
    let (stop, worker) = spawn_loop(Arc::clone(&runtime), Arc::clone(&script.control));
    start_tick().await;
    assert_eq!(script.next_event().await, Event::Build(expected));

    tokio::time::advance(OPERATION).await;
    settle().await;
    let fenced = unavailable(&runtime);
    assert_eq!(script.control.in_flight.load(Ordering::Acquire), 1);
    tokio::time::advance(POLL * 100).await;
    settle().await;
    assert_eq!(runtime.readiness().unwrap(), fenced);
    assert!(matches!(
        script.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));

    script.actions.send(Action::Succeed).unwrap();
    assert_eq!(script.next_event().await, Event::Build(fenced));
    assert_eq!(
        runtime.readiness().unwrap(),
        fenced,
        "late candidate cannot heal"
    );
    assert_eq!(script.control.maximum_in_flight.load(Ordering::Acquire), 1);
    script.actions.send(Action::Succeed).unwrap();
    settle().await;
    assert!(matches!(
        runtime.readiness().unwrap(),
        RuntimeReadiness::Ready { .. }
    ));
    stop.send(()).unwrap();
    assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
}

#[tokio::test(start_paused = true)]
async fn stopping_during_build_joins_without_publishing() {
    let runtime = runtime();
    let expected = runtime
        .mark_not_ready(runtime.readiness().unwrap(), ReadinessCause::SchemaDrift)
        .unwrap();
    let mut script = ScriptHarness::new();
    let (stop, worker) = spawn_loop(Arc::clone(&runtime), Arc::clone(&script.control));
    start_tick().await;
    assert_eq!(script.next_event().await, Event::Build(expected));
    stop.send(()).unwrap();
    settle().await;
    assert!(
        !worker.is_finished(),
        "shutdown must retain outstanding build ownership"
    );
    assert_eq!(script.control.in_flight.load(Ordering::Acquire), 1);
    script.actions.send(Action::Succeed).unwrap();
    assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
    assert_eq!(runtime.readiness().unwrap(), expected);
}

#[tokio::test(start_paused = true)]
async fn panic_after_build_timeout_terminates_instead_of_retrying() {
    let runtime = runtime();
    let expected = runtime
        .mark_not_ready(runtime.readiness().unwrap(), ReadinessCause::SchemaDrift)
        .unwrap();
    let mut script = ScriptHarness::new();
    let (_stop, worker) = spawn_loop(Arc::clone(&runtime), Arc::clone(&script.control));
    start_tick().await;
    assert_eq!(script.next_event().await, Event::Build(expected));
    tokio::time::advance(OPERATION).await;
    settle().await;
    let fenced = unavailable(&runtime);
    script.actions.send(Action::Panic).unwrap();
    settle().await;
    assert!(
        worker.is_finished(),
        "late panic must be observed immediately"
    );
    assert_eq!(worker.await.unwrap(), WorkerExit::Unexpected);
    assert_eq!(runtime.readiness().unwrap(), fenced);
}

#[tokio::test(start_paused = true)]
async fn completed_candidate_polled_at_deadline_cannot_publish() {
    let runtime = runtime();
    let expected = runtime
        .mark_not_ready(runtime.readiness().unwrap(), ReadinessCause::SchemaDrift)
        .unwrap();
    let script = ScriptHarness::new();
    script.actions.send(Action::Succeed).unwrap();
    let (_stop, mut stopped) = oneshot::channel();
    let authority = RuntimeTransitionAuthority::new();
    let mut build = Box::pin(build_once(
        Arc::new(Arc::clone(&script.control)),
        expected,
        &runtime,
        &authority,
        policy(),
        &mut stopped,
    ));
    // Start the build, but hold the coordinator's next poll until its deadline.
    std::future::poll_fn(|cx| {
        assert!(build.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    settle().await;
    tokio::time::advance(OPERATION).await;
    assert!(
        build.await.unwrap().is_none(),
        "ready JoinHandle must not beat deadline"
    );
    unavailable(&runtime);
}

struct PhysicalControl {
    started: mpsc::UnboundedSender<()>,
    release: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
    attempts: AtomicUsize,
    active: AtomicUsize,
}

impl LifecycleControl for Arc<PhysicalControl> {
    type Generation = u64;

    fn probe<'a>(&'a self, _: &'a u64) -> ControlFuture<'a, Result<(), ReadinessCause>> {
        Box::pin(async { Ok(()) })
    }

    fn build<'a>(
        &'a self,
        expected: RuntimeReadiness,
    ) -> ControlFuture<'a, Result<(ValidatedRuntimeCandidate, u64), ReadinessCause>> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::AcqRel);
            let released = self
                .release
                .lock()
                .unwrap()
                .take()
                .expect("one physical build");
            let owner = Arc::clone(self);
            let budget = crate::budget::RequestBudget::after(
                Duration::from_secs(60),
                sf_core::query_control::QueryLimits::new(0, 0, 0, 0),
            );
            crate::pg_generation::candidate_work::run(&budget, move || {
                owner.active.fetch_add(1, Ordering::AcqRel);
                let _guard = InFlight(&owner.active);
                owner.started.send(()).unwrap();
                let _ = released.blocking_recv();
            })
            .await
            .map_err(|_| ReadinessCause::SourceUnavailable)?;
            Ok((
                crate::pg_direct_lifecycle::sealed_candidate_for_test(expected, snapshot(1)),
                1,
            ))
        })
    }
}

#[tokio::test]
async fn native_worker_remains_single_across_timeout_ticks_and_shutdown() {
    let runtime = runtime();
    runtime
        .mark_not_ready(runtime.readiness().unwrap(), ReadinessCause::SchemaDrift)
        .unwrap();
    let (started, mut starts) = mpsc::unbounded_channel();
    let (release, released) = oneshot::channel();
    let control = Arc::new(PhysicalControl {
        started,
        release: std::sync::Mutex::new(Some(released)),
        attempts: AtomicUsize::new(0),
        active: AtomicUsize::new(0),
    });
    let (stop, stopped) = oneshot::channel();
    let worker = tokio::spawn(run_coordinator(
        Arc::clone(&control),
        0,
        Arc::clone(&runtime),
        Arc::new(RuntimeTransitionAuthority::new()),
        PgDirectCoordinatorPolicy::new(Duration::from_millis(1), OPERATION).unwrap(),
        stopped,
    ));
    // Wait for the physical thread, then control virtual time deterministically.
    starts.recv().await.unwrap();
    tokio::time::pause();
    // The timer was registered before pausing; pass its rounding boundary too.
    tokio::time::advance(OPERATION * 2).await;
    settle().await;
    unavailable(&runtime);
    tokio::time::advance(POLL * 100).await;
    settle().await;
    assert_eq!(control.active.load(Ordering::Acquire), 1);
    assert_eq!(control.attempts.load(Ordering::Acquire), 1);
    runtime.mark_current_administratively_not_ready().unwrap();
    let shutdown = runtime.readiness().unwrap();
    stop.send(()).unwrap();
    settle().await;
    assert!(!worker.is_finished());
    release.send(()).unwrap();
    assert_eq!(worker.await.unwrap(), WorkerExit::Planned);
    assert_eq!(control.active.load(Ordering::Acquire), 0);
    assert_eq!(runtime.readiness().unwrap(), shutdown);
}
