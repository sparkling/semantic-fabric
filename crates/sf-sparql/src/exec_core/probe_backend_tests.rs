//! Mock boundary tests for the backend-generic execution preflight.
use std::collections::VecDeque;

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use crate::iq::{
    Branch, ColRef, HopExpr, HopRelation, PathClosure, PathKind, Scan, SqlCond, TermDef,
};
use crate::{DedupScope, Plan, PlanForm};

use super::{ask, select};

struct MockBackend {
    rows: VecDeque<Vec<RawTuple>>,
    metadata: VecDeque<sf_sql::Result<Vec<String>>>,
    probes: Vec<String>,
    opens: usize,
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
    async fn column_names(&mut self, probe: &str) -> sf_sql::Result<Vec<String>> {
        self.probes.push(probe.to_owned());
        self.metadata.pop_front().unwrap_or_else(|| Ok(Vec::new()))
    }
    async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> sf_sql::Result<MockStream> {
        self.opens += 1;
        Ok(MockStream {
            iter: self.rows.pop_front().unwrap_or_default().into_iter(),
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
        dedup_scopes: Vec::new(),
        construct_drops_some_branch_var: false,
    };
    rt.block_on(async move {
        let backend = MockBackend {
            rows: VecDeque::new(),
            metadata: VecDeque::new(),
            probes: Vec::new(),
            opens: 0,
        };
        let fut = async move {
            let mut b = backend;
            ask(&plan, &mut b).await
        };
        let joined = tokio::spawn(assert_send(fut)).await.unwrap();
        assert!(!joined.unwrap());
    });
}

fn column_branch(alias: usize, source: LogicalSource, column: &str) -> Branch {
    let mut branch = Branch::single(Scan { alias, source });
    branch.bindings.insert(
        "v".to_owned(),
        TermDef::Derived {
            term_map: TermMap::Column(column.into(), TermSpec::plain_literal()),
            alias,
        },
    );
    branch
}

fn select_plan(branches: Vec<Branch>) -> Plan {
    Plan {
        branches,
        form: PlanForm::Select {
            vars: vec!["v".to_owned()],
        },
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect: Dialect::Sqlite,
        dedup_scopes: Vec::new(),
        construct_drops_some_branch_var: false,
    }
}

fn backend_with(metadata: Vec<sf_sql::Result<Vec<String>>>) -> MockBackend {
    MockBackend {
        rows: VecDeque::new(),
        metadata: metadata.into(),
        probes: Vec::new(),
        opens: 0,
    }
}

fn run_select(plan: &Plan, backend: &mut MockBackend) -> crate::Result<super::Solutions> {
    super::block_on(select(plan, backend))
}

#[test]
fn malformed_shared_dedup_ownership_fails_before_metadata_io() {
    let mut plan = select_plan(vec![column_branch(
        0,
        LogicalSource::Table("guarded".to_owned()),
        "value",
    )]);
    let key_bindings = plan.branches[0].bindings.clone();
    plan.dedup_scopes = vec![
        Some(DedupScope {
            group_id: 7,
            key_bindings: key_bindings.clone(),
        }),
        Some(DedupScope {
            group_id: 7,
            key_bindings,
        }),
    ];
    let mut backend = backend_with(Vec::new());

    assert!(run_select(&plan, &mut backend).is_err());
    assert!(backend.probes.is_empty());
    assert_eq!(backend.opens, 0);
}

#[test]
fn malformed_shared_dedup_key_fails_before_metadata_io() {
    let branches = vec![
        column_branch(0, LogicalSource::Table("left".to_owned()), "value"),
        column_branch(1, LogicalSource::Table("right".to_owned()), "value"),
    ];
    let mut plan = select_plan(branches);
    plan.dedup_scopes = vec![
        Some(DedupScope {
            group_id: 7,
            key_bindings: Default::default(),
        }),
        Some(DedupScope {
            group_id: 7,
            key_bindings: Default::default(),
        }),
    ];
    let mut backend = backend_with(Vec::new());

    assert!(run_select(&plan, &mut backend).is_err());
    assert!(backend.probes.is_empty());
    assert_eq!(backend.opens, 0);
}

#[test]
fn mutated_shared_dedup_scope_fails_before_metadata_io() {
    let (mut plan, _) = projected_pattern_plan(false);
    for (index, branch) in plan.branches.iter_mut().enumerate() {
        branch.core.push(Scan {
            alias: 10 + index,
            source: LogicalSource::Table("injected_join".to_owned()),
        });
    }
    let mut backend = backend_with(Vec::new());

    assert!(run_select(&plan, &mut backend).is_err());
    assert!(backend.probes.is_empty());
    assert_eq!(backend.opens, 0);
}

fn pattern_branch(alias: usize, table: &str) -> Branch {
    let mut branch = Branch::single(Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    });
    for (variable, column) in [("s", "s_col"), ("o", "o_col")] {
        branch.bindings.insert(
            variable.to_owned(),
            TermDef::Derived {
                term_map: TermMap::Column(column.into(), TermSpec::plain_literal()),
                alias,
            },
        );
    }
    branch.distinct = true;
    branch
}

fn pattern_row(branch: &Branch, subject: &str, object: &str) -> RawTuple {
    let values = branch
        .projection()
        .iter()
        .map(|column| match column.column.as_ref() {
            "s_col" => Some(subject.to_owned()),
            "o_col" => Some(object.to_owned()),
            other => panic!("unexpected projected column {other}"),
        })
        .collect::<Vec<_>>();
    RawTuple {
        codes: vec![None; values.len()],
        values,
    }
}

fn projected_pattern_plan(distinct: bool) -> (Plan, VecDeque<Vec<RawTuple>>) {
    let mut full_branches = vec![pattern_branch(0, "left"), pattern_branch(1, "right")];
    let rows = VecDeque::from([
        vec![
            pattern_row(&full_branches[0], "shared", "one"),
            pattern_row(&full_branches[0], "shared", "two"),
        ],
        vec![pattern_row(&full_branches[1], "shared", "one")],
    ]);
    let scopes = full_branches
        .iter()
        .map(|branch| {
            Some(DedupScope {
                group_id: 7,
                key_bindings: branch.bindings.clone(),
            })
        })
        .collect();
    for branch in &mut full_branches {
        branch.bindings.remove("o");
    }
    let mut plan = select_plan(full_branches);
    plan.form = PlanForm::Select {
        vars: vec!["s".to_owned()],
    };
    plan.distinct = distinct;
    plan.dedup_scopes = scopes;
    (plan, rows)
}

#[test]
fn shared_dedup_uses_the_pre_projection_pattern_key() {
    let (plan, rows) = projected_pattern_plan(false);
    let mut backend = backend_with(vec![
        Ok(vec!["s_col".to_owned(), "o_col".to_owned()]),
        Ok(vec!["s_col".to_owned(), "o_col".to_owned()]),
    ]);
    backend.rows = rows;

    let solutions = run_select(&plan, &mut backend).unwrap();
    assert_eq!(solutions.vars, vec!["s"]);
    assert_eq!(solutions.rows.len(), 2);
    assert!(solutions.rows.iter().all(|row| row.len() == 1));
    assert_eq!(solutions.rows[0], solutions.rows[1]);
}

#[test]
fn outer_distinct_runs_after_the_pattern_key_dedup() {
    let (plan, rows) = projected_pattern_plan(true);
    let mut backend = backend_with(vec![
        Ok(vec!["s_col".to_owned(), "o_col".to_owned()]),
        Ok(vec!["s_col".to_owned(), "o_col".to_owned()]),
    ]);
    backend.rows = rows;

    let solutions = run_select(&plan, &mut backend).unwrap();
    assert_eq!(solutions.rows.len(), 1);
    assert_eq!(solutions.rows[0].len(), 1);
}

fn nested_path_exists_plan(nested_column: &str) -> Plan {
    let mut branch = column_branch(
        0,
        LogicalSource::Table("outer_source".to_owned()),
        "outer_col",
    );
    branch.where_conds.push(SqlCond::PathExists {
        pc: PathClosure {
            alias: 1,
            kind: PathKind::One,
            hop: HopExpr::Pred(HopRelation {
                source: LogicalSource::Table("path_source".to_owned()),
                subj_col: "path_s".into(),
                obj_col: "path_o".into(),
            }),
        },
        conds: vec![SqlCond::Exists {
            scans: vec![Scan {
                alias: 2,
                source: LogicalSource::Table("nested_source".to_owned()),
            }],
            conds: vec![SqlCond::IsNotNull(ColRef::new(2, nested_column))],
        }],
        negated: false,
    });
    select_plan(vec![branch])
}

#[test]
fn second_metadata_probe_error_opens_no_branch() {
    let plan = select_plan(vec![
        column_branch(0, LogicalSource::Table("first".to_owned()), "a"),
        column_branch(1, LogicalSource::Table("second".to_owned()), "b"),
    ]);
    let mut backend = backend_with(vec![
        Ok(vec!["a".to_owned()]),
        Err(sf_sql::Error::Introspection(
            "second probe failed".to_owned(),
        )),
    ]);

    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}

#[test]
fn second_source_missing_required_column_opens_no_branch() {
    let plan = select_plan(vec![
        column_branch(0, LogicalSource::Table("first".to_owned()), "a"),
        column_branch(1, LogicalSource::Table("second".to_owned()), "b"),
    ]);
    let mut backend = backend_with(vec![Ok(vec!["a".to_owned()]), Ok(vec!["not_b".to_owned()])]);

    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}

#[test]
fn nested_path_exists_missing_column_opens_no_branch() {
    let plan = nested_path_exists_plan("nested_required");
    let mut backend = backend_with(vec![
        Ok(vec!["outer_col".to_owned()]),
        Ok(vec!["path_s".to_owned(), "path_o".to_owned()]),
        Ok(vec!["nested_other".to_owned()]),
    ]);

    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}

#[test]
fn nested_path_exists_ambiguous_column_opens_no_branch() {
    let plan = nested_path_exists_plan("NESTED_REQUIRED");
    let mut backend = backend_with(vec![
        Ok(vec!["outer_col".to_owned()]),
        Ok(vec!["path_s".to_owned(), "path_o".to_owned()]),
        Ok(vec![
            "Nested_Required".to_owned(),
            "nested_required".to_owned(),
        ]),
    ]);

    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}

#[test]
fn ambiguous_case_folded_column_opens_no_branch() {
    const QUERY: &str = "SELECT 1 AS SecretColumnSentinel, 2 AS secretcolumnsentinel";
    const COLUMN: &str = "SECRETCOLUMNSENTINEL";
    let plan = select_plan(vec![column_branch(
        0,
        LogicalSource::Query(QUERY.to_owned()),
        COLUMN,
    )]);
    let mut backend = backend_with(vec![Ok(vec![
        "SecretColumnSentinel".to_owned(),
        "secretcolumnsentinel".to_owned(),
    ])]);

    let error = run_select(&plan, &mut backend)
        .err()
        .expect("must fail closed");
    let message = error.to_string();
    assert!(!message.contains(QUERY) && !message.contains(COLUMN));
    assert_eq!(backend.opens, 0);
}

#[test]
fn duplicate_exact_column_name_opens_no_branch() {
    let plan = select_plan(vec![column_branch(
        0,
        LogicalSource::Query("SELECT 1 AS duplicate, 2 AS duplicate".to_owned()),
        "duplicate",
    )]);
    let mut backend = backend_with(vec![Ok(vec![
        "duplicate".to_owned(),
        "duplicate".to_owned(),
    ])]);

    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}

#[test]
fn duplicate_unreferenced_result_name_opens_no_branch() {
    let plan = select_plan(vec![column_branch(
        0,
        LogicalSource::Query("SELECT 1 AS required, 2 AS spare, 3 AS spare".to_owned()),
        "required",
    )]);
    let mut backend = backend_with(vec![Ok(vec![
        "required".to_owned(),
        "spare".to_owned(),
        "spare".to_owned(),
    ])]);

    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}

#[test]
fn table_and_query_with_identical_probe_sql_do_not_share_metadata_identity() {
    let table = LogicalSource::Table("same".to_owned());
    let colliding_query = LogicalSource::Query(Dialect::Sqlite.probe_sql(&table));
    let plan = select_plan(vec![
        column_branch(0, table, "table_col"),
        column_branch(1, colliding_query, "query_col"),
    ]);
    let mut backend = backend_with(vec![
        Ok(vec!["table_col".to_owned()]),
        Ok(vec!["query_col".to_owned()]),
    ]);

    assert!(run_select(&plan, &mut backend).is_ok());
    assert_eq!(backend.probes.len(), 2);
}

#[test]
fn repeated_logical_source_is_probed_once() {
    let plan = select_plan(vec![
        column_branch(0, LogicalSource::Table("same".to_owned()), "a"),
        column_branch(1, LogicalSource::Table("same".to_owned()), "a"),
    ]);
    let mut backend = backend_with(vec![Ok(vec!["a".to_owned()])]);

    assert!(run_select(&plan, &mut backend).is_ok());
    assert_eq!(backend.probes.len(), 1);
}

#[test]
fn later_branch_emission_error_opens_no_branch() {
    let mut invalid = Branch::single(Scan {
        alias: 1,
        source: LogicalSource::Query("this is not valid SQL".to_owned()),
    });
    invalid.bindings.clear();
    let plan = select_plan(vec![
        column_branch(0, LogicalSource::Table("first".to_owned()), "a"),
        invalid,
    ]);
    let mut backend = backend_with(vec![Ok(vec!["a".to_owned()]), Ok(Vec::new())]);

    assert!(run_select(&plan, &mut backend).is_err());
    assert_eq!(backend.opens, 0);
}
