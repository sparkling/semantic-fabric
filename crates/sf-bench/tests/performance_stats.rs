use sf_bench::performance::stats::{summarize, StatsError, MIN_P95_SAMPLES};

#[test]
fn should_compute_exact_half_nanosecond_median_for_even_samples() {
    let raw: Vec<u64> = (1..=18).chain([100, 101]).collect();

    let summary = summarize(&raw).expect("20 samples are sufficient");

    assert_eq!(summary.median.to_string(), "10.5");
}

#[test]
fn should_avoid_overflow_when_even_median_uses_large_values() {
    let mut raw = vec![u64::MAX - 1; MIN_P95_SAMPLES];
    raw[MIN_P95_SAMPLES - 1] = u64::MAX;

    let summary = summarize(&raw).expect("20 samples are sufficient");

    assert_eq!(summary.median.to_string(), (u64::MAX - 1).to_string());
}

#[test]
fn should_use_nearest_rank_for_p95() {
    let raw: Vec<u64> = (1..=20).collect();

    let summary = summarize(&raw).expect("20 samples are sufficient");

    assert_eq!(summary.p95, 19);
}

#[test]
fn should_reject_p95_when_fewer_than_twenty_raw_samples_exist() {
    let error = summarize(&(1..=19).collect::<Vec<_>>()).expect_err("p95 must be bounded");

    assert_eq!(
        error,
        StatsError::InsufficientSamples {
            actual: 19,
            minimum: MIN_P95_SAMPLES,
        }
    );
}
