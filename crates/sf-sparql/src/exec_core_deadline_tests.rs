//! Pull-side cooperative-checkpoint proof for the absolute request deadline.

use std::future::Future;

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use crate::iq::{Branch, Scan, TermDef};
use crate::{Plan, PlanForm};

use super::batch::TERM_GEN_FIRST_BATCH_SIZE;
use super::select_each_async;

struct ReadyBackend {
    rows: Vec<RawTuple>,
}

struct ReadyStream {
    rows: std::vec::IntoIter<RawTuple>,
}

impl BranchStream for ReadyStream {
    async fn next_row(&mut self) -> sf_sql::Result<Option<RawTuple>> {
        Ok(self.rows.next())
    }
}

impl SqlBackend for ReadyBackend {
    type Stream<'s>
        = ReadyStream
    where
        Self: 's;

    async fn column_names(&mut self, _probe: &str) -> sf_sql::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> sf_sql::Result<ReadyStream> {
        Ok(ReadyStream {
            rows: std::mem::take(&mut self.rows).into_iter(),
        })
    }
}

#[test]
fn discarded_ready_rows_reach_a_pull_side_cooperative_checkpoint() {
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
    // Two branches keep OFFSET in the Rust-global path. The ready backend gives
    // the first branch the fixture rows and the second an empty stream.
    let mut second = branch.clone();
    second.core.first_mut().expect("core scan").alias = 1;
    let plan = Plan {
        branches: vec![branch, second],
        form: PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        distinct: false,
        limit: None,
        offset: usize::MAX,
        order: Vec::new(),
        rust_group: None,
        dialect: Dialect::Sqlite,
        dedup_groups: std::collections::HashMap::new(),
        construct_drops_some_branch_var: false,
    };
    let rows = (0..TERM_GEN_FIRST_BATCH_SIZE)
        .map(|i| RawTuple {
            values: vec![Some(i.to_string())],
            codes: vec![None],
        })
        .collect();
    let mut backend = ReadyBackend { rows };
    let sink_calls = std::sync::atomic::AtomicUsize::new(0);
    let mut future = std::pin::pin!(select_each_async(&plan, &mut backend, |_| {
        sink_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::future::ready(Ok(()))
    }));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());

    assert!(matches!(
        future.as_mut().poll(&mut cx),
        std::task::Poll::Pending
    ));
    while future.as_mut().poll(&mut cx).is_pending() {}
    assert_eq!(
        sink_calls.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "OFFSET discards every reconstructed row"
    );
}
