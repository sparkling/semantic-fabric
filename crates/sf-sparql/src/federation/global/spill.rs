//! Private secure-spill comparison substrate for proposed ADR-0040.
//!
//! Nothing in this module is reachable from the public planner or serving
//! surfaces. Linux is intentional: the prototype's filesystem authority is
//! descriptor-relative and relies on no-follow `openat` operations.
//! Concurrent hostile same-UID namespace mutation is explicitly not contained
//! here because Linux offers no unprivileged compare-and-unlink-by-inode call.
//! Retained-byte accounting covers the reusable logical frame buffer; fixed
//! stack state and allocator bookkeeping remain outside QueryBudget's model.
//! A cleanup failure releases the per-query admission token even if residue
//! remains. This is not a global disk high-water accounting scheme; production
//! use requires a janitor plus global residue accounting. Successful reads keep
//! one bounded plaintext block resident until the next mutation or run drop;
//! memory locking, swap exclusion, and plaintext copied by callers are outside
//! this prototype's claim. Key erasure covers the stable owner and RustCrypto's
//! stored cipher state, not compiler-generated or library-internal derived
//! temporaries.

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::ops::Range;
use std::path::Path;

use sf_core::query_control::{QueryBudget, QueryControl, ReservationShape, ReservationToken};
use zeroize::Zeroize;

mod filesystem;
mod frame;

use filesystem::{OwnedArtifacts, MAX_BLOCKS};
use frame::{EphemeralKey, FrameBinding, HEADER_LEN, TAG_LEN};

const MAX_PLAINTEXT_BLOCK: usize = 16 * 1024 * 1024;
// `OwnedArtifacts` retains exactly the root and run directory descriptors.
// Exclusive `&mut self` I/O admits at most one transient block descriptor, so
// the structural peak is 2 + 1 rather than a block-count-dependent value.
const HELD_DIRECTORY_DESCRIPTORS: u64 = 2;
const TRANSIENT_BLOCK_DESCRIPTORS: u64 = 1;
const HELD_FILE_DESCRIPTORS: u64 = HELD_DIRECTORY_DESCRIPTORS + TRANSIENT_BLOCK_DESCRIPTORS;
const HELD_OPERATOR_TASKS: u64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SpillIdentity {
    // Fixed-size non-secret identities only: never raw query text, parameter
    // values, credentials, or schema material.
    query: [u8; 32],
    operator: [u8; 32],
    schema: [u8; 32],
}

impl SpillIdentity {
    const fn new(query: [u8; 32], operator: [u8; 32], schema: [u8; 32]) -> Self {
        Self {
            query,
            operator,
            schema,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SpillConfig {
    max_plaintext_block: usize,
    max_blocks: u64,
}

impl SpillConfig {
    fn new(max_plaintext_block: usize, max_blocks: u64) -> Result<Self, SpillError> {
        if max_plaintext_block == 0
            || max_plaintext_block > MAX_PLAINTEXT_BLOCK
            || max_plaintext_block > u32::MAX as usize
            || max_blocks == 0
            || max_blocks > MAX_BLOCKS as u64
        {
            return Err(SpillError::InvalidConfiguration);
        }
        Ok(Self {
            max_plaintext_block,
            max_blocks,
        })
    }

    fn reservation_shape(self) -> Result<ReservationShape, SpillError> {
        let frame_bytes = frame::encoded_len(self.max_plaintext_block)?;
        checked_reservation_shape(frame_bytes, self.max_blocks)
    }
}

fn checked_reservation_shape(
    frame_bytes: usize,
    max_blocks: u64,
) -> Result<ReservationShape, SpillError> {
    let frame_bytes = u64::try_from(frame_bytes).map_err(|_| SpillError::InvalidConfiguration)?;
    let spill_bytes = frame_bytes
        .checked_mul(max_blocks)
        .ok_or(SpillError::InvalidConfiguration)?;
    // The run directory consumes one governed spill inode in addition to its
    // bounded block files. This deliberately uses the conservative file quota.
    let spill_inodes = max_blocks
        .checked_add(1)
        .ok_or(SpillError::InvalidConfiguration)?;
    Ok(ReservationShape::new(
        frame_bytes,
        spill_bytes,
        spill_inodes,
        HELD_FILE_DESCRIPTORS,
        HELD_OPERATOR_TASKS,
    ))
}

/// Closed errors: no path, query identifier, schema, OS detail, or ciphertext
/// is ever rendered into an error surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
enum SpillError {
    #[error("secure-spill configuration is invalid")]
    InvalidConfiguration,
    #[error("secure-spill resource reservation rejected")]
    ReservationRejected,
    #[error("secure-spill query is terminal")]
    QueryTerminated,
    #[error("secure-spill allocation failed")]
    AllocationFailed,
    #[error("secure-spill entropy unavailable")]
    EntropyUnavailable,
    #[error("secure-spill root unavailable")]
    RootUnavailable,
    #[error("secure-spill run creation failed")]
    RunCreateFailed,
    #[error("secure-spill file creation failed")]
    FileCreateFailed,
    #[error("secure-spill quota exceeded")]
    QuotaExceeded,
    #[error("secure-spill I/O failed")]
    IoFailure,
    #[error("secure-spill block is malformed")]
    MalformedBlock,
    #[error("secure-spill block version is unsupported")]
    UnsupportedVersion,
    #[error("secure-spill block identity does not match")]
    IdentityMismatch,
    #[error("secure-spill block sequence does not match")]
    SequenceMismatch,
    #[error("secure-spill block length does not match")]
    LengthMismatch,
    #[error("secure-spill block is truncated")]
    TruncatedBlock,
    #[error("secure-spill block authentication failed")]
    AuthenticationFailed,
    #[error("secure-spill block encryption failed")]
    EncryptionFailed,
    #[error("secure-spill artifact ownership does not match")]
    OwnershipMismatch,
    #[error("secure-spill has no more blocks")]
    EndOfBlocks,
    #[error("secure-spill run is failed")]
    RunFailed,
    #[error("secure-spill cleanup was incomplete")]
    CleanupFailed,
}

struct SecureSpillRun {
    config: SpillConfig,
    binding: FrameBinding,
    key: EphemeralKey,
    scratch: Vec<u8>,
    artifacts: OwnedArtifacts,
    written_bytes: u64,
    block_count: usize,
    read_index: usize,
    failed: bool,
    cleaned: bool,
    budget: QueryBudget,
    _reservation: ReservationToken,
}

impl fmt::Debug for SecureSpillRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureSpillRun")
            .field("config", &self.config)
            .field("block_count", &self.block_count)
            .field("read_index", &self.read_index)
            .field("failed", &self.failed)
            .field("identity", &"[REDACTED]")
            .finish()
    }
}

impl SecureSpillRun {
    fn create(
        root: &Path,
        budget: &QueryBudget,
        identity: SpillIdentity,
        config: SpillConfig,
    ) -> Result<Self, SpillError> {
        // Validate before reservation, then validate again inside the
        // descriptor-opening authority so it remains safe when called alone.
        filesystem::validate_root_path(root)?;
        let shape = config.reservation_shape()?;

        // This single all-or-nothing token is acquired before the first heap
        // allocation, random generation, descriptor open, or artifact create.
        let reservation = budget
            .reserve(shape)
            .map_err(|_| SpillError::ReservationRejected)?;
        budget
            .checkpoint()
            .map_err(|_| SpillError::QueryTerminated)?;
        let scratch_capacity = usize::try_from(shape.retained_bytes())
            .map_err(|_| SpillError::InvalidConfiguration)?;
        let mut scratch = Vec::new();
        scratch
            .try_reserve_exact(scratch_capacity)
            .map_err(|_| SpillError::AllocationFailed)?;
        let key = EphemeralKey::generate()?;
        let mut run = [0_u8; 16];
        getrandom::fill(&mut run).map_err(|_| SpillError::EntropyUnavailable)?;
        budget
            .checkpoint()
            .map_err(|_| SpillError::QueryTerminated)?;
        let artifacts = OwnedArtifacts::create(root)?;

        let mut result = Self {
            config,
            binding: FrameBinding { run, identity },
            key,
            scratch,
            artifacts,
            written_bytes: 0,
            block_count: 0,
            read_index: 0,
            failed: false,
            cleaned: false,
            budget: budget.clone(),
            _reservation: reservation,
        };
        result.checkpoint()?;
        Ok(result)
    }

    fn write_block(&mut self, plaintext: &[u8]) -> Result<(), SpillError> {
        self.write_block_with_hook(plaintext, || {})
    }

    fn write_block_with_hook(
        &mut self,
        plaintext: &[u8],
        after_io: impl FnOnce(),
    ) -> Result<(), SpillError> {
        if self.failed {
            return Err(SpillError::RunFailed);
        }
        self.checkpoint()?;
        if plaintext.len() > self.config.max_plaintext_block
            || self.block_count >= self.config.max_blocks as usize
        {
            return Err(SpillError::QuotaExceeded);
        }
        let frame_len =
            frame::encoded_len(plaintext.len()).map_err(|_| SpillError::QuotaExceeded)?;
        let next_bytes = self
            .written_bytes
            .checked_add(u64::try_from(frame_len).map_err(|_| SpillError::QuotaExceeded)?)
            .ok_or(SpillError::QuotaExceeded)?;
        if next_bytes > self.config.reservation_shape()?.spill_bytes() {
            return Err(SpillError::QuotaExceeded);
        }
        let sequence = u64::try_from(self.block_count).map_err(|_| SpillError::QuotaExceeded)?;
        if let Err(error) = frame::seal(
            &mut self.scratch,
            &self.key,
            self.binding,
            sequence,
            plaintext,
        ) {
            self.scratch.zeroize();
            self.failed = true;
            return Err(error);
        }
        self.checkpoint()?;

        let result = self.write_new_file();
        if result.is_err() {
            self.failed = true;
            return result;
        }
        after_io();
        self.checkpoint()?;
        self.written_bytes = next_bytes;
        self.block_count += 1;
        Ok(())
    }

    fn write_new_file(&mut self) -> Result<(), SpillError> {
        let mut file = self.artifacts.create_block(self.block_count)?;
        file.write_all(&self.scratch)
            .and_then(|()| file.flush())
            .map_err(|_| SpillError::IoFailure)
    }

    /// Returns plaintext borrowed from the one quota-reserved scratch buffer.
    /// Its residency is bounded by the run and it is overwritten/erased by the
    /// next mutable operation or Drop. Callers can copy it; those copies are
    /// intentionally outside this private substrate's accounting claim.
    fn read_next(&mut self) -> Result<&[u8], SpillError> {
        if self.failed {
            return Err(SpillError::RunFailed);
        }
        self.checkpoint()?;
        if self.read_index >= self.block_count {
            return Err(SpillError::EndOfBlocks);
        }
        let range = match self.read_next_inner() {
            Ok(range) => range,
            Err(error) => {
                self.scratch.zeroize();
                self.failed = true;
                return Err(error);
            }
        };
        self.checkpoint()?;
        self.read_index += 1;
        Ok(&self.scratch[range])
    }

    fn read_next_inner(&mut self) -> Result<Range<usize>, SpillError> {
        let mut opened = self.artifacts.open_block(self.read_index)?;
        let max_frame = frame::encoded_len(self.config.max_plaintext_block)?;
        if opened.size < (HEADER_LEN + TAG_LEN) as u64 {
            return Err(SpillError::TruncatedBlock);
        }
        if opened.size > max_frame as u64 {
            return Err(SpillError::LengthMismatch);
        }

        let mut header = [0_u8; HEADER_LEN];
        read_exact_redacted(&mut opened.file, &mut header)?;
        let expected = frame::inspect_header(
            &header,
            self.binding,
            u64::try_from(self.read_index).map_err(|_| SpillError::SequenceMismatch)?,
            self.config.max_plaintext_block,
        )?;
        if opened.size < expected as u64 {
            return Err(SpillError::TruncatedBlock);
        }
        if opened.size != expected as u64 {
            return Err(SpillError::LengthMismatch);
        }
        // Never let an authenticated or malformed length turn this reader into
        // an allocator. Capacity was admitted and allocated exactly once.
        if expected > self.scratch.capacity() {
            return Err(SpillError::LengthMismatch);
        }

        self.scratch.zeroize();
        self.scratch.resize(expected, 0);
        self.scratch[..HEADER_LEN].copy_from_slice(&header);
        read_exact_redacted(&mut opened.file, &mut self.scratch[HEADER_LEN..])?;
        let mut trailing = [0_u8; 1];
        if opened
            .file
            .read(&mut trailing)
            .map_err(|_| SpillError::IoFailure)?
            != 0
        {
            return Err(SpillError::LengthMismatch);
        }
        frame::open(
            &mut self.scratch,
            &self.key,
            self.binding,
            u64::try_from(self.read_index).map_err(|_| SpillError::SequenceMismatch)?,
            self.config.max_plaintext_block,
        )
    }

    fn block_count(&self) -> usize {
        self.block_count
    }

    fn finish(mut self) -> Result<(), SpillError> {
        let was_failed = self.failed;
        let complete = self.artifacts.cleanup();
        self.cleaned = complete;
        self.scratch.zeroize();
        self.key.erase();
        // `_reservation` is deliberately released when `self` drops even on
        // CleanupFailed. Retaining/leaking it would not govern orphaned bytes
        // and would falsely present per-query admission as global high-water
        // accounting. Such residue remains a non-production janitor concern.
        if !complete {
            Err(SpillError::CleanupFailed)
        } else if was_failed {
            Err(SpillError::RunFailed)
        } else {
            Ok(())
        }
    }

    /// Best-effort cancellation consumes the owner and invokes Drop cleanup.
    /// It deliberately has no cleanup-result channel: any CleanupFailed residue
    /// is not reported here and remains outside the released per-query token.
    /// A production operator needs an auditable janitor/global accounting path.
    fn cancel(self) {
        drop(self);
    }

    #[cfg(test)]
    fn test_fail_after_file_create(&mut self) -> Result<(), SpillError> {
        if self.failed {
            return Err(SpillError::RunFailed);
        }
        self.artifacts.create_block(self.block_count)?;
        self.failed = true;
        Err(SpillError::IoFailure)
    }

    #[cfg(test)]
    fn test_discard_scratch_capacity(&mut self) {
        self.scratch.zeroize();
        self.scratch.shrink_to_fit();
    }

    #[cfg(test)]
    fn test_scratch_is_erased(&self) -> bool {
        self.scratch.is_empty()
    }

    #[cfg(test)]
    fn test_run_path(&self, root: &Path) -> std::path::PathBuf {
        root.join(self.artifacts.run_name())
    }

    #[cfg(test)]
    fn test_block_path(&self, root: &Path, index: usize) -> std::path::PathBuf {
        self.test_run_path(root)
            .join(self.artifacts.block_name(index))
    }

    #[cfg(test)]
    fn test_run_name(&self) -> &str {
        self.artifacts.run_name()
    }

    #[cfg(test)]
    fn test_block_name(&self, index: usize) -> &str {
        self.artifacts.block_name(index)
    }

    fn checkpoint(&mut self) -> Result<(), SpillError> {
        if self.budget.checkpoint().is_err() {
            self.scratch.zeroize();
            self.failed = true;
            Err(SpillError::QueryTerminated)
        } else {
            Ok(())
        }
    }
}

impl Drop for SecureSpillRun {
    fn drop(&mut self) {
        if !self.cleaned {
            self.cleaned = self.artifacts.cleanup();
        }
        self.scratch.zeroize();
        self.key.erase();
        // `_reservation` then drops exactly once and restores all dimensions.
    }
}

fn read_exact_redacted(reader: &mut impl Read, buffer: &mut [u8]) -> Result<(), SpillError> {
    reader.read_exact(buffer).map_err(|error| {
        if error.kind() == ErrorKind::UnexpectedEof {
            SpillError::TruncatedBlock
        } else {
            SpillError::IoFailure
        }
    })
}

#[cfg(test)]
mod adversarial_tests;
#[cfg(test)]
mod tests;
