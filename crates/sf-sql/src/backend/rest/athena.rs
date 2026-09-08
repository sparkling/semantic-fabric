//! AWS Athena backend speaking the **real** Athena API (AWS JSON 1.1 over
//! SigV4), replacing the earlier Presto-protocol placeholder (ADR-0036 #7).
//!
//! # Protocol
//!
//! Every operation is an HTTPS `POST` to one fixed regional endpoint
//! (`https://athena.{region}.amazonaws.com`, overridable via
//! [`AthenaConfig::with_endpoint`]) carrying
//! `Content-Type: application/x-amz-json-1.1` and
//! `X-Amz-Target: AmazonAthena.{Operation}`, signed with SigV4 for service
//! `athena` in the configured region using the official `aws-sigv4` crate.
//! One logical execution is `StartQueryExecution` → poll `GetQueryExecution`
//! → page `GetQueryResults`.
//!
//! # Parameters are LEXICAL, not typed
//!
//! `SqlBackend::open_branch` supplies untyped lexical strings, so this backend
//! cannot offer typed server-side binding. Values are passed in Athena's
//! `ExecutionParameters` (never substituted into the SQL text client-side), and
//! each is rendered as a single-quoted Athena **string** literal with `''`
//! escaping. Comparisons against non-string columns therefore need an explicit
//! cast in the emitted SQL. See
//! [`params::render_execution_parameters`](params) for the exact rules;
//! placeholder-count mismatches and unterminated quoted text are rejected.
//!
//! # Billing note
//!
//! [`SqlBackend::column_names`] has no metadata-only path in the Athena API: it
//! runs the probe query (`SELECT * FROM t LIMIT 0` for a table source, or the
//! R2RML `rr:sqlQuery` verbatim) as a real, **billed** Athena execution and
//! reads the column metadata off the first result page. Athena's minimum
//! per-query data scan applies.
//!
//! # Cancellation
//!
//! A deadline overrun while polling issues `StopQueryExecution`, and a failure
//! to stop is reported alongside the original failure. Dropping an
//! [`AthenaStream`] is NOT remote cancellation: `Drop` cannot await a signed
//! HTTP round trip, and by then the query has already succeeded. A deadline
//! overrun *before* `StartQueryExecution` returns leaves nothing to stop because
//! there is no `QueryExecutionId` yet, so the execution may still run at AWS.
//! Cancelling the `open_branch` or `column_names` future after
//! `StartQueryExecution` returns also prevents this library from awaiting
//! `StopQueryExecution`; cancellation-safe serving integration remains an
//! admission gate.
//!
//! # Verification tier
//!
//! Compile + unit + mocked-HTTP integration (`tests/athena_rest.rs`).
//! Live-parity requires a real AWS account.

mod client;
mod config;
mod credentials;
mod params;
#[cfg(test)]
mod robustness_tests;
mod sign;
mod stream;
mod wire;

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use crate::backend::SqlBackend;
use crate::error::{Error, Result};

pub use config::AthenaConfig;
pub use credentials::AthenaCredentials;
pub use stream::AthenaStream;

use client::AthenaSession;
use sign::client_request_token;
use wire::QueryState;

/// `SqlBackend` over the AWS Athena JSON 1.1 API. See the module docs.
pub struct AthenaBackend {
    session: Arc<AthenaSession>,
}

impl fmt::Debug for AthenaBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AthenaBackend")
            .field("endpoint", &self.session.config.endpoint())
            .field("region", &self.session.config.region())
            .field("workgroup", &self.session.config.workgroup)
            .finish_non_exhaustive()
    }
}

impl AthenaBackend {
    /// Build from an explicit config + credentials pair.
    pub fn new(config: AthenaConfig, credentials: AthenaCredentials) -> Result<Self> {
        Ok(Self {
            session: Arc::new(AthenaSession::new(config, credentials)?),
        })
    }

    /// Build from the environment — see [`AthenaConfig::from_env`] and
    /// [`AthenaCredentials::from_env`].
    pub fn from_env() -> Result<Self> {
        Self::new(AthenaConfig::from_env()?, AthenaCredentials::from_env()?)
    }

    /// The effective configuration (credentials are not exposed).
    pub fn config(&self) -> &AthenaConfig {
        &self.session.config
    }

    /// Start → poll → first page. The returned stream lazily pages the rest.
    async fn execute(&self, sql: &str, lexical_params: &[String]) -> Result<AthenaStream> {
        let session = &self.session;
        let execution_parameters = params::render_execution_parameters(sql, lexical_params)
            .map_err(|m| session.err(format!("athena: {m}")))?;
        let deadline = Instant::now() + session.config.total_deadline;

        // One token per LOGICAL execution; `call` replays this exact body (and
        // therefore this exact token) on retries, so a retried start is
        // idempotent at Athena rather than launching a second query.
        let token = client_request_token();
        let start = wire::start_query_body(&session.config, sql, &token, &execution_parameters);
        let started = session
            .call("StartQueryExecution", &start, deadline)
            .await?;
        let query_execution_id = wire::parse_query_execution_id(&started)
            .map_err(|m| session.err(format!("athena: {m}")))?;

        self.await_success(&query_execution_id, deadline).await?;

        let request =
            wire::get_query_results_body(&query_execution_id, session.config.page_size, None);
        let response = session.call("GetQueryResults", &request, deadline).await?;
        let mut page = wire::parse_result_page(&response, None)
            .map_err(|m| session.err(format!("athena: {m}")))?;
        wire::strip_header_row(&mut page);

        Ok(AthenaStream::new(
            Arc::clone(session),
            query_execution_id,
            page,
            deadline,
        ))
    }

    /// Poll `GetQueryExecution` until the query succeeds, fails, or the overall
    /// deadline expires.
    async fn await_success(&self, query_execution_id: &str, deadline: Instant) -> Result<()> {
        let session = &self.session;
        let request = wire::query_execution_body(query_execution_id);
        loop {
            if Instant::now() >= deadline {
                let cause = session.err(format!(
                    "athena: total deadline exceeded while polling query {query_execution_id}"
                ));
                return Err(self.stop_after(query_execution_id, cause).await);
            }
            let response = match session.call("GetQueryExecution", &request, deadline).await {
                Ok(response) => response,
                Err(cause) => return Err(self.stop_after(query_execution_id, cause).await),
            };
            let state = match wire::parse_query_state(&response) {
                Ok(state) => state,
                Err(message) => {
                    let cause = session.err(format!("athena: {message} (id {query_execution_id})"));
                    return Err(self.stop_after(query_execution_id, cause).await);
                }
            };
            match state {
                QueryState::Succeeded => return Ok(()),
                QueryState::Pending => {}
                QueryState::Failed(reason) => {
                    return Err(session.err(format!("athena: query FAILED: {reason}")));
                }
                QueryState::Cancelled(reason) => {
                    return Err(session.err(format!("athena: query CANCELLED: {reason}")));
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let cause = session.err(format!(
                    "athena: total deadline exceeded while polling query {query_execution_id}"
                ));
                return Err(self.stop_after(query_execution_id, cause).await);
            }
            tokio::time::sleep(session.config.poll_interval.min(remaining)).await;
        }
    }

    /// Best-effort `StopQueryExecution` for a query abandoned on a deadline.
    /// A failure to stop is surfaced ALONGSIDE the original failure, never
    /// swallowed: the query may still be running (and billing) at AWS.
    async fn stop_after(&self, query_execution_id: &str, cause: Error) -> Error {
        let session = &self.session;
        let request = wire::query_execution_body(query_execution_id);
        // The overall deadline is already spent, so give the stop its own small
        // budget rather than failing it instantly.
        let stop_deadline = Instant::now() + session.config.request_timeout;
        match session
            .call("StopQueryExecution", &request, stop_deadline)
            .await
        {
            Ok(_) => session.err(format!(
                "{cause}; StopQueryExecution was issued for {query_execution_id}"
            )),
            Err(stop) => session.err(format!(
                "{cause}; StopQueryExecution ALSO failed, the query may still be running: {stop}"
            )),
        }
    }
}

impl SqlBackend for AthenaBackend {
    type Stream<'s>
        = AthenaStream
    where
        Self: 's;

    /// Runs `probe_sql` as a real (billed) Athena query and returns its
    /// result-column names. Only the first result page is fetched; the stream
    /// is dropped immediately afterwards.
    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        let stream = self.execute(probe_sql, &[]).await?;
        Ok(stream.columns().to_vec())
    }

    async fn open_branch(&mut self, sql: &str, lexical_params: &[String]) -> Result<AthenaStream> {
        self.execute(sql, lexical_params).await
    }
}

#[cfg(test)]
mod tests {
    use super::{AthenaBackend, AthenaConfig, AthenaCredentials};

    fn backend() -> AthenaBackend {
        AthenaBackend::new(
            AthenaConfig::new("us-east-1")
                .unwrap()
                .with_database("gtfs")
                .unwrap(),
            AthenaCredentials::new("AKIDEXAMPLE", "topsecret").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn debug_exposes_endpoint_but_never_credentials() {
        let rendered = format!("{:?}", backend());
        assert!(
            rendered.contains("athena.us-east-1.amazonaws.com"),
            "{rendered}"
        );
        assert!(!rendered.contains("topsecret"), "{rendered}");
        assert!(!rendered.contains("AKIDEXAMPLE"), "{rendered}");
    }

    #[test]
    fn config_accessor_reports_the_effective_settings() {
        let backend = backend();
        assert_eq!(backend.config().region(), "us-east-1");
        assert_eq!(
            backend.config().endpoint(),
            "https://athena.us-east-1.amazonaws.com"
        );
    }
}
