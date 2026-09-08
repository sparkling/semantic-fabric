//! Signal-driven, bounded server shutdown.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::problem::StartupCause;
use crate::{RequestDeadlineService, ServeConfig, ServeError};

/// Normal drain window. Expiry signals every admitted request to cancel.
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
/// The native MySQL stop allowance is two seconds (PostgreSQL one). Keep the
/// runtime alive for those owned attempts plus bounded scheduling/teardown.
const FORCED_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShutdownPhase {
    Running,
    Draining,
    Forced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShutdownOutcome {
    Drained,
    Forced,
}

pub(crate) fn validate_request_timeout(timeout: Duration) -> Result<(), ServeError> {
    if tokio::time::Instant::now().checked_add(timeout).is_none() {
        return Err(ServeError::new(StartupCause::Configuration {
            error: "request timeout exceeds the monotonic clock range".to_owned(),
        }));
    }
    Ok(())
}

pub(crate) fn validate_shutdown_timeout(timeout: Duration) -> Result<(), ServeError> {
    if timeout.is_zero() || tokio::time::Instant::now().checked_add(timeout).is_none() {
        return Err(ServeError::new(StartupCause::Configuration {
            error: "shutdown timeout must be positive and fit the monotonic clock range".to_owned(),
        }));
    }
    Ok(())
}

pub(crate) async fn serve(
    bind: &str,
    app: RequestDeadlineService,
    config: Arc<ServeConfig>,
    drain_timeout: Duration,
) -> Result<(), ServeError> {
    serve_with_background(bind, app, config, drain_timeout, std::future::ready(Ok(()))).await
}

pub(crate) async fn serve_with_background<B>(
    bind: &str,
    app: RequestDeadlineService,
    config: Arc<ServeConfig>,
    drain_timeout: Duration,
    background: B,
) -> Result<(), ServeError>
where
    B: Future<Output = Result<(), std::io::Error>> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(bind).await.map_err(|error| {
        ServeError::new(StartupCause::Bind {
            bind: bind.to_owned(),
            error: error.to_string(),
        })
    })?;
    let addr = listener.local_addr().map_err(|error| {
        ServeError::new(StartupCause::Server {
            error: error.to_string(),
        })
    })?;
    println!("semantic-fabric: SPARQL 1.2 endpoint listening on http://{addr}/sparql");

    let outcome = serve_listener_with_background(
        listener,
        app,
        config,
        production_shutdown_signal(),
        drain_timeout,
        background,
    )
    .await
    .map_err(|error| {
        ServeError::new(StartupCause::Server {
            error: error.to_string(),
        })
    })?;
    if outcome == ShutdownOutcome::Forced {
        eprintln!("semantic-fabric: shutdown drain bound reached; active requests cancelled");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) async fn serve_listener_until_shutdown<F>(
    listener: tokio::net::TcpListener,
    app: RequestDeadlineService,
    config: Arc<ServeConfig>,
    shutdown: F,
    drain_timeout: Duration,
) -> Result<ShutdownOutcome, std::io::Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    serve_listener_with_background(
        listener,
        app,
        config,
        shutdown,
        drain_timeout,
        std::future::ready(Ok(())),
    )
    .await
}

pub(crate) async fn serve_listener_with_background<F, B>(
    listener: tokio::net::TcpListener,
    app: RequestDeadlineService,
    config: Arc<ServeConfig>,
    shutdown: F,
    drain_timeout: Duration,
    background: B,
) -> Result<ShutdownOutcome, std::io::Error>
where
    F: Future<Output = ()> + Send + 'static,
    B: Future<Output = Result<(), std::io::Error>> + Send + 'static,
{
    let (started_tx, started_rx) = oneshot::channel();
    let force_config = config.clone();
    let graceful_signal = async move {
        shutdown.await;
        let started = tokio::time::Instant::now();
        config.begin_shutdown();
        let _ = started_tx.send(started);
    };
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(graceful_signal);
    // Retain the background owner independently when the HTTP drain is forced.
    tokio::pin!(background);
    let mut background_done = false;
    let graceful = async {
        tokio::try_join!(server.into_future(), async {
            let result = background.as_mut().await;
            background_done = true;
            result
        })?;
        // HTTP drain does not imply detached producers/native cleanup ended.
        // They already retain the request admission identity until ownership ends.
        await_owned_work(&force_config).await;
        Ok(())
    };
    let outcome = finish_with_bound(graceful, started_rx, drain_timeout).await;
    if !matches!(outcome, Ok(ShutdownOutcome::Drained)) {
        // An early background/server failure must not bypass owned cleanup.
        force_config.begin_shutdown();
        force_config.force_shutdown();
        tokio::time::timeout(FORCED_CLEANUP_TIMEOUT, async {
            let result = if background_done {
                Ok(())
            } else {
                background.as_mut().await
            };
            await_owned_work(&force_config).await;
            result
        })
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "owned request cleanup exceeded shutdown allowance",
            )
        })??;
    }
    outcome
}

async fn await_owned_work(config: &ServeConfig) {
    let permits = config.request_admission_permits();
    // No new source work is admissible after shutdown. Count the existing gate
    // without narrowing its validated usize capacity to acquire_many's u32.
    while permits.available_permits() != config.max_concurrent_requests()
        || config
            .control_work
            .as_ref()
            .is_some_and(|gate| gate.available_permits() != 1)
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn finish_with_bound<F>(
    server: F,
    shutdown_started: oneshot::Receiver<tokio::time::Instant>,
    drain_timeout: Duration,
) -> Result<ShutdownOutcome, std::io::Error>
where
    F: Future<Output = Result<(), std::io::Error>>,
{
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => result.map(|()| ShutdownOutcome::Drained),
        started = shutdown_started => {
            let deadline = started.unwrap_or_else(|_| tokio::time::Instant::now())
                .checked_add(drain_timeout)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput,
                    "shutdown deadline is not representable"))?;
            match tokio::time::timeout_at(deadline, &mut server).await {
                Ok(result) => result.map(|()| ShutdownOutcome::Drained),
                Err(_) => Ok(ShutdownOutcome::Forced),
            }
        }
    }
}

async fn production_shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_non_draining_server_is_forced_at_the_exact_bound() {
        let (started_tx, started_rx) = oneshot::channel();
        let task = tokio::spawn(finish_with_bound(
            std::future::pending::<Result<(), std::io::Error>>(),
            started_rx,
            Duration::from_secs(5),
        ));
        started_tx
            .send(tokio::time::Instant::now())
            .expect("announce shutdown");
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(task.await.unwrap().unwrap(), ShutdownOutcome::Forced);
    }

    #[test]
    fn shutdown_bound_must_be_positive_and_representable() {
        for timeout in [Duration::ZERO, Duration::MAX] {
            let error = validate_shutdown_timeout(timeout).unwrap_err();
            assert_eq!(error.code(), "startup-configuration");
        }
        validate_shutdown_timeout(DEFAULT_SHUTDOWN_TIMEOUT).unwrap();
    }
}
