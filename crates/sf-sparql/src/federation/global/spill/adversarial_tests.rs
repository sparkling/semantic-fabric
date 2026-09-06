use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::symlink;

use sf_core::query_control::ReservationShape;

use super::frame::{DECLARED_LEN_OFFSET, VERSION_OFFSET};
use super::tests::{budget_for, identity, Fixture};
use super::{SecureSpillRun, SpillConfig, SpillError};

fn configured_run(
    seed: u8,
    blocks: u64,
) -> (Fixture, sf_core::query_control::QueryBudget, SecureSpillRun) {
    let fixture = Fixture::new();
    let config = SpillConfig::new(64, blocks).unwrap();
    let budget = budget_for(config.reservation_shape().unwrap());
    let run = SecureSpillRun::create(fixture.root(), &budget, identity(seed), config).unwrap();
    (fixture, budget, run)
}

fn flip_byte(path: &std::path::Path, offset: u64) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 1;
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&byte).unwrap();
}

#[test]
fn ciphertext_corruption_fails_authentication_and_drop_cleans_owned_artifacts() {
    let (fixture, budget, mut run) = configured_run(10, 1);
    run.write_block(b"authenticate me").unwrap();
    let path = run.test_block_path(fixture.root(), 0);
    let len = fs::metadata(&path).unwrap().len();
    flip_byte(&path, len - 1);

    assert_eq!(run.read_next(), Err(SpillError::AuthenticationFailed));
    assert!(run.test_scratch_is_erased());
    drop(run);
    assert_eq!(fixture.entries(), 0);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn truncated_and_malformed_declared_lengths_fail_closed() {
    let (fixture, _budget, mut run) = configured_run(11, 1);
    run.write_block(b"truncate me").unwrap();
    let path = run.test_block_path(fixture.root(), 0);
    let len = fs::metadata(&path).unwrap().len();
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(len - 1)
        .unwrap();
    assert_eq!(run.read_next(), Err(SpillError::TruncatedBlock));

    let (fixture, _budget, mut run) = configured_run(12, 1);
    run.write_block(b"bad length").unwrap();
    let path = run.test_block_path(fixture.root(), 0);
    flip_byte(&path, DECLARED_LEN_OFFSET as u64);
    assert_eq!(run.read_next(), Err(SpillError::LengthMismatch));
}

#[test]
fn unsupported_version_fails_before_decryption() {
    let (fixture, _budget, mut run) = configured_run(13, 1);
    run.write_block(b"versioned").unwrap();
    flip_byte(
        &run.test_block_path(fixture.root(), 0),
        VERSION_OFFSET as u64,
    );
    assert_eq!(run.read_next(), Err(SpillError::UnsupportedVersion));
}

#[test]
fn malformed_magic_fails_closed_without_reflecting_bytes() {
    let (fixture, _budget, mut run) = configured_run(35, 1);
    run.write_block(b"malformed").unwrap();
    flip_byte(&run.test_block_path(fixture.root(), 0), 0);
    let error = run.read_next().unwrap_err();
    assert_eq!(error, SpillError::MalformedBlock);
    assert_eq!(error.to_string(), "secure-spill block is malformed");
}

#[test]
fn replay_or_reordering_is_rejected_by_expected_sequence() {
    let (fixture, _budget, mut run) = configured_run(14, 2);
    run.write_block(b"same-size-one").unwrap();
    run.write_block(b"same-size-two").unwrap();
    let first = run.test_block_path(fixture.root(), 0);
    let second = run.test_block_path(fixture.root(), 1);
    fs::copy(first, second).unwrap();

    assert_eq!(run.read_next().unwrap(), b"same-size-one");
    assert_eq!(run.read_next(), Err(SpillError::SequenceMismatch));
}

#[test]
fn cross_query_and_cross_schema_blocks_are_rejected() {
    let (source_fixture, _source_budget, mut source) = configured_run(15, 1);
    source.write_block(b"foreign block").unwrap();
    let source_path = source.test_block_path(source_fixture.root(), 0);

    let (target_fixture, _target_budget, mut target) = configured_run(16, 1);
    target.write_block(b"foreign block").unwrap();
    fs::copy(
        &source_path,
        target.test_block_path(target_fixture.root(), 0),
    )
    .unwrap();
    assert_eq!(target.read_next(), Err(SpillError::IdentityMismatch));

    let fixture = Fixture::new();
    let config = SpillConfig::new(64, 1).unwrap();
    let budget = budget_for(config.reservation_shape().unwrap());
    let mut schema_target = SecureSpillRun::create(
        fixture.root(),
        &budget,
        super::SpillIdentity::new([15; 32], [16; 32], [99; 32]),
        config,
    )
    .unwrap();
    schema_target.write_block(b"foreign block").unwrap();
    fs::copy(
        source_path,
        schema_target.test_block_path(fixture.root(), 0),
    )
    .unwrap();
    assert_eq!(schema_target.read_next(), Err(SpillError::IdentityMismatch));
}

#[test]
fn symlinked_root_components_are_never_followed() {
    let fixture = Fixture::new();
    let actual = fixture.root().join("actual");
    fs::create_dir(&actual).unwrap();
    let link = fixture.root().join("link");
    symlink(&actual, &link).unwrap();
    let config = SpillConfig::new(8, 1).unwrap();
    let budget = budget_for(config.reservation_shape().unwrap());

    assert_eq!(
        SecureSpillRun::create(&link, &budget, identity(20), config).unwrap_err(),
        SpillError::RootUnavailable
    );
    assert_eq!(fs::read_dir(actual).unwrap().count(), 0);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn held_root_descriptor_is_not_re_resolved_after_path_replacement() {
    let fixture = Fixture::new();
    let configured = fixture.root().join("configured");
    let held = fixture.root().join("held-original");
    fs::create_dir(&configured).unwrap();
    let config = SpillConfig::new(16, 1).unwrap();
    let shape = config.reservation_shape().unwrap();
    let budget = budget_for(shape);
    let mut run = SecureSpillRun::create(&configured, &budget, identity(23), config).unwrap();

    fs::rename(&configured, &held).unwrap();
    fs::create_dir(&configured).unwrap();
    let replacement_marker = configured.join("do-not-touch");
    fs::write(&replacement_marker, b"replacement").unwrap();
    run.write_block(b"still descriptor relative").unwrap_err();
    // The plaintext cap rejected before I/O; a valid block still targets the
    // already-held run descriptor, never the replacement path.
    run.write_block(b"held fd").unwrap();
    drop(run);

    assert_eq!(fs::read(&replacement_marker).unwrap(), b"replacement");
    assert_eq!(fs::read_dir(&held).unwrap().count(), 0);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn cleanup_never_removes_unowned_siblings_or_follows_replacements() {
    let (fixture, budget, mut run) = configured_run(21, 1);
    run.write_block(b"owned").unwrap();
    let run_path = run.test_run_path(fixture.root());
    let block_path = run.test_block_path(fixture.root(), 0);
    let sibling = run_path.join("unowned-sibling");
    fs::write(&sibling, b"keep me").unwrap();
    let target = fixture.root().join("outside-target");
    fs::write(&target, b"untouched").unwrap();
    fs::remove_file(&block_path).unwrap();
    symlink(&target, &block_path).unwrap();

    assert_eq!(run.read_next(), Err(SpillError::OwnershipMismatch));
    drop(run);
    assert_eq!(fs::read(&target).unwrap(), b"untouched");
    assert!(sibling.exists());
    assert!(fs::symlink_metadata(block_path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn absolute_non_root_path_is_required_without_consuming_capacity() {
    let config = SpillConfig::new(8, 1).unwrap();
    let shape = config.reservation_shape().unwrap();
    let budget = budget_for(shape);
    assert_eq!(
        SecureSpillRun::create(
            std::path::Path::new("relative"),
            &budget,
            identity(22),
            config
        )
        .unwrap_err(),
        SpillError::InvalidConfiguration
    );
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}
