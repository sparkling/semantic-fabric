//! Explicit executable ownership and a synchronous request-local parser scope.
//! Raw compilation outside this scope keeps its caller-owned parser contract.

use super::supervisor::{PreparedParserExecutable, SupervisorError};
use sf_core::query_control::{QueryControl, QueryControlError};
use spargebra::Query;
use std::cell::RefCell;
use std::sync::Arc;

#[derive(Clone)]
pub struct ParserRuntime(Arc<PreparedParserExecutable>);

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
    /// Open and verify the current dispatcher-capable ELF once, before
    /// serving readiness. Launches retain this inode rather than resolving paths.
    pub fn current() -> crate::Result<Self> {
        let runtime = Self(Arc::new(
            PreparedParserExecutable::current_for_runtime().map_err(startup_error)?,
        ));
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
        let runtime = Self(Arc::new(
            PreparedParserExecutable::from_file_for_runtime(file).map_err(startup_error)?,
        ));
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
        let result = scope.runtime.0.parse_public(source);
        scope.control.checkpoint()?;
        result.map_err(|error| public_error(error, scope.control.as_ref()))
    })())
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
    REQUEST.with(|slot| match slot.borrow().as_ref() {
        Some(scope) => scope
            .control
            .checkpoint()
            .map_err(SupervisorError::RequestControl),
        None => Ok(()),
    })
}

pub(super) fn poll_timeout(timeout: i32) -> i32 {
    REQUEST.with(|slot| {
        if slot.borrow().is_some() {
            timeout.min(10)
        } else {
            timeout
        }
    })
}

#[cfg(test)]
pub(super) fn from_prepared_for_test(executable: PreparedParserExecutable) -> ParserRuntime {
    ParserRuntime(Arc::new(executable))
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[path = "runtime/tests.rs"]
mod tests;
