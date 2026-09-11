use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{HopRelation, PathClosure, PathKind};
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::ir::LogicalSource;
use sf_core::query_control::{QueryCharge, QueryControl};

pub(crate) fn empty_rows_work(rows: u64) -> u64 {
    1 + rows * (2 + std::mem::size_of::<Branch>() as u64)
}
pub(crate) fn one_column_rows_work(rows: u64, key: &str) -> u64 {
    empty_rows_work(rows)
        + rows * (2 + key.len() as u64 + std::mem::size_of::<(String, TermDef)>() as u64)
}
pub(crate) fn singleton_work() -> u64 {
    2 + std::mem::size_of::<Branch>() as u64
}
pub(crate) fn scan_work() -> u64 {
    singleton_work() + 1 + std::mem::size_of::<Scan>() as u64
}
pub(super) fn run(node: IqNode, control: &dyn QueryControl) -> Result<Vec<Branch>> {
    lower_node(
        node,
        sf_sql::Dialect::Sqlite,
        false,
        &mut 0,
        &HashSet::new(),
        &StarEnv::new(),
        CompilerWorkMode::Metered(CompileContext::new(control)),
    )
}
fn raw(node: IqNode) -> Vec<Branch> {
    lower_node(
        node,
        sf_sql::Dialect::Sqlite,
        false,
        &mut 0,
        &HashSet::new(),
        &StarEnv::new(),
        CompilerWorkMode::Uncontrolled,
    )
    .unwrap()
}
pub(super) fn scan() -> Scan {
    Scan {
        alias: 4,
        source: LogicalSource::Query("SELECT 1".into()).into(),
    }
}
fn term(s: &str) -> TermDef {
    TermDef::Const(sf_core::Literal::from(s).into())
}

#[test]
fn base_work_values_exact_utf8_duplicate_keys_undef_and_zip_semantics() {
    let source = IqNode::Values {
        vars: vec!["é".into(), "z".into(), "é".into()],
        rows: vec![
            vec![Some(term("first")), None, Some(term("last"))],
            vec![Some(term("first")), Some(term("z")), None],
            vec![],
            vec![
                Some(term("first")),
                None,
                None,
                Some(term(&"ignored".repeat(1000))),
            ],
        ],
    };
    // Entry + 4 output visits/carriers + 4 rows + 9 zipped cells + 14
    // key-copy units + 5 search units + 5 logical map carriers.
    let expected = 37
        + 4 * std::mem::size_of::<Branch>() as u64
        + 5 * std::mem::size_of::<(String, TermDef)>() as u64;
    let control = budget(expected);
    let result = run(source.clone(), &control).unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
    assert_eq!(
        format!("{:?}", result[0].bindings["é"]),
        format!("{:?}", term("last"))
    );
    assert_eq!(result[1].bindings.len(), 2);
    assert!(result[2].bindings.is_empty());
    assert_eq!(result[3].bindings.len(), 1);
    assert_eq!(format!("{result:?}"), format!("{:?}", raw(source.clone())));
    assert!(run(source.clone(), &budget(expected - 1)).is_err());
    every_stop(|control| run(source.clone(), control));
}

#[test]
fn base_work_values_admits_carriers_without_copying_owned_payload() {
    for size in [0, 1, 256] {
        let payload = vec![term(&"large".repeat(100)); size];
        let ptr = payload.as_ptr();
        let node = IqNode::Values {
            vars: vec!["x".into()],
            rows: vec![vec![Some(TermDef::Concat(payload))]],
        };
        let control = budget(one_column_rows_work(1, "x"));
        let result = run(node, &control).unwrap();
        let TermDef::Concat(parts) = &result[0].bindings["x"] else {
            panic!()
        };
        assert_eq!(ptr, parts.as_ptr());
        assert_eq!(
            control.consumed(QueryCharge::CompilerWork),
            one_column_rows_work(1, "x")
        );
    }
    for rows in [0, 1, 4] {
        let node = IqNode::Values {
            vars: vec![],
            rows: vec![vec![]; rows],
        };
        let exact = empty_rows_work(rows as u64);
        let c = budget(exact);
        assert_eq!(run(node.clone(), &c).unwrap().len(), rows);
        assert_eq!(c.consumed(QueryCharge::CompilerWork), exact);
        assert!(run(node.clone(), &budget(exact - 1)).is_err());
        every_stop(|c| run(node.clone(), c));
    }
}

#[test]
fn base_work_leaf_slots_are_exact_and_payload_moves() {
    let closure = PathClosure {
        alias: 7,
        kind: PathKind::One,
        hop: HopExpr::Pred(HopRelation {
            source: LogicalSource::Table("edges".into()),
            subj_col: "subject".repeat(500).into(),
            obj_col: "object".into(),
        }),
    };
    let HopExpr::Pred(hop) = &closure.hop else {
        panic!()
    };
    let ptr = hop.subj_col.as_ptr();
    let result = run(IqNode::Path { closure }, &budget(singleton_work())).unwrap();
    let HopExpr::Pred(hop) = &result[0].path.as_ref().unwrap().hop else {
        panic!()
    };
    assert_eq!(ptr, hop.subj_col.as_ptr());
    for (node, expected) in [
        (IqNode::True, singleton_work()),
        (
            IqNode::Empty {
                vars: vec!["ignored".repeat(1000).into()],
            },
            1,
        ),
        (
            IqNode::Extensional {
                scan: scan(),
                bind: BTreeMap::new(),
            },
            scan_work(),
        ),
        (
            IqNode::Path {
                closure: result[0].path.clone().unwrap(),
            },
            singleton_work(),
        ),
    ] {
        let c = budget(expected);
        let actual = run(node.clone(), &c).unwrap();
        assert_eq!(c.consumed(QueryCharge::CompilerWork), expected);
        assert_eq!(format!("{actual:?}"), format!("{:?}", raw(node.clone())));
        assert!(run(node.clone(), &budget(expected - 1)).is_err());
        every_stop(|c| run(node.clone(), c));
        exact_and_short(actual, |c| run(node.clone(), c));
    }
}
