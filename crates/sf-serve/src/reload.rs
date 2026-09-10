//! Serialized authored-generation reload. Source/policy configuration stays fixed.

use std::sync::Arc;
use std::time::Duration;

use crate::activation::RuntimeManager;
use crate::problem::StartupCause;
use crate::snapshot::RuntimeSnapshot;
use crate::source::PreparedSource;
use crate::startup_inputs::SemanticInputs;
use crate::{ReadinessCause, RuntimeReadiness, ServeConfig, ServeError, ServeOptions};

const BUILD_TIMEOUT: Duration = Duration::from_secs(60);
mod baseline;
use baseline::Attempt;
pub(crate) use baseline::Baseline;

/// Only this coordinator may construct the authored publication capability.
pub(crate) struct ReloadAuthority(());

/// No raw snapshot can enter this type outside the validated builder below.
pub(crate) struct AuthoredCandidate {
    expected: RuntimeReadiness,
    snapshot: RuntimeSnapshot,
}

impl AuthoredCandidate {
    pub(crate) fn into_parts(self) -> (RuntimeReadiness, RuntimeSnapshot) {
        (self.expected, self.snapshot)
    }
}

pub(crate) fn validate_interval(interval: Duration) -> Result<(), ServeError> {
    if !interval.is_zero()
        && !(Duration::from_secs(1)..=Duration::from_secs(86_400)).contains(&interval)
    {
        return Err(ServeError::new(StartupCause::Configuration {
            error: "reload interval must be zero (disabled) or one second through one day".into(),
        }));
    }
    Ok(())
}

pub(crate) async fn serve_async(
    opts: ServeOptions,
    source: PreparedSource,
    additional: Option<PreparedSource>,
    parser: sf_sparql::ParserRuntime,
) -> Result<(), ServeError> {
    let opts = Arc::new(opts);
    let build_opts = Arc::clone(&opts);
    let build_source = source.clone();
    let build_additional = additional.clone();
    let handle = tokio::runtime::Handle::current();
    // All regular-file capture, parsing and semantic validation run away from
    // request executors. Source I/O retains its existing startup deadlines.
    let (mut config, baseline) = tokio::task::spawn_blocking(move || {
        handle.block_on(crate::startup::build_config(
            &build_opts,
            build_source,
            build_additional,
        ))
    })
    .await
    .map_err(|_| {
        ServeError::new(StartupCause::Runtime {
            error: "startup worker failed".into(),
        })
    })??;
    config.set_parser_runtime(parser);
    if opts.require_verified_generation {
        config.control_work = Some(Arc::new(tokio::sync::Semaphore::new(1)));
    }
    let config = Arc::new(config);
    let supervisor = (!opts.reload_interval.is_zero()).then(|| {
        Supervisor::start(
            Arc::clone(&config),
            Arc::clone(&opts),
            source,
            additional,
            Arc::new(baseline),
        )
    });
    let app = match &opts.metrics {
        Some(metrics) => crate::router_with_metrics(Arc::clone(&config), metrics.clone()),
        None => crate::router(Arc::clone(&config)),
    };
    let result =
        crate::lifecycle::serve(&opts.bind, app, Arc::clone(&config), opts.shutdown_timeout).await;
    drop(supervisor);
    result
}

struct Supervisor {
    config: Arc<ServeConfig>,
    worker: tokio::task::JoinHandle<()>,
}

impl Supervisor {
    fn start(
        config: Arc<ServeConfig>,
        opts: Arc<ServeOptions>,
        source: PreparedSource,
        additional: Option<PreparedSource>,
        mut baseline: Arc<Baseline>,
    ) -> Self {
        let worker_config = Arc::clone(&config);
        let worker = tokio::spawn(async move {
            // Panic, cancellation and unexpected return all fail readiness closed.
            let _guard = WorkerGuard(Arc::clone(&worker_config));
            let runtime = worker_config.lifecycle_runtime();
            let mut ticks = tokio::time::interval_at(
                tokio::time::Instant::now() + opts.reload_interval,
                opts.reload_interval,
            );
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticks.tick().await;
                let Ok(expected) = runtime.readiness() else {
                    break;
                };
                if terminal(expected) {
                    break;
                }
                let generation_budget = if opts.require_verified_generation {
                    match crate::startup_authored::control_budget(Some(&worker_config)) {
                        Ok(budget) => Some(budget),
                        Err(_) => break,
                    }
                } else {
                    None
                };
                if let Some(next) = refresh(
                    Arc::clone(&runtime),
                    Arc::clone(&baseline),
                    Arc::clone(&opts),
                    source.clone(),
                    additional.clone(),
                    expected,
                    generation_budget,
                )
                .await
                {
                    baseline = next;
                }
            }
        });
        Self { config, worker }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.config.begin_shutdown();
        self.worker.abort();
    }
}

struct WorkerGuard(Arc<ServeConfig>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.begin_shutdown();
    }
}

fn terminal(state: RuntimeReadiness) -> bool {
    matches!(
        state,
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::Administrative | ReadinessCause::StateRevisionExhausted,
            ..
        }
    )
}

async fn refresh(
    runtime: Arc<RuntimeManager>,
    baseline: Arc<Baseline>,
    opts: Arc<ServeOptions>,
    source: PreparedSource,
    additional: Option<PreparedSource>,
    expected: RuntimeReadiness,
    generation_budget: Option<crate::budget::RequestBudget>,
) -> Option<Arc<Baseline>> {
    let attempt = Arc::new(Attempt::new(Arc::clone(&runtime), baseline, expected));
    let build_attempt = Arc::clone(&attempt);
    let handle = tokio::runtime::Handle::current();
    let worker = tokio::task::spawn_blocking(move || {
        handle.block_on(async move {
            let inputs = build_attempt.capture(&opts)?;
            let mut observations = Default::default();
            let snapshot = crate::startup::build_snapshot(
                &opts,
                &inputs,
                source,
                additional,
                generation_budget.as_ref(),
                |id, source| build_attempt.observe(&mut observations, id, source),
            )
            .await?;
            if SemanticInputs::capture(&opts)? != inputs {
                return Err(ServeError::new(StartupCause::Configuration {
                    error: "semantic files changed during candidate construction".into(),
                }));
            }
            if let Some(budget) = generation_budget.as_ref() {
                use sf_core::query_control::QueryControl;
                budget.checkpoint().map_err(|_| {
                    ServeError::new(StartupCause::SourceConnect {
                        spec: "PostgreSQL".into(),
                        error: "authored candidate control deadline exceeded".into(),
                    })
                })?;
            }
            let candidate = AuthoredCandidate {
                expected: build_attempt.expected()?,
                snapshot,
            };
            Ok((candidate, Arc::new(Baseline::new(inputs, observations))))
        })
    });
    finish_attempt(&runtime, &attempt, worker, BUILD_TIMEOUT).await
}

type BuildResult = Result<(AuthoredCandidate, Arc<Baseline>), ServeError>;

async fn finish_attempt(
    runtime: &RuntimeManager,
    attempt: &Attempt,
    mut worker: tokio::task::JoinHandle<BuildResult>,
    timeout: Duration,
) -> Option<Arc<Baseline>> {
    let result = match tokio::time::timeout(timeout, &mut worker).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => {
            // No recovery authority survives an abnormal builder termination.
            let _ = runtime.mark_current_administratively_not_ready();
            return None;
        }
        Err(_) => {
            let _ = attempt.fence(ReadinessCause::SourceUnavailable);
            event("timeout");
            // A timed-out synchronous worker is not preemptible. Retain its
            // ownership through actual completion and never queue another one.
            if worker.await.is_err() {
                let _ = runtime.mark_current_administratively_not_ready();
            }
            return None;
        }
    };
    match result {
        Err(error) => {
            let _ = attempt.fence(rejection_cause(&error));
            event("rejected");
        }
        Ok((candidate, baseline)) => {
            // Fresh pools are a new resource identity even for equal semantic
            // bytes. Never retain stale SQLite file handles via a digest no-op.
            if runtime
                .activate_authored(&ReloadAuthority(()), candidate)
                .is_ok()
            {
                event("activated");
                return Some(baseline);
            }
        }
    }
    None
}

pub(crate) fn rejection_cause(error: &ServeError) -> ReadinessCause {
    match error.internal_cause() {
        StartupCause::Generation { cause } => *cause,
        _ if error.code() == "startup-source" => ReadinessCause::SourceUnavailable,
        _ => ReadinessCause::SchemaDrift,
    }
}

fn event(outcome: &'static str) {
    tracing::info!(target: sf_core::TELEMETRY_TARGET, schema = crate::telemetry::SCHEMA, event = "runtime.reload", outcome);
}

#[cfg(test)]
mod tests;
