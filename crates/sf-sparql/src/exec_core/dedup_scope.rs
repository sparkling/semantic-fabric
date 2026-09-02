//! Lift resolve-time term-dedup markers onto executable plan branches.
//!
//! Resolve records the stable alias of each standalone mapping arm because IQ
//! lowering can wrap that arm in one or more derived-table [`crate::Plan`]s. The
//! executor, however, runs only the root plan's branches; nested plans are emitted
//! as SQL. This module performs the one safe conversion between those two shapes,
//! after lowering and every cascade rewrite have finished.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::iq::{Branch, TermDef};
use crate::unfold::DedupMarker;
use crate::{DedupScope, Error, PlanForm, Result};

/// Convert resolve-time source markers into root-executor branch-aligned scopes.
///
/// A marker may bubble through any number of pure unary SubPlan wrappers. It may
/// not cross a join, OPTIONAL, aggregate, slice, multi-branch SQL SubPlan, or one
/// executable branch reaching multiple groups: those operators run before the Rust seen
/// set and change its semantic scope. Every source marker must be consumed exactly
/// once, and every group must still span at least two executor branches.
pub(crate) fn lift_dedup_scopes(
    branches: &[Branch],
    source_groups: &HashMap<usize, DedupMarker>,
) -> Result<Vec<Option<DedupScope>>> {
    if source_groups.is_empty() {
        return Ok(Vec::new());
    }

    let expected_groups: BTreeSet<usize> = source_groups
        .values()
        .map(|marker| marker.group_id)
        .collect();
    let mut expected_keys = BTreeMap::<usize, BTreeSet<String>>::new();
    for marker in source_groups.values() {
        if marker.key_bindings.is_empty() {
            return Err(malformed_key());
        }
        let keys: BTreeSet<String> = marker.key_bindings.keys().cloned().collect();
        if expected_keys
            .insert(marker.group_id, keys.clone())
            .is_some_and(|existing| existing != keys)
        {
            return Err(malformed_key());
        }
    }

    let mut owned = BTreeMap::<usize, usize>::new();
    let mut lifted = Vec::with_capacity(branches.len());
    for branch in branches {
        lifted.push(lift_branch_scope(branch, source_groups, &mut owned)?);
    }

    if source_groups
        .keys()
        .any(|alias| owned.get(alias).copied() != Some(1))
    {
        return Err(Error::Unsupported(
            "shared term-dedup marker is not owned by exactly one executable branch -> 501"
                .to_owned(),
        ));
    }

    let mut branch_counts = BTreeMap::<usize, usize>::new();
    for scope in lifted.iter().flatten() {
        *branch_counts.entry(scope.group_id).or_default() += 1;
    }
    if expected_groups
        .iter()
        .any(|group| branch_counts.get(group).copied().unwrap_or(0) < 2)
    {
        return Err(Error::Unsupported(
            "shared term-dedup group no longer spans two executable branches -> 501".to_owned(),
        ));
    }

    Ok(lifted)
}

fn lift_branch_scope(
    branch: &Branch,
    source_groups: &HashMap<usize, DedupMarker>,
    owned: &mut BTreeMap<usize, usize>,
) -> Result<Option<DedupScope>> {
    let reachable = reachable_tagged_aliases(branch, source_groups);
    if reachable.is_empty() {
        return Ok(None);
    }
    let groups: BTreeSet<usize> = reachable
        .iter()
        .filter_map(|alias| source_groups.get(alias).map(|marker| marker.group_id))
        .collect();
    if groups.len() != 1 {
        return Err(Error::Unsupported(
            "one executable branch reaches multiple shared term-dedup groups -> 501".to_owned(),
        ));
    }
    let group = *groups.iter().next().expect("one group checked");

    if branch.path.is_some()
        || branch.agg.is_some()
        || branch.limit.is_some()
        || branch.offset > 0
        || branch.nps
    {
        return Err(impure_scope());
    }

    let contributors = branch.core.len() + branch.opts.len() + branch.subplan_joins.len();
    if contributors != 1 || !branch.opts.is_empty() {
        return Err(impure_scope());
    }

    if let Some(scan) = branch.core.first() {
        if branch.core.len() != 1
            || reachable.len() != 1
            || !reachable.contains(&scan.alias)
            || source_groups
                .get(&scan.alias)
                .is_none_or(|marker| marker.group_id != group)
        {
            return Err(impure_scope());
        }
        *owned.entry(scan.alias).or_default() += 1;
        let marker = source_groups.get(&scan.alias).ok_or_else(impure_scope)?;
        if marker.key_bindings.is_empty() {
            return Err(malformed_key());
        }
        return Ok(Some(DedupScope {
            group_id: marker.group_id,
            key_bindings: marker.key_bindings.clone(),
        }));
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
        return Err(Error::Unsupported(
            "shared term-dedup cannot cross a multi-branch or modifier-bearing SQL SubPlan -> 501"
                .to_owned(),
        ));
    }

    let Some(nested_scope) = lift_branch_scope(&nested.branches[0], source_groups, owned)? else {
        return Err(impure_scope());
    };
    if nested_scope.group_id != group {
        return Err(impure_scope());
    }
    let PlanForm::Select { vars } = &nested.form else {
        return Err(impure_scope());
    };
    if nested_scope
        .key_bindings
        .keys()
        .any(|variable| !vars.contains(variable))
    {
        return Err(Error::Unsupported(
            "shared term-dedup cannot cross a nested projection that removes its key -> 501"
                .to_owned(),
        ));
    }

    let prepared = nested.prepared_branches();
    let prepared_branch = prepared.first().ok_or_else(impure_scope)?;
    if nested_scope
        .key_bindings
        .iter()
        .any(|(variable, expected)| {
            prepared_branch
                .bindings
                .get(variable)
                .is_none_or(|actual| format!("{actual:?}") != format!("{expected:?}"))
        })
    {
        return Err(Error::Unsupported(
            "shared term-dedup cannot cross a wrapper that does not physically emit its key -> 501"
                .to_owned(),
        ));
    }
    let projection = crate::emit::emit_branch(prepared_branch, nested.dialect)?.projection;
    let key_bindings = nested_scope
        .key_bindings
        .iter()
        .map(|(variable, definition)| {
            crate::iq::lower::remap_termdef(definition, &projection, wrapper.alias)
                .map(|remapped| (variable.clone(), remapped))
        })
        .collect::<Result<_>>()?;
    Ok(Some(DedupScope {
        group_id: group,
        key_bindings,
    }))
}

fn reachable_tagged_aliases(
    branch: &Branch,
    source_groups: &HashMap<usize, DedupMarker>,
) -> BTreeSet<usize> {
    let mut aliases = BTreeSet::new();
    collect_tagged_aliases(branch, source_groups, &mut aliases);
    aliases
}

fn collect_tagged_aliases(
    branch: &Branch,
    source_groups: &HashMap<usize, DedupMarker>,
    aliases: &mut BTreeSet<usize>,
) {
    for (alias, _) in branch.alias_sources() {
        if source_groups.contains_key(&alias) {
            aliases.insert(alias);
        }
    }
    for wrapper in &branch.subplan_joins {
        for nested in &wrapper.plan.branches {
            collect_tagged_aliases(nested, source_groups, aliases);
        }
    }
}

fn impure_scope() -> Error {
    Error::Unsupported(
        "shared term-dedup requires a pure single-source branch or unary SubPlan chain -> 501"
            .to_owned(),
    )
}

fn malformed_key() -> Error {
    Error::Unsupported("shared term-dedup key metadata is malformed -> 501".to_owned())
}

pub(super) fn overlay_key_bindings(
    branch: &mut Branch,
    key_bindings: &BTreeMap<String, TermDef>,
) -> Result<()> {
    for (variable, definition) in key_bindings {
        if let Some(existing) = branch.bindings.get(variable) {
            if format!("{existing:?}") != format!("{definition:?}") {
                return Err(Error::Unsupported(
                    "shared term-dedup key collides with a different branch binding -> 501"
                        .to_owned(),
                ));
            }
            continue;
        }
        branch.bindings.insert(variable.clone(), definition.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{Scan, SubPlanJoin, TermDef};
    use crate::{Plan, PlanForm};
    use sf_core::ir::{LogicalSource, TermMap, TermSpec};
    use sf_sql::Dialect;

    fn leaf(alias: usize) -> Branch {
        let mut branch = Branch::single(Scan {
            alias,
            source: LogicalSource::Table(format!("source_{alias}")),
        });
        branch.bindings.insert(
            "v".to_owned(),
            TermDef::Derived {
                term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
                alias,
            },
        );
        branch
    }

    fn plan(branches: Vec<Branch>) -> Plan {
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
            dialect: Dialect::Postgres,
            dedup_scopes: Vec::new(),
            construct_drops_some_branch_var: false,
        }
    }

    fn wrap_without_key(inner: Branch, alias: usize) -> Branch {
        let mut outer = Branch::empty();
        outer.subplan_joins.push(SubPlanJoin {
            alias,
            plan: Box::new(plan(vec![inner])),
            on: Vec::new(),
            left: false,
        });
        outer
    }

    fn wrap(inner: Branch, alias: usize) -> Branch {
        let definition = inner.bindings.get("v").cloned().expect("inner key");
        let nested = plan(vec![inner]);
        let prepared = nested.prepared_branches();
        let projection = crate::emit::emit_branch(&prepared[0], nested.dialect)
            .expect("inner emits")
            .projection;
        let remapped =
            crate::iq::lower::remap_termdef(&definition, &projection, alias).expect("key remaps");
        let mut outer = wrap_without_key(nested.branches[0].clone(), alias);
        outer.bindings.insert("v".to_owned(), remapped);
        outer
    }

    fn marker(alias: usize, group_id: usize) -> (usize, DedupMarker) {
        (
            alias,
            DedupMarker {
                group_id,
                key_bindings: leaf(alias).bindings,
            },
        )
    }

    #[test]
    fn lifts_through_two_pure_wrapper_levels() {
        let mut branches = vec![wrap(wrap(leaf(1), 11), 21), wrap(wrap(leaf(2), 12), 22)];
        for branch in &mut branches {
            branch.bindings.remove("v");
        }
        let groups = HashMap::from([marker(1, 99), marker(2, 99)]);

        let scopes = lift_dedup_scopes(&branches, &groups).unwrap();
        assert_eq!(scopes.len(), 2);
        for (scope, wrapper_alias) in scopes.iter().zip([21, 22]) {
            let scope = scope.as_ref().expect("both branches are group members");
            assert_eq!(scope.group_id, 99);
            assert_eq!(scope.key_bindings.keys().collect::<Vec<_>>(), vec!["v"]);
            assert!(matches!(
                scope.key_bindings.get("v"),
                Some(TermDef::Derived { alias, .. }) if *alias == wrapper_alias
            ));
        }
        let mut executable = branches[0].clone();
        overlay_key_bindings(&mut executable, &scopes[0].as_ref().unwrap().key_bindings).unwrap();
        let sql = crate::emit::emit_branch(&executable, Dialect::Postgres)
            .unwrap()
            .sql;
        assert!(sql.contains("value"));
        assert!(!sql.contains("SELECT 1 AS c0"));
    }

    #[test]
    fn rejects_a_wrapper_whose_declared_key_is_not_physically_emitted() {
        let branches = vec![
            wrap_without_key(wrap_without_key(leaf(1), 11), 21),
            wrap_without_key(wrap_without_key(leaf(2), 12), 22),
        ];
        let groups = HashMap::from([marker(1, 99), marker(2, 99)]);

        assert!(lift_dedup_scopes(&branches, &groups).is_err());
    }

    #[test]
    fn rejects_a_wrapper_that_physically_emits_a_different_key() {
        let mut branches = vec![wrap(wrap(leaf(1), 11), 21), wrap(wrap(leaf(2), 12), 22)];
        branches[0].subplan_joins[0].plan.branches[0]
            .bindings
            .insert(
                "v".to_owned(),
                TermDef::Const(oxrdf::Literal::new_simple_literal("different").into()),
            );
        let groups = HashMap::from([marker(1, 99), marker(2, 99)]);

        assert!(lift_dedup_scopes(&branches, &groups).is_err());
    }

    #[test]
    fn rejects_a_tagged_relation_joined_with_a_direct_source() {
        let mut joined = wrap(leaf(1), 11);
        joined.core.push(Scan {
            alias: 3,
            source: LogicalSource::Table("other".to_owned()),
        });
        let error = lift_dedup_scopes(
            &[joined, wrap(leaf(2), 12)],
            &HashMap::from([marker(1, 99), marker(2, 99)]),
        );

        assert!(matches!(error, Err(Error::Unsupported(_))));
    }

    #[test]
    fn rejects_a_tag_inside_a_multi_branch_sql_subplan() {
        let mut outer = Branch::empty();
        outer.subplan_joins.push(SubPlanJoin {
            alias: 11,
            plan: Box::new(plan(vec![leaf(1), leaf(3)])),
            on: Vec::new(),
            left: false,
        });
        let error = lift_dedup_scopes(
            &[outer, wrap(leaf(2), 12)],
            &HashMap::from([marker(1, 99), marker(2, 99)]),
        );

        assert!(matches!(error, Err(Error::Unsupported(_))));
    }

    #[test]
    fn rejects_an_unowned_or_multiply_owned_source_marker() {
        let groups = HashMap::from([marker(1, 99), marker(2, 99)]);
        assert!(lift_dedup_scopes(&[leaf(1)], &groups).is_err());
        assert!(lift_dedup_scopes(&[leaf(1), leaf(1), leaf(2)], &groups).is_err());
    }

    #[test]
    fn restores_a_direct_key_removed_by_the_outer_projection() {
        let mut projected = leaf(1);
        projected.bindings.remove("v");
        let scopes = lift_dedup_scopes(
            &[projected, leaf(2)],
            &HashMap::from([marker(1, 99), marker(2, 99)]),
        )
        .unwrap();

        assert!(scopes[0]
            .as_ref()
            .is_some_and(|scope| scope.key_bindings.contains_key("v")));
    }

    #[test]
    fn rejects_a_nested_projection_that_removed_the_pattern_key() {
        let mut nested = plan(vec![leaf(1)]);
        nested.form = PlanForm::Select { vars: Vec::new() };
        let mut projected = Branch::empty();
        projected.subplan_joins.push(SubPlanJoin {
            alias: 11,
            plan: Box::new(nested),
            on: Vec::new(),
            left: false,
        });

        assert!(lift_dedup_scopes(
            &[projected, wrap(leaf(2), 12)],
            &HashMap::from([marker(1, 99), marker(2, 99)]),
        )
        .is_err());
    }
}
