//! Candidate-product admission for the tree compiler's inner-join lowering.

use crate::{iq::Branch, CompilerWorkMode, Result};

/// Charge all candidate pairs before building any, including pairs that prune.
/// Each left copy is separately measured immediately before that exact clone.
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
            if let Some(branch) = super::merge(copied, r)? {
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
    use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
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
        measure_compiler_clone_root_v1(CompilerCloneRootV1::Branch(branch))
            .unwrap()
            .deep_clone_work
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
        let exact = budget(6 + 6 * clone_work(&l));
        assert!(join(vec![l; 2], vec![r; 3], &exact).unwrap().is_empty());
        assert_eq!(
            exact.consumed(QueryCharge::CompilerWork),
            exact.limits().max_compiler_work()
        );
    }

    #[test]
    fn exact_pair_and_scalar_clone_boundary_preserves_raw_bag() {
        let l = Branch::empty();
        let work = 6 + 6 * clone_work(&l);
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
            work - clone_work(&Branch::empty())
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
    fn cancellation_between_pairs_prevents_the_next_clone() {
        let after_one = 6 + clone_work(&Branch::empty());
        let control = CancelAfterProduct(budget(u64::MAX), after_one);
        assert!(matches!(
            join(vec![Branch::empty(); 2], vec![Branch::empty(); 3], &control),
            Err(crate::Error::QueryControl(QueryControlError::Cancelled))
        ));
        assert_eq!(control.0.consumed(QueryCharge::CompilerWork), after_one);
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
        let short = budget(3);
        assert!(binding
            .compile_shared_with_work_control(query, &short)
            .is_err());
        assert_eq!(binding.cache_len(), 0);
        assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
        let paid = budget(100_000);
        let first = binding
            .compile_shared_with_work_control(query, &paid)
            .unwrap();
        assert!(paid.consumed(QueryCharge::CompilerWork) > 20);
        let second = binding
            .compile_shared_with_work_control(query, &budget(0))
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(
            format!("{first:?}"),
            format!("{:?}", binding.compile_uncached_shared(query).unwrap())
        );
    }
}
