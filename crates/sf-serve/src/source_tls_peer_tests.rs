//! Loopback-only TLS peers, never an external database.
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

const BOUND: Duration = Duration::from_secs(3);

pub(super) fn identity(name: &str) -> (CertificateDer<'static>, TlsAcceptor) {
    let identity = rcgen::generate_simple_self_signed(vec![name.to_owned()]).unwrap();
    let certificate = identity.cert.der().clone();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.clone()],
        PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into(),
    )
    .unwrap();
    (certificate, TlsAcceptor::from(Arc::new(server)))
}

async fn packet(stream: &mut (impl AsyncReadExt + Unpin)) -> Vec<u8> {
    let size = stream.read_u32().await.unwrap();
    assert!((4..=8192).contains(&size));
    let mut bytes = vec![0; size as usize - 4];
    stream.read_exact(&mut bytes).await.unwrap();
    bytes
}

async fn frame(stream: &mut (impl AsyncWriteExt + Unpin), kind: u8, payload: &[u8]) {
    stream.write_u8(kind).await.unwrap();
    stream.write_u32(payload.len() as u32 + 4).await.unwrap();
    stream.write_all(payload).await.unwrap();
}

fn description() -> Vec<u8> {
    let mut data = vec![0, 1];
    data.extend_from_slice(b"search_path\0");
    for part in [
        0u32.to_be_bytes().as_slice(),
        0u16.to_be_bytes().as_slice(),
        25u32.to_be_bytes().as_slice(),
        (-1i16).to_be_bytes().as_slice(),
        (-1i32).to_be_bytes().as_slice(),
        0u16.to_be_bytes().as_slice(),
    ] {
        data.extend_from_slice(part);
    }
    data
}

async fn row(stream: &mut (impl AsyncWriteExt + Unpin)) {
    let value = crate::source::POSTGRES_RELATION_SCOPE_SETTING.as_bytes();
    let mut row = vec![0, 1];
    row.extend_from_slice(&(value.len() as u32).to_be_bytes());
    row.extend_from_slice(value);
    frame(stream, b'D', &row).await;
    frame(stream, b'C', b"SELECT 1\0").await;
}

async fn pg_peer(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    pause: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
) {
    let (mut socket, _) = listener.accept().await.unwrap();
    assert_eq!(packet(&mut socket).await, 80877103u32.to_be_bytes());
    socket.write_all(b"S").await.unwrap();
    let mut stream = acceptor.accept(socket).await.unwrap();
    assert_eq!(&packet(&mut stream).await[..4], &196608u32.to_be_bytes());
    frame(&mut stream, b'R', &0u32.to_be_bytes()).await;
    frame(&mut stream, b'K', &[0, 0, 0, 23, 0, 0, 0, 42]).await;
    frame(&mut stream, b'Z', b"I").await;
    // The production checked-connection boundary uses the extended query protocol.
    loop {
        let kind = stream.read_u8().await.unwrap();
        let _payload = packet(&mut stream).await;
        match kind {
            b'P' => frame(&mut stream, b'1', &[]).await,
            b'D' => {
                frame(&mut stream, b't', &[0, 0]).await;
                frame(&mut stream, b'T', &description()).await;
            }
            b'B' => frame(&mut stream, b'2', &[]).await,
            b'E' => row(&mut stream).await,
            b'S' => {
                frame(&mut stream, b'Z', b"I").await;
            }
            _ => {}
        }
        if kind == b'E' {
            assert_eq!(stream.read_u8().await.unwrap(), b'S');
            let _ = packet(&mut stream).await;
            frame(&mut stream, b'Z', b"I").await;
            break;
        }
    }
    let (mut cancellation, _) = listener.accept().await.unwrap();
    if let Some((ready, release)) = pause {
        ready.send(()).unwrap();
        let _ = release.await;
    }
    assert_eq!(packet(&mut cancellation).await, 80877103u32.to_be_bytes());
    cancellation.write_all(b"S").await.unwrap();
    let mut cancellation = acceptor.accept(cancellation).await.unwrap();
    assert_eq!(
        packet(&mut cancellation).await,
        [4, 210, 22, 46, 0, 0, 0, 23, 0, 0, 0, 42]
    );
}

#[tokio::test]
async fn trusted_query_and_dirty_connection_cancel_share_private_trust() {
    checked_connection_cancel(true).await;
}

#[tokio::test]
async fn ordinary_checked_connection_drop_cancels_before_pool_reuse() {
    checked_connection_cancel(false).await;
}

async fn checked_connection_cancel(mark_generation: bool) {
    tokio::time::timeout(BOUND, async {
        let (certificate, acceptor) = identity("127.0.0.1");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(pg_peer(listener, acceptor, None));
        let mut config = format!(
            "host=127.0.0.1 port={} user=test sslmode=require",
            address.port()
        )
        .parse()
        .unwrap();
        super::postgres(&mut config).unwrap();
        let pool = crate::pg_pool::build_with_tls(
            config,
            1,
            Duration::from_secs(1),
            super::client_config(Some(&[certificate])).unwrap(),
        )
        .unwrap();
        let connection = pool.get().await.unwrap();
        let checked = crate::backend::PgConn::checked(connection, pool.tls.clone())
            .await
            .unwrap();
        if mark_generation {
            checked.mark_generation_dirty();
        }
        drop(checked);
        peer.await.unwrap();
        while pool.status().size != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn native_cancel_retains_request_and_pool_capacity_until_cleanup_ends() {
    tokio::time::timeout(BOUND, async {
        let (certificate, acceptor) = identity("127.0.0.1");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(pg_peer(listener, acceptor, Some((ready_tx, release_rx))));
        let mut config = format!(
            "host=127.0.0.1 port={} user=test sslmode=require",
            address.port()
        )
        .parse()
        .unwrap();
        super::postgres(&mut config).unwrap();
        let pool = crate::pg_pool::build_with_tls(
            config,
            1,
            Duration::from_secs(1),
            super::client_config(Some(&[certificate])).unwrap(),
        )
        .unwrap();
        let admission = Arc::new(tokio::sync::Semaphore::new(1));
        let mut budget = crate::budget::RequestBudget::after(
            BOUND,
            sf_core::query_control::QueryLimits::new(100, 100, 100, 100),
        );
        budget
            .retain_admission(admission.clone().acquire_owned().await.unwrap())
            .unwrap();
        let checked = crate::backend::PgConn::checked_for_request(
            pool.get().await.unwrap(),
            pool.tls.clone(),
            budget,
        )
        .await
        .unwrap();
        drop(checked);
        ready_rx.await.unwrap();
        assert_eq!(admission.available_permits(), 0);
        assert_eq!(pool.status().size, 1);
        assert_eq!(pool.status().available, 0);
        release_tx.send(()).unwrap();
        peer.await.unwrap();
        while pool.status().size != 0 || admission.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn wrong_name_and_untrusted_certificate_never_reach_startup() {
    for (name, trust) in [("wrong.example", true), ("127.0.0.1", false)] {
        tokio::time::timeout(BOUND, async {
            let (certificate, acceptor) = identity(name);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                assert_eq!(packet(&mut socket).await, 80877103u32.to_be_bytes());
                socket.write_all(b"S").await.unwrap();
                assert!(acceptor.accept(socket).await.is_err());
            });
            let roots = if trust {
                vec![certificate]
            } else {
                vec![identity("other.example").0]
            };
            let config = format!(
                "host=127.0.0.1 port={} user=test sslmode=require",
                address.port()
            )
            .parse()
            .unwrap();
            let pool = crate::pg_pool::build_with_tls(
                config,
                1,
                Duration::from_millis(500),
                super::client_config(Some(&roots)).unwrap(),
            )
            .unwrap();
            assert!(pool.get().await.is_err());
            peer.await.unwrap();
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn refusal_and_stalled_handshake_fail_without_plaintext_startup() {
    for refuse in [true, false] {
        tokio::time::timeout(BOUND, async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                assert_eq!(packet(&mut socket).await, 80877103u32.to_be_bytes());
                if refuse {
                    socket.write_all(b"N").await.unwrap();
                }
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            });
            let config = format!(
                "host=127.0.0.1 port={} user=test sslmode=require",
                address.port()
            )
            .parse()
            .unwrap();
            let pool = crate::pg_pool::build(config, 1, Duration::from_millis(100)).unwrap();
            assert!(pool.get().await.is_err());
            assert_eq!(pool.status().size, 0);
            peer.await.unwrap();
        })
        .await
        .unwrap();
    }
}
