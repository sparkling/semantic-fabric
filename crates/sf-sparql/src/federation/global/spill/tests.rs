use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sf_core::query_control::{
    QueryBudget, QueryControlError, QueryLimits, ReservationLimits, ReservationShape,
};

use super::{
    checked_reservation_shape, frame, SecureSpillRun, SpillConfig, SpillError, SpillIdentity,
    MAX_BLOCKS, MAX_PLAINTEXT_BLOCK,
};

static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

pub(super) struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("sf-secure-spill-test-{}-{id}", std::process::id()));
        fs::create_dir(&root).expect("unique fixture root");
        Self { root }
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn entries(&self) -> usize {
        fs::read_dir(&self.root).expect("fixture root").count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn identity(seed: u8) -> SpillIdentity {
    SpillIdentity::new(
        [seed; 32],
        [seed.wrapping_add(1); 32],
        [seed.wrapping_add(2); 32],
    )
}

pub(super) fn budget_for(shape: ReservationShape) -> QueryBudget {
    QueryBudget::new(
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX).with_reservation_limits(
            ReservationLimits::new(
                shape.retained_bytes(),
                shape.spill_bytes(),
                shape.spill_files(),
                shape.file_descriptors(),
                shape.operator_tasks(),
            ),
        ),
    )
}

#[test]
fn exact_reservation_is_held_until_drop_then_fully_released() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(64, 2).expect("valid config");
    let shape = config.reservation_shape().expect("bounded shape");
    let budget = budget_for(shape);

    let run = SecureSpillRun::create(fixture.root(), &budget, identity(1), config)
        .expect("exact reservation succeeds");
    assert_eq!(budget.reserved(), shape);
    assert_eq!(fixture.entries(), 1);

    drop(run);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);
}

#[test]
fn every_n_plus_one_dimension_rejects_before_artifact_creation() {
    let config = SpillConfig::new(64, 2).expect("valid config");
    let exact = config.reservation_shape().expect("bounded shape");
    let insufficient = [
        ReservationLimits::new(
            exact.retained_bytes() - 1,
            exact.spill_bytes(),
            exact.spill_files(),
            exact.file_descriptors(),
            exact.operator_tasks(),
        ),
        ReservationLimits::new(
            exact.retained_bytes(),
            exact.spill_bytes() - 1,
            exact.spill_files(),
            exact.file_descriptors(),
            exact.operator_tasks(),
        ),
        ReservationLimits::new(
            exact.retained_bytes(),
            exact.spill_bytes(),
            exact.spill_files() - 1,
            exact.file_descriptors(),
            exact.operator_tasks(),
        ),
        ReservationLimits::new(
            exact.retained_bytes(),
            exact.spill_bytes(),
            exact.spill_files(),
            exact.file_descriptors() - 1,
            exact.operator_tasks(),
        ),
        ReservationLimits::new(
            exact.retained_bytes(),
            exact.spill_bytes(),
            exact.spill_files(),
            exact.file_descriptors(),
            exact.operator_tasks() - 1,
        ),
    ];

    for limits in insufficient {
        let fixture = Fixture::new();
        let budget = QueryBudget::new(
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_reservation_limits(limits),
        );
        assert!(matches!(
            SecureSpillRun::create(fixture.root(), &budget, identity(2), config),
            Err(SpillError::ReservationRejected)
        ));
        assert_eq!(budget.reserved(), ReservationShape::ZERO);
        assert_eq!(fixture.entries(), 0, "reservation must precede creation");
    }
}

#[test]
fn frame_overhead_inode_count_and_max_multiplication_are_checked_exactly() {
    let config = SpillConfig::new(37, 4).unwrap();
    let frame_bytes = u64::try_from(frame::HEADER_LEN + 37 + frame::TAG_LEN).unwrap();
    let shape = config.reservation_shape().unwrap();
    assert_eq!(shape.retained_bytes(), frame_bytes);
    assert_eq!(shape.spill_bytes(), frame_bytes * 4);
    assert_eq!(shape.spill_files(), 5, "four blocks plus the run directory");

    assert!(SpillConfig::new(MAX_PLAINTEXT_BLOCK, MAX_BLOCKS as u64).is_ok());
    assert_eq!(
        SpillConfig::new(MAX_PLAINTEXT_BLOCK + 1, MAX_BLOCKS as u64),
        Err(SpillError::InvalidConfiguration)
    );
    assert_eq!(
        SpillConfig::new(MAX_PLAINTEXT_BLOCK, MAX_BLOCKS as u64 + 1),
        Err(SpillError::InvalidConfiguration)
    );
    assert_eq!(
        checked_reservation_shape(2, u64::MAX - 1),
        Err(SpillError::InvalidConfiguration),
        "multiplication overflow must fail closed"
    );
    assert_eq!(
        checked_reservation_shape(1, u64::MAX),
        Err(SpillError::InvalidConfiguration),
        "run-directory inode addition must not wrap"
    );
}

#[test]
fn bounded_blocks_round_trip_as_one_reused_borrowed_buffer() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(64, 2).expect("valid config");
    let budget = budget_for(config.reservation_shape().unwrap());
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(3), config).unwrap();

    run.write_block(b"first bounded block").unwrap();
    run.write_block(b"second bounded block").unwrap();
    let first = run.read_next().unwrap();
    let first_address = first.as_ptr();
    assert_eq!(first, b"first bounded block");
    let second = run.read_next().unwrap();
    assert_eq!(
        second.as_ptr(),
        first_address,
        "scratch allocation is reused"
    );
    assert_eq!(second, b"second bounded block");
    assert_eq!(run.read_next(), Err(SpillError::EndOfBlocks));
}

#[test]
fn reader_refuses_to_grow_beyond_the_preallocated_scratch_capacity() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(64, 1).unwrap();
    let budget = budget_for(config.reservation_shape().unwrap());
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(35), config).unwrap();
    run.write_block(b"authenticated but no longer capacity-backed")
        .unwrap();
    run.test_discard_scratch_capacity();

    assert_eq!(run.read_next(), Err(SpillError::LengthMismatch));
    assert!(run.test_scratch_is_erased());
    assert_eq!(run.read_next(), Err(SpillError::RunFailed));
}

#[test]
fn block_and_byte_caps_reject_before_a_new_file_exists() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(4, 2).unwrap();
    let budget = budget_for(config.reservation_shape().unwrap());
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(4), config).unwrap();

    assert_eq!(run.write_block(b"12345"), Err(SpillError::QuotaExceeded));
    assert_eq!(run.block_count(), 0);
    run.write_block(b"1234").unwrap();
    run.write_block(b"5678").unwrap();
    assert_eq!(run.write_block(b"x"), Err(SpillError::QuotaExceeded));
    assert_eq!(run.block_count(), 2);
    assert_eq!(
        fs::read_dir(run.test_run_path(fixture.root()))
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn run_and_files_have_private_exact_modes_and_random_names() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(8, 1).unwrap();
    let budget = budget_for(config.reservation_shape().unwrap());
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(5), config).unwrap();
    run.write_block(b"private").unwrap();

    let run_path = run.test_run_path(fixture.root());
    let run_mode = fs::metadata(&run_path).unwrap().permissions().mode() & 0o777;
    let block_path = run.test_block_path(fixture.root(), 0);
    let block_mode = fs::metadata(&block_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(run_mode, 0o700);
    assert_eq!(block_mode, 0o600);
    assert!(run.test_run_name().starts_with("run-"));
    assert!(run.test_block_name(0).starts_with("blk-"));
}

#[test]
fn explicit_finish_cancel_and_injected_error_all_release_and_clean() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(16, 1).unwrap();
    let shape = config.reservation_shape().unwrap();

    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(6), config).unwrap();
    run.write_block(b"finish").unwrap();
    assert_eq!(run.finish(), Ok(()));
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);

    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(7), config).unwrap();
    run.write_block(b"cancel").unwrap();
    run.cancel();
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);

    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(8), config).unwrap();
    assert_eq!(
        run.test_fail_after_file_create(),
        Err(SpillError::IoFailure)
    );
    assert_eq!(
        fs::read_dir(run.test_run_path(fixture.root()))
            .unwrap()
            .count(),
        1
    );
    drop(run);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);
}

#[test]
fn panic_unwind_runs_owned_cleanup_and_exact_token_drop() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(16, 1).unwrap();
    let shape = config.reservation_shape().unwrap();
    let budget = budget_for(shape);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(9), config).unwrap();
        run.write_block(b"unwind").unwrap();
        panic!("injected unwind");
    }));
    assert!(result.is_err());
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);
}

#[test]
fn budget_cancellation_is_checked_before_and_after_bounded_io() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(16, 1).unwrap();
    let shape = config.reservation_shape().unwrap();

    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(30), config).unwrap();
    budget.terminate(QueryControlError::Cancelled);
    assert_eq!(
        run.write_block(b"no file"),
        Err(SpillError::QueryTerminated)
    );
    assert_eq!(
        fs::read_dir(run.test_run_path(fixture.root()))
            .unwrap()
            .count(),
        0
    );
    drop(run);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);

    let budget = budget_for(shape);
    let cancellation = budget.clone();
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(31), config).unwrap();
    assert_eq!(
        run.write_block_with_hook(b"one block", || {
            cancellation.terminate(QueryControlError::Cancelled);
        }),
        Err(SpillError::QueryTerminated)
    );
    assert_eq!(
        fs::read_dir(run.test_run_path(fixture.root()))
            .unwrap()
            .count(),
        1,
        "post-I/O cancellation leaves the owned file for Drop cleanup"
    );
    drop(run);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);
}

#[test]
fn cancellation_before_read_rejects_and_drop_cleans_the_existing_block() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(16, 1).unwrap();
    let shape = config.reservation_shape().unwrap();
    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(32), config).unwrap();
    run.write_block(b"read later").unwrap();
    budget.terminate(QueryControlError::Cancelled);
    assert_eq!(run.read_next(), Err(SpillError::QueryTerminated));
    drop(run);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);
}

#[test]
fn seal_failure_is_sticky_and_cleanup_residue_is_outside_released_admission() {
    let fixture = Fixture::new();
    let config = SpillConfig::new(16, 1).unwrap();
    let shape = config.reservation_shape().unwrap();

    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(33), config).unwrap();
    run.test_discard_scratch_capacity();
    assert_eq!(
        run.write_block(b"cannot seal"),
        Err(SpillError::QuotaExceeded)
    );
    assert_eq!(run.write_block(b"retry"), Err(SpillError::RunFailed));
    assert_eq!(run.finish(), Err(SpillError::RunFailed));
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert_eq!(fixture.entries(), 0);

    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(fixture.root(), &budget, identity(34), config).unwrap();
    assert_eq!(
        run.test_fail_after_file_create(),
        Err(SpillError::IoFailure)
    );
    let run_path = run.test_run_path(fixture.root());
    let sibling = run_path.join("unowned");
    fs::write(&sibling, b"keep").unwrap();
    assert_eq!(run.finish(), Err(SpillError::CleanupFailed));
    // The residue is real, but a per-query admission token cannot honestly
    // represent global disk high-water after its owner has ended.
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
    assert!(sibling.exists());
    assert_eq!(fs::read(&sibling).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&run_path).unwrap().count(), 1);
}
