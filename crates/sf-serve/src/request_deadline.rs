//! True outer service boundary for the request-scoped absolute deadline.

use std::convert::Infallible;
use std::future::{ready, Future, Ready};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use axum::Router;
use sf_core::query_control::QueryControl;
use tokio::sync::TryAcquireError;
use tower::Service;

use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::problem;

/// Service that mints the request budget before dispatching into Axum routing.
///
/// Hyper has already parsed the request target by the time it calls this service.
/// Axum path/method matching, fallbacks, extraction, and handler work all happen
/// inside the one absolute deadline created here. The same boundary fail-fast
/// admits active application work before the Router or request body is polled;
/// overload never creates another internal waiter.
#[derive(Clone)]
pub struct RequestDeadlineService {
    inner: Router,
    cfg: Arc<ServeConfig>,
}

impl RequestDeadlineService {
    pub(crate) fn new(inner: Router, cfg: Arc<ServeConfig>) -> Self {
        Self { inner, cfg }
    }

    /// Adapt this request service for production use with [`axum::serve`].
    pub fn into_make_service(self) -> RequestDeadlineMakeService {
        RequestDeadlineMakeService { service: self }
    }
}

impl Service<Request<Body>> for RequestDeadlineService {
    type Response = Response;
    type Error = Infallible;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Aggregate request admission deliberately sheds in `call`. Returning
        // Pending here would let Hyper/Tower build an unbounded external queue.
        <Router as Service<Request<Body>>>::poll_ready(&mut self.inner, cx)
    }

    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        let mut budget = RequestBudget::after(self.cfg.timeout, self.cfg.query_limits);
        if let Err(error) = budget.checkpoint() {
            return deadline_checked_response(budget, problem::response_for_control(error));
        }

        let permit = match self.cfg.request_admission_permits().try_acquire_owned() {
            Ok(permit) => permit,
            Err(TryAcquireError::NoPermits) => {
                return deadline_checked_response(
                    budget,
                    problem::response_with_retry_after(problem::ProblemCode::ServiceOverloaded),
                )
            }
            Err(TryAcquireError::Closed) => {
                return deadline_checked_response(
                    budget,
                    problem::response(problem::ProblemCode::Internal),
                )
            }
        };
        if let Err(error) = budget.checkpoint() {
            drop(permit);
            return deadline_checked_response(budget, problem::response_for_control(error));
        }
        if budget.retain_admission(permit).is_err() {
            return deadline_checked_response(
                budget,
                problem::response(problem::ProblemCode::Internal),
            );
        }
        request.extensions_mut().insert(budget.clone());

        // Move the ready instance into the future while retaining a clone for the
        // next request. This preserves tower's poll_ready/call contract.
        let replacement = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, replacement);

        Box::pin(async move {
            let mut cancellation = budget.cancellation_guard();
            let inner_response = async move { inner.call(request).await };
            let response = match budget.run_until_deadline(inner_response).await {
                Ok(Ok(response)) => response,
                Ok(Err(never)) => match never {},
                Err(error) => problem::response_for_control(error),
            };
            cancellation.disarm();
            Ok(response)
        })
    }
}

fn deadline_checked_response(budget: RequestBudget, response: Response) -> ResponseFuture {
    Box::pin(async move {
        let response = match budget.run_until_deadline(async move { response }).await {
            Ok(response) => response,
            Err(error) => problem::response_for_control(error),
        };
        Ok(response)
    })
}

type ResponseFuture = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send + 'static>>;

/// Clone-per-connection adapter used by [`axum::serve`].
#[derive(Clone)]
pub struct RequestDeadlineMakeService {
    service: RequestDeadlineService,
}

impl<T> Service<T> for RequestDeadlineMakeService {
    type Response = RequestDeadlineService;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _target: T) -> Self::Future {
        ready(Ok(self.service.clone()))
    }
}
