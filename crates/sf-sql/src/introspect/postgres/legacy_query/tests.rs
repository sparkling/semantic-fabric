use std::cell::Cell;

use futures_util::{stream, StreamExt};

use super::*;

#[tokio::test]
async fn bounded_collector_accepts_the_exact_cap() {
    let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2)]);
    assert_eq!(
        collect_bounded_rows(rows, 2, "test rows")
            .await
            .expect("exact cap is admitted"),
        vec![1, 2]
    );
}

#[tokio::test]
async fn bounded_collector_polls_only_the_first_overflow_row() {
    let polls = Cell::new(0);
    let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2), Ok(3), Ok(4)])
        .inspect(|_| polls.set(polls.get() + 1));
    let error = collect_bounded_rows(rows, 2, "test rows")
        .await
        .expect_err("cap plus one must reject");
    assert_eq!(polls.get(), 3);
    assert!(error.to_string().contains("test rows"));
}

#[test]
fn production_row_limits_match_adr_0051() {
    assert_eq!(MAX_LEGACY_RELATIONS_PG16_V1, 4_096);
    assert_eq!(MAX_LEGACY_ROWS_PER_SET_PG16_V1, 65_536);
    assert_eq!(LEGACY_RELATION_QUERY_LIMIT_PG16_V1, 4_097);
    assert_eq!(LEGACY_SET_QUERY_LIMIT_PG16_V1, 65_537);
}
