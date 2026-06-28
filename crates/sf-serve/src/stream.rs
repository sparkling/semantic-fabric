//! Bounded-memory serialisation + HTTP-body streaming (ADR-0010 §C, ADR-0006).
//!
//! **Every** result path streams end to end — none collects the result set or the
//! whole serialised body:
//!
//! * **SQLite** is synchronous, so the row→serialise→flush loop runs inside a
//!   `spawn_blocking` task ([`ChannelWriter`]): bytes are pushed chunk by chunk
//!   into a tokio channel backing the axum body. SELECT drives
//!   [`sf_sparql::exec::select_each`]; CONSTRUCT drives `exec::construct`.
//! * **PostgreSQL** is asynchronous, so the loop runs in a `spawn`ed task that
//!   serialises each row/triple into a small shared buffer ([`SharedBuf`]) and
//!   `send().await`s a chunk once it fills — backpressure flows straight back to
//!   the `query_raw` server-side cursor ([`sf_sparql::exec_pg::select_each_pg`] /
//!   [`construct_each_pg`](sf_sparql::exec_pg::construct_each_pg)).
//!
//! In both, a slow/aborted client (receiver dropped) makes the next send fail →
//! the producer stops (cancel-on-drop, ADR-0010 §C), and a passed deadline aborts
//! at the next chunk/row (the request timeout). ASK is a single boolean — bounded
//! by construction — and is serialised whole via [`collected_body`].

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::{Body, Bytes};
use oxjsonld::JsonLdSerializer;
use oxrdf::{GraphNameRef, Term, Triple, Variable};
use oxttl::{NTriplesSerializer, TurtleSerializer};
use rusqlite::Connection;
use sparesults::{QueryResultsFormat, QueryResultsSerializer};
use tokio::sync::mpsc::Sender;
use tokio_postgres::Client;
use tokio_stream::wrappers::ReceiverStream;

use sf_sparql::{exec, exec_pg, Plan};

/// Bytes per streamed body chunk (a flush boundary, not a result-size cap).
const CHUNK: usize = 16 * 1024;
/// Bound on chunks in flight — the HTTP-body backpressure window (ADR-0006).
const CHANNEL_CAP: usize = 8;

/// The RDF serialisation chosen by content negotiation for CONSTRUCT/DESCRIBE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RdfFormat {
    Turtle,
    NTriples,
    JsonLd,
}

impl RdfFormat {
    pub fn media_type(self) -> &'static str {
        match self {
            RdfFormat::Turtle => "text/turtle; charset=utf-8",
            RdfFormat::NTriples => "application/n-triples; charset=utf-8",
            RdfFormat::JsonLd => "application/ld+json",
        }
    }
}

/// A `std::io::Write` that forwards completed chunks to a tokio channel via
/// `blocking_send` (so it lives inside a `spawn_blocking` task — never on the async
/// runtime). Enforces the request deadline and cancel-on-drop at chunk granularity.
struct ChannelWriter {
    tx: tokio::sync::mpsc::Sender<Result<Bytes, io::Error>>,
    buf: Vec<u8>,
    deadline: Option<Instant>,
}

impl ChannelWriter {
    fn new(
        tx: tokio::sync::mpsc::Sender<Result<Bytes, io::Error>>,
        deadline: Option<Instant>,
    ) -> Self {
        Self {
            tx,
            buf: Vec::with_capacity(CHUNK),
            deadline,
        }
    }

    fn send_chunk(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = Bytes::from(std::mem::take(&mut self.buf));
        self.buf = Vec::with_capacity(CHUNK);
        self.tx.blocking_send(Ok(chunk)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "client disconnected (cancel-on-drop)",
            )
        })
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(d) = self.deadline {
            if Instant::now() > d {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "request timeout (ADR-0010)",
                ));
            }
        }
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.send_chunk()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send_chunk()
    }
}

/// A triple serialiser over an arbitrary writer, dispatched by negotiated format.
/// CONSTRUCT produces triples; JSON-LD output places them in the default graph.
enum TripleSink<W: Write> {
    Turtle(oxttl::turtle::WriterTurtleSerializer<W>),
    NTriples(oxttl::ntriples::WriterNTriplesSerializer<W>),
    JsonLd(oxjsonld::WriterJsonLdSerializer<W>),
}

impl<W: Write> TripleSink<W> {
    fn start(fmt: RdfFormat, w: W) -> Self {
        match fmt {
            RdfFormat::Turtle => TripleSink::Turtle(TurtleSerializer::new().for_writer(w)),
            RdfFormat::NTriples => TripleSink::NTriples(NTriplesSerializer::new().for_writer(w)),
            RdfFormat::JsonLd => TripleSink::JsonLd(JsonLdSerializer::new().for_writer(w)),
        }
    }

    fn write(&mut self, t: &Triple) -> io::Result<()> {
        match self {
            TripleSink::Turtle(s) => s.serialize_triple(t.as_ref()),
            TripleSink::NTriples(s) => s.serialize_triple(t.as_ref()),
            TripleSink::JsonLd(s) => {
                s.serialize_quad(t.as_ref().in_graph(GraphNameRef::DefaultGraph))
            }
        }
    }

    /// Finish, returning the writer so the caller can flush its tail.
    fn finish(self) -> io::Result<W> {
        match self {
            TripleSink::Turtle(s) => s.finish(),
            TripleSink::NTriples(s) => Ok(s.finish()),
            TripleSink::JsonLd(s) => s.finish(),
        }
    }
}

/// Stream a SQLite CONSTRUCT/dump end-to-end: `exec::construct` feeds each triple
/// into the serialiser over a [`ChannelWriter`]; the body is the receiver side.
/// Errors (including the deadline / client-drop) terminate the stream.
pub fn construct_body_sqlite(
    conn: Arc<Mutex<Connection>>,
    plan: Plan,
    fmt: RdfFormat,
    deadline: Option<Instant>,
) -> Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(CHANNEL_CAP);
    let err_tx = tx.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut sink = TripleSink::start(fmt, ChannelWriter::new(tx, deadline));
        let mut io_err: Option<io::Error> = None;
        let exec_res = exec::construct(&plan, &conn, |triple| {
            if io_err.is_some() {
                return;
            }
            if let Err(e) = sink.write(&triple) {
                io_err = Some(e);
            }
        });
        let result = io_err
            .map(Err)
            .unwrap_or(Ok(()))
            .and_then(|()| exec_res.map_err(|e| io::Error::other(e.to_string())))
            .and_then(|_| sink.finish())
            .and_then(|mut w| w.flush());
        if let Err(e) = result {
            let _ = err_tx.blocking_send(Err(e));
        }
    });
    Body::from_stream(ReceiverStream::new(rx))
}

/// Wrap an already-serialised, bounded body in a chunked stream (uniform response
/// shape; used for SELECT/ASK and the collecting PostgreSQL paths).
pub fn collected_body(bytes: Vec<u8>) -> Body {
    let chunks: Vec<Result<Bytes, io::Error>> = bytes
        .chunks(CHUNK)
        .map(|c| Ok(Bytes::copy_from_slice(c)))
        .collect();
    Body::from_stream(tokio_stream::iter(chunks))
}

/// Serialise an ASK result to the negotiated SPARQL-Results format (a single
/// boolean — bounded by construction, so it is fine to serialise whole).
pub fn serialize_boolean(value: bool, fmt: QueryResultsFormat) -> Result<Vec<u8>, String> {
    QueryResultsSerializer::from_format(fmt)
        .serialize_boolean_to_writer(Vec::new(), value)
        .map_err(|e| e.to_string())
}

/// The SPARQL-Results variable header for `vars` (deferred parse: the projection
/// names are already valid `?var` tokens).
fn variables(vars: &[String]) -> Vec<Variable> {
    vars.iter().map(Variable::new_unchecked).collect()
}

/// Stream a SQLite SELECT end to end: drive [`exec::select_each`] inside a
/// `spawn_blocking` task, serialising each solution row straight into the
/// negotiated SPARQL-Results serialiser over a [`ChannelWriter`]. Nothing holds
/// the result set or the whole serialised body (ADR-0010 §C).
pub fn select_body_sqlite(
    conn: Arc<Mutex<Connection>>,
    plan: Plan,
    fmt: QueryResultsFormat,
    vars: Vec<String>,
    deadline: Option<Instant>,
) -> Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(CHANNEL_CAP);
    let err_tx = tx.clone();
    tokio::task::spawn_blocking(move || {
        let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
        let varv = variables(&vars);
        let result = (|| -> io::Result<()> {
            let mut writer = QueryResultsSerializer::from_format(fmt)
                .serialize_solutions_to_writer(ChannelWriter::new(tx, deadline), varv.clone())?;
            exec::select_each(&plan, &conn, |row| {
                writer
                    .serialize(solution_pairs(row, &varv))
                    .map_err(|e| sf_sparql::Error::Sql(e.to_string()))
            })
            .map_err(|e| io::Error::other(e.to_string()))?;
            let mut inner = writer.finish()?;
            inner.flush()
        })();
        if let Err(e) = result {
            let _ = err_tx.blocking_send(Err(e));
        }
    });
    Body::from_stream(ReceiverStream::new(rx))
}

/// Stream a PostgreSQL SELECT end to end: drive [`exec_pg::select_each_pg`] in a
/// `spawn`ed task, serialising each row into a small [`SharedBuf`] and flushing a
/// chunk once it fills. `send().await` backpressures the `query_raw` cursor, so
/// memory stays bounded regardless of result size (ADR-0010 §C).
pub fn select_body_pg(
    client: Arc<Client>,
    plan: Plan,
    fmt: QueryResultsFormat,
    vars: Vec<String>,
    deadline: Option<Instant>,
) -> Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(CHANNEL_CAP);
    let err_tx = tx.clone();
    tokio::spawn(async move {
        let varv = variables(&vars);
        let buf = SharedBuf::default();
        let result: io::Result<()> = async {
            let mut writer = QueryResultsSerializer::from_format(fmt)
                .serialize_solutions_to_writer(buf.clone(), varv.clone())?;
            exec_pg::select_each_pg(&plan, &client, |row| {
                let prepared = (|| -> io::Result<Vec<u8>> {
                    check_deadline(deadline)?;
                    writer.serialize(solution_pairs(&row, &varv))?;
                    Ok(buf.take_if_full())
                })();
                let tx = tx.clone();
                async move { send_prepared(&tx, prepared).await }
            })
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
            writer.finish()?;
            send_chunk(&tx, buf.take_all()).await
        }
        .await;
        if let Err(e) = result {
            let _ = err_tx.send(Err(e)).await;
        }
    });
    Body::from_stream(ReceiverStream::new(rx))
}

/// Stream a PostgreSQL CONSTRUCT end to end (the async mirror of
/// [`construct_body_sqlite`]): drive [`exec_pg::construct_each_pg`], serialising
/// each solution's triples into a [`SharedBuf`] and flushing chunks as they fill.
pub fn construct_body_pg(
    client: Arc<Client>,
    plan: Plan,
    fmt: RdfFormat,
    deadline: Option<Instant>,
) -> Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(CHANNEL_CAP);
    let err_tx = tx.clone();
    tokio::spawn(async move {
        let buf = SharedBuf::default();
        let result: io::Result<()> = async {
            let mut sink = TripleSink::start(fmt, buf.clone());
            exec_pg::construct_each_pg(&plan, &client, |triples| {
                let prepared = (|| -> io::Result<Vec<u8>> {
                    check_deadline(deadline)?;
                    for t in &triples {
                        sink.write(t)?;
                    }
                    Ok(buf.take_if_full())
                })();
                let tx = tx.clone();
                async move { send_prepared(&tx, prepared).await }
            })
            .await
            .map_err(|e| io::Error::other(e.to_string()))?;
            sink.finish()?;
            send_chunk(&tx, buf.take_all()).await
        }
        .await;
        if let Err(e) = result {
            let _ = err_tx.send(Err(e)).await;
        }
    });
    Body::from_stream(ReceiverStream::new(rx))
}

/// One solution row's bound `(variable, term)` pairs in projection order (unbound
/// positions dropped — SPARQL-Results omits them).
fn solution_pairs<'a>(
    row: &'a [Option<Term>],
    vars: &'a [Variable],
) -> impl Iterator<Item = (oxrdf::VariableRef<'a>, oxrdf::TermRef<'a>)> {
    row.iter()
        .zip(vars.iter())
        .filter_map(|(term, var)| term.as_ref().map(|t| (var.as_ref(), t.as_ref())))
}

/// Error out if the request deadline (ADR-0010 timeout) has passed.
fn check_deadline(deadline: Option<Instant>) -> io::Result<()> {
    match deadline {
        Some(d) if Instant::now() > d => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "request timeout (ADR-0010)",
        )),
        _ => Ok(()),
    }
}

/// A `std::io::Write` over a shared byte buffer the async PostgreSQL streamers
/// serialise into and drain between rows (the serialiser owns one clone; the
/// streamer keeps another). `write` only appends — it never blocks — so it is
/// safe to call from the serialiser on the async runtime; draining + sending is
/// done explicitly at an `.await` point.
#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    /// Take the buffered bytes once they reach a chunk (else leave them to batch).
    fn take_if_full(&self) -> Vec<u8> {
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if g.len() >= CHUNK {
            std::mem::take(&mut *g)
        } else {
            Vec::new()
        }
    }

    /// Take everything buffered (the final tail after `finish`).
    fn take_all(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl Write for SharedBuf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Send a chunk (skipping empties), mapping a dropped receiver to the sparql
/// error type the async executor sink expects (terminating the stream).
async fn send_prepared(
    tx: &Sender<Result<Bytes, io::Error>>,
    prepared: io::Result<Vec<u8>>,
) -> sf_sparql::Result<()> {
    let bytes = prepared.map_err(|e| sf_sparql::Error::Sql(e.to_string()))?;
    send_chunk(tx, bytes)
        .await
        .map_err(|e| sf_sparql::Error::Sql(e.to_string()))
}

/// Flush one chunk into the body channel; a dropped receiver (client gone) is a
/// broken pipe → the producer stops (cancel-on-drop, ADR-0010 §C).
async fn send_chunk(tx: &Sender<Result<Bytes, io::Error>>, bytes: Vec<u8>) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    tx.send(Ok(Bytes::from(bytes))).await.map_err(|_| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "client disconnected (cancel-on-drop)",
        )
    })
}
