//! Deadline-governed serving admission for one SQLite pool member.

#[cfg(test)]
use std::future::Future;
#[cfg(test)]
use std::task::Poll;

use axum::response::Response;
use sf_core::query_control::QueryControl;
use sf_sql::backend::sqlite::SqliteOwnedLease;

use crate::backend::SqlitePool;
use crate::budget::RequestBudget;
use crate::problem::{self, ProblemCode};

pub(crate) async fn acquire(
    pool: &SqlitePool,
    budget: &RequestBudget,
) -> Result<SqliteOwnedLease, Response> {
    let handle = pool.pick_owned();

    #[cfg(not(test))]
    let acquired = budget.run(handle.acquire()).await;

    #[cfg(test)]
    let acquired = {
        let mut inner = Box::pin(handle.acquire());
        let mut observed = false;
        let observed_acquire = std::future::poll_fn(|cx| match inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !observed {
                    observed = true;
                    pool.observe_admission_pending();
                }
                Poll::Pending
            }
            Poll::Ready(result) => Poll::Ready(result),
        });
        budget.run(observed_acquire).await
    };

    let lease = match acquired {
        Err(error) => return Err(problem::response_for_control(error)),
        Ok(Err(_)) => return Err(problem::response(ProblemCode::Internal)),
        Ok(Ok(lease)) => lease,
    };
    budget.checkpoint().map_err(problem::response_for_control)?;
    Ok(lease)
}
