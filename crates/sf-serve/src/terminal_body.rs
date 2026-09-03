//! Bounded data transport with an out-of-band, fail-closed terminal outcome.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::Stream;

use crate::budget::RequestBudget;

const STREAM_FAILURE_MESSAGE: &str = "result stream failed";

#[derive(Clone, Copy)]
enum ProducerOutcome {
    Complete,
    Failed,
}

/// The sole producer-side terminal authority. Sending an outcome is synchronous
/// and cannot wait behind queued body data. Dropping it without an outcome is an
/// unexpected producer disappearance, which the receiver treats as failure.
struct ProducerTerminal(Option<oneshot::Sender<ProducerOutcome>>);

impl ProducerTerminal {
    fn finish(mut self, outcome: ProducerOutcome) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(outcome);
        }
    }
}

enum BodyPhase {
    Data,
    Terminal,
    Done,
}

/// Drain the bounded FIFO prefix before observing its out-of-band terminal
/// outcome. Exactly one failure item can be emitted; all later polls are EOF.
struct FusedBodyStream {
    data: mpsc::Receiver<Bytes>,
    terminal: oneshot::Receiver<ProducerOutcome>,
    phase: BodyPhase,
}

impl Stream for FusedBodyStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.phase {
                BodyPhase::Data => match self.data.poll_recv(cx) {
                    Poll::Ready(Some(bytes)) => return Poll::Ready(Some(Ok(bytes))),
                    Poll::Ready(None) => self.phase = BodyPhase::Terminal,
                    Poll::Pending => return Poll::Pending,
                },
                BodyPhase::Terminal => {
                    let outcome = match Pin::new(&mut self.terminal).poll(cx) {
                        Poll::Ready(Ok(outcome)) => outcome,
                        Poll::Ready(Err(_producer_disappeared)) => ProducerOutcome::Failed,
                        Poll::Pending => return Poll::Pending,
                    };
                    self.phase = BodyPhase::Done;
                    return match outcome {
                        ProducerOutcome::Complete => Poll::Ready(None),
                        ProducerOutcome::Failed => Poll::Ready(Some(Err(stream_failure_error()))),
                    };
                }
                BodyPhase::Done => return Poll::Ready(None),
            }
        }
    }
}

fn channel(capacity: usize) -> (mpsc::Sender<Bytes>, ProducerTerminal, Body) {
    let (data_tx, data_rx) = mpsc::channel(capacity);
    let (terminal_tx, terminal_rx) = oneshot::channel();
    let body = Body::from_stream(FusedBodyStream {
        data: data_rx,
        terminal: terminal_rx,
        phase: BodyPhase::Data,
    });
    (data_tx, ProducerTerminal(Some(terminal_tx)), body)
}

/// Spawn one governed producer. The terminal outcome is stored before the sole
/// watcher sender drops and closes the data channel, so a live body never sees
/// silent EOF. The returned handle is detached by production callers and retained
/// only by deterministic unit tests.
pub(crate) fn spawn<F, Fut>(
    capacity: usize,
    budget: RequestBudget,
    produce: F,
) -> (Body, JoinHandle<()>)
where
    F: FnOnce(mpsc::Sender<Bytes>, RequestBudget) -> Fut + Send + 'static,
    Fut: Future<Output = io::Result<()>> + Send + 'static,
{
    let (data_tx, terminal, body) = channel(capacity);
    let task = tokio::spawn(async move {
        // Exactly one sender lives outside the governed producer future. It is
        // used only for receiver-drop observation and guarantees the data channel
        // cannot close before the terminal outcome is stored below.
        let closed_tx = data_tx.clone();
        let result = {
            let phase_budget = budget.clone();
            let guarded = budget.run(produce(data_tx, phase_budget));
            tokio::select! {
                biased;
                _ = closed_tx.closed() => {
                    budget.cancel();
                    return;
                }
                result = guarded => result
                    .map_or(ProducerOutcome::Failed, |result| {
                        if result.is_ok() {
                            ProducerOutcome::Complete
                        } else {
                            ProducerOutcome::Failed
                        }
                    }),
            }
        };
        terminal.finish(result);
        drop(closed_tx);
    });
    (body, task)
}

pub(crate) fn stream_failure_error() -> io::Error {
    io::Error::other(STREAM_FAILURE_MESSAGE)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::HttpBody;
    use http_body_util::BodyExt;
    use sf_core::query_control::QueryLimits;
    use tokio::sync::{oneshot, Semaphore};

    use super::*;

    const CAPACITY: usize = 8;

    fn budget(timeout: Duration) -> RequestBudget {
        RequestBudget::after(timeout, QueryLimits::new(u64::MAX, u64::MAX, u64::MAX))
    }

    #[test]
    fn streamed_body_does_not_report_an_exact_content_length() {
        let (_tx, _terminal, body) = channel(1);

        assert_eq!(body.size_hint().exact(), None);
        assert_eq!(body.size_hint().upper(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn full_unpolled_channel_finishes_at_deadline_then_drains_prefix_error_and_fuses() {
        let timeout = Duration::from_secs(10);
        let request_permits = Arc::new(Semaphore::new(1));
        let request_permit = request_permits
            .clone()
            .try_acquire_owned()
            .expect("take aggregate request capacity");
        let mut request_budget = budget(timeout);
        request_budget
            .retain_admission(request_permit)
            .expect("attach request admission before cloning");
        let (full_tx, full_rx) = oneshot::channel();
        let (mut body, producer) = spawn(CAPACITY, request_budget, move |tx, _budget| async move {
            for index in 0..CAPACITY {
                tx.send(Bytes::from(format!("prefix-{index}")))
                    .await
                    .expect("test body remains live");
            }
            let _ = full_tx.send(());
            tx.send(Bytes::from_static(b"must-not-enter-prefix"))
                .await
                .expect("deadline must cancel this full-channel send");
            Ok(())
        });
        full_rx.await.expect("producer filled the data channel");

        tokio::time::advance(timeout).await;
        tokio::time::timeout(Duration::from_secs(1), producer)
            .await
            .expect("producer must terminate without polling the full body")
            .expect("producer task must not panic");
        assert_eq!(
            request_permits.available_permits(),
            1,
            "producer returns aggregate capacity while completed bytes remain unread"
        );

        for index in 0..CAPACITY {
            let frame = body
                .frame()
                .await
                .expect("queued prefix frame")
                .expect("queued prefix is data");
            assert_eq!(
                frame.into_data().expect("queued prefix data"),
                Bytes::from(format!("prefix-{index}"))
            );
        }
        let error = body
            .frame()
            .await
            .expect("terminal error frame")
            .expect_err("deadline must fail the body");
        assert_eq!(error.to_string(), STREAM_FAILURE_MESSAGE);
        assert!(
            body.frame().await.is_none(),
            "error must be followed by EOF"
        );
        assert!(
            body.frame().await.is_none(),
            "body must remain fused at EOF"
        );
    }

    #[tokio::test]
    async fn unexpected_producer_disappearance_is_one_error_then_fused_eof() {
        let (tx, terminal, mut body) = channel(1);
        tx.send(Bytes::from_static(b"queued"))
            .await
            .expect("body remains live");
        drop(terminal);
        drop(tx);

        let prefix = body
            .frame()
            .await
            .expect("queued frame")
            .expect("queued data");
        assert_eq!(
            prefix.into_data().expect("data frame"),
            Bytes::from_static(b"queued")
        );
        let error = body
            .frame()
            .await
            .expect("fail-closed frame")
            .expect_err("missing terminal outcome must fail");
        assert_eq!(error.to_string(), STREAM_FAILURE_MESSAGE);
        assert!(body.frame().await.is_none());
        assert!(body.frame().await.is_none());
    }

    #[tokio::test]
    async fn completed_producer_drains_prefix_then_fused_eof() {
        let (tx, terminal, mut body) = channel(1);
        tx.send(Bytes::from_static(b"complete"))
            .await
            .expect("body remains live");
        terminal.finish(ProducerOutcome::Complete);
        drop(tx);

        let prefix = body
            .frame()
            .await
            .expect("queued frame")
            .expect("queued data");
        assert_eq!(
            prefix.into_data().expect("data frame"),
            Bytes::from_static(b"complete")
        );
        assert!(body.frame().await.is_none());
        assert!(body.frame().await.is_none());
    }
}
