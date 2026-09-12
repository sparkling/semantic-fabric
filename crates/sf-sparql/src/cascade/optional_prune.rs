//! DISTINCT removes optional multiplicity, not dependencies on optional values.
use super::{collect_cond_cols, Branch, CascadeCtx, SqlCond};
use std::collections::BTreeSet;

pub(super) fn self_left_extra_with_work(
    opt: &crate::iq::OptJoin,
    table: &sf_sql::TableSchema,
    work: crate::build::control::BuildWork<'_>,
) -> crate::Result<bool> {
    let mut nullable = None;
    for condition in &opt.extra {
        work.charge(1)?;
        let column = match condition {
            SqlCond::DecodedIsNotNull(column) if column.alias == opt.scan.alias => {
                if !super::control_distinct::non_null(table, &column.column, work)? {
                    let mut guarded = false;
                    for condition in &opt.extra {
                        work.charge(1)?;
                        if let SqlCond::IsNotNull(other) = condition {
                            if super::control_fd::same(other, column, work)? {
                                guarded = true;
                                break;
                            }
                        }
                    }
                    if !guarded {
                        return Ok(false);
                    }
                }
                continue;
            }
            SqlCond::IsNotNull(column) if column.alias == opt.scan.alias => column,
            _ => return Ok(false),
        };
        if !super::control_distinct::non_null(table, &column.column, work)? {
            if let Some(previous) = nullable {
                if !super::control_fd::same(previous, column, work)? {
                    return Ok(false);
                }
            }
            nullable = Some(column);
        }
    }
    work.checkpoint()?;
    Ok(true)
}

/// Decoder obligations are relocated, never discarded or used as key proof.
/// Their NULL behavior must already be covered by ordinary guards/schema.
pub(super) fn self_left_extra_compatible(
    opt: &crate::iq::OptJoin,
    table: &sf_sql::TableSchema,
) -> bool {
    let mut nullable = None;
    for cond in &opt.extra {
        let col = match cond {
            SqlCond::DecodedIsNotNull(col) if col.alias == opt.scan.alias => {
                if !super::key_is_non_null(table, &col.column)
                    && !opt
                        .extra
                        .iter()
                        .any(|c| matches!(c, SqlCond::IsNotNull(other) if other == col))
                {
                    return false;
                }
                continue;
            }
            SqlCond::IsNotNull(col) if col.alias == opt.scan.alias => col,
            _ => return false,
        };
        if !super::key_is_non_null(table, &col.column) {
            if nullable.is_some_and(|other| other != col) {
                return false;
            }
            nullable = Some(col);
        }
    }
    true
}

/// An unused LEFT JOIN only repeats the same projected tuple (or preserves one
/// NULL-extended row), so DISTINCT absorbs its multiplicity. This proof fails
/// if a surviving condition or operator consumes the optional values.
pub(super) fn distinct_prune_unused_opts(b: &mut Branch, ctx: &CascadeCtx) {
    if !ctx.distinct
        || !b.order.is_empty()
        || b.agg.is_some()
        || b.path.is_some()
        || !b.subplan_joins.is_empty()
    {
        return;
    }
    let Some(project) = ctx.project else { return };
    let mut required = BTreeSet::new();
    for (var, def) in &b.bindings {
        if project.contains(var) {
            required.extend(def.columns().into_iter().map(|col| col.alias));
        }
    }
    for cond in &b.where_conds {
        condition_aliases(cond, &mut |alias| {
            required.insert(alias);
        });
    }
    for opt in &b.opts {
        if opt
            .on
            .iter()
            .chain(&opt.extra)
            .any(|c| crate::iq::decode_valid::references(c, None))
        {
            required.insert(opt.scan.alias);
        }
        // Its own ON cannot filter the preserved left row, but another opt's
        // correlation may need this scan even when its binding is unprojected.
        for cond in opt.on.iter().chain(&opt.extra) {
            condition_aliases(cond, &mut |alias| {
                if alias != opt.scan.alias {
                    required.insert(alias);
                }
            });
        }
    }
    // Conservatively retain dependencies of even an opt removed in this pass.
    // This is one bounded walk, not a new fixed-point optimization.
    b.opts.retain(|opt| required.contains(&opt.scan.alias));
}

fn condition_aliases(cond: &SqlCond, visit: &mut impl FnMut(usize)) {
    match cond {
        SqlCond::Not(inner) => condition_aliases(inner, visit),
        SqlCond::And(conds)
        | SqlCond::Or(conds)
        | SqlCond::Exists { conds, .. }
        | SqlCond::NotExists { conds, .. }
        | SqlCond::PathExists { conds, .. } => {
            // Unlike ordinary projection collection, pruning must inspect
            // existential correlations. Inner aliases may over-retain only.
            for cond in conds {
                condition_aliases(cond, visit);
            }
        }
        _ => collect_cond_cols(cond, &mut |col| visit(col.alias)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{ColRef, OptJoin, Scan, TermDef};
    use sf_core::ir::{LogicalSource, TermMap, TermSpec};

    fn scan(alias: usize) -> Scan {
        Scan {
            alias,
            source: LogicalSource::Table("items".to_owned()).into(),
        }
    }
    #[test]
    fn paid_parent_coverage_keeps_alias_specific_decoder_guards() {
        use crate::{
            build::control::BuildWork, compiler_control::CompileContext, CompilerWorkMode,
        };
        use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
        for variant in 0..3 {
            let mut b = branch();
            b.where_conds
                .push(SqlCond::Not(Box::new(SqlCond::DecodedIsNotNull(
                    ColRef::new(if variant == 0 { 1 } else { 9 }, "id"),
                ))));
            let allowed: &[&str] = if variant == 2 {
                &["id"]
            } else {
                &["id", "value"]
            };
            let run = |budget: &QueryBudget| {
                super::super::control_distinct::parent_columns(
                    &b,
                    1,
                    allowed,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(budget))),
                )
            };
            let budget =
                |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
            let measured = budget(u64::MAX);
            assert_eq!(run(&measured).unwrap(), variant == 1);
            let units = measured.consumed(QueryCharge::CompilerWork);
            assert_eq!(run(&budget(units)).unwrap(), variant == 1);
            assert!(matches!(
                run(&budget(units - 1)),
                Err(crate::Error::QueryControl(
                    sf_core::query_control::QueryControlError::CompilerWorkExceeded
                ))
            ));
        }
    }

    #[test]
    fn foreign_key_does_not_prove_filtered_optional_matches() {
        let mut b = branch();
        let mut table = sf_sql::TableSchema::new("items");
        table.primary_key = vec!["id".into()];
        table.foreign_keys.push(sf_sql::ForeignKey {
            columns: vec!["id".into()],
            parent_table: "items".into(),
            parent_columns: vec!["id".into()],
        });
        // Even a self FK cannot make a nullable payload IS NOT NULL true.
        // LEFT preserves a row with NULL payload; INNER would remove it.
        super::super::joinelim::lj_to_ij_fk_downgrade(&mut b, &[table]);
        assert_eq!(b.opts.len(), 1, "extra predicate can reject the FK match");
        assert_eq!(b.core.len(), 1);
    }
    fn binding(alias: usize) -> TermDef {
        TermDef::Derived {
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
            alias,
        }
    }
    fn branch() -> Branch {
        let mut b = Branch::single(scan(0));
        b.bindings.insert("value".into(), binding(0));
        b.opts.push(OptJoin {
            scan: scan(1),
            on: vec![SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id"))],
            extra: vec![SqlCond::IsNotNull(ColRef::new(1, "value"))],
        });
        b
    }
    fn prune(b: &mut Branch) {
        let mut controlled = b.clone();
        let control = sf_core::query_control::QueryBudget::new(
            sf_core::query_control::QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
        );
        super::super::control_distinct::prune_optional(
            &mut controlled,
            &CascadeCtx {
                distinct: true,
                project: Some(&["value".to_owned()]),
            },
            crate::build::control::BuildWork::new(crate::CompilerWorkMode::Metered(
                crate::compiler_control::CompileContext::new(&control),
            )),
        )
        .unwrap();
        distinct_prune_unused_opts(
            b,
            &CascadeCtx {
                distinct: true,
                project: Some(&["value".to_owned()]),
            },
        );
        assert_eq!(format!("{controlled:?}"), format!("{b:?}"));
    }

    #[test]
    fn controlled_optional_prune_exact_and_every_stop_preserve_unpublished_input() {
        use sf_core::query_control::{
            QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        let budget =
            |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
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
        let run = |b: &mut Branch, control: &dyn QueryControl| {
            super::super::control_distinct::prune_optional(
                b,
                &CascadeCtx {
                    distinct: true,
                    project: Some(&["value".into()]),
                },
                crate::build::control::BuildWork::new(crate::CompilerWorkMode::Metered(
                    crate::compiler_control::CompileContext::new(control),
                )),
            )
        };
        let original = branch();
        let measured = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        let mut pruned = original.clone();
        run(&mut pruned, &measured).unwrap();
        assert!(pruned.opts.is_empty());
        let units = measured.budget.consumed(QueryCharge::CompilerWork);
        run(&mut original.clone(), &budget(units)).unwrap();
        let mut failed = original.clone();
        assert!(matches!(
            run(&mut failed, &budget(units - 1)),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        assert_eq!(format!("{failed:?}"), format!("{original:?}"));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=measured.calls.load(Ordering::Relaxed) {
                let control = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                let mut failed = original.clone();
                assert!(
                    matches!(run(&mut failed,&control),Err(crate::Error::QueryControl(actual)) if actual==cause)
                );
                assert_eq!(format!("{failed:?}"), format!("{original:?}"));
            }
        }
    }

    #[test]
    fn where_and_nested_existential_consumers_retain_unprojected_optional() {
        let condition = SqlCond::NullSafeEq(ColRef::new(1, "value"), ColRef::new(0, "value"));
        for cond in [
            condition.clone(),
            SqlCond::Or(vec![SqlCond::NotExists {
                scans: vec![scan(2)],
                conds: vec![condition],
            }]),
        ] {
            let mut b = branch();
            b.where_conds.push(cond);
            prune(&mut b);
            assert_eq!(b.opts.len(), 1);
        }
    }

    #[test]
    fn later_optional_correlation_retains_its_unprojected_input() {
        let mut b = branch();
        b.bindings.insert("value".into(), binding(2));
        b.opts.push(OptJoin {
            scan: scan(2),
            on: vec![SqlCond::NullSafeEq(
                ColRef::new(1, "value"),
                ColRef::new(2, "value"),
            )],
            extra: vec![],
        });
        prune(&mut b);
        assert_eq!(
            b.opts.iter().map(|opt| opt.scan.alias).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn own_on_and_extra_do_not_keep_a_truly_unused_optional() {
        let mut b = branch();
        prune(&mut b);
        assert!(b.opts.is_empty());
    }

    #[test]
    fn hidden_decoder_validation_retains_optional_and_has_no_key_authority() {
        let mut b = branch();
        let condition = SqlCond::DecodedIsNotNull(ColRef::new(1, "value"));
        b.opts[0].extra.push(SqlCond::And(vec![condition.clone()]));
        prune(&mut b);
        assert_eq!(b.opts.len(), 1);
        let mut only_validation = Branch::single(scan(1));
        only_validation.where_conds.push(condition);
        assert!(super::super::distinct_scan::lexical_keys(&only_validation, 1).is_empty());
        assert!(super::super::distinct_scan::native_keys(&only_validation, 1).is_empty());
    }

    #[test]
    fn only_verified_nonnull_guards_are_tautologies_for_self_left_join() {
        let mut b = branch();
        b.opts[0]
            .extra
            .push(SqlCond::IsNotNull(ColRef::new(1, "id")));
        let mut table = sf_sql::TableSchema::new("items");
        table.primary_key = vec!["id".to_owned()];
        let tables = [table];
        let schema = super::super::build_schema_map(&tables);
        assert!(super::super::find_self_left_join(&b, &schema).is_some());
        // Two independent nullable components condition the entire optional
        // row: one non-NULL component cannot stand in for the missing other.
        b.opts[0]
            .extra
            .push(SqlCond::IsNotNull(ColRef::new(1, "other")));
        assert!(super::super::find_self_left_join(&b, &schema).is_none());
        assert!(super::super::find_self_left_join(&b, &vec![]).is_none());
    }

    #[test]
    fn self_left_join_retains_nullable_decoder_obligations_on_the_same_row() {
        let mut b = branch();
        b.opts[0].extra.extend([
            SqlCond::IsNotNull(ColRef::new(1, "id")),
            SqlCond::DecodedIsNotNull(ColRef::new(1, "id")),
            SqlCond::DecodedIsNotNull(ColRef::new(1, "value")),
        ]);
        for value_type in ["text", "NUMERIC", "unknown"] {
            let mut table = sf_sql::TableSchema::new("items");
            table.primary_key = vec!["id".into()];
            table.columns = vec![
                sf_sql::Column::new("id", "text", true),
                sf_sql::Column::new("value", value_type, false),
            ];
            let tables = [table];
            let schema = super::super::build_schema_map(&tables);
            let mut candidate = b.clone();
            super::super::self_left_join_elimination(&mut candidate, &schema);
            assert!(candidate.opts.is_empty(), "{value_type}");
            for name in ["id", "value"] {
                let col = ColRef::new(0, name);
                assert!(candidate.where_conds.iter().any(|cond| matches!(cond,
                    SqlCond::Or(parts) if matches!(parts.as_slice(),
                        [SqlCond::IsNull(a), SqlCond::DecodedIsNotNull(b)] if a == &col && b == &col)
                )), "decoder survives NULL-tolerantly: {name}, {value_type}");
            }
            assert!(
                !candidate
                    .where_conds
                    .iter()
                    .any(|c| matches!(c, SqlCond::IsNotNull(_) | SqlCond::DecodedIsNotNull(_))),
                "nullable OPTIONAL must not filter the left row"
            );
        }
        b.opts[0]
            .extra
            .retain(|c| !matches!(c, SqlCond::IsNotNull(col) if col.column.as_ref() == "value"));
        let mut table = sf_sql::TableSchema::new("items");
        table.primary_key = vec!["id".into()];
        assert!(
            !self_left_extra_compatible(&b.opts[0], &table),
            "unpaired nullable marker cannot license elimination"
        );
    }
}
