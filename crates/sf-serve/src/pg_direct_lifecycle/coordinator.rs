//! Serialized readiness coordinator and fail-closed task supervision.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::{Instant, MissedTickBehavior};

use super::{PgDirectLifecycleSpec, ValidatedRuntimeCandidate};
use crate::activation::{ActivationError, RuntimeManager};
use crate::pg_generation::PostgresDirectExpectation;
use crate::{ReadinessCause, RuntimeReadiness, ServeConfig};

type ControlFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Unforgeable capability held only by the lifecycle worker and its supervisor.
pub(crate) struct RuntimeTransitionAuthority {
    _sealed: (),
}

impl RuntimeTransitionAuthority {
    fn new() -> Self {
        Self { _sealed: () }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::new()
    }
}

/// Fixed timing law for one coordinator instance.
#[derive(Clone, Copy)]
pub(crate) struct PgDirectCoordinatorPolicy {
    poll_interval: Duration,
    operation_timeout: Duration,
}

impl PgDirectCoordinatorPolicy {
    pub(crate) fn for_spec(
        poll_interval: Duration,
        spec: &PgDirectLifecycleSpec,
    ) -> Result<Self, ReadinessCause> {
        Self::new(poll_interval, spec.operation_timeout())
    }

    fn new(poll_interval: Duration, operation_timeout: Duration) -> Result<Self, ReadinessCause> {
        if poll_interval.is_zero()
            || operation_timeout.is_zero()
            || Instant::now().checked_add(poll_interval).is_none()
            || Instant::now().checked_add(operation_timeout).is_none()
        {
            return Err(ReadinessCause::CapabilityDrift);
        }
        Ok(Self {
            poll_interval,
            operation_timeout,
        })
    }

    fn matches(&self, spec: &PgDirectLifecycleSpec) -> bool {
        self.operation_timeout == spec.operation_timeout()
    }
}

trait LifecycleControl: Send + Sync + 'static {
    type Generation: Send + 'static;

    fn probe<'a>(
        &'a self,
        generation: &'a Self::Generation,
    ) -> ControlFuture<'a, Result<(), ReadinessCause>>;

    fn build<'a>(
        &'a self,
        expected: RuntimeReadiness,
    ) -> ControlFuture<'a, Result<(ValidatedRuntimeCandidate, Self::Generation), ReadinessCause>>;
}

impl LifecycleControl for PgDirectLifecycleSpec {
    type Generation = PostgresDirectExpectation;

    fn probe<'a>(
        &'a self,
        generation: &'a Self::Generation,
    ) -> ControlFuture<'a, Result<(), ReadinessCause>> {
        Box::pin(PgDirectLifecycleSpec::probe(self, generation))
    }

    fn build<'a>(
        &'a self,
        expected: RuntimeReadiness,
    ) -> ControlFuture<'a, Result<(ValidatedRuntimeCandidate, Self::Generation), ReadinessCause>>
    {
        Box::pin(async move { Ok(self.build_candidate(expected).await?.into_parts()) })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerExit {
    Planned,
    Unexpected,
}

/// Owns the sole worker, its stop capability, and the monitor that fences any
/// abnormal termination. Dropping an armed supervisor also fences immediately.
pub(crate) struct PgDirectLifecycleSupervisor {
    config: Arc<ServeConfig>,
    runtime: Arc<RuntimeManager>,
    authority: Arc<RuntimeTransitionAuthority>,
    planned: Arc<AtomicBool>,
    stop: Option<oneshot::Sender<()>>,
    worker_abort: AbortHandle,
    monitor: Option<JoinHandle<()>>,
    armed: bool,
}

impl PgDirectLifecycleSupervisor {
    pub(crate) fn start(
        config: Arc<ServeConfig>,
        control: PgDirectLifecycleSpec,
        generation: PostgresDirectExpectation,
        policy: PgDirectCoordinatorPolicy,
    ) -> Result<Self, ReadinessCause> {
        if !policy.matches(&control) {
            return Err(ReadinessCause::CapabilityDrift);
        }
        config.claim_pg_direct_lifecycle()?;
        let runtime = config.lifecycle_runtime();
        let authority = Arc::new(RuntimeTransitionAuthority::new());
        let (stop, stop_rx) = oneshot::channel();
        let worker = tokio::spawn(run_coordinator(
            control,
            generation,
            Arc::clone(&runtime),
            Arc::clone(&authority),
            policy,
            stop_rx,
        ));
        Ok(Self::supervise(config, runtime, authority, stop, worker))
    }

    fn supervise<F>(
        config: Arc<ServeConfig>,
        runtime: Arc<RuntimeManager>,
        authority: Arc<RuntimeTransitionAuthority>,
        stop: oneshot::Sender<()>,
        worker: JoinHandle<F>,
    ) -> Self
    where
        F: IntoWorkerExit + Send + 'static,
    {
        let worker_abort = worker.abort_handle();
        let planned = Arc::new(AtomicBool::new(false));
        let monitor_runtime = Arc::clone(&runtime);
        let monitor_authority = Arc::clone(&authority);
        let monitor_planned = Arc::clone(&planned);
        let monitor = tokio::spawn(async move {
            let clean = match worker.await {
                Ok(exit) => exit.into_worker_exit() == WorkerExit::Planned,
                Err(_) => false,
            } && monitor_planned.load(Ordering::Acquire);
            if !clean {
                fail_closed(&monitor_runtime, &monitor_authority);
            }
        });
        Self {
            config,
            runtime,
            authority,
            planned,
            stop: Some(stop),
            worker_abort,
            monitor: Some(monitor),
            armed: true,
        }
    }

    pub(crate) async fn shutdown(mut self) -> Result<(), ActivationError> {
        self.config.begin_shutdown();
        self.planned.store(true, Ordering::Release);
        if self.stop.take().is_some_and(|stop| stop.send(()).is_err()) {
            fail_closed(&self.runtime, &self.authority);
        }
        if let Some(monitor) = self.monitor.take() {
            if monitor.await.is_err() {
                fail_closed(&self.runtime, &self.authority);
            }
        }
        self.armed = false;
        match self.runtime.readiness()? {
            RuntimeReadiness::NotReady {
                cause: ReadinessCause::Administrative,
                ..
            } => Ok(()),
            _ => Err(ActivationError::ShuttingDown),
        }
    }

    #[cfg(test)]
    pub(crate) fn abort_worker(&self) {
        self.worker_abort.abort();
    }

    #[cfg(test)]
    async fn wait_for_failure(mut self) {
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.await;
        }
        self.armed = false;
    }
}

impl Drop for PgDirectLifecycleSupervisor {
    fn drop(&mut self) {
        if self.armed {
            fail_closed(&self.runtime, &self.authority);
            self.worker_abort.abort();
            if let Some(monitor) = &self.monitor {
                monitor.abort();
            }
        }
    }
}

trait IntoWorkerExit {
    fn into_worker_exit(self) -> WorkerExit;
}

impl IntoWorkerExit for WorkerExit {
    fn into_worker_exit(self) -> WorkerExit {
        self
    }
}

async fn run_coordinator<C>(
    control: C,
    mut generation: C::Generation,
    runtime: Arc<RuntimeManager>,
    authority: Arc<RuntimeTransitionAuthority>,
    policy: PgDirectCoordinatorPolicy,
    mut stop: oneshot::Receiver<()>,
) -> WorkerExit
where
    C: LifecycleControl,
{
    let start = Instant::now() + policy.poll_interval;
    let mut ticks = tokio::time::interval_at(start, policy.poll_interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            stopped = &mut stop => {
                return if stopped.is_ok() { WorkerExit::Planned } else { WorkerExit::Unexpected };
            }
            _ = ticks.tick() => {}
        }

        let expected = match runtime.readiness() {
            Ok(expected) => expected,
            Err(_) => return WorkerExit::Unexpected,
        };
        match expected {
            RuntimeReadiness::Ready { .. } => {
                let result = tokio::select! {
                    biased;
                    stopped = &mut stop => {
                        return if stopped.is_ok() { WorkerExit::Planned } else { WorkerExit::Unexpected };
                    }
                    result = tokio::time::timeout(policy.operation_timeout, control.probe(&generation)) => {
                        result.unwrap_or(Err(ReadinessCause::SourceUnavailable))
                    }
                };
                if let Err(cause) = result {
                    if transition_failure(&runtime, &authority, expected, cause).is_err() {
                        return WorkerExit::Unexpected;
                    }
                }
            }
            RuntimeReadiness::NotReady {
                cause: ReadinessCause::Administrative | ReadinessCause::StateRevisionExhausted,
                ..
            } => {}
            RuntimeReadiness::NotReady { .. } => {
                let result = tokio::select! {
                    biased;
                    stopped = &mut stop => {
                        return if stopped.is_ok() { WorkerExit::Planned } else { WorkerExit::Unexpected };
                    }
                    result = tokio::time::timeout(policy.operation_timeout, control.build(expected)) => {
                        result.unwrap_or(Err(ReadinessCause::SourceUnavailable))
                    }
                };
                match result {
                    Ok((candidate, successor)) => {
                        match runtime.activate_candidate(&authority, candidate) {
                            Ok(_) => generation = successor,
                            Err(ActivationError::StaleState { .. }) => {}
                            Err(_) => return WorkerExit::Unexpected,
                        }
                    }
                    Err(cause) => {
                        if transition_failure(&runtime, &authority, expected, cause).is_err() {
                            return WorkerExit::Unexpected;
                        }
                    }
                }
            }
        }
    }
}

fn transition_failure(
    runtime: &RuntimeManager,
    authority: &RuntimeTransitionAuthority,
    expected: RuntimeReadiness,
    cause: ReadinessCause,
) -> Result<(), ActivationError> {
    match runtime.transition_not_ready(authority, expected, cause) {
        Ok(_) | Err(ActivationError::StaleState { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

fn fail_closed(runtime: &RuntimeManager, authority: &RuntimeTransitionAuthority) {
    loop {
        let Ok(expected) = runtime.readiness() else {
            return;
        };
        if matches!(
            expected,
            RuntimeReadiness::NotReady {
                cause: ReadinessCause::Administrative | ReadinessCause::StateRevisionExhausted,
                ..
            }
        ) {
            return;
        }
        match runtime.transition_not_ready(authority, expected, ReadinessCause::SourceUnavailable) {
            Ok(_) | Err(ActivationError::StateRevisionExhausted { .. }) => return,
            Err(ActivationError::StaleState { .. }) => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
#[path = "coordinator/tests.rs"]
mod tests;
