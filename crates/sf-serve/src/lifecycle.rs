//! Signal-driven, bounded server shutdown.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::problem::StartupCause;
use crate::{RequestDeadlineService, ServeConfig, ServeError};

/// Normal drain window. Expiry forcibly drops the serving future and its connections.
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

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

    let outcome = serve_listener_until_shutdown(
        listener,
        app,
        config,
        production_shutdown_signal(),
        drain_timeout,
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
    let (started_tx, started_rx) = oneshot::channel();
    let graceful_signal = async move {
        shutdown.await;
        config.begin_shutdown();
        let _ = started_tx.send(());
    };
    let server =
        axum::serve(listener, app.into_make_service()).with_graceful_shutdown(graceful_signal);
    finish_with_bound(server.into_future(), started_rx, drain_timeout).await
}

async fn finish_with_bound<F>(
    server: F,
    shutdown_started: oneshot::Receiver<()>,
    drain_timeout: Duration,
) -> Result<ShutdownOutcome, std::io::Error>
where
    F: Future<Output = Result<(), std::io::Error>>,
{
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => result.map(|()| ShutdownOutcome::Drained),
        _ = shutdown_started => {
            match tokio::time::timeout(drain_timeout, &mut server).await {
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
        started_tx.send(()).expect("announce shutdown");
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
