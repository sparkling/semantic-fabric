//! End-to-end listener lifecycle and capacity-release checks.

use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::{QueryControl, QueryControlError};
use sf_sparql::Tbox;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

use crate::lifecycle::{serve_listener_until_shutdown, ShutdownOutcome};
use crate::{router, Backend, ServeConfig};

fn config() -> Arc<ServeConfig> {
    let mut config = ServeConfig::new_unchecked(
        Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
        Vec::new(),
        Tbox::default(),
        Vec::new(),
    );
    config.timeout = Duration::from_secs(60);
    config.set_max_concurrent_requests(1).unwrap();
    Arc::new(config)
}

#[test]
fn shutdown_rejects_new_budgets_without_cancelling_existing_ones() {
    let config = config();
    let in_flight = config.request_budget();

    config.begin_shutdown();

    assert_eq!(in_flight.checkpoint(), Ok(()));
    assert_eq!(
        config.request_budget().checkpoint(),
        Err(QueryControlError::Cancelled)
    );
}

#[tokio::test]
async fn request_already_in_flight_drains_successfully_after_shutdown_starts() {
    let config = config();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(serve_listener_until_shutdown(
        listener,
        router(config.clone()),
        config.clone(),
        async move {
            let _ = shutdown_rx.await;
        },
        Duration::from_secs(1),
    ));

    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client
        .write_all(
            b"POST /sparql HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/sparql-query\r\nContent-Length: 6\r\n\r\nAS",
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while config.available_request_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("partial request admitted before shutdown");

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !matches!(
            config.runtime_readiness().unwrap(),
            crate::RuntimeReadiness::NotReady {
                cause: crate::ReadinessCause::Administrative,
                ..
            }
        ) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shutdown transition observed");
    client.write_all(b"K {}").await.unwrap();

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), client.read_to_end(&mut response))
        .await
        .expect("in-flight response drained")
        .unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("\"boolean\":true"), "{response}");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("server completed its drain")
            .unwrap()
            .unwrap(),
        ShutdownOutcome::Drained
    );
    assert_eq!(config.available_request_permits(), 1);
}

#[tokio::test]
async fn shutdown_forces_non_draining_ingress_and_releases_capacity_at_bound() {
    let config = config();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(serve_listener_until_shutdown(
        listener,
        router(config.clone()),
        config.clone(),
        async move {
            let _ = shutdown_rx.await;
        },
        Duration::from_millis(250),
    ));

    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client
        .write_all(
            b"POST /sparql HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/sparql-query\r\nContent-Length: 100\r\n\r\n",
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while config.available_request_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("request admitted before shutdown");

    shutdown_tx.send(()).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .expect("configured shutdown watchdog")
        .unwrap()
        .unwrap();
    assert_eq!(outcome, ShutdownOutcome::Forced);
    tokio::time::timeout(Duration::from_secs(1), async {
        while config.available_request_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("forced connection drop releases request capacity");
    assert!(matches!(
        config.runtime_readiness().unwrap(),
        crate::RuntimeReadiness::NotReady {
            cause: crate::ReadinessCause::Administrative,
            ..
        }
    ));
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    drop(client);
}
