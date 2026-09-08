use super::*;

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sql::Dialect;
use spargebra::algebra::Expression;
use spargebra::term::Variable;

use crate::iq::Scan;

fn scan(alias: usize) -> Scan {
    Scan {
        alias,
        source: (LogicalSource::Table(format!("table_{alias}"))).into(),
    }
}

fn branch(alias: usize) -> Branch {
    Branch::single(scan(alias))
}

fn derived(alias: usize, column: &str) -> TermDef {
    TermDef::Derived {
        term_map: TermMap::Column(column.into(), TermSpec::iri()),
        alias,
    }
}

fn constant(value: &'static str) -> TermDef {
    TermDef::Const(sf_core::Term::NamedNode(sf_core::NamedNode::new_unchecked(
        value,
    )))
}

fn bound(variable: &str) -> Expression {
    Expression::Bound(Variable::new(variable).expect("valid variable"))
}

fn combined_filter() -> Expression {
    Expression::And(Box::new(bound("shared")), Box::new(bound("right")))
}

fn assert_derived(definition: &TermDef, expected_alias: usize, expected_column: &str) {
    match definition {
        TermDef::Derived {
            term_map: TermMap::Column(column, _),
            alias,
        } => {
            assert_eq!(*alias, expected_alias);
            assert_eq!(column.as_ref(), expected_column);
        }
        other => panic!("expected derived binding, got {other:?}"),
    }
}

fn nullable_left_and_right() -> (Branch, Branch) {
    let mut left = branch(0);
    left.opts.push(OptJoin {
        scan: scan(1),
        on: Vec::new(),
        extra: Vec::new(),
    });
    left.bindings
        .insert("shared".to_owned(), derived(1, "left_shared"));
    left.bindings
        .insert("left".to_owned(), derived(0, "left_only"));

    let mut right = branch(2);
    right
        .bindings
        .insert("shared".to_owned(), derived(2, "right_shared"));
    right
        .bindings
        .insert("right".to_owned(), derived(2, "right_only"));
    (left, right)
}

fn assert_merged_bindings(branch: &Branch) {
    match branch.bindings.get("shared").expect("shared binding") {
        TermDef::Coalesce(left, right) => {
            assert_derived(left, 1, "left_shared");
            assert_derived(right, 2, "right_shared");
        }
        other => panic!("expected nullable shared binding to coalesce, got {other:?}"),
    }
    assert_derived(
        branch.bindings.get("right").expect("right-only binding"),
        2,
        "right_only",
    );
}

fn assert_combined_filter_bindings(condition: &SqlCond) {
    let SqlCond::And(conditions) = condition else {
        panic!("expected combined FILTER condition, got {condition:?}");
    };
    let [SqlCond::IsNotNull(left), SqlCond::IsNotNull(right)] = conditions.as_slice() else {
        panic!("expected two BOUND conditions, got {conditions:?}");
    };
    assert_eq!((left.alias, left.column.as_ref()), (1, "left_shared"));
    assert_eq!((right.alias, right.column.as_ref()), (2, "right_only"));
}

#[test]
fn single_scan_no_match_returns_the_owned_left_branch_unchanged() {
    let mut left = branch(0);
    left.bindings
        .insert("shared".to_owned(), constant("urn:left"));
    left.where_conds
        .push(SqlCond::IsNotNull(ColRef::new(0, "left_key")));
    let before = format!("{left:#?}");

    let mut right = branch(1);
    right
        .bindings
        .insert("shared".to_owned(), constant("urn:right"));

    let output = left_join_branches(vec![left], vec![right], None, Dialect::Sqlite)
        .expect("disjoint constants leave the optional unmatched");

    assert_eq!(output.len(), 1);
    assert_eq!(format!("{:#?}", output[0]), before);
}

#[test]
fn single_scan_filter_uses_left_preferred_bindings_before_coalesce() {
    let (left, right) = nullable_left_and_right();
    let filter = combined_filter();
    let output = left_join_branches(vec![left], vec![right], Some(&filter), Dialect::Sqlite)
        .expect("right-only FILTER binding is available");

    let joined = &output[0];
    assert_eq!(joined.opts.len(), 2);
    assert_eq!(joined.opts[1].extra.len(), 1);
    assert_combined_filter_bindings(&joined.opts[1].extra[0]);
    assert_merged_bindings(joined);
}

#[test]
fn decomposed_match_reuses_its_output_binding_map_for_the_filter() {
    let (left, right) = nullable_left_and_right();
    let before = format!("{left:#?}");
    let filter = combined_filter();
    let joined = inner_join_one(&left, &right, Some(&filter), Dialect::Sqlite)
        .expect("inner-join lowering succeeds")
        .expect("the branch can match");

    assert_eq!(format!("{left:#?}"), before);
    let filter = joined
        .where_conds
        .iter()
        .find(|condition| matches!(condition, SqlCond::And(_)))
        .expect("combined FILTER is present");
    assert_combined_filter_bindings(filter);
    assert_merged_bindings(&joined);
}

#[test]
fn nullable_template_guard_keeps_equality_first_without_template_copies() {
    let condition = SqlCond::TemplateEq(
        vec![
            Segment::Literal("urn:left:".into()),
            Segment::Column("left_id".into()),
        ],
        3,
        vec![
            Segment::Literal("urn:right:".into()),
            Segment::Column("right_id".into()),
        ],
        4,
        true,
    );

    let SqlCond::Or(disjuncts) = null_safe(condition, true) else {
        panic!("nullable template equality must become a disjunction");
    };
    assert!(matches!(disjuncts.first(), Some(SqlCond::TemplateEq(..))));
    assert!(matches!(
        &disjuncts[1],
        SqlCond::IsNull(column)
            if column.alias == 3 && column.column.as_ref() == "left_id"
    ));
    assert!(matches!(
        &disjuncts[2],
        SqlCond::IsNull(column)
            if column.alias == 4 && column.column.as_ref() == "right_id"
    ));
}
