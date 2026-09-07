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
    request: deadpool_postgres::Pool,
    control: deadpool_postgres::Pool,
}

impl PgDirectPools {
    pub(crate) fn from_resolved_config(
        config: tokio_postgres::Config,
        request_maximum: usize,
        wait_timeout: Duration,
    ) -> Result<Self, PgDirectPoolError> {
        if request_maximum == 0
            || wait_timeout.is_zero()
            || config.get_options() != Some(POSTGRES_RELATION_SCOPE_OPTIONS)
        {
            return Err(PgDirectPoolError::InvalidConfiguration);
        }
        let request = crate::pg_pool::build(config.clone(), request_maximum, wait_timeout)
            .map_err(|_| PgDirectPoolError::PoolBuild)?;
        let control = crate::pg_pool::build(config, CONTROL_POOL_SIZE, wait_timeout)
            .map_err(|_| PgDirectPoolError::PoolBuild)?;
        Ok(Self { request, control })
    }

    pub(crate) fn request(&self) -> deadpool_postgres::Pool {
        self.request.clone()
    }

    pub(crate) fn control(&self) -> deadpool_postgres::Pool {
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
