//! Execution helpers that return a verified connection only after the driver
//! future has released its owned view, allowing an acknowledged rollback.

use std::future::Future;

use sf_core::{Term, Triple};
use sf_sparql::{exec_pg, Plan};

use super::{PgGenerationError, RequestBudget, VerifiedPostgresGenerationLease};

impl VerifiedPostgresGenerationLease {
    pub(crate) async fn select_each<F, Fut>(
        self,
        plan: &Plan,
        control: &RequestBudget,
        sink: F,
    ) -> Result<sf_sparql::Result<()>, PgGenerationError>
    where
        F: FnMut(Vec<Option<Term>>) -> Fut + Send,
        Fut: Future<Output = sf_sparql::Result<()>> + Send,
    {
        let result =
            exec_pg::select_each_pg_controlled(plan, self.execution_client(), control, sink).await;
        self.finish_with_budget(control).await?;
        Ok(result)
    }

    pub(crate) async fn construct_each<F, Fut>(
        self,
        plan: &Plan,
        control: &RequestBudget,
        sink: F,
    ) -> Result<sf_sparql::Result<()>, PgGenerationError>
    where
        F: FnMut(Vec<Triple>) -> Fut + Send,
        Fut: Future<Output = sf_sparql::Result<()>> + Send,
    {
        let result =
            exec_pg::construct_each_pg_controlled(plan, self.execution_client(), control, sink)
                .await;
        self.finish_with_budget(control).await?;
        Ok(result)
    }

    pub(crate) async fn ask(
        self,
        plan: &Plan,
        control: &RequestBudget,
    ) -> Result<sf_sparql::Result<bool>, PgGenerationError> {
        let result = exec_pg::ask_pg_controlled(plan, self.execution_client(), control).await;
        self.finish_with_budget(control).await?;
        Ok(result)
    }
}
