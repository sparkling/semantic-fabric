use super::optional_work_test_support::{budget, every_stop, exact_and_short};
use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::iri_cmp::{IriComparison, IriOperand, IriPart};
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use sf_core::ir::TermSpec;
use sf_core::query_control::QueryCharge;

#[test]
fn optional_condition_work_all_arms_match_raw_at_inclusive_bounds_and_every_stop() {
    let column = ColRef::new(1, "café/identifier");
    let mut cases = vec![
        SqlCond::ExpressionError,
        SqlCond::ColEq(column.clone(), ColRef::new(2, "rhs")),
        SqlCond::Cmp(column.clone(), CmpOp::Eq, "value".into()),
        SqlCond::TemplateEq(
            vec![
                Segment::Literal("prefix".into()),
                Segment::Column("café".into()),
                Segment::Column("id".into()),
            ],
            1,
            vec![Segment::Column("rhs".into())],
            2,
            true,
        ),
        SqlCond::IriCmp(Box::new(IriComparison {
            left: IriOperand::Template {
                parts: vec![IriPart::Literal("fixed".into()); 128],
                base: None,
            },
            right: IriOperand::Column {
                column: column.clone(),
                base: None,
            },
        })),
        SqlCond::IriCmp(Box::new(IriComparison {
            left: IriOperand::Template {
                parts: vec![
                    IriPart::Literal("prefix/".into()),
                    IriPart::Column(column.clone()),
                    IriPart::Column(column.clone()),
                ],
                base: None,
            },
            right: IriOperand::Constant(sf_core::NamedNode::new("urn:fixed").unwrap()),
        })),
    ];
    for value_op in [None, Some(CmpOp::Eq)] {
        cases.push(SqlCond::LiteralCmp(Box::new(LiteralComparison {
            left: LiteralOperand::Column {
                column: column.clone(),
                spec: TermSpec::iri(),
            },
            right: LiteralOperand::Constant(sf_core::Literal::from("right")),
            value_op,
        })));
    }
    for condition in cases {
        for nullable in [false, true] {
            let raw = null_safe(condition.clone(), nullable);
            let run = |control: &dyn sf_core::query_control::QueryControl| {
                conditions::null_safe(
                    condition.clone(),
                    nullable,
                    CompilerWorkMode::Metered(CompileContext::new(control)),
                )
            };
            exact_and_short(raw, run);
            every_stop(run);
        }
    }
}

#[test]
fn optional_condition_work_moves_owned_payload_without_recopying_constant() {
    let column = ColRef::new(1, "id");
    let huge = SqlCond::Cmp(column.clone(), CmpOp::Eq, "v".repeat(100_000));
    let tiny = SqlCond::Cmp(column, CmpOp::Eq, "v".into());
    let costs: Vec<_> = [huge, tiny]
        .into_iter()
        .map(|condition| {
            let control = budget(u64::MAX);
            let out = conditions::null_safe(
                condition,
                true,
                CompilerWorkMode::Metered(CompileContext::new(&control)),
            )
            .unwrap();
            let SqlCond::Or(parts) = out else {
                panic!("R1 guard missing")
            };
            assert_eq!(parts.len(), 2);
            assert!(matches!(parts[0], SqlCond::Cmp(..)));
            control.consumed(QueryCharge::CompilerWork)
        })
        .collect();
    assert_eq!(
        costs[0], costs[1],
        "constant ownership moves; only the column is copied"
    );
}
