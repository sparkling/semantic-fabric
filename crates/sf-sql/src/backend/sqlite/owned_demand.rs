//! Consumer-driven demand for early-stop opens of the owned SQLite bridge.
//!
//! A full scan keeps the cap-1 prefetch bridge: the worker decodes the next
//! row while its consumer handles the previous one. A caller that may stop
//! before EOF (ASK, or a LIMIT applied above SQL) opens with `early_stop`; the
//! worker then steps and decodes a row only after its consumer asks for it, so
//! a row that is never read is never decoded or charged.
//!
//! Each consumer call mints at most one demand. A dropped `next_row` future
//! leaves its demand outstanding and the retry waits for that same row rather
//! than minting another; after a terminal EOF or error no demand is minted at
//! all. An idle worker re-checks request control on a bounded interval, so a
//! cancellation or deadline releases the connection even while the receiver is
//! retained but not polled. The first terminal cause is sticky: when an unread
//! row still occupies the channel, the cause is kept here and replaces EOF.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use sf_core::query_control::QueryControl;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::backend::{BranchStream, RawTuple};
use crate::error::{Error, Result};

#[cfg(test)]
#[path = "owned_demand_tests.rs"]
mod tests;

/// How long an idle demand worker waits before re-checking request control.
const IDLE_CONTROL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Default)]
struct DemandState {
    /// Demands minted by the consumer; never more than one ahead of `served`.
    issued: u64,
    /// Demands the worker has started to answer.
    served: u64,
    /// The consumer dropped its stream.
    closed: bool,
    /// A terminal cause that found an unread row still buffered.
    stashed: Option<Error>,
}

#[derive(Default)]
struct DemandGate {
    state: Mutex<DemandState>,
    changed: Condvar,
}

impl DemandGate {
    fn lock(&self) -> MutexGuard<'_, DemandState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn request(&self) {
        let mut state = self.lock();
        state.issued = state.issued.saturating_add(1);
        drop(state);
        self.changed.notify_all();
    }

    fn close(&self) {
        self.lock().closed = true;
        self.changed.notify_all();
    }

    fn stash(&self, error: Error) {
        let mut state = self.lock();
        if state.stashed.is_none() {
            state.stashed = Some(error);
        }
    }

    fn take_stashed(&self) -> Option<Error> {
        self.lock().stashed.take()
    }

    /// Worker side: `Ok(true)` once a row is demanded, `Ok(false)` once the
    /// consumer is gone, or the request's sticky terminal cause.
    fn wait(&self, control: Option<&dyn QueryControl>) -> Result<bool> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return Ok(false);
            }
            if let Some(control) = control {
                control.checkpoint()?;
            }
            if state.served < state.issued {
                state.served += 1;
                return Ok(true);
            }
            state = self
                .changed
                .wait_timeout(state, IDLE_CONTROL_INTERVAL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// Build one branch bridge. `early_stop` selects consumer-driven demand;
/// otherwise the cap-1 prefetch bridge is unchanged.
pub(super) fn bridge(early_stop: bool) -> (RowSender, SqliteReceiverStream) {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<RawTuple>>(1);
    let gate = early_stop.then(|| Arc::new(DemandGate::default()));
    let sender = RowSender {
        tx,
        demand: gate.clone(),
        terminal: false,
    };
    let demand = gate.map(|gate| ConsumerDemand {
        gate,
        outstanding: false,
        finished: false,
    });
    (sender, SqliteReceiverStream { rx, demand })
}

/// Worker end of one branch bridge.
pub(super) struct RowSender {
    tx: Sender<Result<RawTuple>>,
    demand: Option<Arc<DemandGate>>,
    terminal: bool,
}

impl RowSender {
    /// Gate the next cursor step. Prefetch always proceeds; a demand bridge
    /// waits for its consumer and reports a terminal control cause itself.
    pub(super) fn ready(&mut self, control: Option<&dyn QueryControl>) -> bool {
        let Some(gate) = self.demand.as_ref() else {
            return true;
        };
        match gate.wait(control) {
            Ok(ready) => ready,
            Err(error) => {
                // The failing checkpoint was the last step before delivery.
                self.fail(None, error);
                false
            }
        }
    }

    /// Deliver one decoded row or its terminal error; `false` stops the cursor.
    pub(super) fn row(&mut self, item: Result<RawTuple>) -> bool {
        let terminal = item.is_err();
        // Prefetch blocks here for backpressure. A demand bridge sends only for
        // an outstanding demand whose consumer already drained the previous
        // item, so this send never waits on an idle receiver.
        if self.tx.blocking_send(item).is_err() {
            return false;
        }
        self.terminal |= terminal;
        !terminal
    }

    /// Deliver a setup, cursor or teardown error. `control` is checkpointed
    /// first unless the caller has just observed the failing checkpoint.
    pub(super) fn fail(&mut self, control: Option<&dyn QueryControl>, error: Error) {
        // Preserve an already-classified error; checkpoint stays mandatory before send.
        if let Some(control) = control {
            let _ = control.checkpoint();
        }
        let Some(gate) = self.demand.as_ref() else {
            let _ = self.tx.blocking_send(Err(error));
            return;
        };
        if std::mem::replace(&mut self.terminal, true) {
            return; // the first terminal cause is sticky
        }
        // Never wait on a retained idle receiver: behind an unread row the
        // cause waits in the gate and replaces clean EOF.
        if let Err(TrySendError::Full(Err(error))) = self.tx.try_send(Err(error)) {
            gate.stash(error);
        }
    }
}

/// Consumer end of one demand bridge.
struct ConsumerDemand {
    gate: Arc<DemandGate>,
    outstanding: bool,
    finished: bool,
}

impl Drop for ConsumerDemand {
    fn drop(&mut self) {
        self.gate.close();
    }
}

/// The receive end of the cap-1 bridge: each `next_row` awaits the next
/// `Result<RawTuple>` produced by the blocking cursor. `None` ⇒ clean EOF;
/// `Some(Err)` ⇒ a HARD mid-stream marshalling/driver error (design A2), never a
/// silent short read.
pub struct SqliteReceiverStream {
    rx: Receiver<Result<RawTuple>>,
    demand: Option<ConsumerDemand>,
}

impl SqliteReceiverStream {
    #[cfg(test)]
    pub(super) fn issued_demands(&self) -> Option<u64> {
        let demand = self.demand.as_ref()?;
        Some(demand.gate.lock().issued)
    }
}

impl BranchStream for SqliteReceiverStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        if let Some(demand) = self.demand.as_mut() {
            // A retried call after a dropped future reuses its demand.
            if !demand.outstanding && !demand.finished {
                demand.outstanding = true;
                demand.gate.request();
            }
        }
        let item = self.rx.recv().await;
        if let Some(demand) = self.demand.as_mut() {
            demand.outstanding = false;
            match &item {
                Some(Ok(_)) => {}
                Some(Err(_)) => demand.finished = true,
                None => {
                    demand.finished = true;
                    if let Some(error) = demand.gate.take_stashed() {
                        return Err(error);
                    }
                }
            }
        }
        match item {
            None => Ok(None), // producer finished ⇒ clean EOF
            Some(Ok(tuple)) => Ok(Some(tuple)),
            Some(Err(e)) => Err(e), // forwarded marshalling/driver error (A2)
        }
    }
}
