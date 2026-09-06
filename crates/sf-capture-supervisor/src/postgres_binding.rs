// SPDX-License-Identifier: MIT OR Apache-2.0

use deadpool_postgres::{Object, Pool};

use crate::{
    postgres_state::{pg, pg_pool, text},
    AuthorityError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrimaryBinding {
    system_identifier: String,
    database_oid: String,
}

impl PrimaryBinding {
    pub(crate) async fn from_pools(
        writer_pool: &Pool,
        recovery_pool: &Pool,
    ) -> Result<Self, AuthorityError> {
        let writer = writer_pool.get().await.map_err(pg_pool)?;
        let recovery = recovery_pool.get().await.map_err(pg_pool)?;
        let writer_binding = Self::read(&writer).await?;
        let recovery_binding = Self::read(&recovery).await?;
        if writer_binding != recovery_binding {
            return Err(AuthorityError::Postgres(
                "writer/recovery pool binding mismatch",
            ));
        }
        Ok(writer_binding)
    }

    pub(crate) async fn verify(&self, client: &Object) -> Result<(), AuthorityError> {
        if &Self::read(client).await? != self {
            return Err(AuthorityError::Postgres("database primary binding drift"));
        }
        Ok(())
    }

    async fn read(client: &Object) -> Result<Self, AuthorityError> {
        let row = client
            .query_one(
                "SELECT control.system_identifier::text, database.oid::text, \
                 pg_is_in_recovery() FROM pg_control_system() AS control \
                 CROSS JOIN pg_database AS database WHERE database.datname = current_database()",
                &[],
            )
            .await
            .map_err(pg)?;
        let in_recovery: bool = row.try_get(2).map_err(pg)?;
        if in_recovery {
            return Err(AuthorityError::Postgres("writable primary required"));
        }
        Ok(Self {
            system_identifier: text(&row, 0)?.to_owned(),
            database_oid: text(&row, 1)?.to_owned(),
        })
    }
}
