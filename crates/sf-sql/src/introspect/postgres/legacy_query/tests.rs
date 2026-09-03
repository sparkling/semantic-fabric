use std::cell::Cell;

use futures_util::{stream, StreamExt};

use super::*;

#[tokio::test]
async fn bounded_collector_accepts_the_exact_cap() {
    let mapped = Cell::new(0);
    let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2)]);
    assert_eq!(
        collect_bounded_mapped_rows(
            rows,
            2,
            |_| Error::Introspection("driver failure".to_owned()),
            || Error::Introspection("test rows exceeds the configured limit".to_owned()),
            |row| {
                mapped.set(mapped.get() + 1);
                Ok(row)
            },
        )
        .await
        .expect("exact cap is admitted"),
        vec![1, 2]
    );
    assert_eq!(mapped.get(), 2);
}

#[tokio::test]
async fn bounded_collector_polls_only_the_first_overflow_row() {
    let polls = Cell::new(0);
    let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2), Ok(3), Ok(4)])
        .inspect(|_| polls.set(polls.get() + 1));
    let error = collect_bounded_mapped_rows(
        rows,
        2,
        |_| Error::Introspection("driver failure".to_owned()),
        || Error::Introspection("test rows exceeds the configured limit".to_owned()),
        Ok,
    )
    .await
    .expect_err("cap plus one must reject");
    assert_eq!(polls.get(), 3);
    assert!(error.to_string().contains("test rows"));
}

#[tokio::test]
async fn bounded_mapped_collector_rejects_cap_plus_one_before_mapping_it() {
    let polls = Cell::new(0);
    let mapped = Cell::new(0);
    let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2), Ok(3), Ok(4)])
        .inspect(|_| polls.set(polls.get() + 1));
    let error = collect_bounded_mapped_rows(
        rows,
        2,
        |_| Error::Introspection("driver failure".to_owned()),
        || Error::Introspection("mapped test rows exceeds the configured limit".to_owned()),
        |row| {
            mapped.set(mapped.get() + 1);
            Ok(row)
        },
    )
    .await
    .expect_err("cap plus one must reject before mapping the sentinel");
    assert_eq!(polls.get(), 3);
    assert_eq!(mapped.get(), 2);
    assert!(error.to_string().contains("mapped test rows"));
}

#[test]
fn production_row_limits_match_adr_0051() {
    assert_eq!(MAX_LEGACY_RELATIONS_PG16_V1, 4_096);
    assert_eq!(MAX_LEGACY_ROWS_PER_SET_PG16_V1, 65_536);
    assert_eq!(LEGACY_RELATION_QUERY_LIMIT_PG16_V1, 4_097);
    assert_eq!(LEGACY_SET_QUERY_LIMIT_PG16_V1, 65_537);
}
