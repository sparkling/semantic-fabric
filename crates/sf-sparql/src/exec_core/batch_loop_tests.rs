//! `run_branches`' buffer -> parallel-map -> emit-in-order loop (batch-fill,
//! the `first_batch` ramp, batch-exhaustion detection) end to end through
//! [`select`], over a stream spanning many batches — a plan-level mock
//! backend, not a real SQL source, so this is fast and deterministic.
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::Term;
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use crate::iq::{Branch, OrderKey, Scan, TermDef};
use crate::{Plan, PlanForm};

use super::batch::TERM_GEN_BATCH_SIZE;
use super::select;

struct MockBackend {
    rows: Vec<RawTuple>,
}
struct MockStream {
    iter: std::vec::IntoIter<RawTuple>,
}
impl BranchStream for MockStream {
    async fn next_row(&mut self) -> sf_sql::Result<Option<RawTuple>> {
        Ok(self.iter.next())
    }
}
impl SqlBackend for MockBackend {
    type Stream<'s>
        = MockStream
    where
        Self: 's;
    async fn column_names(&mut self, _probe: &str) -> sf_sql::Result<Vec<String>> {
        Ok(vec!["val".to_owned()])
    }
    async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> sf_sql::Result<MockStream> {
        Ok(MockStream {
            iter: std::mem::take(&mut self.rows).into_iter(),
        })
    }
}

/// ORDER BY over a stream spanning several `TERM_GEN_BATCH_SIZE` batches
/// (plus the small first-batch ramp): rows arrive in STRICTLY REVERSED value
/// order, so a correct result requires the plan-wide sort buffer
/// (`run_branches`' `buffer`) to have accumulated EVERY row across EVERY
/// batch-fill iteration — a bug that reset or truncated it per batch would
/// fail this, where a single-batch-sized fixture could not catch it.
#[test]
fn order_by_spans_multiple_batches_correctly() {
    let n = 2 * TERM_GEN_BATCH_SIZE + 137;
    let mut branch = Branch::single(Scan {
        alias: 0,
        source: LogicalSource::Table("t".to_owned()),
    });
    branch.bindings.insert(
        "v".to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column("val".into(), TermSpec::plain_literal()),
            alias: 0,
        },
    );
    let plan = Plan {
        branches: vec![branch],
        form: PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![OrderKey {
            var: "v".to_owned(),
            descending: false,
            expr: None,
        }],
        rust_group: None,
        dialect: Dialect::Sqlite,
        dedup_scopes: Vec::new(),
        construct_drops_some_branch_var: false,
    };
    // Reversed, zero-padded so lexical order (plain-literal comparison)
    // matches numeric order: row k carries the value belonging at sorted
    // position n-1-k.
    let rows: Vec<RawTuple> = (0..n)
        .map(|k| RawTuple {
            values: vec![Some(format!("{:07}", n - 1 - k))],
            codes: vec![None],
        })
        .collect();
    let mut backend = MockBackend { rows };

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let sol = rt.block_on(select(&plan, &mut backend)).unwrap();

    assert_eq!(
        sol.rows.len(),
        n,
        "no row dropped/duplicated across batches"
    );
    let expected: Vec<String> = (0..n).map(|i| format!("{i:07}")).collect();
    let actual: Vec<String> = sol
        .rows
        .iter()
        .map(|row| match &row[0] {
            Some(Term::Literal(l)) => l.value().to_owned(),
            other => panic!("expected a literal, got {other:?}"),
        })
        .collect();
    assert_eq!(
        actual, expected,
        "ORDER BY must span every batch, not just within one"
    );
}
