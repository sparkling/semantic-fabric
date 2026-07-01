//! Bounded-memory serialisation + HTTP-body streaming (ADR-0010 §C, ADR-0006).
//!
//! **Every** result path streams end to end through one generic streamer per query
//! form — none collects the result set or the whole serialised body. Since ADR-0024
//! M5 the three backends share a single async pipeline: each `spawn`ed task acquires
//! its backend (SQLite `SqliteOwnedBackend` over a `spawn_blocking` cap-1 bridge; PG
//! `PgBackend<Arc<Client>>`; MySQL a DEDICATED pooled `Conn`), then drives the
//! driver-agnostic core ([`sf_sparql::exec_core::select_each_async`] /
//! [`construct_each_async`](sf_sparql::exec_core::construct_each_async)) serialising
//! each row/triple into a small shared buffer ([`SharedBuf`]) and `send().await`ing a
//! chunk once it fills — backpressure flows straight back to the server-side cursor
//! (PG `query_raw`, SQLite cap-1 bridge, MySQL packet-bounded `exec_iter`).
//!
//! A slow/aborted client (receiver dropped) makes the next send fail → the producer
//! stops (cancel-on-drop, ADR-0010 §C), and a passed deadline aborts at the next
//! chunk/row (the request timeout). ASK is a single boolean — bounded by
//! construction — and is serialised whole via [`collected_body`].

use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::{Body, Bytes};
use oxjsonld::JsonLdSerializer;
use oxrdf::{GraphNameRef, Term, Triple, Variable};
use oxttl::{NTriplesSerializer, TurtleSerializer};
use sparesults::{QueryResultsFormat, QueryResultsSerializer};
use tokio::sync::mpsc::Sender;
use tokio_stream::wrappers::ReceiverStream;

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

/// Wrap an already-serialised, bounded body in a chunked stream (uniform response
/// shape; used for ASK).
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

/// A boxed, `Send` future the type-erased serve-lane sinks and drivers return, so
/// one generic streamer serves all three concrete backends. (The generic core's
/// per-backend `select_each`/`construct_each` futures are only provably `Send` when
/// monomorphized at a concrete backend — AFIT gives no `Send` guarantee for an
/// abstract `B` — so sf-serve erases the concrete driver future here rather than
/// spawning `run::<B>` generically.)
type BoxedResult = Pin<Box<dyn Future<Output = sf_sparql::Result<()>> + Send>>;

/// A type-erased per-row solution sink (built by [`select_body_streaming`], consumed
/// by the concrete `exec_*::select_each_*` driver).
type RowSink = Box<dyn FnMut(Vec<Option<Term>>) -> BoxedResult + Send>;

/// A type-erased per-solution triple sink (built by [`construct_body_streaming`],
/// consumed by the concrete `exec_*::construct_each_*` driver).
type TripleStreamSink = Box<dyn FnMut(Vec<Triple>) -> BoxedResult + Send>;

/// Stream a SELECT end to end over **any** backend (ADR-0024 M5 uniform lane). The
/// `spawn`ed task builds a [`RowSink`] (serialise each row into a [`SharedBuf`],
/// flush a chunk once it fills) and hands it to `drive` — a thin closure that runs
/// the concrete `exec_*::select_each_*` for the request's backend (SQLite cap-1
/// bridge / PG `Arc<Client>` / MySQL dedicated `Conn`). `send().await` backpressures
/// the server-side cursor, so memory stays bounded regardless of result size
/// (ADR-0010 §C); a dropped receiver or passed deadline terminates the stream and
/// releases the backend (PG cancel-on-drop / MySQL dedicated-conn discard §4.2 /
/// SQLite bridge thread stops).
pub fn select_body_streaming<D>(
    drive: D,
    fmt: QueryResultsFormat,
    vars: Vec<String>,
    deadline: Option<Instant>,
) -> Body
where
    D: FnOnce(RowSink) -> BoxedResult + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(CHANNEL_CAP);
    let err_tx = tx.clone();
    tokio::spawn(async move {
        let varv = variables(&vars);
        let buf = SharedBuf::default();
        let result: io::Result<()> = async {
            // The serialiser writer is shared with the sink via Arc<Mutex<Option>> so
            // the sink can serialise each row and the outer scope can `finish()` +
            // flush the tail after the driver returns (the sink's clone is dropped
            // when the driver future completes).
            let writer = Arc::new(Mutex::new(Some(
                QueryResultsSerializer::from_format(fmt)
                    .serialize_solutions_to_writer(buf.clone(), varv.clone())?,
            )));
            let sink: RowSink = {
                let writer = Arc::clone(&writer);
                let buf = buf.clone();
                let varv = varv.clone();
                let tx = tx.clone();
                Box::new(move |row: Vec<Option<Term>>| {
                    let prepared = (|| -> io::Result<Vec<u8>> {
                        check_deadline(deadline)?;
                        writer
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .as_mut()
                            .expect("writer live during streaming")
                            .serialize(solution_pairs(&row, &varv))?;
                        Ok(buf.take_if_full())
                    })();
                    let tx = tx.clone();
                    Box::pin(async move { send_prepared(&tx, prepared).await }) as BoxedResult
                })
            };
            drive(sink)
                .await
                .map_err(|e| io::Error::other(e.to_string()))?;
            let writer = writer
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .expect("writer live after streaming");
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

/// Stream a CONSTRUCT end to end over **any** backend (the CONSTRUCT sibling of
/// [`select_body_streaming`]): build a [`TripleStreamSink`] and hand it to the
/// concrete `exec_*::construct_each_*` `drive` closure, serialising each solution's
/// triples into a [`SharedBuf`] and flushing chunks as they fill.
pub fn construct_body_streaming<D>(drive: D, fmt: RdfFormat, deadline: Option<Instant>) -> Body
where
    D: FnOnce(TripleStreamSink) -> BoxedResult + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(CHANNEL_CAP);
    let err_tx = tx.clone();
    tokio::spawn(async move {
        let buf = SharedBuf::default();
        let result: io::Result<()> = async {
            let sink_ser = Arc::new(Mutex::new(Some(TripleSink::start(fmt, buf.clone()))));
            let sink: TripleStreamSink = {
                let sink_ser = Arc::clone(&sink_ser);
                let buf = buf.clone();
                let tx = tx.clone();
                Box::new(move |triples: Vec<Triple>| {
                    let prepared = (|| -> io::Result<Vec<u8>> {
                        check_deadline(deadline)?;
                        let mut guard = sink_ser.lock().unwrap_or_else(|p| p.into_inner());
                        let ser = guard.as_mut().expect("serialiser live during streaming");
                        for t in &triples {
                            ser.write(t)?;
                        }
                        Ok(buf.take_if_full())
                    })();
                    let tx = tx.clone();
                    Box::pin(async move { send_prepared(&tx, prepared).await }) as BoxedResult
                })
            };
            drive(sink)
                .await
                .map_err(|e| io::Error::other(e.to_string()))?;
            let ser = sink_ser
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .expect("serialiser live after streaming");
            ser.finish()?;
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

/// A `std::io::Write` over a shared byte buffer the async streamers serialise into
/// and drain between rows (the serialiser owns one clone; the streamer keeps
/// another). `write` only appends — it never blocks — so it is safe to call from the
/// serialiser on the async runtime; draining + sending is done explicitly at an
/// `.await` point.
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
