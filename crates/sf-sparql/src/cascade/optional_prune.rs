//! DISTINCT removes optional multiplicity, not dependencies on optional values.
use super::{collect_cond_cols, Branch, CascadeCtx, SqlCond};
use std::collections::BTreeSet;

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
        distinct_prune_unused_opts(
            b,
            &CascadeCtx {
                distinct: true,
                project: Some(&["value".to_owned()]),
            },
        );
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
}
