//! Verified source transport policy. No certificate or hostname bypass exists.

use std::net::IpAddr;
use std::sync::Arc;
use tokio_postgres::config::{Host, SslMode};

pub(crate) fn parse_roots(
    pem: &str,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, &'static str> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    if pem.len() > 64 * 1024 {
        return Err("source trust bundle exceeds the byte limit");
    }
    let roots: Vec<_> = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|_| "invalid source trust bundle")?;
    if roots.is_empty() || roots.len() > 64 {
        return Err("source trust certificate count is invalid");
    }
    client_config(Some(&roots))?;
    Ok(roots)
}

pub(crate) fn client_config(
    certificates: Option<&[rustls::pki_types::CertificateDer<'static>]>,
) -> Result<rustls::ClientConfig, &'static str> {
    let mut roots = rustls::RootCertStore::empty();
    if let Some(certificates) = certificates {
        for certificate in certificates {
            roots
                .add(certificate.clone())
                .map_err(|_| "invalid source trust certificate")?;
        }
        if roots.is_empty() {
            return Err("source trust bundle is empty");
        }
    } else {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "source TLS versions are unavailable")?
    .with_root_certificates(roots)
    .with_no_client_auth())
}

/// Plaintext is confined to Unix sockets or literal loopback addresses. DNS
/// names (including localhost) are not proof of a local destination.
pub(crate) fn postgres(config: &mut tokio_postgres::Config) -> Result<(), &'static str> {
    let local_hosts = config.get_hosts().iter().all(|host| match host {
        Host::Tcp(name) => name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
        #[cfg(unix)]
        Host::Unix(_) => true,
    });
    let local_addresses = config.get_hostaddrs().iter().all(IpAddr::is_loopback);
    let local = local_hosts && local_addresses;
    if config.get_ssl_mode() == SslMode::Disable && !local {
        return Err("remote PostgreSQL sources require verified TLS");
    }
    if !local || config.get_ssl_mode() == SslMode::Require {
        #[cfg(unix)]
        if config
            .get_hosts()
            .iter()
            .any(|host| matches!(host, Host::Unix(_)))
        {
            return Err("TLS requires a TCP source identity");
        }
        // A connector is not enough: Prefer permits server refusal/downgrade.
        config.ssl_mode(SslMode::Require);
    } else {
        config.ssl_mode(SslMode::Disable);
    }
    Ok(())
}

pub(crate) fn mysql(options: mysql_async::Opts) -> Result<mysql_async::Opts, &'static str> {
    // mysql_async uses implicit Rustls builders; both Cargo crypto providers may
    // be enabled. Respect an embedding's default, otherwise explicitly select ring.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let hostname = options.ip_or_hostname();
    let hostname = hostname
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(hostname);
    let local = hostname.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    let ssl = options.ssl_opts();
    if options.socket().is_some()
        || ssl.is_some_and(|ssl| {
            ssl.accept_invalid_certs()
                || ssl.skip_domain_validation()
                || ssl.tls_hostname_override().is_some()
        })
    {
        return Err("unsafe MySQL TLS configuration");
    }
    let ssl = if local {
        ssl.cloned()
    } else {
        Some(ssl.cloned().unwrap_or_default())
    };
    // This also selects Tokio DNS instead of the URL driver's synchronous lookup.
    let hostname = hostname.to_owned();
    Ok(mysql_async::OptsBuilder::from_opts(options)
        .ip_or_hostname(hostname)
        .ssl_opts(ssl)
        .prefer_socket(false)
        .into())
}

#[cfg(test)]
#[path = "source_tls_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "source_tls_peer_tests.rs"]
mod peer_tests;

#[cfg(test)]
#[path = "source_tls_mysql_tests.rs"]
mod mysql_peer_tests;
