//! Transaction state stays inside the cancellation/discard owner.
use super::*;
use crate::mysql_generation::{schema, session};
use crate::pg_generation::PgGenerationError;
use sf_sql::source_work::SourceWork;
use std::sync::Arc;

impl MysqlQuery {
    pub(crate) async fn acquire_generation(
        conn: Conn,
        budget: RequestBudget,
        tables: Arc<[String]>,
        expected: Option<&Arc<schema::Schema>>,
    ) -> std::result::Result<(Self, Arc<schema::Schema>), PgGenerationError> {
        let work = SourceWork::new(Some(&budget));
        let mut query = Self::acquire_inner(conn, budget.clone(), true)
            .await
            .map_err(error)?;
        let observed = budget
            .run(async {
                session::setup(query.target.conn()).await?;
                query
                    .target
                    .conn()
                    .query_drop("START TRANSACTION READ ONLY")
                    .await
                    .map_err(|_| PgGenerationError::SourceUnavailable)?;
                session::transaction(query.target.conn(), true)?;
                if tables.is_empty()
                    || tables.len() > 256
                    || tables.iter().any(|name| !schema::identifier(name))
                {
                    return Err(PgGenerationError::CapabilityDrift);
                }
                let size = tables.iter().map(|name| name.len() + 64).sum();
                work.charge(size).map_err(schema::sql_error)?;
                let mut sql = String::with_capacity(size);
                for (i, name) in tables.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(" UNION ALL ");
                    }
                    use std::fmt::Write;
                    write!(&mut sql, "SELECT 1 FROM `{name}` WHERE FALSE").unwrap();
                }
                // A single global table list acquires every MDL before execution.
                // Never insert a catalogue/data read between START and this barrier.
                query
                    .target
                    .conn()
                    .query_drop(sql)
                    .await
                    .map_err(|_| PgGenerationError::SourceUnavailable)?;
                let captured = schema::capture(query.target.conn(), &tables, work).await?;
                if let Some(expected) = expected {
                    if !captured.matches(expected, work)? {
                        return Err(PgGenerationError::SchemaDrift);
                    }
                }
                Ok::<_, PgGenerationError>(Arc::new(captured))
            })
            .await??;
        query.generation = Some((tables, observed.clone()));
        Ok((query, observed))
    }

    pub(super) async fn close_generation(&mut self) -> Result<()> {
        let Some((names, expected)) = self.generation.clone() else {
            return Ok(());
        };
        let budget = self.budget.clone();
        let work = SourceWork::new(Some(&budget));
        let captured = schema::capture(self.target.conn(), &names, work)
            .await
            .map_err(execution_error)?;
        session::transaction(self.target.conn(), true).map_err(execution_error)?;
        if !captured.matches(&expected, work).map_err(execution_error)? {
            return Err(failure());
        }
        self.target
            .conn()
            .query_drop("ROLLBACK")
            .await
            .map_err(|_| failure())?;
        session::transaction(self.target.conn(), false).map_err(execution_error)?;
        self.generation = None;
        Ok(())
    }
}

pub(crate) fn error(error: Error) -> PgGenerationError {
    match error {
        Error::QueryControl(error) => PgGenerationError::Control(error),
        _ => PgGenerationError::SourceUnavailable,
    }
}

fn execution_error(error: PgGenerationError) -> Error {
    match error {
        PgGenerationError::Control(error) => Error::QueryControl(error),
        _ => failure(),
    }
}
