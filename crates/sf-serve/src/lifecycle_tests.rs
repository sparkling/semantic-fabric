//! End-to-end listener lifecycle and capacity-release checks.

use std::sync::Arc;
use std::time::Duration;

use sf_sparql::Tbox;
use tokio::io::AsyncWriteExt;
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

#[tokio::test]
async fn shutdown_cancels_pending_ingress_and_releases_capacity_within_bound() {
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
    assert!(matches!(
        outcome,
        ShutdownOutcome::Drained | ShutdownOutcome::Forced
    ));
    assert_eq!(config.available_request_permits(), 1);
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
