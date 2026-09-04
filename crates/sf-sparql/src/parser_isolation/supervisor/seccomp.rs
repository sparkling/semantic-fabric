//! Stage-one x86-64 Linux seccomp policy for descriptor-exact launch.
//!
//! This is deliberately a default-allow deny-list, not a general sandbox. It
//! prevents process/thread creation, process-group escape, limit relaxation,
//! and every exec except the launcher's one descriptor-exact `execveat`. A
//! future worker must stack its separately reviewed final policy before Ready.
//! Until that post-exec policy exists, no untrusted bytes may reach the worker.

use std::os::fd::RawFd;

use super::SupervisorError;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const X32_SYSCALL_BIT: u32 = 0x4000_0000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SECCOMP_DATA_NR: u32 = 0;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SECCOMP_DATA_ARCH: u32 = 4;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SECCOMP_DATA_ARGS: u32 = 16;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const BPF_LD_W_ABS: u16 = 0x20;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const BPF_JMP_JEQ_K: u16 = 0x15;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const BPF_JMP_JGE_K: u16 = 0x35;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const BPF_RET_K: u16 = 0x06;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
// EPERM is deliberate: it is deterministic and prevents libc compatibility
// fallbacks (notably clone3 -> clone) that ENOSYS can trigger. These denials
// expose an invariant violation to the trusted setup path/tests; they are not
// evidence that this default-allow policy is a general sandbox.
const STAGE_ONE_DENIED: u32 = SECCOMP_RET_ERRNO | libc::EPERM as u32;

pub(super) struct StageOnePolicy {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    filters: Box<[libc::sock_filter]>,
}

impl StageOnePolicy {
    pub(super) fn new(
        executable_fd: RawFd,
        empty_path_address: usize,
        argv_address: usize,
        environment_address: usize,
    ) -> Result<Self, SupervisorError> {
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (
                executable_fd,
                empty_path_address,
                argv_address,
                environment_address,
            );
            Err(SupervisorError::UnsupportedPlatform)
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let filters = build_filters(
                executable_fd,
                empty_path_address,
                argv_address,
                environment_address,
            )?
            .into_boxed_slice();
            u16::try_from(filters.len()).map_err(|_| {
                SupervisorError::InvalidState("stage-one seccomp program is too long")
            })?;
            Ok(Self { filters })
        }
    }

    /// Installs prebuilt POD using only the raw seccomp syscall.
    ///
    /// # Safety
    ///
    /// Called in the single-threaded child between fork and exec. The policy
    /// and its backing filter array must remain live for this call.
    pub(super) unsafe fn install_in_child(&self) -> std::io::Result<()> {
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            // POD construction cannot allocate or lock after fork. Its pointer
            // remains valid for the duration of the syscall because `filters`
            // is held by this borrowed policy.
            let program = libc::sock_fprog {
                len: self.filters.len() as u16,
                filter: self.filters.as_ptr().cast_mut(),
            };
            // SAFETY: program points into live immutable `filters`, and the
            // caller established no-new-privileges before this raw syscall.
            let result = unsafe {
                libc::syscall(
                    libc::SYS_seccomp,
                    libc::SECCOMP_SET_MODE_FILTER,
                    0,
                    &program as *const libc::sock_fprog,
                )
            };
            if result == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn build_filters(
    executable_fd: RawFd,
    empty_path_address: usize,
    argv_address: usize,
    environment_address: usize,
) -> Result<Vec<libc::sock_filter>, SupervisorError> {
    let executable_fd = u32::try_from(executable_fd).map_err(|_| {
        SupervisorError::InvalidState("stage-one executable descriptor is negative")
    })?;
    let mut filters = vec![
        statement(BPF_LD_W_ABS, SECCOMP_DATA_ARCH),
        jump(BPF_JMP_JEQ_K, AUDIT_ARCH_X86_64, 1, 0),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_LD_W_ABS, SECCOMP_DATA_NR),
        jump(BPF_JMP_JGE_K, X32_SYSCALL_BIT, 0, 1),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
    ];

    // Blanket clone denial is load-bearing: the future worker is single
    // threaded after this point. Tokio, Rayon, and lazy background-thread
    // initialization are forbidden on that private path.
    for syscall in [
        libc::SYS_clone,
        libc::SYS_clone3,
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_execve,
        libc::SYS_setsid,
        libc::SYS_setpgid,
        libc::SYS_setrlimit,
    ] {
        filters.push(jump(BPF_JMP_JEQ_K, syscall as u32, 0, 1));
        filters.push(statement(BPF_RET_K, STAGE_ONE_DENIED));
    }

    // Permit only query-only self observation (pid 0, new_limit NULL), and
    // reject cross-process queries and every mutation. The future worker must
    // verify inherited caps before Ready without gaining a way to relax them.
    filters.push(jump(BPF_JMP_JEQ_K, libc::SYS_prlimit64 as u32, 0, 13));
    filters.extend([
        statement(BPF_LD_W_ABS, argument_word(0, 0)),
        jump(BPF_JMP_JEQ_K, 0, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(0, 1)),
        jump(BPF_JMP_JEQ_K, 0, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(2, 0)),
        jump(BPF_JMP_JEQ_K, 0, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(2, 1)),
        jump(BPF_JMP_JEQ_K, 0, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_RET_K, SECCOMP_RET_ALLOW),
    ]);

    let empty_path_address = empty_path_address as u64;
    let argv_address = argv_address as u64;
    let environment_address = environment_address as u64;
    // A false comparison skips the thirty checking instructions to ALLOW. The
    // pointer checks bind exec to the immutable call tuple built before fork;
    // trusting the pointed-to bytes remains a property of that private setup.
    filters.push(jump(BPF_JMP_JEQ_K, libc::SYS_execveat as u32, 0, 30));
    filters.extend([
        statement(BPF_LD_W_ABS, argument_word(0, 0)),
        jump(BPF_JMP_JEQ_K, executable_fd, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(0, 1)),
        jump(BPF_JMP_JEQ_K, 0, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(1, 0)),
        jump(BPF_JMP_JEQ_K, empty_path_address as u32, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(1, 1)),
        jump(BPF_JMP_JEQ_K, (empty_path_address >> 32) as u32, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(2, 0)),
        jump(BPF_JMP_JEQ_K, argv_address as u32, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(2, 1)),
        jump(BPF_JMP_JEQ_K, (argv_address >> 32) as u32, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(3, 0)),
        jump(BPF_JMP_JEQ_K, environment_address as u32, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(3, 1)),
        jump(BPF_JMP_JEQ_K, (environment_address >> 32) as u32, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(4, 0)),
        jump(BPF_JMP_JEQ_K, libc::AT_EMPTY_PATH as u32, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_LD_W_ABS, argument_word(4, 1)),
        jump(BPF_JMP_JEQ_K, 0, 1, 0),
        statement(BPF_RET_K, STAGE_ONE_DENIED),
        statement(BPF_RET_K, SECCOMP_RET_ALLOW),
    ]);
    Ok(filters)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const fn argument_word(index: u32, high_word: u32) -> u32 {
    SECCOMP_DATA_ARGS + index * 8 + high_word * 4
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const fn statement(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}
