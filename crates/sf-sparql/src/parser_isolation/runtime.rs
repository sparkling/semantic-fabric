//! Explicit executable ownership and a synchronous request-local parser scope.
//! Raw compilation outside this scope keeps its caller-owned parser contract.

use super::sql_canonicalize_protocol::SqlDialectCodeV1;
use super::supervisor::{PreparedParserExecutable, SupervisorError};
use sf_core::query_control::{QueryControl, QueryControlError};
use spargebra::Query;
use std::cell::RefCell;
use std::sync::Arc;

#[derive(Clone)]
pub struct ParserRuntime {
    executable: Arc<PreparedParserExecutable>,
    #[cfg(feature = "sql-canonicalize-evidence")]
    sql_evidence: Option<(
        crate::parser_isolation::SqlCanonicalizeEvidenceMode,
        Arc<crate::parser_isolation::SqlCanonicalizeEvidenceState>,
    )>,
}

#[derive(Clone)]
struct RequestParser {
    runtime: ParserRuntime,
    control: Arc<dyn QueryControl>,
}

thread_local! {
    static REQUEST: RefCell<Option<RequestParser>> = const { RefCell::new(None) };
}

struct Restore(Option<RequestParser>);
impl Drop for Restore {
    fn drop(&mut self) {
        REQUEST.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

impl ParserRuntime {
    #[cfg(feature = "runtime-identity-evidence")]
    pub fn prepare_identity_for_evidence(path: &std::path::Path) -> crate::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(startup_error)?;
        Ok(Self {
            executable: Arc::new(
                PreparedParserExecutable::from_file_for_runtime(file).map_err(startup_error)?,
            ),
            #[cfg(feature = "sql-canonicalize-evidence")]
            sql_evidence: None,
        })
    }

    #[cfg(feature = "runtime-identity-evidence")]
    pub fn same_instance_for_evidence(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.executable, &other.executable)
    }

    /// Open and verify the current dispatcher-capable ELF once, before
    /// serving readiness. Launches retain this inode rather than resolving paths.
    pub fn current() -> crate::Result<Self> {
        let runtime = Self {
            executable: Arc::new(
                PreparedParserExecutable::current_for_runtime().map_err(startup_error)?,
            ),
            #[cfg(feature = "sql-canonicalize-evidence")]
            sql_evidence: None,
        };
        runtime.verify_host()?;
        Ok(runtime)
    }

    /// Embeddings supply an absolute, non-symlink dispatcher-capable executable.
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    pub fn prepare(path: &std::path::Path) -> crate::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(startup_error(()));
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(startup_error)?;
        let runtime = Self {
            executable: Arc::new(
                PreparedParserExecutable::from_file_for_runtime(file).map_err(startup_error)?,
            ),
            #[cfg(feature = "sql-canonicalize-evidence")]
            sql_evidence: None,
        };
        runtime.verify_host()?;
        Ok(runtime)
    }

    #[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
    pub fn prepare(_: &std::path::Path) -> crate::Result<Self> {
        Err(startup_error(()))
    }

    fn verify_host(&self) -> crate::Result<()> {
        self.with_request(
            Arc::new(sf_core::query_control::UncontrolledQueryControl),
            || crate::parse_query("ASK {}").map(|_| ()),
        )
    }

    /// Scope only this synchronous compiler closure. Ownership is restored on
    /// success, error and unwinding; neither other threads nor later requests
    /// inherit parser or control authority. No in-process parse fallback occurs.
    pub fn with_request<T>(
        &self,
        control: Arc<dyn QueryControl>,
        work: impl FnOnce() -> crate::Result<T>,
    ) -> crate::Result<T> {
        control.checkpoint()?;
        let prior = REQUEST.with(|slot| {
            slot.replace(Some(RequestParser {
                runtime: self.clone(),
                control: control.clone(),
            }))
        });
        let _restore = Restore(prior);
        let result = work();
        control.checkpoint()?;
        result
    }

    /// Parse and canonically re-render `skeleton` under `dialect` inside a
    /// fresh bounded child, reusing this held executable. Callable from
    /// either compile or execution code; unlike [`Self::with_request`] this
    /// does not touch the `Arc`-owning `REQUEST` scope at all -- `control`
    /// and `work` are threaded explicitly as parameters straight through to
    /// every bounded step (handshake, I/O, pidfd wait/reap), never a
    /// thread-local, so nothing here can be held across an `.await` or
    /// silently override a different invocation's real control.
    async fn canonicalize_sql(
        &self,
        control: &dyn QueryControl,
        dialect: sf_sql::Dialect,
        skeleton: &str,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> crate::Result<String> {
        control.checkpoint()?;
        let code = SqlDialectCodeV1::from_dialect(dialect).ok_or_else(|| {
            crate::Error::Unsupported(format!(
                "{dialect:?} has no governed SQL canonicalize wire code"
            ))
        })?;
        #[cfg(feature = "sql-canonicalize-evidence")]
        let result = if let Some((mode, state)) = &self.sql_evidence {
            self.executable
                .canonicalize_hostile_sql_for_evidence(*mode, state, control, work)
                .await
        } else {
            self.executable
                .canonicalize_sql_public(code, skeleton, control, work)
                .await
        };
        #[cfg(not(feature = "sql-canonicalize-evidence"))]
        let result = self
            .executable
            .canonicalize_sql_public(code, skeleton, control, work)
            .await;
        control.checkpoint()?;
        result.map_err(|error| sql_public_error(error, control))
    }
}

#[cfg(all(
    feature = "sql-canonicalize-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
impl ParserRuntime {
    pub(crate) fn with_sql_evidence(
        &self,
        mode: crate::parser_isolation::SqlCanonicalizeEvidenceMode,
        state: Arc<crate::parser_isolation::SqlCanonicalizeEvidenceState>,
    ) -> Self {
        Self {
            executable: Arc::clone(&self.executable),
            sql_evidence: Some((mode, state)),
        }
    }

    /// Evidence-only accessor for the real round trip (see
    /// `parser_isolation::exercise_sql_canonicalize_for_evidence`). Identical
    /// to what production emission invokes; it only makes the private method
    /// reachable from a focused test behind a non-default feature.
    pub(super) async fn canonicalize_sql_for_evidence(
        &self,
        control: &dyn QueryControl,
        dialect: sf_sql::Dialect,
        skeleton: &str,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> crate::Result<String> {
        self.canonicalize_sql(control, dialect, skeleton, work)
            .await
    }

    pub(super) async fn canonicalize_hostile_sql_for_evidence(
        &self,
        mode: crate::parser_isolation::SqlCanonicalizeEvidenceMode,
        state: &crate::parser_isolation::SqlCanonicalizeEvidenceState,
        control: &dyn QueryControl,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> crate::Result<String> {
        control.checkpoint()?;
        let result = self
            .executable
            .canonicalize_hostile_sql_for_evidence(mode, state, control, work)
            .await;
        control.checkpoint()?;
        result.map_err(|error| sql_public_error(error, control))
    }
}

impl std::fmt::Debug for ParserRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ParserRuntime(<held-executable>)")
    }
}

fn startup_error(_: impl Sized) -> crate::Error {
    crate::Error::Mapping("parser executable is unavailable or invalid".into())
}

pub(crate) fn parse_in_scope(source: &str) -> Option<crate::Result<Query>> {
    let scope = REQUEST.with(|slot| slot.borrow().clone())?;
    Some((|| {
        scope.control.checkpoint()?;
        let result = scope.runtime.executable.parse_public(source);
        scope.control.checkpoint()?;
        result.map_err(|error| public_error(error, scope.control.as_ref()))
    })())
}

/// `None` when `control` carries no `ParserRuntime` capability (raw, offline,
/// CLI and test callers whose `QueryControl` never overrides `capability`),
/// matching `parse_in_scope`'s fallback contract. `Some` covers both success
/// and every propagated failure. Reads `control`'s own request-scoped
/// capability port -- never a process-global -- so two requests admitted
/// across a config reload each keep exactly the runtime their own snapshot
/// observed (see `sf-serve`'s `ServeConfig::request_budget_for` /
/// `RequestBudget::capability`).
pub(crate) async fn canonicalize_sql_in_scope(
    dialect: sf_sql::Dialect,
    skeleton: &str,
    control: &dyn QueryControl,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Option<crate::Result<String>> {
    let runtime = sql_isolation_capability(control)?;
    Some(
        runtime
            .canonicalize_sql(control, dialect, skeleton, work)
            .await,
    )
}

/// Whether `control` carries a `ParserRuntime` capability, without launching
/// anything. Lets a caller commit to (and charge for) the isolated path
/// before actually running it.
pub(crate) fn sql_isolation_capability(control: &dyn QueryControl) -> Option<&ParserRuntime> {
    control
        .capability(std::any::TypeId::of::<ParserRuntime>())?
        .downcast_ref::<ParserRuntime>()
}

fn sql_public_error(error: SupervisorError, control: &dyn QueryControl) -> crate::Error {
    match error {
        SupervisorError::RequestControl(cause) => crate::Error::QueryControl(cause),
        #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
        SupervisorError::SqlRejected(super::sql_canonicalize_protocol::SqlRejectionV1::Syntax) => {
            crate::Error::Sql("isolated SQL canonicalization rejected the emitted skeleton".into())
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
        SupervisorError::SqlRejected(
            super::sql_canonicalize_protocol::SqlRejectionV1::ResourceExhausted,
        ) => control
            .terminate(QueryControlError::CompilerResourceExhausted)
            .into(),
        #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
        SupervisorError::SqlRejected(
            super::sql_canonicalize_protocol::SqlRejectionV1::Envelope,
        ) => control
            .terminate(QueryControlError::CompilerEnvelopeExceeded)
            .into(),
        SupervisorError::DeadlineExceeded => control
            .terminate(QueryControlError::DeadlineExceeded)
            .into(),
        _ => crate::Error::Sql("isolated SQL canonicalization failed".into()),
    }
}

fn public_error(error: SupervisorError, control: &dyn QueryControl) -> crate::Error {
    match error {
        SupervisorError::RequestControl(cause) => crate::Error::QueryControl(cause),
        SupervisorError::ParseRejected(super::parse_protocol::ParseRejectionV1::Syntax) => {
            crate::Error::Parse("invalid SPARQL query".into())
        }
        SupervisorError::ParseRejected(
            super::parse_protocol::ParseRejectionV1::ResourceExhausted,
        ) => control
            .terminate(QueryControlError::CompilerResourceExhausted)
            .into(),
        SupervisorError::ParseRejected(super::parse_protocol::ParseRejectionV1::QueryEnvelope) => {
            control
                .terminate(QueryControlError::CompilerEnvelopeExceeded)
                .into()
        }
        SupervisorError::DeadlineExceeded => control
            .terminate(QueryControlError::DeadlineExceeded)
            .into(),
        #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
        SupervisorError::ParseFrame(
            super::parse_protocol::ParseFrameError::SourceLimitExceeded,
        ) => control
            .terminate(QueryControlError::CompilerEnvelopeExceeded)
            .into(),
        _ => crate::Error::Mapping("isolated parser failed".into()),
    }
}

pub(super) fn checkpoint() -> Result<(), SupervisorError> {
    match REQUEST.with(|slot| {
        slot.borrow().as_ref().map(|scope| {
            scope
                .control
                .checkpoint()
                .map_err(SupervisorError::RequestControl)
        })
    }) {
        Some(result) => result,
        None => Ok(()),
    }
}

pub(super) fn poll_timeout(timeout: i32) -> i32 {
    if REQUEST.with(|slot| slot.borrow().is_some()) {
        timeout.min(10)
    } else {
        timeout
    }
}

#[cfg(test)]
pub(super) fn from_prepared_for_test(executable: PreparedParserExecutable) -> ParserRuntime {
    ParserRuntime {
        executable: Arc::new(executable),
        #[cfg(feature = "sql-canonicalize-evidence")]
        sql_evidence: None,
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[path = "runtime/tests.rs"]
mod tests;
