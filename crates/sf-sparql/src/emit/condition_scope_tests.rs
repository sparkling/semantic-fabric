use super::*;
use crate::iq::{CmpOp, Scan};
use sf_core::datatype::XsdTypeCode;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::mem::size_of;
use std::sync::Mutex;

#[path = "condition_dialect_scope_tests.rs"]
mod dialect_scope;

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}
fn native(alias: usize, name: &str, value: &str) -> SqlCond {
    SqlCond::NativeCmp(ColRef::new(alias, name), CmpOp::Eq, value.into())
}
fn scan(alias: usize, table: &str) -> Scan {
    Scan {
        alias,
        source: LogicalSource::Table(table.into()).into(),
    }
}

// Every metadata key is 4 bytes, so hash iteration order cannot change a charge sequence.
fn catalog() -> ColumnCatalog {
    let mut catalog = ColumnCatalog::default();
    for (table, columns) in [
        ("alpha", ["col0", "col1"]),
        ("beta", ["col0", "col1"]),
        ("gamma", ["COL0", "COL1"]),
    ] {
        catalog.insert(
            &LogicalSource::Table(table.into()),
            columns.map(String::from).to_vec(),
        );
    }
    catalog
}

fn populated() -> AliasActuals {
    AliasActuals {
        datatype_columns: HashMap::from([
            ("col0".to_owned(), Some(XsdTypeCode::Integer)),
            ("col1".to_owned(), None),
        ]),
        natural_columns: HashMap::from([("col0".to_owned(), Some(XsdTypeCode::Decimal))]),
        scalar_columns: HashMap::from([
            ("col0".to_owned(), NativeScalarKey::Integer),
            ("col1".to_owned(), NativeScalarKey::PostgresNumeric),
        ]),
        source_kind: AliasSourceKind::Table,
        columns: vec!["col0".to_owned(), "col1".to_owned()],
        path: false,
        text_columns: HashMap::new(),
        static_iri_columns: HashSet::from(["col0".to_owned()]),
        iri_unreserved_columns: HashSet::from(["col1".to_owned()]),
        sqlite_columns: HashMap::new(),
        lexical_columns: HashMap::new(),
        lexical_comparison_columns: HashMap::new(),
    }
}

fn outer() -> ActualColumns {
    HashMap::from([(0, populated()), (1, populated())])
}

fn nested_cond() -> SqlCond {
    SqlCond::Exists {
        scans: vec![scan(2, "alpha")],
        conds: vec![
            native(2, "col0", "p1"),
            SqlCond::NotExists {
                scans: vec![scan(3, "beta")],
                conds: vec![native(3, "col1", "p2"), native(0, "col0", "p3")],
            },
            native(1, "col1", "p4"),
        ],
    }
}

fn render(
    cond: &SqlCond,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    work: SourceWork<'_>,
) -> Result<(String, Vec<String>)> {
    let mut params = vec![];
    let mut index = 0;
    let sql = condition(
        cond,
        Dialect::Sqlite,
        catalog,
        actuals,
        &mut params,
        &mut index,
        work,
    )?;
    assert_eq!(index, params.len());
    Ok((sql, params))
}

fn dismantle(condition: SqlCond) {
    let mut pending = vec![condition];
    while let Some(condition) = pending.pop() {
        match condition {
            SqlCond::Not(inner) => pending.push(*inner),
            SqlCond::And(inner)
            | SqlCond::Or(inner)
            | SqlCond::Exists { conds: inner, .. }
            | SqlCond::NotExists { conds: inner, .. } => pending.extend(inner),
            _ => {}
        }
    }
}

struct Trace {
    budget: QueryBudget,
    seen: Mutex<Vec<u64>>,
    stop_at: usize,
    reason: QueryControlError,
}
impl Trace {
    fn new(stop_at: usize, reason: QueryControlError) -> Self {
        Self {
            budget: budget(u64::MAX),
            seen: Mutex::new(vec![]),
            stop_at,
            reason,
        }
    }
    fn seen(&self) -> Vec<u64> {
        self.seen.lock().unwrap().clone()
    }
}
impl QueryControl for Trace {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
    fn consume(&self, kind: QueryCharge, units: u64) -> std::result::Result<(), QueryControlError> {
        let ordinal = (kind == QueryCharge::SourceWork).then(|| {
            let mut seen = self.seen.lock().unwrap();
            seen.push(units);
            seen.len()
        });
        self.budget.consume(kind, units)?;
        if ordinal == Some(self.stop_at) {
            self.budget.terminate(self.reason);
        }
        self.budget.checkpoint()
    }
}

// Charge recipe of condition_metadata::copy, derived from the input metadata alone.
fn map_cost<V>(map: &HashMap<String, V>) -> Vec<u64> {
    let mut cost = vec![(map.len() * size_of::<(String, V)>()) as u64];
    cost.extend(map.keys().map(|key| key.len() as u64));
    cost
}
fn set_cost(set: &HashSet<String>) -> Vec<u64> {
    let mut cost = vec![(set.len() * (1 + size_of::<String>())) as u64];
    cost.extend(set.iter().map(|name| name.len() as u64));
    cost
}
fn source_block(source: &AliasActuals) -> Vec<u64> {
    let mut block = vec![(source.columns.len() * (1 + size_of::<String>())) as u64];
    block.extend(source.columns.iter().map(|name| name.len() as u64));
    block.extend(map_cost(&source.datatype_columns));
    block.extend(map_cost(&source.natural_columns));
    block.extend(map_cost(&source.scalar_columns));
    block.extend(map_cost(&source.text_columns));
    block.extend(map_cost(&source.sqlite_columns));
    block.extend(map_cost(&source.lexical_columns));
    block.extend(map_cost(&source.lexical_comparison_columns));
    block.extend(set_cost(&source.static_iri_columns));
    block.extend(set_cost(&source.iri_unreserved_columns));
    block
}
fn copy_recipe(actuals: &ActualColumns) -> (u64, Vec<Vec<u64>>) {
    let header = (actuals.len() * (1 + size_of::<(usize, AliasActuals)>())) as u64;
    (header, actuals.values().map(source_block).collect())
}
fn consume_blocks(observed: &[u64], blocks: &mut Vec<Vec<u64>>) -> Option<usize> {
    if blocks.is_empty() {
        return Some(0);
    }
    for index in 0..blocks.len() {
        if observed.starts_with(&blocks[index]) {
            let block = blocks.remove(index);
            let length = block.len();
            let rest = consume_blocks(&observed[length..], blocks);
            blocks.insert(index, block);
            if let Some(rest) = rest {
                return Some(length + rest);
            }
        }
    }
    None
}
fn find_copy(observed: &[u64], from: usize, header: u64, blocks: &[Vec<u64>]) -> Option<usize> {
    (from..observed.len())
        .filter(|start| observed[*start] == header)
        .find_map(|start| {
            consume_blocks(&observed[start + 1..], &mut blocks.to_vec())
                .map(|length| start + 1 + length)
        })
}

#[test]
fn nested_exists_with_populated_metadata_keeps_sql_and_parameter_order() {
    let (catalog, outer, cond) = (catalog(), outer(), nested_cond());
    let expected_sql = "EXISTS (SELECT 1 FROM \"alpha\" t2 WHERE t2.\"col0\" = ? AND NOT EXISTS (SELECT 1 FROM \"beta\" t3 WHERE t3.\"col1\" = ? AND t0.\"col0\" = ?) AND t1.\"col1\" = ?)";
    let control = budget(u64::MAX);
    let controlled = render(&cond, &catalog, &outer, SourceWork::new(Some(&control))).unwrap();
    assert_eq!(controlled.0, expected_sql);
    assert_eq!(controlled.1, ["p1", "p2", "p3", "p4"]);
    assert_eq!(
        controlled,
        render(&cond, &catalog, &outer, SourceWork::new(None)).unwrap()
    );
}

#[test]
fn sibling_and_shadowing_scopes_restore_populated_outer_metadata() {
    let (catalog, outer) = (catalog(), outer());
    let cond = SqlCond::And(vec![
        SqlCond::Exists {
            scans: vec![scan(0, "gamma")],
            conds: vec![native(0, "col0", "in")],
        },
        native(0, "col0", "out"),
        SqlCond::NotExists {
            scans: vec![scan(3, "gamma")],
            conds: vec![native(3, "col1", "b")],
        },
        native(3, "col1", "gone"),
    ]);
    let control = budget(u64::MAX);
    let (sql, params) = render(&cond, &catalog, &outer, SourceWork::new(Some(&control))).unwrap();
    assert_eq!(
        sql,
        "(EXISTS (SELECT 1 FROM \"gamma\" t0 WHERE t0.\"COL0\" = ?) AND t0.\"col0\" = ? AND NOT EXISTS (SELECT 1 FROM \"gamma\" t3 WHERE t3.\"COL1\" = ?) AND t3.\"col1\" = ?)"
    );
    assert_eq!(params, ["in", "out", "b", "gone"]);
}

#[test]
fn nested_scope_copies_are_charged_to_the_caller_control() {
    let (catalog, outer, cond) = (catalog(), outer(), nested_cond());
    let trace = Trace::new(usize::MAX, QueryControlError::Cancelled);
    render(&cond, &catalog, &outer, SourceWork::new(Some(&trace))).unwrap();
    let observed = trace.seen();

    let alias = metadata::scan_actuals_controlled(
        &scan(2, "alpha"),
        Dialect::Sqlite,
        &catalog,
        SourceWork::new(None),
    )
    .unwrap();
    let mut inner = outer.clone();
    inner.insert(2, alias);
    let (first, first_blocks) = copy_recipe(&outer);
    let (second, second_blocks) = copy_recipe(&inner);

    assert!(first_blocks.iter().map(Vec::len).sum::<usize>() >= 20);
    assert!(find_copy(&observed, 0, first + 1, &first_blocks).is_none());
    let end = find_copy(&observed, 0, first, &first_blocks).expect("outer scope copy charged");
    find_copy(&observed, end, second, &second_blocks).expect("nested scope copy charged");
}

#[test]
fn populated_nested_scopes_admit_exactly_and_refuse_one_unit_less() {
    let (catalog, outer, cond) = (catalog(), outer(), nested_cond());
    let meter = budget(u64::MAX);
    let expected = render(&cond, &catalog, &outer, SourceWork::new(Some(&meter))).unwrap();
    let units = meter.consumed(QueryCharge::SourceWork);
    let exact = budget(units);
    assert_eq!(
        expected,
        render(&cond, &catalog, &outer, SourceWork::new(Some(&exact))).unwrap()
    );
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 0);
    let short = budget(units - 1);
    assert!(matches!(
        render(&cond, &catalog, &outer, SourceWork::new(Some(&short))),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
}

#[test]
fn populated_nested_scopes_keep_first_terminal_cause_at_every_charge() {
    let (catalog, outer, cond) = (catalog(), outer(), nested_cond());
    let measured = Trace::new(usize::MAX, QueryControlError::Cancelled);
    render(&cond, &catalog, &outer, SourceWork::new(Some(&measured))).unwrap();
    let charges = measured.seen().len();
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=charges {
            let control = Trace::new(at, reason);
            let result = render(&cond, &catalog, &outer, SourceWork::new(Some(&control)));
            assert!(
                matches!(result, Err(Error::QueryControl(found)) if found == reason),
                "{reason:?} at charge {at}"
            );
            assert_eq!(control.checkpoint(), Err(reason));
        }
    }
}

fn chain(depth: usize) -> SqlCond {
    let mut cond = native(9 + depth, "col0", "leaf");
    for level in (0..depth).rev() {
        let alias = 10 + level;
        let (scans, conds) = (
            vec![scan(alias, "alpha")],
            vec![native(alias, "col0", &format!("p{level}")), cond],
        );
        cond = if level % 2 == 0 {
            SqlCond::Exists { scans, conds }
        } else {
            SqlCond::NotExists { scans, conds }
        };
    }
    cond
}

#[test]
fn depth_128_populated_exists_chain_renders_on_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let depth = 128;
            let (catalog, outer, cond) = (catalog(), outer(), chain(depth));
            let mut expected_sql = String::new();
            let mut expected_params = vec![];
            for level in 0..depth {
                let not = if level % 2 == 0 { "" } else { "NOT " };
                let alias = 10 + level;
                expected_sql.push_str(&format!(
                    "{not}EXISTS (SELECT 1 FROM \"alpha\" t{alias} WHERE t{alias}.\"col0\" = ? AND "
                ));
                expected_params.push(format!("p{level}"));
            }
            expected_sql.push_str(&format!("t{}.\"col0\" = ?", 9 + depth));
            expected_sql.push_str(&")".repeat(depth));
            expected_params.push("leaf".to_owned());

            let meter = budget(u64::MAX);
            let rendered = render(&cond, &catalog, &outer, SourceWork::new(Some(&meter)));
            let units = meter.consumed(QueryCharge::SourceWork);
            let exact = render(
                &cond,
                &catalog,
                &outer,
                SourceWork::new(Some(&budget(units))),
            );
            let short = render(
                &cond,
                &catalog,
                &outer,
                SourceWork::new(Some(&budget(units - 1))),
            );
            dismantle(cond);
            let rendered = rendered.unwrap();
            assert_eq!(rendered, (expected_sql, expected_params));
            assert_eq!(exact.unwrap(), rendered);
            assert!(matches!(
                short,
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
        })
        .unwrap()
        .join()
        .unwrap();
}
