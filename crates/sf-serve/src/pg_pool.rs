//! PostgreSQL pool construction shared by serving and lifecycle control lanes.

use std::time::Duration;

use crate::source::POSTGRES_RELATION_SCOPE_RECYCLE_SQL;

pub(crate) fn build(
    config: tokio_postgres::Config,
    maximum: usize,
    wait_timeout: Duration,
) -> Result<deadpool_postgres::Pool, deadpool_postgres::BuildError> {
    let manager = deadpool_postgres::Manager::from_config(
        config,
        tokio_postgres::NoTls,
        deadpool_postgres::ManagerConfig {
            recycling_method: deadpool_postgres::RecyclingMethod::Custom(
                POSTGRES_RELATION_SCOPE_RECYCLE_SQL.to_owned(),
            ),
        },
    );
    deadpool_postgres::Pool::builder(manager)
        .max_size(maximum)
        .wait_timeout(Some(wait_timeout))
        .runtime(deadpool_postgres::Runtime::Tokio1)
        .build()
}
