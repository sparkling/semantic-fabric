//! Native MySQL cancellation with owned, session-bound control and hard discard.
use crate::budget::RequestBudget;
use mysql_async::{prelude::Queryable, Conn, Opts, OptsBuilder};
use sf_core::query_control::{QueryCharge, QueryControl};
use sf_core::{Term, Triple};
use sf_sparql::{Error, Plan, Result};
use sf_sql::backend::mysql::MysqlBackend;
use std::{
    borrow::{Borrow, BorrowMut},
    future::Future,
    task::{Context, Waker},
    time::Duration,
};

const CLEANUP: Duration = Duration::from_secs(2);
mod generation;

// Pinned mysql_async disconnect sets its disconnected flag before the first
// await. Poll once even on runtime shutdown/expired cleanup: merely dropping
// an unpolled disconnect future would hand unread work to the pool recycler.
fn poll_once(future: impl Future) {
    let mut future = std::pin::pin!(future);
    let _ = future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
}
struct Discard(Option<Conn>);
impl Discard {
    fn conn(&mut self) -> &mut Conn {
        self.0.as_mut().expect("owned connection")
    }
}
impl Drop for Discard {
    fn drop(&mut self) {
        if let Some(conn) = self.0.take() {
            poll_once(conn.disconnect());
        }
    }
}

pub(crate) struct MysqlQuery {
    target: Discard,
    marker: String,
    original_timeout: u64,
    budget: RequestBudget,
    reusable: bool,
    generation: Option<(
        std::sync::Arc<[String]>,
        std::sync::Arc<crate::mysql_generation::schema::Schema>,
    )>,
}
impl Borrow<Conn> for MysqlQuery {
    fn borrow(&self) -> &Conn {
        self.target.0.as_ref().expect("owned query")
    }
}
impl BorrowMut<Conn> for MysqlQuery {
    fn borrow_mut(&mut self) -> &mut Conn {
        self.target.conn()
    }
}

fn nonce() -> Result<String> {
    let mut bytes = [0; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| failure())?;
    use std::fmt::Write;
    let mut name = String::from("sf-stop-");
    for byte in bytes {
        write!(&mut name, "{byte:02x}").unwrap();
    }
    Ok(name)
}
fn failure() -> Error {
    Error::Sql("native source control failed".into())
}
fn milliseconds(budget: &RequestBudget) -> Result<u64> {
    Ok(budget
        .remaining_duration()?
        .unwrap_or(Duration::from_secs(30))
        .as_millis()
        .saturating_add(1)
        .clamp(1, u32::MAX as u128) as u64)
}
async fn pin(conn: &mut Conn, name: &str) -> Result<()> {
    let acquired: Option<Option<u8>> = conn
        .exec_first("SELECT GET_LOCK(?, 0)", (name,))
        .await
        .map_err(|_| failure())?;
    let owner: Option<Option<u32>> = conn
        .exec_first("SELECT IS_USED_LOCK(?)", (name,))
        .await
        .map_err(|_| failure())?;
    if acquired == Some(Some(1)) && owns_session(owner, conn.id()) {
        Ok(())
    } else {
        Err(failure())
    }
}

fn owns_session(owner: Option<Option<u32>>, id: u32) -> bool {
    owner == Some(Some(id))
}

fn control_options(options: Opts) -> Opts {
    // No setup queries may run inside Conn::new before Discard owns the
    // connection. Preserve endpoint/authentication/TLS, but never replay caller
    // setup or discover driver settings with a result that could outlive timeout.
    OptsBuilder::from_opts(options)
        .max_allowed_packet(Some(16 * 1024))
        .wait_timeout(Some(3))
        .prefer_socket(false)
        .socket::<String>(None)
        .init(Vec::<String>::new())
        .setup(Vec::<String>::new())
        .after_connect(|_| Box::pin(async { Ok(()) }))
        .into()
}

pub(crate) fn target_options(options: Opts) -> std::result::Result<Opts, &'static str> {
    if !options.init().is_empty()
        || !options.setup().is_empty()
        || options.after_connect().is_some()
    {
        return Err("MySQL constructor queries are outside the owned serving profile");
    }
    // Explicit client-side settings avoid unowned discovery queries in both
    // initial and replacement pool connections. The idle TTL is NOT a native
    // statement timeout. Preserve deliberate packet/idle settings.
    let packet = options.max_allowed_packet().unwrap_or(4 * 1024 * 1024);
    let idle = options.wait_timeout().unwrap_or(30);
    Ok(OptsBuilder::from_opts(options)
        .max_allowed_packet(Some(packet))
        .wait_timeout(Some(idle))
        .prefer_socket(false)
        .socket::<String>(None)
        .into())
}

impl MysqlQuery {
    pub(crate) async fn acquire(conn: Conn, budget: RequestBudget) -> Result<Self> {
        Self::acquire_inner(conn, budget, false).await
    }

    async fn acquire_inner(conn: Conn, budget: RequestBudget, reset: bool) -> Result<Self> {
        // The discard owner is built before any fallible setup or first await.
        let target = Discard(Some(conn));
        let marker = nonce()?;
        let mut query = Self {
            target,
            marker,
            original_timeout: 0,
            budget,
            reusable: false,
            generation: None,
        };
        let budget = query.budget.clone();
        budget
            .run(async {
                if reset {
                    budget.consume(QueryCharge::SourceWork, 16)?;
                    if !query.target.conn().reset().await.map_err(|_| failure())? {
                        return Err(failure());
                    }
                }
                pin(query.target.conn(), &query.marker).await?;
                query.original_timeout = query
                    .target
                    .conn()
                    .query_first("SELECT @@session.max_execution_time")
                    .await
                    .map_err(|_| failure())?
                    .ok_or_else(failure)?;
                let mut limit = milliseconds(&budget)?;
                if query.original_timeout != 0 {
                    limit = limit.min(query.original_timeout);
                }
                query
                    .target
                    .conn()
                    .query_drop(format!("SET SESSION max_execution_time = {limit}"))
                    .await
                    .map_err(|_| failure())
            })
            .await??;
        Ok(query)
    }

    pub(crate) async fn finish<T>(mut self, result: Result<T>) -> Result<T> {
        let value = result?;
        let budget = self.budget.clone();
        budget
            .run(async {
                self.close_generation().await?;
                // query_first drains preceding unread results before this barrier.
                self.target
                    .conn()
                    .query_drop(format!(
                        "SET SESSION max_execution_time = {}",
                        self.original_timeout
                    ))
                    .await
                    .map_err(|_| failure())?;
                let released: Option<Option<u8>> = self
                    .target
                    .conn()
                    .exec_first("SELECT RELEASE_LOCK(?)", (&self.marker,))
                    .await
                    .map_err(|_| failure())?;
                if released == Some(Some(1)) {
                    Ok(())
                } else {
                    Err(failure())
                }
            })
            .await??;
        self.reusable = true;
        Ok(value)
    }
}
impl Drop for MysqlQuery {
    fn drop(&mut self) {
        if self.reusable {
            // Only acknowledged draining, timeout restoration and lock release
            // can return this connection to normal pool recycling.
            drop(self.target.0.take());
            return;
        }
        let Some(conn) = self.target.0.take() else {
            return;
        };
        let id = conn.id();
        let options = control_options(conn.opts().clone());
        let target = Discard(Some(conn));
        let marker = self.marker.clone();
        let budget = self.budget.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(CLEANUP, async {
                    let control = Conn::new(options).await.map_err(|_| failure())?;
                    let mut control = Discard(Some(control));
                    // Pin both sessions before the witness/KILL sequence. This
                    // supports session-affine endpoints, not arbitrary proxies.
                    pin(control.conn(), &nonce()?).await?;
                    let owner: Option<Option<u32>> = control
                        .conn()
                        .exec_first("SELECT IS_USED_LOCK(?)", (&marker,))
                        .await
                        .map_err(|_| failure())?;
                    if !owns_session(owner, id) {
                        return Err(failure());
                    }
                    // The number is exclusively the native driver's u32, never
                    // authored/request SQL. Keep target owned; never retry KILL.
                    control
                        .conn()
                        .query_drop(format!("KILL QUERY {id}"))
                        .await
                        .map_err(|_| failure())
                })
                .await;
                // KILL acknowledges a flag, not termination. Discard regardless
                // of outcome; native timeout is a separate SELECT-only bound.
                drop(target);
                drop(budget);
            });
        }
    }
}

pub(crate) async fn select<F, Fut>(
    plan: &Plan,
    query: MysqlQuery,
    control: &dyn QueryControl,
    sink: F,
) -> Result<()>
where
    F: FnMut(Vec<Option<Term>>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut backend = MysqlBackend::new(query);
    let result =
        sf_sparql::exec_core::select_each_async_controlled(plan, &mut backend, control, sink).await;
    backend.into_inner().finish(result).await
}
pub(crate) async fn construct<F, Fut>(
    plan: &Plan,
    query: MysqlQuery,
    control: &dyn QueryControl,
    sink: F,
) -> Result<()>
where
    F: FnMut(Vec<Triple>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let mut backend = MysqlBackend::new(query);
    let result =
        sf_sparql::exec_core::construct_each_async_controlled(plan, &mut backend, control, sink)
            .await;
    backend.into_inner().finish(result).await
}
pub(crate) async fn ask(
    plan: &Plan,
    query: MysqlQuery,
    control: &dyn QueryControl,
) -> Result<bool> {
    let mut backend = MysqlBackend::new(query);
    let result = sf_sparql::exec_core::ask_controlled(plan, &mut backend, control).await;
    backend.into_inner().finish(result).await
}

pub(crate) async fn lineage(
    plan: &Plan,
    spec: &sf_sparql::lineage::LineageSpec,
    query: MysqlQuery,
    control: &dyn QueryControl,
    sink: crate::stream::OriginSink,
) -> Result<()> {
    let mut backend = MysqlBackend::new(query);
    let result = sf_sparql::exec_core::lineage_each_async_controlled(
        plan,
        spec,
        &mut backend,
        control,
        sink,
    )
    .await;
    backend.into_inner().finish(result).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discard_polls_before_dropping_pending_disconnect() {
        struct Probe<'a>(&'a mut bool);
        impl Future for Probe<'_> {
            type Output = ();
            fn poll(
                mut self: std::pin::Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> std::task::Poll<()> {
                *self.0 = true;
                std::task::Poll::Pending
            }
        }
        impl Drop for Probe<'_> {
            fn drop(&mut self) {
                assert!(*self.0, "unpolled disconnect may recycle");
            }
        }
        poll_once(Probe(&mut false));
    }
    #[test]
    fn control_names_are_independent_bounded_and_not_caller_values() {
        let a = nonce().unwrap();
        let b = nonce().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 40);
    }
    #[test]
    fn missing_null_and_mismatched_witnesses_never_authorize_kill() {
        for owner in [None, Some(None), Some(Some(8))] {
            assert!(!owns_session(owner, 7));
        }
        assert!(owns_session(Some(Some(7)), 7));
    }
    #[test]
    fn control_constructor_has_no_query_settings_or_caller_setup() {
        let original: Opts = OptsBuilder::default()
            .init(vec!["SELECT 1"])
            .setup(vec!["SELECT 2"])
            .after_connect(|_| Box::pin(async { panic!("caller callback must not run") }))
            .into();
        let options = control_options(original);
        assert!(options.max_allowed_packet().is_some());
        assert!(options.wait_timeout().is_some());
        assert!(!options.prefer_socket());
        assert!(options.socket().is_none());
        assert!(options.init().is_empty());
        assert!(options.setup().is_empty());
        // The replacing callback is deliberately trivial; the original must
        // not escape into native constructor execution. No connection is made.
        assert!(options.after_connect().is_some());
    }
    #[test]
    fn target_constructor_skips_discovery_without_overriding_explicit_limits() {
        let defaults = target_options(Opts::default()).unwrap();
        assert_eq!(defaults.max_allowed_packet(), Some(4 * 1024 * 1024));
        assert_eq!(defaults.wait_timeout(), Some(30));
        let explicit = target_options(
            OptsBuilder::default()
                .max_allowed_packet(Some(8 * 1024 * 1024))
                .wait_timeout(Some(60))
                .into(),
        )
        .unwrap();
        assert_eq!(explicit.max_allowed_packet(), Some(8 * 1024 * 1024));
        assert_eq!(explicit.wait_timeout(), Some(60));
        assert!(target_options(OptsBuilder::default().init(vec!["SELECT 1"]).into()).is_err());
        assert!(target_options(OptsBuilder::default().setup(vec!["SELECT 1"]).into()).is_err());
        assert!(target_options(
            OptsBuilder::default()
                .after_connect(|_| Box::pin(async { Ok(()) }))
                .into()
        )
        .is_err());
    }
}
