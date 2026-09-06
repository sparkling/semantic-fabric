//! Response-body lifetime telemetry retained beyond HTTP response handoff.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use http_body::{Body as HttpBody, Frame, SizeHint};
use sf_core::TELEMETRY_TARGET;
use tracing::Span;

use crate::telemetry::CorrelationId;

const SCHEMA: &str = "semantic-fabric.telemetry.v1";

#[derive(Clone, Copy)]
enum BodyOutcome {
    Complete,
    Error,
    Dropped,
    NotApplicable,
}

impl BodyOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Error => "error",
            Self::Dropped => "dropped",
            Self::NotApplicable => "not_applicable",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum BodyDisposition {
    Expected,
    ProtocolNoBody,
}

struct TracedResponseBody {
    inner: Body,
    span: Span,
    correlation: CorrelationId,
    disposition: BodyDisposition,
    finished: AtomicBool,
}

impl TracedResponseBody {
    fn new(
        inner: Body,
        span: Span,
        correlation: CorrelationId,
        disposition: BodyDisposition,
    ) -> Self {
        Self {
            inner,
            span,
            correlation,
            disposition,
            finished: AtomicBool::new(false),
        }
    }

    const fn eof_outcome(&self) -> BodyOutcome {
        match self.disposition {
            BodyDisposition::Expected => BodyOutcome::Complete,
            BodyDisposition::ProtocolNoBody => BodyOutcome::NotApplicable,
        }
    }

    const fn drop_outcome(&self) -> BodyOutcome {
        match self.disposition {
            BodyDisposition::Expected => BodyOutcome::Dropped,
            BodyDisposition::ProtocolNoBody => BodyOutcome::NotApplicable,
        }
    }

    fn finish(&self, outcome: BodyOutcome) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        tracing::info!(
            target: TELEMETRY_TARGET,
            parent: &self.span,
            schema = SCHEMA,
            event = "response.body.finished",
            outcome = outcome.as_str(),
            correlation_id = self.correlation.as_str(),
        );
    }
}

impl HttpBody for TracedResponseBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.as_mut().get_mut();
        let span = this.span.clone();
        let polled = span.in_scope(|| Pin::new(&mut this.inner).poll_frame(context));
        match &polled {
            Poll::Ready(None) => this.finish(this.eof_outcome()),
            Poll::Ready(Some(Err(_))) => this.finish(BodyOutcome::Error),
            Poll::Ready(Some(Ok(_))) | Poll::Pending => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        let ended = self.inner.is_end_stream();
        if ended {
            self.finish(self.eof_outcome());
        }
        ended
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for TracedResponseBody {
    fn drop(&mut self) {
        self.finish(self.drop_outcome());
    }
}

pub(crate) fn wrap(
    body: Body,
    span: Span,
    correlation: CorrelationId,
    disposition: BodyDisposition,
) -> Body {
    Body::new(TracedResponseBody::new(
        body,
        span,
        correlation,
        disposition,
    ))
}
