//! Exact late-applied seccomp policy for the prepared loader observation.
//!
//! Bubblewrap applies this same classic-BPF program to its PID-1 reaper and
//! final command child after namespace and mount setup. The policy therefore
//! covers only that late lifecycle tail plus the copied loader's `--list`
//! diagnostic. It is not a syscall trace, application sandbox, host-tool
//! closure, execution attestation, or admission authority.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::FileExt;

use sha2::{Digest, Sha256};

use super::{ExpectedPreparedSeccompPolicy, MAX_TRANSFER_FD, MIN_TRANSFER_FD};

pub(in super::super) const PREPARED_SECCOMP_POLICY: &str =
    "x86_64-prepared-loader-late-cbpf-default-kill-v1";

const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
const X32_SYSCALL_BIT: u32 = 0x4000_0000;
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_DATA_ARCH: u32 = 4;
const SECCOMP_DATA_NR: u32 = 0;
const SECCOMP_DATA_ARGS: u32 = 16;
const FILTER_BYTES: usize = 8;
const MAX_FILTERS: usize = 4096;

const BPF_LD_W_ABS: u16 = 0x20;
const BPF_ALU_AND_K: u16 = 0x54;
const BPF_JMP_JEQ_K: u16 = 0x15;
const BPF_JMP_JGE_K: u16 = 0x35;
const BPF_RET_K: u16 = 0x06;

const READ_ONLY_OPENAT_DENY_MASK: u32 = 0x0041_0243;
const PROT_WRITE_EXEC: u32 = 0x6;
const REQUIRED_SEALS: libc::c_int =
    libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct Filter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

impl Filter {
    const fn statement(code: u16, k: u32) -> Self {
        Self {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> Self {
        Self { code, jt, jf, k }
    }
}

#[derive(Debug, Clone, Copy)]
enum Constraint {
    Allow,
    ArgumentEquals { index: u32, value: u32 },
    ArgumentMaskZero { index: u32, mask: u32 },
    ArgumentMaskNotEquals { index: u32, mask: u32, value: u32 },
}

#[derive(Debug, Clone, Copy)]
struct Rule {
    syscall: u32,
    constraint: Constraint,
}

// Fixed x86-64 syscall numbers. This list is semantically justified by the
// pinned bubblewrap 0.9.0 late PID-1 tail and the pinned glibc loader `--list`
// operation; trace output is diagnostic evidence, never an auto-widening input.
const RULES: [Rule; 17] = [
    rule(0, Constraint::Allow),                                 // read
    rule(1, Constraint::ArgumentEquals { index: 0, value: 4 }), // PID-1 eventfd
    rule(3, Constraint::Allow),                                 // close
    rule(5, Constraint::Allow),                                 // fstat
    rule(
        9,
        Constraint::ArgumentMaskNotEquals {
            index: 2,
            mask: PROT_WRITE_EXEC,
            value: PROT_WRITE_EXEC,
        },
    ), // mmap without W+X
    rule(12, Constraint::Allow),                                // brk
    rule(17, Constraint::Allow),                                // pread64
    rule(20, Constraint::ArgumentEquals { index: 0, value: 1 }), // loader stdout
    rule(21, Constraint::Allow),                                // access
    rule(59, Constraint::Allow),                                // final execve
    rule(61, Constraint::Allow),                                // PID-1 wait4
    rule(158, Constraint::Allow),                               // arch_prctl
    rule(218, Constraint::Allow),                               // set_tid_address
    rule(231, Constraint::Allow),                               // exit_group
    rule(
        257,
        Constraint::ArgumentMaskZero {
            index: 2,
            mask: READ_ONLY_OPENAT_DENY_MASK,
        },
    ), // read-only openat
    rule(273, Constraint::Allow),                               // set_robust_list
    rule(334, Constraint::Allow),                               // rseq
];

const fn rule(syscall: u32, constraint: Constraint) -> Rule {
    Rule {
        syscall,
        constraint,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct PreparedSeccompIdentity {
    id: &'static str,
    sha256: String,
    byte_length: u64,
}

impl PreparedSeccompIdentity {
    pub(in super::super) fn id(&self) -> &'static str {
        self.id
    }

    pub(in super::super) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(in super::super) fn byte_length(&self) -> u64 {
        self.byte_length
    }
}

#[derive(Debug)]
pub(super) struct PreparedSeccompPolicy {
    identity: PreparedSeccompIdentity,
    bytes: Vec<u8>,
    transfer: File,
}

impl PreparedSeccompPolicy {
    pub(super) fn new(expected: &ExpectedPreparedSeccompPolicy) -> Result<Self, String> {
        if !cfg!(target_arch = "x86_64") {
            return Err("prepared seccomp policy requires x86-64 Linux".to_owned());
        }
        let bytes = canonical_bytes();
        validate_program(&bytes)?;
        let identity = identity(&bytes)?;
        expected.assert_matches(identity.id(), identity.sha256(), identity.byte_length())?;
        let transfer = sealed_transfer(&bytes)?;
        let policy = Self {
            identity,
            bytes,
            transfer,
        };
        policy.assert_current(expected)?;
        Ok(policy)
    }

    pub(super) fn identity(&self) -> &PreparedSeccompIdentity {
        &self.identity
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.transfer.as_raw_fd()
    }

    pub(super) fn rewind(&self) -> Result<(), String> {
        // SAFETY: the descriptor is live and SEEK_SET does not dereference memory.
        if unsafe { libc::lseek(self.raw_fd(), 0, libc::SEEK_SET) } != 0 {
            return Err(format!(
                "rewind prepared seccomp policy: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub(super) fn assert_current(
        &self,
        expected: &ExpectedPreparedSeccompPolicy,
    ) -> Result<(), String> {
        expected.assert_matches(
            self.identity.id(),
            self.identity.sha256(),
            self.identity.byte_length(),
        )?;
        validate_program(&self.bytes)?;
        let actual = read_exact(&self.transfer, self.identity.byte_length())?;
        if actual != self.bytes || format!("{:x}", Sha256::digest(&actual)) != self.identity.sha256
        {
            return Err("sealed prepared seccomp policy bytes changed".to_owned());
        }
        require_descriptor_policy(&self.transfer, self.identity.byte_length())
    }

    #[cfg(test)]
    pub(super) fn replace_transfer_for_test(&mut self, replacement: File) {
        self.transfer = replacement;
    }
}

fn build_program() -> Vec<Filter> {
    let mut filters = vec![
        Filter::statement(BPF_LD_W_ABS, SECCOMP_DATA_ARCH),
        Filter::jump(BPF_JMP_JEQ_K, AUDIT_ARCH_X86_64, 1, 0),
        Filter::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        Filter::statement(BPF_LD_W_ABS, SECCOMP_DATA_NR),
        Filter::jump(BPF_JMP_JGE_K, X32_SYSCALL_BIT, 0, 1),
        Filter::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
    ];
    for rule in RULES {
        emit_rule(&mut filters, rule);
    }
    filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));
    filters
}

fn emit_rule(filters: &mut Vec<Filter>, rule: Rule) {
    match rule.constraint {
        Constraint::Allow => {
            filters.push(Filter::jump(BPF_JMP_JEQ_K, rule.syscall, 0, 1));
            filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_ALLOW));
        }
        Constraint::ArgumentEquals { index, value } => {
            filters.push(Filter::jump(BPF_JMP_JEQ_K, rule.syscall, 0, 4));
            filters.push(Filter::statement(BPF_LD_W_ABS, argument_offset(index)));
            filters.push(Filter::jump(BPF_JMP_JEQ_K, value, 0, 1));
            filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_ALLOW));
            filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));
        }
        Constraint::ArgumentMaskZero { index, mask } => {
            filters.push(Filter::jump(BPF_JMP_JEQ_K, rule.syscall, 0, 5));
            filters.push(Filter::statement(BPF_LD_W_ABS, argument_offset(index)));
            filters.push(Filter::statement(BPF_ALU_AND_K, mask));
            filters.push(Filter::jump(BPF_JMP_JEQ_K, 0, 0, 1));
            filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_ALLOW));
            filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));
        }
        Constraint::ArgumentMaskNotEquals { index, mask, value } => {
            filters.push(Filter::jump(BPF_JMP_JEQ_K, rule.syscall, 0, 5));
            filters.push(Filter::statement(BPF_LD_W_ABS, argument_offset(index)));
            filters.push(Filter::statement(BPF_ALU_AND_K, mask));
            filters.push(Filter::jump(BPF_JMP_JEQ_K, value, 1, 0));
            filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_ALLOW));
            filters.push(Filter::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));
        }
    }
}

const fn argument_offset(index: u32) -> u32 {
    SECCOMP_DATA_ARGS + index * 8
}

fn canonical_bytes() -> Vec<u8> {
    encode(&build_program())
}

fn encode(filters: &[Filter]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(filters.len() * FILTER_BYTES);
    for filter in filters {
        bytes.extend_from_slice(&filter.code.to_le_bytes());
        bytes.push(filter.jt);
        bytes.push(filter.jf);
        bytes.extend_from_slice(&filter.k.to_le_bytes());
    }
    bytes
}

fn decode(bytes: &[u8]) -> Result<Vec<Filter>, String> {
    if bytes.is_empty()
        || bytes.len() > MAX_FILTERS * FILTER_BYTES
        || !bytes.len().is_multiple_of(8)
    {
        return Err("prepared seccomp byte length is outside policy".to_owned());
    }
    Ok(bytes
        .chunks_exact(FILTER_BYTES)
        .map(|chunk| Filter {
            code: u16::from_le_bytes([chunk[0], chunk[1]]),
            jt: chunk[2],
            jf: chunk[3],
            k: u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
        })
        .collect())
}

fn validate_program(bytes: &[u8]) -> Result<(), String> {
    let filters = decode(bytes)?;
    if filters.last() != Some(&Filter::statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS)) {
        return Err("prepared seccomp default action is not kill-process".to_owned());
    }
    for (index, filter) in filters.iter().enumerate() {
        match filter.code {
            BPF_LD_W_ABS => {
                if ![
                    SECCOMP_DATA_NR,
                    SECCOMP_DATA_ARCH,
                    argument_offset(0),
                    argument_offset(2),
                ]
                .contains(&filter.k)
                {
                    return Err("prepared seccomp load offset is outside policy".to_owned());
                }
            }
            BPF_ALU_AND_K => {}
            BPF_JMP_JEQ_K | BPF_JMP_JGE_K => {
                for distance in [filter.jt, filter.jf] {
                    if index + 1 + usize::from(distance) >= filters.len() {
                        return Err("prepared seccomp jump target is outside policy".to_owned());
                    }
                }
            }
            BPF_RET_K => {
                if ![SECCOMP_RET_ALLOW, SECCOMP_RET_KILL_PROCESS].contains(&filter.k) {
                    return Err("prepared seccomp return action is outside policy".to_owned());
                }
            }
            _ => return Err("prepared seccomp instruction is outside policy".to_owned()),
        }
    }
    if filters != build_program() {
        return Err("prepared seccomp canonical program drift".to_owned());
    }
    Ok(())
}

fn identity(bytes: &[u8]) -> Result<PreparedSeccompIdentity, String> {
    let byte_length = u64::try_from(bytes.len())
        .map_err(|_| "prepared seccomp byte length does not fit u64".to_owned())?;
    Ok(PreparedSeccompIdentity {
        id: PREPARED_SECCOMP_POLICY,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_length,
    })
}

pub(super) fn canonical_identity() -> Result<PreparedSeccompIdentity, String> {
    identity(&canonical_bytes())
}

fn sealed_transfer(bytes: &[u8]) -> Result<File, String> {
    // SAFETY: the static name is NUL terminated and the returned descriptor is owned.
    let descriptor = unsafe {
        libc::memfd_create(
            c"semantic-fabric-prepared-seccomp".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if descriptor < 0 {
        return Err(format!(
            "create prepared seccomp memfd: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: ownership of the new descriptor transfers to File.
    let mut source = unsafe { File::from_raw_fd(descriptor) };
    source
        .write_all(bytes)
        .map_err(|error| format!("write prepared seccomp memfd: {error}"))?;
    // SAFETY: fcntl operates on the live owned memfd.
    if unsafe { libc::fcntl(source.as_raw_fd(), libc::F_ADD_SEALS, REQUIRED_SEALS) } != 0 {
        return Err(format!(
            "seal prepared seccomp memfd: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: F_DUPFD_CLOEXEC creates a new independently owned descriptor.
    let transfer_fd =
        unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD_CLOEXEC, MIN_TRANSFER_FD) };
    if !(MIN_TRANSFER_FD..=MAX_TRANSFER_FD).contains(&transfer_fd) {
        if transfer_fd >= 0 {
            // SAFETY: this branch owns the just-created descriptor.
            unsafe { libc::close(transfer_fd) };
        }
        return Err("prepared seccomp descriptor is outside its fixed range".to_owned());
    }
    // SAFETY: ownership of the duplicated descriptor transfers to File.
    let transfer = unsafe { File::from_raw_fd(transfer_fd) };
    require_descriptor_policy(&transfer, bytes.len() as u64)?;
    Ok(transfer)
}

fn require_descriptor_policy(file: &File, byte_length: u64) -> Result<(), String> {
    let descriptor = file.as_raw_fd();
    // SAFETY: fcntl only inspects the live descriptor and memfd seals.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    let seals = unsafe { libc::fcntl(descriptor, libc::F_GET_SEALS) };
    let length = file
        .metadata()
        .map_err(|error| format!("inspect prepared seccomp memfd: {error}"))?
        .len();
    if !(MIN_TRANSFER_FD..=MAX_TRANSFER_FD).contains(&descriptor)
        || flags < 0
        || flags & libc::FD_CLOEXEC == 0
        || seals != REQUIRED_SEALS
        || length != byte_length
    {
        return Err("prepared seccomp descriptor policy drift".to_owned());
    }
    Ok(())
}

fn read_exact(file: &File, byte_length: u64) -> Result<Vec<u8>, String> {
    let length = usize::try_from(byte_length)
        .map_err(|_| "prepared seccomp byte length does not fit memory".to_owned())?;
    let mut bytes = vec![0; length];
    let mut offset = 0u64;
    while offset < byte_length {
        let start = usize::try_from(offset)
            .map_err(|_| "prepared seccomp offset does not fit memory".to_owned())?;
        let read = file
            .read_at(&mut bytes[start..], offset)
            .map_err(|error| format!("read prepared seccomp memfd: {error}"))?;
        if read == 0 {
            return Err("prepared seccomp memfd became shorter".to_owned());
        }
        offset += read as u64;
    }
    let mut extra = [0u8; 1];
    if file
        .read_at(&mut extra, byte_length)
        .map_err(|error| format!("probe prepared seccomp memfd length: {error}"))?
        != 0
    {
        return Err("prepared seccomp memfd grew".to_owned());
    }
    Ok(bytes)
}

#[cfg(test)]
pub(in super::super) fn policy_identity_for_test() -> PreparedSeccompIdentity {
    canonical_identity().expect("canonical seccomp identity")
}

#[cfg(test)]
pub(in super::super) fn canonical_bytes_for_test() -> Vec<u8> {
    canonical_bytes()
}

#[cfg(test)]
pub(in super::super) fn validate_bytes_for_test(bytes: &[u8]) -> Result<(), String> {
    validate_program(bytes)
}
