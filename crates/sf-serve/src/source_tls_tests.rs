use super::*;

#[test]
fn remote_postgres_cannot_disable_or_downgrade_tls() {
    for host in ["db.example", "localhost", "192.0.2.1"] {
        let mut config: tokio_postgres::Config =
            format!("host={host} sslmode=disable").parse().unwrap();
        assert!(postgres(&mut config).is_err());
        config.ssl_mode(SslMode::Prefer);
        postgres(&mut config).unwrap();
        assert_eq!(config.get_ssl_mode(), SslMode::Require);
    }
    let mut config: tokio_postgres::Config = "host=127.0.0.1 hostaddr=192.0.2.1 sslmode=disable"
        .parse()
        .unwrap();
    assert!(postgres(&mut config).is_err());
}

#[test]
fn literal_loopback_remains_available_for_local_development() {
    let mut config: tokio_postgres::Config = "host=127.0.0.1".parse().unwrap();
    postgres(&mut config).unwrap();
    assert_eq!(config.get_ssl_mode(), SslMode::Disable);
    config.ssl_mode(SslMode::Require);
    postgres(&mut config).unwrap();
    assert_eq!(config.get_ssl_mode(), SslMode::Require);
}

#[test]
fn mysql_requires_verification_and_disables_socket_fallback() {
    let options = mysql(mysql_async::Opts::from_url("mysql://db.example/db").unwrap()).unwrap();
    assert!(options.ssl_opts().is_some());
    assert!(!options.prefer_socket());
    for extra in ["verify_ca=false", "verify_identity=false"] {
        assert!(mysql(
            mysql_async::Opts::from_url(&format!("mysql://db.example/db?require_ssl=true&{extra}"))
                .unwrap()
        )
        .is_err());
    }
    let options =
        mysql(mysql_async::Opts::from_url("mysql://[::1]/db?require_ssl=true").unwrap()).unwrap();
    assert_eq!(options.ip_or_hostname(), "::1");
    assert!(options.ssl_opts().is_some());
}

#[test]
fn private_root_bundles_are_bounded_and_validated() {
    for invalid in [
        String::new(),
        "not a certificate".to_owned(),
        "x".repeat(65537),
    ] {
        assert!(parse_roots(&invalid).is_err());
    }
    let cert = rcgen::generate_simple_self_signed(vec!["db.example".into()])
        .unwrap()
        .cert;
    assert_eq!(parse_roots(&cert.pem()).unwrap().len(), 1);
    assert!(parse_roots(&cert.pem().repeat(65)).is_err());
}

#[tokio::test(start_paused = true)]
async fn mysql_stalled_greeting_is_bounded_by_source_startup() {
    use std::time::Duration;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let source = crate::SourceRef::inline(format!(
        "mysql://127.0.0.1:{}/db?require_ssl=true",
        address.port()
    ))
    .resolve()
    .unwrap()
    .prepare()
    .unwrap();
    let opening = tokio::spawn(crate::run::open_backend(
        source,
        1,
        Duration::from_secs(1),
        1,
    ));
    let (_socket, _) = listener.accept().await.unwrap();
    tokio::time::advance(Duration::from_secs(31)).await;
    let result = opening.await.unwrap();
    assert!(result.is_err());
}
