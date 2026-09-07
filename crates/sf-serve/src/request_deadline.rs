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
use http_body_util::BodyExt;
use sf_core::query_control::QueryControl;
use tokio::sync::TryAcquireError;
use tower::Service;
use tracing::Instrument;

use crate::activation::{RuntimeSnapshotLease, SnapshotUnavailable};
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::problem;
use crate::telemetry::{self, CorrelationId, RequestTrace, Stage};

/// Service that mints the request budget before dispatching into Axum routing.
///
/// Hyper has already parsed the request target by the time it calls this service.
/// Axum path/method matching, fallbacks, extraction, and handler work all happen
/// inside the one absolute deadline created here. The same boundary fail-fast
/// admits active application work before the Router or request body is polled;
/// overload never creates another internal waiter. Fixed health and W3C service-
/// description discovery responses are control metadata and bypass query-work
/// admission, runtime leases, and the query deadline.
#[derive(Clone)]
pub struct RequestDeadlineService {
    inner: Router,
    cfg: Arc<ServeConfig>,
    metrics_enabled: bool,
}

impl RequestDeadlineService {
    pub(crate) fn new(inner: Router, cfg: Arc<ServeConfig>, metrics_enabled: bool) -> Self {
        Self {
            inner,
            cfg,
            metrics_enabled,
        }
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
        let correlation = CorrelationId::generate();
        let trace = RequestTrace::new(&request, correlation.clone());
        request.headers_mut().remove("x-correlation-id");
        if crate::service_description::is_request(&request) {
            let response = trace.in_scope(|| {
                crate::service_description::response(
                    request.headers(),
                    self.cfg.query_mode(),
                    request.method() == axum::http::Method::HEAD,
                )
            });
            return completed_response(trace, response);
        }
        if crate::health::is_health_path(request.uri().path()) {
            let replacement = self.inner.clone();
            let mut inner = std::mem::replace(&mut self.inner, replacement);
            let span = trace.span();
            return Box::pin(
                async move {
                    let response = match inner.call(request).await {
                        Ok(response) => response,
                        Err(never) => match never {},
                    };
                    Ok(trace.complete(response))
                }
                .instrument(span),
            );
        }
        if self.metrics_enabled && crate::metrics::is_path(request.uri().path()) {
            let replacement = self.inner.clone();
            let mut inner = std::mem::replace(&mut self.inner, replacement);
            let span = trace.span();
            return Box::pin(
                async move {
                    let response = match inner.call(request).await {
                        Ok(response) => response,
                        Err(never) => match never {},
                    };
                    Ok(trace.complete(response))
                }
                .instrument(span),
            );
        }

        enum Admission {
            Admitted {
                budget: RequestBudget,
                snapshot: RuntimeSnapshotLease,
            },
            Rejected {
                budget: RequestBudget,
                response: Response,
            },
        }
        let admission = trace.in_scope(|| {
            telemetry::in_stage_sync(Stage::RequestAdmission, || {
                let mut budget = self.cfg.request_budget_for(correlation);
                if request.uri().path() == "/sparql" {
                    let admitted = self.cfg.query_admission.authenticate(request.headers());
                    request
                        .headers_mut()
                        .remove(axum::http::header::AUTHORIZATION);
                    match admitted {
                        Ok(context) => {
                            if context
                                .is_some_and(|context| budget.retain_security(context).is_err())
                            {
                                return Admission::Rejected {
                                    budget,
                                    response: problem::response(problem::ProblemCode::Internal),
                                };
                            }
                            crate::access_telemetry::record(
                                crate::access_telemetry::AccessDecision::Allow,
                            );
                        }
                        Err(code) => {
                            crate::access_telemetry::record(
                                crate::access_telemetry::AccessDecision::Deny,
                            );
                            return Admission::Rejected {
                                budget,
                                response: problem::response(code),
                            };
                        }
                    }
                }
                if let Err(error) = budget.checkpoint() {
                    return Admission::Rejected {
                        budget,
                        response: problem::response_for_control(error),
                    };
                }

                let snapshot = match self.cfg.runtime_lease() {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return Admission::Rejected {
                            budget,
                            response: snapshot_error_response(error),
                        }
                    }
                };

                let permit = match self.cfg.request_admission_permits().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(TryAcquireError::NoPermits) => {
                        return Admission::Rejected {
                            budget,
                            response: problem::response_with_retry_after(
                                problem::ProblemCode::ServiceOverloaded,
                            ),
                        }
                    }
                    Err(TryAcquireError::Closed) => {
                        return Admission::Rejected {
                            budget,
                            response: problem::response(problem::ProblemCode::Internal),
                        }
                    }
                };
                if let Err(error) = budget.checkpoint() {
                    drop(permit);
                    return Admission::Rejected {
                        budget,
                        response: problem::response_for_control(error),
                    };
                }
                if budget.retain_admission(permit).is_err() {
                    return Admission::Rejected {
                        budget,
                        response: problem::response(problem::ProblemCode::Internal),
                    };
                }
                Admission::Admitted { budget, snapshot }
            })
        });
        let (budget, snapshot) = match admission {
            Admission::Admitted { budget, snapshot } => (budget, snapshot),
            Admission::Rejected { budget, response } => {
                return deadline_checked_response(budget, response, trace)
            }
        };
        request.extensions_mut().insert(budget.clone());
        request.extensions_mut().insert(snapshot.clone());

        // Move the ready instance into the future while retaining a clone for the
        // next request. This preserves tower's poll_ready/call contract.
        let replacement = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, replacement);

        let span = trace.span();
        Box::pin(
            async move {
                let mut cancellation = budget.cancellation_guard();
                let inner_response = async move { inner.call(request).await };
                let response = match budget.run_until_deadline(inner_response).await {
                    Ok(Ok(response)) => response,
                    Ok(Err(never)) => match never {},
                    Err(error) => problem::response_for_control(error),
                };
                cancellation.disarm();
                Ok(pin_snapshot(trace.complete(response), snapshot))
            }
            .instrument(span),
        )
    }
}

fn snapshot_error_response(error: SnapshotUnavailable) -> Response {
    match error {
        SnapshotUnavailable::NotReady { .. } => {
            problem::response_with_retry_after(problem::ProblemCode::SourceUnavailable)
        }
        SnapshotUnavailable::StatePoisoned => problem::response(problem::ProblemCode::Internal),
    }
}

/// Keep the request's immutable generation alive until the response body is
/// consumed or dropped, including after its producer has filled a bounded
/// channel and released backend work.
fn pin_snapshot(response: Response, snapshot: RuntimeSnapshotLease) -> Response {
    response.map(|body| {
        Body::new(body.map_frame(move |frame| {
            let _activation_id = snapshot.activation_id();
            frame
        }))
    })
}

fn completed_response(trace: RequestTrace, response: Response) -> ResponseFuture {
    let span = trace.span();
    Box::pin(async move { Ok(trace.complete(response)) }.instrument(span))
}

fn deadline_checked_response(
    budget: RequestBudget,
    response: Response,
    trace: RequestTrace,
) -> ResponseFuture {
    let span = trace.span();
    Box::pin(
        async move {
            let response = match budget.run_until_deadline(async move { response }).await {
                Ok(response) => response,
                Err(error) => problem::response_for_control(error),
            };
            Ok(trace.complete(response))
        }
        .instrument(span),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use sf_core::{SourceId, SourceMapping};
    use sf_sparql::Epoch;

    use crate::{Backend, IntrospectedSource, RuntimeSnapshot, RuntimeSource};

    #[test]
    fn snapshot_unavailability_has_closed_public_classification() {
        let activation_id = cfg().runtime_readiness().unwrap().activation_id();
        let not_ready = snapshot_error_response(SnapshotUnavailable::NotReady {
            activation_id,
            cause: crate::ReadinessCause::SourceUnavailable,
        });
        assert_eq!(
            not_ready.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(not_ready.headers()[axum::http::header::RETRY_AFTER], "1");

        let poisoned = snapshot_error_response(SnapshotUnavailable::StatePoisoned);
        assert_eq!(
            poisoned.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(poisoned
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .is_none());
    }

    fn cfg() -> ServeConfig {
        ServeConfig::new_with_unverified_source(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
            crate::test_support::empty_ontology(),
            Vec::new(),
        )
        .unwrap()
    }

    fn candidate() -> RuntimeSnapshot {
        RuntimeSnapshot::single(
            Epoch(1),
            crate::test_support::empty_ontology(),
            RuntimeSource::new(
                IntrospectedSource::unchecked(
                    Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
                    Vec::new(),
                ),
                SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
            ),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn response_body_pins_the_old_snapshot_until_body_termination() {
        let cfg = cfg();
        let old_state = cfg.runtime_readiness().unwrap();
        let lease = cfg.runtime_lease().unwrap();
        let old_snapshot = lease.weak_snapshot();
        cfg.activate_snapshot(old_state, candidate()).unwrap();

        let response = pin_snapshot(
            Response::new(Body::new(http_body_util::Full::new(Bytes::from_static(
                b"pinned",
            )))),
            lease,
        );
        assert!(old_snapshot.upgrade().is_some());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[..], b"pinned");
        assert!(old_snapshot.upgrade().is_none());
    }

    #[tokio::test]
    async fn dropping_an_unpolled_response_body_releases_its_snapshot() {
        let cfg = cfg();
        let old_state = cfg.runtime_readiness().unwrap();
        let lease = cfg.runtime_lease().unwrap();
        let old_snapshot = lease.weak_snapshot();
        cfg.activate_snapshot(old_state, candidate()).unwrap();

        let response = pin_snapshot(
            Response::new(Body::new(http_body_util::Full::new(Bytes::from_static(
                b"unpolled",
            )))),
            lease,
        );
        assert!(old_snapshot.upgrade().is_some());
        drop(response);
        assert!(old_snapshot.upgrade().is_none());
    }

    #[tokio::test]
    async fn response_body_error_releases_its_snapshot() {
        let cfg = cfg();
        let old_state = cfg.runtime_readiness().unwrap();
        let lease = cfg.runtime_lease().unwrap();
        let old_snapshot = lease.weak_snapshot();
        cfg.activate_snapshot(old_state, candidate()).unwrap();

        let stream = tokio_stream::iter([Err::<Bytes, std::io::Error>(std::io::Error::other(
            "test body failure",
        ))]);
        let response = pin_snapshot(Response::new(Body::from_stream(stream)), lease);
        assert!(old_snapshot.upgrade().is_some());
        assert!(response.into_body().collect().await.is_err());
        assert!(old_snapshot.upgrade().is_none());
    }

    struct PendingBody {
        first_poll: Option<tokio::sync::oneshot::Sender<()>>,
    }

    impl tokio_stream::Stream for PendingBody {
        type Item = Result<Bytes, std::io::Error>;

        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            _context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            if let Some(first_poll) = self.first_poll.take() {
                let _ = first_poll.send(());
            }
            std::task::Poll::Pending
        }
    }

    #[tokio::test]
    async fn cancelled_body_consumer_releases_its_snapshot() {
        let cfg = cfg();
        let old_state = cfg.runtime_readiness().unwrap();
        let lease = cfg.runtime_lease().unwrap();
        let old_snapshot = lease.weak_snapshot();
        cfg.activate_snapshot(old_state, candidate()).unwrap();

        let (first_poll, observed_poll) = tokio::sync::oneshot::channel();
        let response = pin_snapshot(
            Response::new(Body::from_stream(PendingBody {
                first_poll: Some(first_poll),
            })),
            lease,
        );
        let consumer = tokio::spawn(async move { response.into_body().collect().await });
        observed_poll.await.unwrap();
        assert!(old_snapshot.upgrade().is_some());
        consumer.abort();
        assert!(consumer.await.unwrap_err().is_cancelled());
        assert!(old_snapshot.upgrade().is_none());
    }

    #[test]
    fn old_snapshot_drops_only_after_its_last_response_pin() {
        let cfg = cfg();
        let old_state = cfg.runtime_readiness().unwrap();
        let lease = cfg.runtime_lease().unwrap();
        let old_snapshot = lease.weak_snapshot();
        cfg.activate_snapshot(old_state, candidate()).unwrap();

        let first = pin_snapshot(Response::new(Body::empty()), lease.clone());
        let second = pin_snapshot(Response::new(Body::empty()), lease);
        assert_eq!(old_snapshot.strong_count(), 2);
        drop(first);
        assert_eq!(old_snapshot.strong_count(), 1);
        drop(second);
        assert!(old_snapshot.upgrade().is_none());
    }
}
