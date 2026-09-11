use super::cache_key::{constant_compile_work, CONSTANT_QUERIES};
use super::*;

async fn set_work(cfg: &mut Arc<ServeConfig>, work: u64) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(cfg) = Arc::get_mut(cfg) {
                cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("compiler owners release the same configuration");
}

#[tokio::test]
async fn constant_row_rules_reject_unpaid_work_then_recover_and_reuse_warm_plans() {
    for secured in [false, true] {
        for query in CONSTANT_QUERIES {
            let mut cfg = Arc::new(if secured {
                protected(u64::MAX)
            } else {
                config(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX))
            });
            let input = query.len() as u64;
            let prerequisites = input + key_work(query) + build_work(query);
            let exact = input + constant_compile_work(query);
            assert!(exact > prerequisites, "normalization is independently paid");
            for work in [prerequisites, exact - 1] {
                set_work(&mut cfg, work).await;
                assert_budget_problem(
                    router(cfg.clone())
                        .oneshot(authenticated(query))
                        .await
                        .unwrap(),
                )
                .await;
            }
            set_work(&mut cfg, exact).await;
            assert_values(
                router(cfg.clone())
                    .oneshot(authenticated(query))
                    .await
                    .unwrap(),
            )
            .await;
            // Same runtime and cache; only the per-request allowance changes.
            let warm = input + key_work(query);
            set_work(&mut cfg, warm - 1).await;
            assert_budget_problem(
                router(cfg.clone())
                    .oneshot(authenticated(query))
                    .await
                    .unwrap(),
            )
            .await;
            set_work(&mut cfg, warm).await;
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
