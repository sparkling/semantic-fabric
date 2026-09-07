//! Lazy, one-page-at-a-time row cursor over `GetQueryResults`.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use crate::backend::{BranchStream, RawTuple};
use crate::error::Result;

use super::client::AthenaSession;
use super::wire::{self, ResultPage};

/// A bounded pull cursor over one Athena query's result set.
///
/// At most ONE page is buffered: the next `GetQueryResults` call is issued only
/// after the current page's rows have been fully drained, so memory stays
/// proportional to `AthenaConfig::with_page_size`, not to the result set.
///
/// The `NextToken` is opaque and is only ever sent as a JSON body field to the
/// same fixed, signed endpoint — it never becomes part of a URL, so a hostile
/// token cannot redirect a request.
///
/// Dropping the stream does NOT cancel the query at AWS. Athena has no
/// "close cursor" call, `Drop` cannot await a signed HTTP round trip, and the
/// query has already succeeded by the time a stream exists. Cancellation is
/// only issued on a deadline overrun during polling
/// (see [`AthenaBackend`](super::AthenaBackend)).
pub struct AthenaStream {
    session: Arc<AthenaSession>,
    query_execution_id: String,
    columns: Vec<String>,
    rows: VecDeque<RawTuple>,
    next_token: Option<String>,
    token_cycle: TokenCycleDetector,
    deadline: Instant,
}

/// Brent cycle detection over the token sequence. This catches cycles of any
/// length while retaining one token rather than every page token.
struct TokenCycleDetector {
    anchor: Option<String>,
    power: u64,
    span: u64,
}

impl TokenCycleDetector {
    fn observe(&mut self, token: &str) -> bool {
        let Some(anchor) = self.anchor.as_deref() else {
            self.anchor = Some(token.to_owned());
            self.power = 1;
            return false;
        };
        if anchor == token {
            return true;
        }
        self.span = self.span.saturating_add(1);
        if self.span == self.power {
            self.anchor = Some(token.to_owned());
            self.power = self.power.saturating_mul(2);
            self.span = 0;
        }
        false
    }
}

impl fmt::Debug for AthenaStream {
    /// Reports paging progress only. The session behind this stream holds
    /// credentials, so nothing from it is printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AthenaStream")
            .field("query_execution_id", &self.query_execution_id)
            .field("columns", &self.columns)
            .field("buffered_rows", &self.rows.len())
            .field("has_next_page", &self.next_token.is_some())
            .finish_non_exhaustive()
    }
}

impl AthenaStream {
    pub(crate) fn new(
        session: Arc<AthenaSession>,
        query_execution_id: String,
        first_page: ResultPage,
        deadline: Instant,
    ) -> Self {
        Self {
            session,
            query_execution_id,
            columns: first_page.columns,
            rows: first_page.rows.into(),
            next_token: first_page.next_token,
            token_cycle: TokenCycleDetector {
                anchor: None,
                power: 0,
                span: 0,
            },
            deadline,
        }
    }

    /// Result-column names, in projection order, from `ResultSetMetadata`.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// Fetch exactly one further page. Returns `false` once paging is done.
    async fn fetch_next_page(&mut self) -> Result<bool> {
        let Some(token) = self.next_token.take() else {
            return Ok(false);
        };
        // A repeated token means the service is not making progress; following
        // it would loop forever inside `next_row`.
        if self.token_cycle.observe(&token) {
            return Err(self.session.err(format!(
                "athena GetQueryResults: query {} returned a cyclic NextToken; \
                 refusing to page in a loop",
                self.query_execution_id
            )));
        }
        let body = wire::get_query_results_body(
            &self.query_execution_id,
            self.session.config.page_size,
            Some(&token),
        );
        let response = self
            .session
            .call("GetQueryResults", &body, self.deadline)
            .await?;
        let page = wire::parse_result_page(&response, Some(&self.columns))
            .map_err(|m| self.session.err(format!("athena {m}")))?;
        self.next_token = page.next_token;
        self.rows.extend(page.rows);
        Ok(true)
    }
}

impl BranchStream for AthenaStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        loop {
            if let Some(row) = self.rows.pop_front() {
                return Ok(Some(row));
            }
            // Only now — with the buffer drained — is another page fetched.
            if !self.fetch_next_page().await? {
                return Ok(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TokenCycleDetector;

    fn detector() -> TokenCycleDetector {
        TokenCycleDetector {
            anchor: None,
            power: 0,
            span: 0,
        }
    }

    #[test]
    fn detects_adjacent_and_multi_token_cycles_with_constant_state() {
        let mut adjacent = detector();
        assert!(!adjacent.observe("A"));
        assert!(adjacent.observe("A"));

        let mut alternating = detector();
        assert!(!alternating.observe("A"));
        assert!(!alternating.observe("B"));
        assert!(!alternating.observe("A"));
        assert!(alternating.observe("B"));

        let mut three = detector();
        let observations: Vec<bool> = ["A", "B", "C", "A", "B", "C", "A"]
            .iter()
            .map(|token| three.observe(token))
            .collect();
        assert!(observations.last().copied().unwrap());
    }
}
