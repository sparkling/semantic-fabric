//! Structural NORMALIZE rejection, exact results and same-runtime cache recovery.
use super::cache_key::{structural_compile_work, STRUCTURAL_QUERIES};
use super::*;

async fn set_work(cfg: &mut Arc<ServeConfig>, work: u64) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(cfg) = Arc::get_mut(cfg) {
                cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("existing request owners release the configuration");
}

#[tokio::test]
async fn structural_rules_reject_before_lowering_then_preserve_results_and_hits() {
    for secured in [false, true] {
        for query in STRUCTURAL_QUERIES {
            let mut cfg = Arc::new(if secured {
                protected(u64::MAX)
            } else {
                config(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX))
            });
            let prefix = query.len() as u64
                + key_work(query)
                + miss_work(query)
                + rewrite_work(query)
                + build_work(query);
            let normal = normalization_work(query, &[]);
            assert!(normal > 0);
            // Both failures are within NORMALIZE, never a later LOWER copy or
            // product. Repeating the same miss proves failure did not cache a plan.
            for work in [prefix, prefix + normal - 1, prefix + normal - 1] {
                set_work(&mut cfg, work).await;
                assert_budget_problem(
                    router(cfg.clone())
                        .oneshot(authenticated(query))
                        .await
                        .unwrap(),
                )
                .await;
            }
            let exact = query.len() as u64 + structural_compile_work(query);
            assert!(exact >= prefix + normal);
            set_work(&mut cfg, exact).await;
            assert_values(
                router(cfg.clone())
                    .oneshot(authenticated(query))
                    .await
                    .unwrap(),
            )
            .await;
            // Warm plans skip BUILD/NORMALIZE/LOWER, not paid input/key processing.
            let warm = query.len() as u64 + warm_work(query);
            set_work(&mut cfg, warm - 1).await;
            assert_budget_problem(
                router(cfg.clone())
                    .oneshot(authenticated(query))
                    .await
                    .unwrap(),
            )
            .await;
            set_work(&mut cfg, warm + cache_key::admission_work(query, &[])).await;
            assert_values(
                router(cfg.clone())
                    .oneshot(authenticated(query))
                    .await
                    .unwrap(),
            )
            .await;
        }
    }
}
