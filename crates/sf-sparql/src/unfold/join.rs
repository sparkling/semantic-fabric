//! Candidate-product admission for the tree compiler's inner-join lowering.

use crate::{iq::Branch, CompilerWorkMode, Result};

/// Charge all candidate pairs before building any, including pairs that prune.
/// Each left copy is separately measured immediately before that exact clone.
/// Before direct right-field copies, reserve a conservative whole right branch.
/// This is operation-local accounting, not a whole-compiler or merge-work bound.
pub(crate) fn join_branches_with_work_mode(
    left: Vec<Branch>,
    right: Vec<Branch>,
    work: CompilerWorkMode<'_>,
) -> Result<Vec<Branch>> {
    if let CompilerWorkMode::Metered(context) = work {
        context.checkpoint()?;
        context.reserve_checked_product(&[left.len(), right.len()])?;
    }
    let mut out = Vec::new();
    for l in &left {
        for r in &right {
            let copied = match work {
                CompilerWorkMode::Uncontrolled => l.clone(),
                CompilerWorkMode::Metered(context) => context.clone_branch(l)?,
            };
            let merged = match work {
                // The path guard in merge returns before any direct right copy.
                CompilerWorkMode::Metered(context) if copied.path.is_none() && r.path.is_none() => {
                    context.with_reserved_branch_copy(r, |source| super::merge(copied, source))?
                }
                _ => super::merge(copied, r)?,
            };
            if let Some(branch) = merged {
                out.push(branch);
            }
            if let CompilerWorkMode::Metered(context) = work {
                context.checkpoint()?;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_control::CompileContext;
    use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };

    fn budget(work: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
    }

    fn join(
        left: Vec<Branch>,
        right: Vec<Branch>,
        control: &dyn QueryControl,
    ) -> Result<Vec<Branch>> {
        join_branches_with_work_mode(
            left,
            right,
            CompilerWorkMode::Metered(CompileContext::new(control)),
        )
    }

    fn clone_work(branch: &Branch) -> u64 {
        measure_copy_root(CompilerCloneRootV1::Branch(branch))
            .unwrap()
            .total_work
    }

    #[test]
    fn right_branch_payload_cannot_use_only_the_pair_and_left_copy_allowance() {
        let left = Branch::empty();
        let mut right = Branch::single(crate::iq::Scan {
            alias: 2,
            source: sf_core::ir::LogicalSource::Table("source".repeat(1024)).into(),
        });
        right.bindings.insert(
            "right".into(),
            crate::iq::TermDef::Const(
                sf_core::Literal::new_simple_literal("payload".repeat(1024)).into(),
            ),
        );
        let only_left = budget(1 + clone_work(&left));
        assert!(matches!(
            join(vec![left], vec![right], &only_left),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
    }

    #[test]
    fn every_direct_right_field_is_reserved_and_preserves_its_payload() {
        use crate::iq::{ColRef, OptJoin, Scan, SqlCond, SubPlanJoin, TermDef};
        let scan = Scan {
            alias: 2,
            source: sf_core::ir::LogicalSource::Table("source".repeat(1024)).into(),
        };
        let condition = SqlCond::IsNull(ColRef::new(2, "column".repeat(1024)));
        let mut variants = vec![Branch::empty(); 5];
        variants[0].core.push(scan.clone());
        variants[1].bindings.insert(
            "variable".repeat(1024),
            TermDef::Const(sf_core::Literal::new_simple_literal("payload".repeat(1024)).into()),
        );
        variants[2].where_conds.push(condition.clone());
        variants[3].opts.push(OptJoin {
            scan: scan.clone(),
            on: vec![condition.clone()],
            extra: vec![condition.clone()],
        });
        variants[4].subplan_joins.push(SubPlanJoin {
            alias: 3,
            plan: Box::new(crate::Plan {
                branches: vec![Branch::single(scan)],
                form: crate::PlanForm::Select {
                    vars: vec!["inner".repeat(1024)],
                },
                distinct: false,
                limit: None,
                offset: 0,
                order: vec![],
                rust_group: None,
                dialect: sf_sql::Dialect::Sqlite,
                dedup_scopes: vec![],
                construct_drops_some_branch_var: false,
            }),
            on: vec![condition],
            left: false,
        });
        for right in variants {
            let left = Branch::empty();
            let left_work = clone_work(&left);
            let work = 1 + left_work + clone_work(&right);
            let short = budget(work - 1);
            assert!(matches!(
                join(vec![left.clone()], vec![right.clone()], &short),
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
            let measured = measure_copy_root(CompilerCloneRootV1::Branch(&right)).unwrap();
            assert_eq!(
                short.consumed(QueryCharge::CompilerWork),
                1 + left_work + measured.measurement_work
            );
            let exact = budget(work);
            let actual = join(vec![left.clone()], vec![right.clone()], &exact).unwrap();
            let raw = super::super::join_branches(vec![left], vec![right]).unwrap();
            assert_eq!(format!("{actual:?}"), format!("{raw:?}"));
            assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
        }
    }

    #[test]
    fn path_rejection_precedes_any_right_copy_reservation() {
        use crate::iq::{HopExpr, HopRelation, PathClosure, PathKind};
        let mut path = Branch::empty();
        path.path = Some(PathClosure {
            alias: 2,
            kind: PathKind::OneOrMore,
            hop: HopExpr::Pred(HopRelation {
                source: sf_core::ir::LogicalSource::Table("edges".into()),
                subj_col: "s".into(),
                obj_col: "o".into(),
            }),
        });
        for (left, right) in [(path.clone(), Branch::empty()), (Branch::empty(), path)] {
            let work = 1 + clone_work(&left);
            let control = budget(work);
            assert!(matches!(
                join(vec![left], vec![right], &control),
                Err(crate::Error::Unsupported(_))
            ));
            assert_eq!(control.consumed(QueryCharge::CompilerWork), work);
            assert_eq!(control.checkpoint(), Ok(()));
        }
    }

    #[test]
    fn candidate_product_is_reserved_before_any_clone_including_pruned_pairs() {
        // Even when every pair will prune, reserve the attempted product first.
        let branch = |iri| {
            let mut b = Branch::empty();
            b.bindings.insert(
                "shared".into(),
                crate::iq::TermDef::Const(sf_core::NamedNode::new(iri).unwrap().into()),
            );
            b
        };
        let l = branch("http://example.test/left");
        let r = branch("http://example.test/right");
        let insufficient = budget(5);
        assert!(matches!(
            join(vec![l.clone(); 2], vec![r.clone(); 3], &insufficient),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerWorkExceeded
            ))
        ));
        assert_eq!(insufficient.consumed(QueryCharge::CompilerWork), 0);
        let only_pairs = budget(6);
        assert!(join(vec![l.clone(); 2], vec![r.clone(); 3], &only_pairs).is_err());
        assert_eq!(only_pairs.consumed(QueryCharge::CompilerWork), 6);
        let exact = budget(6 + 6 * (clone_work(&l) + clone_work(&r)));
        assert!(join(vec![l; 2], vec![r; 3], &exact).unwrap().is_empty());
        assert_eq!(
            exact.consumed(QueryCharge::CompilerWork),
            exact.limits().max_compiler_work()
        );
    }

    #[test]
    fn exact_pair_and_both_scalar_copy_boundary_preserves_raw_bag() {
        let l = Branch::empty();
        let work = 6 + 12 * clone_work(&l);
        let exact = budget(work);
        let actual = join(vec![l.clone(); 2], vec![l.clone(); 3], &exact).unwrap();
        let raw = super::super::join_branches(vec![l.clone(); 2], vec![l.clone(); 3]).unwrap();
        assert_eq!(actual.len(), 6);
        assert_eq!(format!("{actual:?}"), format!("{raw:?}"));
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
        let short = budget(work - 1);
        assert!(join(vec![l.clone(); 2], vec![l; 3], &short).is_err());
        assert_eq!(
            short.consumed(QueryCharge::CompilerWork),
            work - measure_copy_root(CompilerCloneRootV1::Branch(&Branch::empty()))
                .unwrap()
                .deep_clone_work
        );
    }

    #[test]
    fn empty_products_cost_zero_but_do_not_ignore_cancellation() {
        let zero = budget(0);
        assert!(join(vec![], vec![Branch::empty()], &zero)
            .unwrap()
            .is_empty());
        assert!(join(vec![Branch::empty()], vec![], &zero)
            .unwrap()
            .is_empty());
        zero.terminate(QueryControlError::Cancelled);
        assert!(matches!(
            join(vec![], vec![], &zero),
            Err(crate::Error::QueryControl(QueryControlError::Cancelled))
        ));
    }

    struct CancelAfterProduct(QueryBudget, u64);

    impl QueryControl for CancelAfterProduct {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.0.checkpoint()
        }
        fn consume(
            &self,
            charge: QueryCharge,
            amount: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.0.consume(charge, amount)?;
            if self.0.consumed(QueryCharge::CompilerWork) >= self.1 {
                self.0.terminate(QueryControlError::Cancelled);
            }
            Ok(())
        }
        fn terminate(&self, error: QueryControlError) -> QueryControlError {
            self.0.terminate(error)
        }
    }

    #[test]
    fn cancellation_after_product_reservation_prevents_first_clone() {
        let control = CancelAfterProduct(budget(u64::MAX), 6);
        assert!(matches!(
            join(vec![Branch::empty(); 2], vec![Branch::empty(); 3], &control),
            Err(crate::Error::QueryControl(QueryControlError::Cancelled))
        ));
        assert_eq!(control.0.consumed(QueryCharge::CompilerWork), 6);
    }

    #[test]
    fn cancellation_after_left_copy_prevents_right_reservation() {
        let after_one = 6 + clone_work(&Branch::empty());
        let control = CancelAfterProduct(budget(u64::MAX), after_one);
        assert!(matches!(
            join(vec![Branch::empty(); 2], vec![Branch::empty(); 3], &control),
            Err(crate::Error::QueryControl(QueryControlError::Cancelled))
        ));
        assert_eq!(control.0.consumed(QueryCharge::CompilerWork), after_one);
    }

    #[test]
    fn cancellation_during_right_reservation_prevents_copy_operation() {
        let source = Branch::empty();
        let control = CancelAfterProduct(budget(u64::MAX), clone_work(&source));
        let called = std::cell::Cell::new(false);
        let result = CompileContext::new(&control).with_reserved_branch_copy(&source, |_| {
            called.set(true);
            Ok(())
        });
        assert!(matches!(
            result,
            Err(crate::Error::QueryControl(QueryControlError::Cancelled))
        ));
        assert!(!called.get());
        assert_eq!(
            control.0.consumed(QueryCharge::CompilerWork),
            clone_work(&source)
        );
    }

    #[test]
    fn product_failure_does_not_cache_and_success_retains_shared_hits() {
        use crate::{CompilerBinding, CompilerSchema, Tbox};
        use sf_core::{SourceId, SourceMapping};
        let binding = CompilerBinding::new(
            SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
            sf_sql::Dialect::Sqlite,
            Tbox::default(),
            CompilerSchema::from_unverified_observation(vec![]),
            8,
        );
        let query = "SELECT ?a ?b WHERE { VALUES ?a { 0 1 2 3 } VALUES ?b { 0 1 2 3 } }";
        let key_control = budget(u64::MAX);
        crate::cache::bounded_key::plan_key_with_work_control(
            &crate::parse_query(query).unwrap(),
            binding.scope(),
            crate::cache::CompileProfileId::Uncontrolled,
            &key_control,
        )
        .unwrap();
        let key_work = key_control.consumed(QueryCharge::CompilerWork);
        let crate::Query::Select { pattern, .. } = crate::parse_query(query).unwrap() else {
            panic!()
        };
        let build_control = budget(u64::MAX);
        crate::build::build_tree_with_work_control(&pattern, None, &build_control).unwrap();
        let build_work = build_control.consumed(QueryCharge::CompilerWork);
        let resolve_control = budget(u64::MAX);
        let tbox = Tbox::default();
        let mut resolve_cx =
            crate::iq::resolve::ResolveCx::new(&[], &tbox, sf_sql::Dialect::Sqlite, &[])
                .with_work_mode(crate::CompilerWorkMode::Metered(CompileContext::new(
                    &resolve_control,
                )));
        let resolved = crate::iq::resolve::resolve(
            crate::build::build_tree(&pattern, None).unwrap(),
            &mut resolve_cx,
        )
        .unwrap();
        let normalize_control = budget(u64::MAX);
        let normalized =
            crate::iq::normalize::normalize_with_work_control(resolved, &normalize_control)
                .unwrap();
        let prefix = crate::iq::lower::scope_test_support::entry_work(&normalized).0;
        let (miss, hit, _) = crate::cache::test_work(8, query);
        let prerequisites = key_work
            + miss
            + crate::star::rewrite_work(query)
            + build_work
            + resolve_control.consumed(QueryCharge::CompilerWork)
            + normalize_control.consumed(QueryCharge::CompilerWork)
            + prefix
            + crate::iq::lower::base_work_tests::singleton_work()
            + 1
            + crate::iq::lower::base_work_tests::one_column_rows_work(4, "a");
        // Reach the unpaid first 1×4 product after independent BUILD/RESOLVE/NORMALIZE
        // and LOWER scope/seed/first-child VALUES, never a whole-LOWER estimate.
        let short = budget(prerequisites + 3);
        assert!(binding
            .compile_shared_with_work_control(query, &short)
            .is_err());
        assert_eq!(binding.cache_len(), 0);
        assert_eq!(short.consumed(QueryCharge::CompilerWork), prerequisites);
        let paid = budget(100_000);
        let first = binding
            .compile_shared_with_work_control(query, &paid)
            .unwrap();
        assert!(paid.consumed(QueryCharge::CompilerWork) > 20);
        let second = binding
            .compile_shared_with_work_control(query, &budget(key_work + hit))
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(
            format!("{first:?}"),
            format!("{:?}", binding.compile_uncached_shared(query).unwrap())
        );
    }
}
