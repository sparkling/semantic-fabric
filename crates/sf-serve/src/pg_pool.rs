//! PostgreSQL pool construction shared by serving and lifecycle control lanes.

use std::time::Duration;

use crate::source::POSTGRES_RELATION_SCOPE_RECYCLE_SQL;

/// A PostgreSQL pool carrying the same immutable TLS policy for queries and
/// cancellation. Cloning preserves both pool identity and trust configuration.
#[derive(Clone)]
pub struct PostgresPool {
    inner: deadpool_postgres::Pool,
    pub(crate) tls: tokio_postgres_rustls::MakeRustlsConnect,
}

impl PostgresPool {
    /// Wrap a caller-built pool with its matching verified TLS configuration.
    pub fn with_tls(inner: deadpool_postgres::Pool, config: rustls::ClientConfig) -> Self {
        Self {
            inner,
            tls: tokio_postgres_rustls::MakeRustlsConnect::new(config),
        }
    }
}

impl std::ops::Deref for PostgresPool {
    type Target = deadpool_postgres::Pool;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl From<deadpool_postgres::Pool> for PostgresPool {
    /// Legacy caller-built pools use public roots for cancellation. A pool with
    /// private trust roots must use `with_tls` so cancellation retains them.
    fn from(inner: deadpool_postgres::Pool) -> Self {
        Self::with_tls(
            inner,
            crate::source_tls::client_config(None).expect("built-in trust roots"),
        )
    }
}

pub(crate) fn build(
    config: tokio_postgres::Config,
    maximum: usize,
    wait_timeout: Duration,
) -> Result<PostgresPool, &'static str> {
    build_with_tls(
        config,
        maximum,
        wait_timeout,
        crate::source_tls::client_config(None).expect("built-in trust roots"),
    )
}

pub(crate) fn build_with_tls(
    mut config: tokio_postgres::Config,
    maximum: usize,
    wait_timeout: Duration,
    tls: rustls::ClientConfig,
) -> Result<PostgresPool, &'static str> {
    if maximum == 0 || wait_timeout.is_zero() {
        return Err("PostgreSQL pool bounds must be positive");
    }
    crate::source_tls::postgres(&mut config)?;
    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls);
    let manager = deadpool_postgres::Manager::from_config(
        config,
        tls.clone(),
        deadpool_postgres::ManagerConfig {
            recycling_method: deadpool_postgres::RecyclingMethod::Custom(
                POSTGRES_RELATION_SCOPE_RECYCLE_SQL.to_owned(),
            ),
        },
    );
    let inner = deadpool_postgres::Pool::builder(manager)
        .max_size(maximum)
        .wait_timeout(Some(wait_timeout))
        .create_timeout(Some(wait_timeout))
        .recycle_timeout(Some(wait_timeout))
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .build()
        .map_err(|_| "PostgreSQL pool construction failed")?;
    Ok(PostgresPool { inner, tls })
}
