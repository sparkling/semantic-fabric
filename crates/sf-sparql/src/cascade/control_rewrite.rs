//! Alias-only rewrites on unpublished optimizer candidates; never clone payloads.
use super::{Branch, ColRef, R2rmlGraphScope, Segment, SqlCond, TermDef};
use crate::build::control::BuildWork;
use crate::Result;

fn alias(value: &mut usize, from: usize, to: usize, work: BuildWork<'_>) -> Result<()> {
    work.charge(1)?;
    if *value == from {
        *value = to;
    }
    Ok(())
}

fn column(value: &mut ColRef, from: usize, to: usize, work: BuildWork<'_>) -> Result<()> {
    alias(&mut value.alias, from, to, work)
}

pub(super) fn definition(
    value: &mut TermDef,
    from: usize,
    to: usize,
    work: BuildWork<'_>,
) -> Result<()> {
    let work = work.enter()?;
    match value {
        TermDef::Const(_) => (),
        TermDef::Derived { alias: value, .. } => alias(value, from, to, work)?,
        TermDef::R2rmlBlank {
            alias: value,
            graph,
            ..
        } => {
            alias(value, from, to, work)?;
            if let R2rmlGraphScope::Mapped { alias: value, .. } = graph {
                alias(value, from, to, work)?;
            }
        }
        TermDef::Coalesce(left, right) => {
            definition(left, from, to, work)?;
            definition(right, from, to, work)?;
        }
        TermDef::Concat(parts) => {
            for part in parts {
                work.charge(1)?;
                definition(part, from, to, work)?;
            }
        }
        TermDef::Agg { col, .. } => column(col, from, to, work)?,
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            definition(subject, from, to, work)?;
            definition(predicate, from, to, work)?;
            definition(object, from, to, work)?;
        }
    }
    work.checkpoint()
}

pub(super) fn condition(
    value: &mut SqlCond,
    from: usize,
    to: usize,
    work: BuildWork<'_>,
) -> Result<()> {
    use crate::iq::{
        iri_cmp::{IriOperand, IriPart},
        literal_cmp::LiteralOperand,
    };
    let work = work.enter()?;
    match value {
        SqlCond::ExpressionError => (),
        SqlCond::LiteralCmp(cmp) => {
            for operand in [&mut cmp.left, &mut cmp.right] {
                work.charge(1)?;
                if let LiteralOperand::Column { column: value, .. } = operand {
                    column(value, from, to, work)?;
                }
            }
        }
        SqlCond::IriCmp(cmp) => {
            for operand in [&mut cmp.left, &mut cmp.right] {
                work.charge(1)?;
                match operand {
                    IriOperand::Column { column: value, .. } => column(value, from, to, work)?,
                    IriOperand::Template { parts, .. } => {
                        for part in parts {
                            work.charge(1)?;
                            if let IriPart::Column(value) = part {
                                column(value, from, to, work)?;
                            }
                        }
                    }
                    IriOperand::Constant(_) => (),
                }
            }
        }
        SqlCond::ColEq(a, b) | SqlCond::NativeColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            column(a, from, to, work)?;
            column(b, from, to, work)?;
        }
        SqlCond::Cmp(value, ..)
        | SqlCond::NativeCmp(value, ..)
        | SqlCond::IsNotNull(value)
        | SqlCond::DecodedIsNotNull(value)
        | SqlCond::IsNull(value)
        | SqlCond::StrMatch { col: value, .. } => column(value, from, to, work)?,
        SqlCond::Not(inner) => condition(inner, from, to, work)?,
        SqlCond::And(children)
        | SqlCond::Or(children)
        | SqlCond::NotExists {
            conds: children, ..
        }
        | SqlCond::Exists {
            conds: children, ..
        }
        | SqlCond::PathExists {
            conds: children, ..
        } => {
            for child in children {
                work.charge(1)?;
                condition(child, from, to, work)?;
            }
        }
        SqlCond::TemplateEq(left, a, right, b, _) => {
            for (segments, value) in [(left, a), (right, b)] {
                for segment in segments {
                    work.charge(1)?;
                    if matches!(segment, Segment::Column(_)) {
                        alias(value, from, to, work)?;
                    }
                }
            }
        }
    }
    work.checkpoint()
}

pub(super) fn branch(
    value: &mut Branch,
    from: usize,
    to: usize,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    for def in value.bindings.values_mut() {
        work.charge(1)?;
        definition(def, from, to, work)?;
    }
    for cond in &mut value.where_conds {
        work.charge(1)?;
        condition(cond, from, to, work)?;
    }
    for optional in &mut value.opts {
        work.charge(1)?;
        for cond in optional.on.iter_mut().chain(&mut optional.extra) {
            work.charge(1)?;
            condition(cond, from, to, work)?;
        }
    }
    if let Some(aggregate) = &mut value.agg {
        for key in &mut aggregate.keys {
            work.charge(1)?;
            for value in &mut key.cols {
                column(value, from, to, work)?;
            }
        }
        for aggregate in &mut aggregate.aggs {
            work.charge(1)?;
            if let Some(value) = &mut aggregate.arg {
                column(value, from, to, work)?;
            }
            column(&mut aggregate.out, from, to, work)?;
        }
    }
    work.checkpoint()
}

pub(super) fn remove<T>(values: &mut Vec<T>, index: usize, work: BuildWork<'_>) -> Result<T> {
    // Indices are produced by a borrow of this exact, still unchanged vector.
    let moved = values.len() - index - 1;
    work.charge(moved + 1)?;
    if let crate::CompilerWorkMode::Metered(context) = work.mode {
        context.reserve_checked_product(&[moved, std::mem::size_of::<T>()])?;
    }
    work.checkpoint()?;
    Ok(values.remove(index))
}

pub(super) fn retain_scan(
    scans: &mut Vec<super::Scan>,
    drop: usize,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(scans.len())?;
    if let crate::CompilerWorkMode::Metered(context) = work.mode {
        context.reserve_checked_product(&[scans.len(), std::mem::size_of::<super::Scan>()])?;
    }
    work.checkpoint()?;
    scans.retain(|scan| scan.alias != drop);
    work.checkpoint()
}

#[cfg(test)]
mod tests {
    use super::super::{control_join, LogicalSource, Scan, TableSchema};
    use super::*;
    use crate::{compiler_control::CompileContext, CompilerWorkMode};
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn budget(units: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
    }
    fn work(control: &dyn QueryControl) -> BuildWork<'_> {
        BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)))
    }
    struct Stop {
        budget: QueryBudget,
        calls: AtomicUsize,
        at: usize,
        cause: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            kind: QueryCharge,
            units: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(kind, units)?;
            if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
                self.budget.terminate(self.cause);
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    fn fixture(variant: usize) -> (Branch, Vec<TableSchema>) {
        let scans: Vec<_> = [4, 1, 7]
            .into_iter()
            .map(|alias| Scan {
                alias,
                source: LogicalSource::Table("items".into()).into(),
            })
            .collect();
        let mut branch = Branch::single(scans[0].clone());
        branch.core = scans;
        branch.where_conds = vec![
            SqlCond::ColEq(ColRef::new(4, "id"), ColRef::new(1, "id")),
            SqlCond::ColEq(ColRef::new(1, "id"), ColRef::new(7, "id")),
            SqlCond::ColEq(ColRef::new(4, "id"), ColRef::new(4, "id")),
        ];
        let mut table = TableSchema::new("items");
        table.unique = vec![vec!["id".into()]];
        table.columns = vec![sf_sql::Column::new("id", "text", variant != 1)];
        if variant == 2 {
            table.unique.clear();
            table.primary_key = vec!["id".into(), "other".into()];
            branch.where_conds.extend([
                SqlCond::ColEq(ColRef::new(4, "other"), ColRef::new(1, "other")),
                SqlCond::ColEq(ColRef::new(1, "other"), ColRef::new(7, "other")),
            ]);
        }
        if variant == 3 {
            let scans = std::mem::take(&mut branch.core);
            let mut conds = std::mem::take(&mut branch.where_conds);
            conds.insert(
                0,
                SqlCond::ColEq(ColRef::new(99, "id"), ColRef::new(4, "id")),
            );
            branch.where_conds = vec![SqlCond::Exists { scans, conds }];
        }
        (branch, vec![table])
    }

    #[test]
    fn join_fixpoints_match_raw_and_stop_at_every_charge() {
        for variant in 0..4 {
            let (original, schema) = fixture(variant);
            let map = super::super::build_schema_map(&schema);
            let mut expected = original.clone();
            if variant == 3 {
                super::super::self_join_elimination_in_subqueries(&mut expected.where_conds, &map);
            } else {
                super::super::self_join_elimination(&mut expected, &map);
                super::super::nullable_unique_self_join_elimination(&mut expected, &map);
                assert_eq!(expected.core.len(), 1);
            }
            let run = |value: &mut Branch, control: &dyn QueryControl| -> Result<()> {
                if variant == 3 {
                    control_join::subqueries(&mut value.where_conds, &map, work(control))
                } else {
                    control_join::inner(value, &map, work(control))?;
                    control_join::nullable(value, &map, work(control))
                }
            };
            let measured = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at: usize::MAX,
                cause: QueryControlError::Cancelled,
            };
            let mut actual = original.clone();
            run(&mut actual, &measured).unwrap();
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            let units = measured.budget.consumed(QueryCharge::CompilerWork);
            run(&mut original.clone(), &budget(units)).unwrap();
            assert!(matches!(
                run(&mut original.clone(), &budget(units - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
            for cause in [
                QueryControlError::Cancelled,
                QueryControlError::DeadlineExceeded,
            ] {
                for at in 1..=measured.calls.load(Ordering::Relaxed) {
                    let stopped = Stop {
                        budget: budget(u64::MAX),
                        calls: AtomicUsize::new(0),
                        at,
                        cause,
                    };
                    assert!(
                        matches!(run(&mut original.clone(),&stopped),Err(crate::Error::QueryControl(actual)) if actual==cause),
                        "variant {variant}, charge {at}"
                    );
                    assert_eq!(stopped.checkpoint(), Err(cause));
                }
            }
        }
    }

    #[test]
    fn alias_rewrite_preserves_template_guards_and_bounds_recursive_depth() {
        let mut conditions = vec![
            SqlCond::TemplateEq(
                vec![Segment::Literal("unchanged".into())],
                4,
                vec![Segment::Column("id".into())],
                4,
                false,
            ),
            SqlCond::Not(Box::new(SqlCond::DecodedIsNotNull(ColRef::new(4, "id")))),
        ];
        let mut expected = conditions.clone();
        for condition in &mut expected {
            super::super::rewrite_cond_alias(condition, &|column| {
                if column.alias == 4 {
                    column.alias = 1;
                }
            });
        }
        for value in &mut conditions {
            condition(value, 4, 1, work(&budget(u64::MAX))).unwrap();
        }
        assert_eq!(format!("{conditions:?}"), format!("{expected:?}"));
        let SqlCond::TemplateEq(_, left, _, right, _) = &conditions[0] else {
            panic!()
        };
        assert_eq!((*left, *right), (4, 1));
        let mut deep = SqlCond::IsNull(ColRef::new(4, "id"));
        for _ in 0..140 {
            deep = SqlCond::Not(Box::new(deep));
        }
        assert!(matches!(
            condition(&mut deep, 4, 1, work(&budget(u64::MAX))),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerEnvelopeExceeded
            ))
        ));
    }

    #[test]
    fn left_join_decoder_contradiction_and_nullable_guards_match_raw() {
        for variant in 0..4 {
            let (mut original, mut schema) = fixture(0);
            original.where_conds.clear();
            if variant == 3 {
                schema[0].columns[0].not_null = false;
            }
            let extra = match variant {
                0 | 3 => vec![SqlCond::DecodedIsNotNull(ColRef::new(8, "id"))],
                1 => vec![
                    SqlCond::IsNotNull(ColRef::new(8, "value")),
                    SqlCond::DecodedIsNotNull(ColRef::new(8, "value")),
                ],
                _ => {
                    original.where_conds.push(SqlCond::Cmp(
                        ColRef::new(4, "value"),
                        super::super::CmpOp::Eq,
                        "b".into(),
                    ));
                    vec![SqlCond::Cmp(
                        ColRef::new(8, "value"),
                        super::super::CmpOp::Eq,
                        "a".into(),
                    )]
                }
            };
            original.opts.push(crate::iq::OptJoin {
                scan: Scan {
                    alias: 8,
                    source: LogicalSource::Table("items".into()).into(),
                },
                on: vec![SqlCond::NullSafeEq(
                    ColRef::new(4, "id"),
                    ColRef::new(8, "id"),
                )],
                extra,
            });
            let map = super::super::build_schema_map(&schema);
            let mut expected = original.clone();
            super::super::self_left_join_elimination(&mut expected, &map);
            assert_eq!(expected.opts.len(), usize::from(variant == 3));
            let run = |branch: &mut Branch, control: &dyn QueryControl| {
                control_join::left(branch, &map, work(control))
            };
            let measured = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at: usize::MAX,
                cause: QueryControlError::Cancelled,
            };
            let mut actual = original.clone();
            run(&mut actual, &measured).unwrap();
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            let units = measured.budget.consumed(QueryCharge::CompilerWork);
            run(&mut original.clone(), &budget(units)).unwrap();
            assert!(matches!(
                run(&mut original.clone(), &budget(units - 1)),
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
            for cause in [
                QueryControlError::Cancelled,
                QueryControlError::DeadlineExceeded,
            ] {
                for at in 1..=measured.calls.load(Ordering::Relaxed) {
                    let stopped = Stop {
                        budget: budget(u64::MAX),
                        calls: AtomicUsize::new(0),
                        at,
                        cause,
                    };
                    assert!(
                        matches!(run(&mut original.clone(),&stopped),Err(crate::Error::QueryControl(actual)) if actual==cause)
                    );
                }
            }
        }
    }
}
