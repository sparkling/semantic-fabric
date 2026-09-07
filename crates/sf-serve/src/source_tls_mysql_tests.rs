//! Driver-level TLS tests run in a fresh process to test crypto initialization.
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn send(stream: &mut (impl AsyncWriteExt + Unpin), sequence: u8, data: &[u8]) {
    let size = data.len() as u32;
    stream
        .write_all(&[size as u8, (size >> 8) as u8, (size >> 16) as u8, sequence])
        .await
        .unwrap();
    stream.write_all(data).await.unwrap();
}

async fn receive(stream: &mut (impl AsyncReadExt + Unpin)) -> Vec<u8> {
    let mut header = [0; 4];
    stream.read_exact(&mut header).await.unwrap();
    let size = u32::from_le_bytes([header[0], header[1], header[2], 0]);
    assert!(size <= 8192);
    let mut data = vec![0; size as usize];
    stream.read_exact(&mut data).await.unwrap();
    data
}

fn greeting(ssl: bool) -> Vec<u8> {
    // Protocol 4.1, secure connection and native-password authentication.
    let capabilities: u32 = 0x0008_8201 | if ssl { 0x800 } else { 0 };
    let mut packet = b"\x0a8.0.36\0".to_vec();
    packet.extend_from_slice(&23u32.to_le_bytes());
    packet.extend_from_slice(b"12345678\0");
    packet.extend_from_slice(&(capabilities as u16).to_le_bytes());
    packet.push(45);
    packet.extend_from_slice(&2u16.to_le_bytes());
    packet.extend_from_slice(&((capabilities >> 16) as u16).to_le_bytes());
    packet.push(21);
    packet.extend_from_slice(&[0; 10]);
    packet.extend_from_slice(b"123456789012\0mysql_native_password\0");
    packet
}

async fn cases() {
    use mysql_async::prelude::Queryable;
    for (name, trusted, ssl) in [
        ("127.0.0.1", true, true),
        ("wrong.example", true, true),
        ("127.0.0.1", false, true),
        ("127.0.0.1", true, false),
    ] {
        tokio::time::timeout(Duration::from_secs(3), async {
            let (certificate, acceptor) = super::peer_tests::identity(name);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let succeeds = name == "127.0.0.1" && trusted && ssl;
            let peer = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                send(&mut socket, 0, &greeting(ssl)).await;
                if !ssl {
                    let mut byte = [0];
                    assert_eq!(
                        socket.read(&mut byte).await.unwrap(),
                        0,
                        "no credentials before TLS"
                    );
                    return;
                }
                let request = receive(&mut socket).await;
                assert_eq!(request.len(), 32, "only SSLRequest may be plaintext");
                let connection = acceptor.accept(socket).await;
                if !succeeds {
                    assert!(connection.is_err());
                    return;
                }
                let mut stream = connection.unwrap();
                let authenticated = receive(&mut stream).await;
                assert!(authenticated.windows(5).any(|bytes| bytes == b"test\0"));
                send(&mut stream, 3, &[0, 0, 0, 2, 0, 0, 0]).await;
                let query = receive(&mut stream).await;
                assert_eq!(query, b"\x03DO 1");
                send(&mut stream, 1, &[0, 0, 0, 2, 0, 0, 0]).await;
                let _ = receive(&mut stream).await; // COM_QUIT
            });
            let root = if trusted {
                certificate
            } else {
                super::peer_tests::identity("other.example").0
            };
            let ssl_opts = mysql_async::SslOpts::default()
                .with_disable_built_in_roots(true)
                .with_root_certs(vec![root.as_ref().to_vec().into()]);
            let options = mysql_async::OptsBuilder::default()
                .ip_or_hostname("127.0.0.1")
                .tcp_port(address.port())
                .user(Some("test"))
                .pass(Some("test-only-password"))
                .ssl_opts(Some(ssl_opts))
                .max_allowed_packet(Some(1024))
                .wait_timeout(Some(30))
                .setup(Vec::<String>::new());
            let result = mysql_async::Conn::new(super::mysql(options.into()).unwrap()).await;
            if succeeds {
                let mut connection = result.expect("trusted MySQL TLS connection");
                connection.query_drop("DO 1").await.unwrap();
                connection.disconnect().await.unwrap();
            } else {
                assert!(result.is_err());
            }
            peer.await.unwrap();
        })
        .await
        .unwrap();
    }
}

#[test]
fn mysql_verified_tls_in_fresh_process() {
    const CHILD: &str = "SF_TLS_TEST_PROCESS";
    if std::env::var_os(CHILD).is_some() {
        assert!(rustls::crypto::CryptoProvider::get_default().is_none());
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(cases());
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "source_tls::mysql_peer_tests::mysql_verified_tls_in_fresh_process",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, "1")
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("MySQL TLS child exceeded test bound");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
