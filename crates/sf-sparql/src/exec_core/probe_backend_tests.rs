//! Fail-fast device: a throwaway `'static` backend that proves AFIT + GAT + the
//! generic async sink monomorphize to a **`Send`** future — the one novel
//! language-feature risk — before any live-DB adapter is exercised.
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use crate::{Plan, PlanForm};

use super::ask;

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
        Ok(Vec::new())
    }
    async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> sf_sql::Result<MockStream> {
        Ok(MockStream {
            iter: std::mem::take(&mut self.rows).into_iter(),
        })
    }
}

fn assert_send<T: Send>(t: T) -> T {
    t
}

#[test]
fn send_future_monomorphizes_and_spawns() {
    // Monomorphizes `run::<MockBackend>` (here: `ask`) and proves the future is
    // `Send + 'static` enough to `tokio::spawn` — the M2 exit gate half (ii).
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let plan = Plan {
        branches: Vec::new(),
        form: PlanForm::Ask,
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect: Dialect::Sqlite,
        dedup_groups: std::collections::HashMap::new(),
        construct_drops_some_branch_var: false,
    };
    rt.block_on(async move {
        let backend = MockBackend { rows: Vec::new() };
        let fut = async move {
            let mut b = backend;
            ask(&plan, &mut b).await
        };
        let joined = tokio::spawn(assert_send(fut)).await.unwrap();
        assert!(!joined.unwrap());
    });
}
