//! Execution-time revalidation for persisted shared term-dedup scopes.
use std::collections::BTreeMap;

use crate::iq::Branch;
use crate::{DedupScope, Error, PlanForm, Result};

#[cfg(test)]
mod group_tests {
    use super::*;
    use crate::iq::{Scan, TermDef};
    use sf_core::ir::{LogicalSource, TermMap, TermSpec};
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn budget(units: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
    }
    #[test]
    fn borrowed_selection_ignores_unselected_recipes_and_preserves_optional_collision() {
        let scope = scope(1, "x").unwrap();
        let mut current = scope.key_bindings.clone();
        current.insert(
            "unselected".into(),
            TermDef::Derived {
                alias: 999,
                term_map: TermMap::Column("other".into(), TermSpec::iri()),
            },
        );
        let run = |branch: &Branch, current: &BTreeMap<_, _>| {
            validate_selected_scope(
                branch,
                &scope.key_bindings,
                current,
                sf_sql::source_work::SourceWork::new(None),
                None,
            )
        };
        let mut branch = branch();
        assert!(run(&branch, &current).is_ok());
        branch.bindings.insert("x".into(), current["x"].clone());
        assert!(run(&branch, &current).is_ok());
        branch
            .bindings
            .insert("x".into(), current["unselected"].clone());
        assert!(run(&branch, &current).is_err());
        current.remove("x");
        assert!(run(&branch, &current).is_err());
    }
    #[test]
    fn paid_key_lookup_matches_ordered_map_for_unicode_hits_and_misses() {
        let bindings: BTreeMap<_, _> = ["", "a", "α", "🙂"]
            .into_iter()
            .map(|key| scope(0, key).unwrap().key_bindings.pop_first().unwrap())
            .collect();
        for name in ["", "a", "b", "α", "β", "🙂", "🙃"] {
            let run = |control: &QueryBudget| {
                find_binding(
                    &bindings,
                    name,
                    sf_sql::source_work::SourceWork::new(Some(control)),
                )
            };
            let measured = budget(u64::MAX);
            let expected = bindings.get(name).map(std::ptr::from_ref);
            assert_eq!(run(&measured).unwrap().map(std::ptr::from_ref), expected);
            let total = measured.consumed(QueryCharge::SourceWork);
            assert_eq!(
                run(&budget(total)).unwrap().map(std::ptr::from_ref),
                expected
            );
            assert!(matches!(
                run(&budget(total - 1)),
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
        }
    }
    fn branch() -> Branch {
        Branch::single(Scan {
            alias: 0,
            source: LogicalSource::Table("t".into()).into(),
        })
    }
    fn scope(group_id: usize, name: &str) -> Option<DedupScope> {
        Some(DedupScope {
            group_id,
            key_bindings: BTreeMap::from([(
                name.into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Column("key".into(), TermSpec::iri()),
                },
            )]),
        })
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
    #[test]
    fn borrowed_groups_preserve_interleaving_and_sticky_work_boundaries() {
        let branches: Vec<_> = (0..5).map(|_| branch()).collect();
        let scopes = vec![
            scope(9, "α"),
            None,
            scope(1, "z"),
            scope(9, "α"),
            scope(1, "z"),
        ];
        let original = format!("{scopes:?}");
        let counted = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        assert!(validate_runtime_scopes(&branches, &scopes, &counted).unwrap());
        let total = counted.budget.consumed(QueryCharge::SourceWork);
        assert!(validate_runtime_scopes(&branches, &scopes, &budget(total)).unwrap());
        assert!(matches!(
            validate_runtime_scopes(&branches, &scopes, &budget(total - 1)),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=counted.calls.load(Ordering::Relaxed) {
                let stopped = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(
                    matches!(validate_runtime_scopes(&branches, &scopes, &stopped),
                    Err(Error::QueryControl(actual)) if actual == cause)
                );
            }
        }
        assert_eq!(format!("{scopes:?}"), original);
        assert!(!validate_runtime_scopes(&branches, &[], &budget(0)).unwrap());
        assert!(!validate_runtime_scopes(&branches, &vec![None; 5], &budget(5)).unwrap());
    }
    #[test]
    fn malformed_group_metadata_still_refuses_before_execution() {
        let branches = vec![branch(), branch()];
        for (scopes, message) in [
            (vec![None], "branch ownership"),
            (vec![scope(1, "x"), scope(1, "y")], "key metadata"),
            (vec![scope(1, "x"), scope(2, "x")], "spans two"),
            (
                vec![
                    Some(DedupScope {
                        group_id: 1,
                        key_bindings: BTreeMap::new(),
                    }),
                    None,
                ],
                "key metadata",
            ),
        ] {
            assert!(
                validate_runtime_scopes(&branches, &scopes, &budget(u64::MAX))
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
        }
    }

    #[test]
    fn alias_ownership_walk_is_iterative_and_uses_source_allowance() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut definition = TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Column("key".into(), TermSpec::iri()),
                };
                for _ in 0..4096 {
                    definition = TermDef::Concat(vec![definition]);
                }
                let mut scope = DedupScope {
                    group_id: 1,
                    key_bindings: BTreeMap::from([("x".into(), definition)]),
                };
                let run = |control: &dyn QueryControl| {
                    validate_key_aliases(
                        &scope,
                        0,
                        sf_sql::source_work::SourceWork::new(Some(control)),
                    )
                };
                let measured = budget(u64::MAX);
                run(&measured).unwrap();
                let total = measured.consumed(QueryCharge::SourceWork);
                run(&budget(total)).unwrap();
                assert!(matches!(
                    run(&budget(total - 1)),
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                assert!(matches!(
                    validate_key_aliases(
                        &scope,
                        1,
                        sf_sql::source_work::SourceWork::new(Some(&budget(u64::MAX)))
                    ),
                    Err(Error::Unsupported(_))
                ));
                // The separately tracked recursive IQ Drop is not this alias proof.
                let mut definition = scope.key_bindings.pop_first().unwrap().1;
                while let TermDef::Concat(mut children) = definition {
                    definition = children.pop().unwrap();
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

fn impure_scope() -> Error {
    Error::Unsupported(
        "shared term-dedup requires a pure single-source branch or unary SubPlan chain -> 501"
            .to_owned(),
    )
}

/// Validate group membership using borrowed key maps.
#[cfg(test)]
pub(super) fn validate_runtime_scopes(
    branches: &[Branch],
    scopes: &[Option<DedupScope>],
    control: &dyn sf_core::query_control::QueryControl,
) -> Result<bool> {
    validate_runtime_scopes_with_slice(branches, scopes, control, None)
}

pub(super) fn validate_runtime_scopes_with_slice(
    branches: &[Branch],
    scopes: &[Option<DedupScope>],
    control: &dyn sf_core::query_control::QueryControl,
    slice_override: Option<(Option<usize>, usize)>,
) -> Result<bool> {
    use sf_sql::source_work::{SourceVec, SourceWork};
    let work = SourceWork::new(Some(control));
    work.checkpoint().map_err(super::sql_error::map_sql_err)?;
    if !scopes.is_empty() && scopes.len() != branches.len() {
        return Err(Error::Unsupported(
            "shared term-dedup branch ownership is malformed -> 501".into(),
        ));
    }
    let malformed =
        || Error::Unsupported("shared term-dedup key metadata is malformed -> 501".into());
    let mut groups: SourceVec<(usize, &DedupScope, usize)> = Default::default();
    for (branch, scope) in branches.iter().zip(scopes) {
        work.charge(1).map_err(super::sql_error::map_sql_err)?;
        let Some(scope) = scope else { continue };
        validate_selected_scope(
            branch,
            &scope.key_bindings,
            &scope.key_bindings,
            work,
            slice_override,
        )?;
        work.checkpoint().map_err(super::sql_error::map_sql_err)?;
        if scope.key_bindings.is_empty() {
            return Err(malformed());
        }
        let mut existing = None;
        for (index, (id, _, _)) in groups.as_slice().iter().enumerate() {
            work.charge(1).map_err(super::sql_error::map_sql_err)?;
            if *id == scope.group_id {
                existing = Some(index);
                break;
            }
        }
        if let Some(index) = existing {
            let (id, prior, count) = groups.as_slice()[index];
            if prior.key_bindings.len() != scope.key_bindings.len() {
                return Err(malformed());
            }
            for (left, right) in prior.key_bindings.keys().zip(scope.key_bindings.keys()) {
                work.charge(1).map_err(super::sql_error::map_sql_err)?;
                work.charge(left.len().min(right.len()))
                    .map_err(super::sql_error::map_sql_err)?;
                if left != right {
                    return Err(malformed());
                }
            }
            // There is at most one increment per element of the input slice.
            groups
                .replace_copy(index, (id, prior, count + 1), work)
                .map_err(super::sql_error::map_sql_err)?;
        } else {
            groups
                .push((scope.group_id, scope, 1), work)
                .map_err(super::sql_error::map_sql_err)?;
        }
    }
    for (_, _, count) in groups.as_slice() {
        work.charge(1).map_err(super::sql_error::map_sql_err)?;
        if *count < 2 {
            return Err(Error::Unsupported(
                "shared term-dedup group no longer spans two executable branches -> 501".into(),
            ));
        }
    }
    work.checkpoint().map_err(super::sql_error::map_sql_err)?;
    Ok(!groups.as_slice().is_empty())
}

fn validate_selected_scope<'a>(
    mut branch: &'a Branch,
    selected: &BTreeMap<String, crate::iq::TermDef>,
    mut definitions: &'a BTreeMap<String, crate::iq::TermDef>,
    work: sf_sql::source_work::SourceWork<'_>,
    mut slice_override: Option<(Option<usize>, usize)>,
) -> Result<()> {
    let mut cleared_slice = false;
    loop {
        work.charge(1).map_err(super::sql_error::map_sql_err)?;
        let (limit, offset) = slice_override
            .take()
            .unwrap_or((branch.limit, branch.offset));
        if branch.path.is_some()
            || branch.agg.is_some()
            || (!cleared_slice && (limit.is_some() || offset > 0))
            || branch.nps
        {
            return Err(impure_scope());
        }
        if !matches!(
            (
                branch.core.len(),
                branch.opts.len(),
                branch.subplan_joins.len()
            ),
            (1, 0, 0) | (0, 0, 1)
        ) {
            return Err(impure_scope());
        }

        if let Some(scan) = branch.core.first() {
            validate_selected_aliases(selected, definitions, scan.alias, work)?;
            for variable in selected.keys() {
                let expected =
                    find_binding(definitions, variable, work)?.ok_or_else(impure_scope)?;
                if let Some(actual) = find_binding(&branch.bindings, variable, work)? {
                    if !super::driver::source_prepare::recipes_same(actual, expected, work)? {
                        return Err(impure_scope());
                    }
                }
            }
            return Ok(());
        }

        let wrapper = branch.subplan_joins.first().ok_or_else(impure_scope)?;
        let nested = &wrapper.plan;
        if branch.subplan_joins.len() != 1
            || wrapper.left
            || !wrapper.on.is_empty()
            || nested.branches.len() != 1
            || nested.limit.is_some()
            || nested.offset > 0
            || nested.rust_group.is_some()
            || !matches!(nested.form, PlanForm::Select { .. })
            || !nested.dedup_scopes.is_empty()
        {
            return Err(impure_scope());
        }
        validate_selected_aliases(selected, definitions, wrapper.alias, work)?;
        let PlanForm::Select { vars } = &nested.form else {
            return Err(impure_scope());
        };
        for variable in selected.keys() {
            let mut found = false;
            for candidate in vars {
                if key_order(candidate, variable, work)? == std::cmp::Ordering::Equal {
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(impure_scope());
            }
        }
        let nested_branch = nested.branches.first().ok_or_else(impure_scope)?;
        // Preserve missing-key refusal before projection/remapping of any key.
        for variable in selected.keys() {
            find_binding(&nested_branch.bindings, variable, work)?.ok_or_else(impure_scope)?;
        }
        let projection = crate::emit::projection_controlled(
            nested_branch,
            nested.dialect,
            nested.distinct,
            work,
        )?;
        for variable in selected.keys() {
            let definition =
                find_binding(&nested_branch.bindings, variable, work)?.ok_or_else(impure_scope)?;
            let expected = find_binding(definitions, variable, work)?.ok_or_else(impure_scope)?;
            if !super::driver::source_prepare::recipes_remapped_same(
                expected,
                definition,
                &projection,
                wrapper.alias,
                work,
            )? {
                return Err(impure_scope());
            }
        }
        // Single unordered prepared branches inherit the already-proven empty
        // plan slice. DISTINCT affects projection, not this pure ownership proof.
        cleared_slice = nested.order.is_empty();
        branch = nested_branch;
        definitions = &nested_branch.bindings;
    }
}

fn key_order(
    left: &str,
    right: &str,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<std::cmp::Ordering> {
    work.charge(1).map_err(super::sql_error::map_sql_err)?;
    work.charge(left.len().min(right.len()))
        .map_err(super::sql_error::map_sql_err)?;
    Ok(left.cmp(right))
}

/// Paid comparisons over the existing ordered map; no cloned search keys or
/// hidden tree-comparison work. Stop once the sorted keys pass the target.
pub(super) fn find_binding<'a>(
    bindings: &'a BTreeMap<String, crate::iq::TermDef>,
    name: &str,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<&'a crate::iq::TermDef>> {
    work.charge(1).map_err(super::sql_error::map_sql_err)?;
    for (key, value) in bindings {
        match key_order(key, name, work)? {
            std::cmp::Ordering::Equal => return Ok(Some(value)),
            std::cmp::Ordering::Greater => return Ok(None),
            std::cmp::Ordering::Less => {}
        }
    }
    Ok(None)
}

/// The alias proof needs no copied ColRef list and no recursive TermDef walk.
#[cfg(test)]
fn validate_key_aliases(
    scope: &DedupScope,
    alias: usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    validate_selected_aliases(&scope.key_bindings, &scope.key_bindings, alias, work)
}

fn validate_selected_aliases(
    selected: &BTreeMap<String, crate::iq::TermDef>,
    definitions: &BTreeMap<String, crate::iq::TermDef>,
    alias: usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    for variable in selected.keys() {
        let definition = find_binding(definitions, variable, work)?.ok_or_else(impure_scope)?;
        crate::emit::validate_definition_columns(definition, work, |actual, _| {
            if actual == alias {
                Ok(())
            } else {
                Err(impure_scope())
            }
        })?;
    }
    work.checkpoint().map_err(super::sql_error::map_sql_err)
}
