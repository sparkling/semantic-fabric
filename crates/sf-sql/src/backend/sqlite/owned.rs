//! Owned cap-1 SQLite serve-lane bridge (ADR-0024 §4.1).

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

use super::{column_meta, marshal_row};

/// An **owned, `'static`** SQLite backend over `Arc<Mutex<Connection>>` — the serve
/// lane's flavor (design §4.1). Its stream ([`SqliteReceiverStream`]) is the receive
/// end of a **cap-1** channel fed by a `spawn_blocking` cursor thread, so the sync,
/// `!Send` `Connection` never crosses a thread boundary and the stream is
/// `Send + 'static` — what lets the generic core's `for<'s> B::Stream<'s>: Send`
/// bound hold across `tokio::spawn`.
pub struct SqliteOwnedBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteOwnedBackend {
    /// Wrap a shared connection handle (the serve lane already holds this shape).
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

/// The receive end of the cap-1 bridge: each `next_row` awaits the next
/// `Result<RawTuple>` produced by the blocking cursor. `None` ⇒ clean EOF;
/// `Some(Err)` ⇒ a HARD mid-stream marshalling/driver error (design A2), never a
/// silent short read.
pub struct SqliteReceiverStream {
    rx: tokio::sync::mpsc::Receiver<Result<RawTuple>>,
}

impl BranchStream for SqliteReceiverStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        match self.rx.recv().await {
            None => Ok(None), // producer finished ⇒ clean EOF
            Some(Ok(tuple)) => Ok(Some(tuple)),
            Some(Err(e)) => Err(e), // forwarded marshalling/driver error (A2)
        }
    }
}

impl SqlBackend for SqliteOwnedBackend {
    type Stream<'s>
        = SqliteReceiverStream
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        // Lock + prepare inside `spawn_blocking`, mirroring `open_branch` below
        // (ADR-0024 §4.1) — NOT inline as this used to do. A `std::sync::Mutex`
        // taken inline in an async fn blocks the tokio WORKER THREAD itself (no
        // yield point), so `N` concurrent callers contending for one connection
        // can wedge every worker thread simultaneously: a genuine deadlock once
        // `N > worker_threads` (`sf-serve/tests/endpoint.rs`
        // `sqlite_pool_concurrency_receipt` doc + `column_names_spawn_blocking_
        // deadlock_regression`).
        let conn = Arc::clone(&self.conn);
        let probe_sql = probe_sql.to_owned();
        let joined = tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            crate::stream::sqlite_column_names(&guard, &probe_sql)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(e) => Err(Error::Introspection(format!(
                "column_names spawn_blocking task join error: {e}"
            ))),
        }
    }

    async fn open_branch(
        &mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<SqliteReceiverStream> {
        // cap-1, FIFO (=_bag-preserving) channel: at most one buffered row in flight
        // + one `&Row` live on the blocking thread ⇒ ~2-row materialisation.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<RawTuple>>(1);
        let conn = Arc::clone(&self.conn);
        let sql = sql.to_owned();
        let params: Vec<String> = lexical_params.to_vec();
        // The `!Send` Connection / Statement / Rows live ONLY on this blocking
        // thread; `blocking_send` on the cap-1 channel blocks the cursor until the
        // reactor consumes ⇒ explicit backpressure (strengthens bounded memory).
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            let (decl_codes, pads, nproj) = match column_meta(&guard, &sql) {
                Ok(m) => m,
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    return;
                }
            };
            let mut stmt = match guard.prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::from(e)));
                    return;
                }
            };
            let mut rows = match stmt.query(rusqlite::params_from_iter(params.iter())) {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::from(e)));
                    return;
                }
            };
            loop {
                match rows.next() {
                    Ok(Some(row)) => {
                        let sent = match marshal_row(row, &decl_codes, &pads, nproj) {
                            Ok(tuple) => tx.blocking_send(Ok(tuple)).is_ok(),
                            Err(e) => {
                                let _ = tx.blocking_send(Err(e));
                                false // hard marshalling error ⇒ stop the cursor (A2)
                            }
                        };
                        if !sent {
                            break; // receiver gone (cancel-on-drop) or error sent
                        }
                    }
                    Ok(None) => break, // clean EOF ⇒ drop tx ⇒ next_row sees None
                    Err(e) => {
                        let _ = tx.blocking_send(Err(Error::from(e)));
                        break;
                    }
                }
            }
        });
        Ok(SqliteReceiverStream { rx })
    }
}
