//! Opt-in generated-query admission over the existing compiler entry points.
//!
//! Each entry parses exactly once through `crate::parse_query`, structurally
//! screens that parsed query and runs the caller's constant check before key
//! construction, cache lookup or lowering. The check reruns on every request,
//! cold or warm; its verdict is never cached. It is a checking seam only: not
//! mapping proof, row authority, dataset allowlist semantics or issued identity.
//! A syntax-level parse failure (any UPDATE, or malformed input) is refused as the
//! named `form-not-admitted` rule; resource, control and compile failures keep
//! their original typed errors.
//!
//! The `_deferred` siblings also parse once but report a refusal as data, next to
//! an optional uncached plan that exists only for later row authorization.

use std::fmt;
use std::sync::Arc;

use sf_core::query_control::{QueryControl, QueryControlError};
use spargebra::Query;

use super::generated_query_shape::{visit_parsed_query_constants, ConstantRejection, ShapeRefusal};
use crate::cache::CompilerBinding;
use crate::Plan;

pub use super::generated_query_shape::{ConstantOccurrence, ConstantRole, ShapeRule};

#[path = "generated_dataset.rs"]
mod dataset;

pub use dataset::{
    DatasetAllowlistError, DatasetGraphAllowlist, DatasetRule, GeneratedDatasetError,
    MAX_DATASET_GRAPHS, MAX_DATASET_IRI_BYTES, MAX_GRAPH_IRI_BYTES,
};

/// Failure reported by the caller's constant-coverage check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstantCoverageError {
    /// The constant is not covered; admission is refused.
    Uncovered,
    /// The check hit a request-control failure; it is surfaced as that control error.
    Control(QueryControlError),
}

/// Redacted admission refusal. Carries no query text or constant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratedQueryRefusal {
    /// A named structural rule refused the parsed query.
    Rule(ShapeRule),
    /// The caller's check reported an uncovered constant.
    CoverageRefused,
}

impl fmt::Display for GeneratedQueryRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule(rule) => write!(f, "generated-query admission refused: {}", rule.code()),
            Self::CoverageRefused => f.write_str("generated-query admission refused: coverage"),
        }
    }
}

impl std::error::Error for GeneratedQueryRefusal {}

/// Failure of an opt-in generated-query compile. Refusal and compiler or
/// request-control failures stay distinct.
#[derive(Debug, thiserror::Error)]
pub enum GeneratedCompileError {
    #[error(transparent)]
    Refused(#[from] GeneratedQueryRefusal),
    #[error(transparent)]
    Compiler(#[from] crate::Error),
}

/// Outcome of deferred-refusal generated admission.
///
/// `Admitted` is exactly the plan the non-deferred sibling returns. `Refused`
/// carries the redacted refusal and, when the refused query could be lowered, an
/// uncached authorization-only plan. That plan is diagnostic row-authorization
/// input ONLY: it must never be executed, cached, or treated as admitted, and
/// it confers no identity or source access.
pub enum GeneratedDeferred {
    Admitted(Arc<Plan>),
    Refused {
        refusal: GeneratedQueryRefusal,
        authorization_plan: Option<Arc<Plan>>,
    },
}

impl fmt::Debug for GeneratedDeferred {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Refused { refusal, .. } = self else {
            return f.write_str("Admitted(<plan>)");
        };
        f.debug_struct("Refused")
            .field("refusal", refusal)
            .finish_non_exhaustive()
    }
}

/// Screen `query` structurally and run `check` for every constant occurrence.
/// The check is per-call state: no global, thread-local or stored callback.
pub(crate) fn admit_parsed<F>(
    query: &Query,
    control: &dyn QueryControl,
    mut check: F,
) -> Result<(), GeneratedCompileError>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    let mut failure = None;
    let screened = visit_parsed_query_constants(query, control, |occurrence| {
        check(occurrence).map_err(|error| {
            failure = Some(error);
            ConstantRejection
        })
    });
    match screened {
        Ok(_) => Ok(()),
        Err(ShapeRefusal::Rule(rule)) => Err(GeneratedQueryRefusal::Rule(rule).into()),
        Err(ShapeRefusal::Control(cause)) => Err(crate::Error::QueryControl(cause).into()),
        Err(ShapeRefusal::ConstantRejected(_)) => Err(match failure {
            Some(ConstantCoverageError::Control(cause)) => {
                crate::Error::QueryControl(control.terminate(cause)).into()
            }
            _ => GeneratedQueryRefusal::CoverageRefused.into(),
        }),
    }
}

/// Checkpoint, parse once, then admit. Returns the parsed query for compilation.
pub(crate) fn parse_and_admit<F>(
    sparql: &str,
    control: &dyn QueryControl,
    check: F,
) -> Result<Query, GeneratedCompileError>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    control.checkpoint().map_err(crate::Error::from)?;
    let query = match crate::parse_query(sparql) {
        Ok(query) => query,
        // Only a syntax failure is a form refusal; limit and control errors stay typed.
        Err(crate::Error::Parse(_)) => {
            return Err(GeneratedQueryRefusal::Rule(ShapeRule::FormNotAdmitted).into())
        }
        Err(error) => return Err(error.into()),
    };
    admit_parsed(&query, control, check)?;
    Ok(query)
}

/// Parse-stage result of deferred admission. A refusal keeps the parsed query
/// (absent after a syntax refusal) so the same parse can be lowered once.
pub(crate) enum ParsedAdmission {
    Admitted(Query),
    Refused {
        refusal: GeneratedQueryRefusal,
        query: Option<Query>,
    },
}

/// Checkpoint, parse once, then admit, reporting refusals as data. Control,
/// resource and other compiler failures stay typed errors.
pub(crate) fn parse_and_admit_deferred<F>(
    sparql: &str,
    control: &dyn QueryControl,
    check: F,
) -> crate::Result<ParsedAdmission>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    control.checkpoint()?;
    let query = match crate::parse_query(sparql) {
        Ok(query) => query,
        Err(crate::Error::Parse(_)) => {
            control.checkpoint()?;
            return Ok(ParsedAdmission::Refused {
                refusal: GeneratedQueryRefusal::Rule(ShapeRule::FormNotAdmitted),
                query: None,
            });
        }
        Err(error) => return Err(error),
    };
    match admit_parsed(&query, control, check) {
        Ok(()) => Ok(ParsedAdmission::Admitted(query)),
        Err(GeneratedCompileError::Refused(refusal)) => Ok(ParsedAdmission::Refused {
            refusal,
            query: Some(query),
        }),
        Err(GeneratedCompileError::Compiler(error)) => Err(error),
    }
}

/// Only an unsupported construct may lack an authorization plan; any other
/// lowering failure is never replaced by the refusal.
pub(crate) fn authorization_only(
    lowered: crate::Result<Arc<Plan>>,
) -> crate::Result<Option<Arc<Plan>>> {
    match lowered {
        Ok(plan) => Ok(Some(plan)),
        Err(crate::Error::Unsupported(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

impl CompilerBinding {
    /// Cached compile behind generated-query admission. The parsed query is
    /// screened and `check` runs before key construction, cache lookup and
    /// lowering, so a warm entry never bypasses a later denial. The request
    /// control is used unwrapped: charges, cancellation and deadlines keep
    /// their exact meaning, with screen work added on top of default work.
    pub fn compile_shared_with_generated_admission<F>(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
        check: F,
    ) -> Result<Arc<Plan>, GeneratedCompileError>
    where
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        let query = parse_and_admit(sparql, control, check)?;
        let plan = self.compile_parsed_shared_with_work_control(&query, control)?;
        Ok(plan)
    }

    /// Uncached preflight counterpart. Reads and populates no cache.
    pub fn compile_uncached_shared_with_generated_admission<F>(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
        check: F,
    ) -> Result<Arc<Plan>, GeneratedCompileError>
    where
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        let query = parse_and_admit(sparql, control, check)?;
        let plan = self.compile_parsed_uncached_shared_with_work_control(&query, control)?;
        Ok(plan)
    }

    /// Deferred-refusal sibling of [`Self::compile_shared_with_generated_admission`].
    ///
    /// Parses once. An admitted query takes the unchanged cached path. A refusal
    /// is returned as data; the same parsed query is lowered uncached, with no
    /// cache key, lookup or insertion, solely as row-authorization input. That
    /// plan must never be executed or treated as admitted. Control, mapping, SQL
    /// and core failures propagate typed; only `Unsupported` lowering yields no plan.
    pub fn compile_shared_with_generated_admission_deferred<F>(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
        check: F,
    ) -> crate::Result<GeneratedDeferred>
    where
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        match parse_and_admit_deferred(sparql, control, check)? {
            ParsedAdmission::Admitted(query) => {
                let plan = self.compile_parsed_shared_with_work_control(&query, control)?;
                Ok(GeneratedDeferred::Admitted(plan))
            }
            ParsedAdmission::Refused { refusal, query } => {
                self.defer_refusal(refusal, query, control)
            }
        }
    }

    /// Uncached deferred-refusal counterpart. Reads and populates no cache; the
    /// refused-plan rules of [`Self::compile_shared_with_generated_admission_deferred`] apply.
    pub fn compile_uncached_shared_with_generated_admission_deferred<F>(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
        check: F,
    ) -> crate::Result<GeneratedDeferred>
    where
        F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
    {
        match parse_and_admit_deferred(sparql, control, check)? {
            ParsedAdmission::Admitted(query) => {
                let plan =
                    self.compile_parsed_uncached_shared_with_work_control(&query, control)?;
                Ok(GeneratedDeferred::Admitted(plan))
            }
            ParsedAdmission::Refused { refusal, query } => {
                self.defer_refusal(refusal, query, control)
            }
        }
    }

    pub(crate) fn defer_refusal(
        &self,
        refusal: GeneratedQueryRefusal,
        query: Option<Query>,
        control: &dyn QueryControl,
    ) -> crate::Result<GeneratedDeferred> {
        let authorization_plan = match query {
            Some(query) => {
                let plan = self.compile_parsed_uncached_shared_with_work_control(&query, control);
                authorization_only(plan)?
            }
            None => None,
        };
        control.checkpoint()?;
        Ok(GeneratedDeferred::Refused {
            refusal,
            authorization_plan,
        })
    }
}

#[cfg(test)]
#[path = "generated_compile_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "generated_form_policy_tests.rs"]
mod form_policy_tests;

#[cfg(test)]
#[path = "generated_deferred_tests.rs"]
mod deferred_tests;

#[cfg(test)]
mod test_support {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing::Dispatch;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::fmt::MakeWriter;

    /// Run `test` in an isolated child process so the process-wide tracing callsite
    /// interest cannot be affected by other tests.
    pub(crate) fn isolated(test: impl FnOnce()) {
        const CHILD: &str = "SF_GENERATED_ADMISSION_TEST_CHILD";
        const COMPLETED: i32 = 77;
        let thread = std::thread::current();
        let name = thread.name().expect("named Rust test thread");
        if std::env::var(CHILD).as_deref() == Ok(name) {
            test();
            std::process::exit(COMPLETED);
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture", "--test-threads=1"])
            .env_clear()
            .env(CHILD, name)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(
                    status.code(),
                    Some(COMPLETED),
                    "child must execute all assertions; zero matching tests cannot pass"
                );
                return;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("generated admission child exceeded its bound");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[derive(Clone, Default)]
    struct Capture {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    struct CaptureWriter(Capture);

    impl Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for Capture {
        type Writer = CaptureWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CaptureWriter(self.clone())
        }
    }

    /// Run `operation` under a capturing subscriber and count parse-stage spans.
    /// Only meaningful inside [`isolated`].
    pub(crate) fn parse_spans<T>(operation: impl FnOnce() -> T) -> (T, usize) {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_span_events(FmtSpan::NEW)
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::INFO)
            .finish();
        let value = tracing::dispatcher::with_default(&Dispatch::new(subscriber), operation);
        let output = String::from_utf8(capture.bytes.lock().unwrap().clone()).unwrap();
        (value, output.matches("\"stage\":\"parse\"").count())
    }
}
