//! Immutable, physically distinct PostgreSQL serving and control pools.

use std::time::Duration;

use crate::source::POSTGRES_RELATION_SCOPE_OPTIONS;

pub(super) const CONTROL_POOL_SIZE: usize = 1;

/// Two pools created independently from one consumed resolved configuration.
///
/// Cloning either accessor preserves that lane's identity; construction never
/// derives the control lane by cloning the request pool.
#[derive(Clone)]
pub(crate) struct PgDirectPools {
    request: crate::PostgresPool,
    control: crate::PostgresPool,
}

impl PgDirectPools {
    #[cfg(test)]
    pub(crate) fn from_resolved_config(
        config: tokio_postgres::Config,
        request_maximum: usize,
        wait_timeout: Duration,
    ) -> Result<Self, PgDirectPoolError> {
        Self::with_tls(
            config,
            request_maximum,
            wait_timeout,
            crate::source_tls::client_config(None).expect("built-in roots"),
        )
    }

    pub(crate) fn with_tls(
        config: tokio_postgres::Config,
        request_maximum: usize,
        wait_timeout: Duration,
        tls: rustls::ClientConfig,
    ) -> Result<Self, PgDirectPoolError> {
        if request_maximum == 0
            || wait_timeout.is_zero()
            || config.get_options() != Some(POSTGRES_RELATION_SCOPE_OPTIONS)
        {
            return Err(PgDirectPoolError::InvalidConfiguration);
        }
        let request = crate::pg_pool::build_with_tls(
            config.clone(),
            request_maximum,
            wait_timeout,
            tls.clone(),
        )
        .map_err(|_| PgDirectPoolError::PoolBuild)?;
        let control = crate::pg_pool::build_with_tls(config, CONTROL_POOL_SIZE, wait_timeout, tls)
            .map_err(|_| PgDirectPoolError::PoolBuild)?;
        Ok(Self { request, control })
    }

    pub(crate) fn request(&self) -> crate::PostgresPool {
        self.request.clone()
    }

    pub(crate) fn control(&self) -> crate::PostgresPool {
        self.control.clone()
    }

    #[cfg(test)]
    pub(crate) fn capacities(&self) -> (usize, usize) {
        (
            self.request.status().max_size,
            self.control.status().max_size,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PgDirectPoolError {
    #[error("PostgreSQL Direct-Mapping pool configuration is unavailable")]
    InvalidConfiguration,
    #[error("PostgreSQL Direct-Mapping pools are unavailable")]
    PoolBuild,
}

#[cfg(test)]
#[path = "pools/tests.rs"]
mod tests;
