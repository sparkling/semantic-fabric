//! End-to-end listener lifecycle and capacity-release checks.

use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::{QueryControl, QueryControlError};
use sf_core::{SourceId, SourceMapping};
use sf_sparql::Epoch;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

use crate::lifecycle::{serve_listener_until_shutdown, ShutdownOutcome};
use crate::{
    router, ActivationError, Backend, IntrospectedSource, ReadinessCause, RuntimeReadiness,
    RuntimeSnapshot, RuntimeSource, ServeConfig,
};

fn config() -> Arc<ServeConfig> {
    let mut config = ServeConfig::new_with_unverified_source(
        Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
        Vec::new(),
        crate::test_support::empty_ontology(),
        Vec::new(),
    )
    .unwrap();
    config.timeout = Duration::from_secs(60);
    config.set_max_concurrent_requests(1).unwrap();
    Arc::new(config)
}

fn candidate() -> RuntimeSnapshot {
    RuntimeSnapshot::single(
        Epoch(1),
        crate::test_support::empty_ontology(),
        RuntimeSource::new(
            IntrospectedSource::unchecked(
                Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
                Vec::new(),
            ),
            SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
        ),
    )
    .unwrap()
}

#[test]
fn shutdown_advances_one_administrative_revision_and_fences_later_transitions() {
    let config = config();
    let ready = config.runtime_readiness().unwrap();
    let drifted = config
        .mark_runtime_not_ready(ready, ReadinessCause::SchemaDrift)
        .unwrap();

    config.begin_shutdown();
    let shutdown = config.runtime_readiness().unwrap();
    assert_eq!(shutdown.activation_id(), ready.activation_id());
    assert!(shutdown.state_revision() > drifted.state_revision());
    assert!(matches!(
        shutdown,
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::Administrative,
            ..
        }
    ));

    config.begin_shutdown();
    assert_eq!(config.runtime_readiness().unwrap(), shutdown);
    assert_eq!(
        config.activate_snapshot(shutdown, candidate()),
        Err(ActivationError::ShuttingDown)
    );
    assert_eq!(
        config.mark_runtime_not_ready(shutdown, ReadinessCause::CapabilityDrift),
        Err(ActivationError::ShuttingDown)
    );
}

#[test]
fn concurrent_activation_and_shutdown_always_finish_administratively_not_ready() {
    let config = config();
    let expected = config.runtime_readiness().unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let activation_config = config.clone();
    let activation_barrier = barrier.clone();
    let activation = std::thread::spawn(move || {
        let candidate = candidate();
        activation_barrier.wait();
        activation_config.activate_snapshot(expected, candidate)
    });
    let shutdown_config = config.clone();
    let shutdown_barrier = barrier.clone();
    let shutdown = std::thread::spawn(move || {
        shutdown_barrier.wait();
        shutdown_config.begin_shutdown();
    });

    barrier.wait();
    let activation = activation.join().unwrap();
    shutdown.join().unwrap();
    let final_state = config.runtime_readiness().unwrap();
    assert!(matches!(
        final_state,
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::Administrative,
            ..
        }
    ));
    assert!(config.runtime_lease().is_err());
    match activation {
        Ok(activation_id) => assert_eq!(final_state.activation_id(), activation_id),
        Err(ActivationError::ShuttingDown) => {}
        Err(error) => panic!("unexpected activation result: {error:?}"),
    }
}

#[test]
fn concurrent_drift_report_and_shutdown_never_surface_a_stale_state_error() {
    let config = config();
    let expected = config.runtime_readiness().unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let reporter_config = config.clone();
    let reporter_barrier = barrier.clone();
    let reporter = std::thread::spawn(move || {
        reporter_barrier.wait();
        reporter_config.mark_runtime_not_ready(expected, ReadinessCause::SchemaDrift)
    });
    let shutdown_config = config.clone();
    let shutdown_barrier = barrier.clone();
    let shutdown = std::thread::spawn(move || {
        shutdown_barrier.wait();
        shutdown_config.begin_shutdown();
    });

    barrier.wait();
    let report = reporter.join().unwrap();
    shutdown.join().unwrap();
    assert!(matches!(
        report,
        Ok(RuntimeReadiness::NotReady {
            cause: ReadinessCause::SchemaDrift,
            ..
        }) | Err(ActivationError::ShuttingDown)
    ));
    assert!(matches!(
        config.runtime_readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::Administrative,
            ..
        }
    ));
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
    tokio::time::timeout(Duration::from_secs(1), async {
        while config.available_request_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("drained response releases request capacity");
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

#[tokio::test(start_paused = true)]
async fn http_drain_waits_for_detached_owned_work_without_cancelling_it() {
    let config = config();
    let mut budget = config.request_budget();
    budget
        .retain_admission(
            config
                .request_admission_permits()
                .try_acquire_owned()
                .unwrap(),
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (signal, received) = oneshot::channel();
    let server = tokio::spawn(serve_listener_until_shutdown(
        listener,
        router(config.clone()),
        config.clone(),
        async move {
            received.await.unwrap();
        },
        Duration::from_secs(5),
    ));
    signal.send(()).unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        !server.is_finished(),
        "HTTP completion must not discard active native cleanup"
    );
    assert_eq!(
        budget.checkpoint(),
        Ok(()),
        "graceful drain must preserve owned work"
    );
    drop(budget);
    assert_eq!(server.await.unwrap().unwrap(), ShutdownOutcome::Drained);
}

#[tokio::test(start_paused = true)]
async fn forced_shutdown_waits_for_cleanup_but_never_waits_unboundedly() {
    for release in [true, false] {
        let config = config();
        let mut budget = config.request_budget();
        budget
            .retain_admission(
                config
                    .request_admission_permits()
                    .try_acquire_owned()
                    .unwrap(),
            )
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (signal, received) = oneshot::channel();
        let server = tokio::spawn(serve_listener_until_shutdown(
            listener,
            router(config.clone()),
            config.clone(),
            async move {
                received.await.unwrap();
            },
            Duration::from_secs(5),
        ));
        signal.send(()).unwrap();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_secs(5)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
        assert!(
            !server.is_finished(),
            "forced signal is not completed cleanup"
        );
        if release {
            drop(budget);
            assert_eq!(server.await.unwrap().unwrap(), ShutdownOutcome::Forced);
        } else {
            tokio::time::advance(Duration::from_secs(3)).await;
            assert_eq!(
                server.await.unwrap().unwrap_err().kind(),
                std::io::ErrorKind::TimedOut
            );
        }
    }
}
