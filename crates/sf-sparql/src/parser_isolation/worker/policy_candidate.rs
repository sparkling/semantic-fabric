//! Default-kill control-ready policy candidate for the single-use worker.

const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
const X32_SYSCALL_BIT: u32 = 0x4000_0000;
const SECCOMP_DATA_NR: u32 = 0;
const SECCOMP_DATA_ARCH: u32 = 4;
const SECCOMP_DATA_ARGS: u32 = 16;
const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JMP_JEQ_K: u16 = 0x15;
const BPF_JMP_JGE_K: u16 = 0x35;
const BPF_JMP_JSET_K: u16 = 0x45;
const BPF_RET_K: u16 = 0x06;
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const PROBE_DENIED: u32 = SECCOMP_RET_ERRNO | libc::EPERM as u32;

pub(super) struct ControlReadyPolicyCandidate {
    filters: Box<[libc::sock_filter]>,
}

impl ControlReadyPolicyCandidate {
    pub(super) fn new() -> Option<Self> {
        let filters = build_filters().into_boxed_slice();
        u16::try_from(filters.len()).ok()?;
        Some(Self { filters })
    }

    /// Stack the control-ready candidate and distinguish it from the inherited
    /// broad stage-one filter with a harmless, deliberately forbidden syscall.
    pub(super) fn install_and_verify(&self) -> bool {
        let program = libc::sock_fprog {
            len: self.filters.len() as u16,
            filter: self.filters.as_ptr().cast_mut(),
        };
        // SAFETY: the immutable filter storage outlives both syscalls and the
        // parent launcher already established no-new-privileges.
        let installed = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                libc::SECCOMP_FILTER_FLAG_TSYNC,
                &program as *const libc::sock_fprog,
            )
        };
        if installed != 0 {
            return false;
        }

        // Stage one permits `uname`, where a null output pointer yields EFAULT.
        // The candidate policy instead returns EPERM before argument evaluation.
        let probe =
            unsafe { libc::syscall(libc::SYS_uname, std::ptr::null_mut::<libc::utsname>()) };
        probe == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

fn build_filters() -> Vec<libc::sock_filter> {
    let mut filters = vec![
        statement(BPF_LD_W_ABS, SECCOMP_DATA_ARCH),
        jump(BPF_JMP_JEQ_K, AUDIT_ARCH_X86_64, 1, 0),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_LD_W_ABS, SECCOMP_DATA_NR),
        jump(BPF_JMP_JGE_K, X32_SYSCALL_BIT, 0, 1),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
    ];

    append_fd_only(&mut filters, libc::SYS_read as u32, libc::STDIN_FILENO);
    append_fd_only(&mut filters, libc::SYS_write as u32, libc::STDOUT_FILENO);
    append_no_exec_memory(&mut filters, libc::SYS_mmap as u32);
    append_no_exec_memory(&mut filters, libc::SYS_mprotect as u32);

    // This is the candidate surface for bounded framing and non-executable
    // parser allocation. It is not parser-qualified until QueryV1 and the
    // governed corpus execute beneath it. There is no filesystem, network,
    // ioctl, process, thread, identity, or new-exec syscall in the allowlist.
    for syscall in [
        libc::SYS_brk,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_futex,
        libc::SYS_clock_gettime,
        libc::SYS_getrandom,
        libc::SYS_exit,
        libc::SYS_exit_group,
    ] {
        filters.push(jump(BPF_JMP_JEQ_K, syscall as u32, 0, 1));
        filters.push(statement(BPF_RET_K, SECCOMP_RET_ALLOW));
    }
    // One harmless probe has a distinctive closed result so the worker can
    // prove this second policy was installed. Every other omitted syscall
    // kills the process; an omission can never become a library fallback or
    // later be misreported as an invalid query.
    filters.push(jump(BPF_JMP_JEQ_K, libc::SYS_uname as u32, 0, 1));
    filters.push(statement(BPF_RET_K, PROBE_DENIED));
    filters.push(statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));
    filters
}

fn append_fd_only(filters: &mut Vec<libc::sock_filter>, syscall: u32, descriptor: i32) {
    filters.extend([
        jump(BPF_JMP_JEQ_K, syscall, 0, 7),
        statement(BPF_LD_W_ABS, argument_word(0, 0)),
        jump(BPF_JMP_JEQ_K, descriptor as u32, 1, 0),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_LD_W_ABS, argument_word(0, 1)),
        jump(BPF_JMP_JEQ_K, 0, 1, 0),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_RET_K, SECCOMP_RET_ALLOW),
    ]);
}

fn append_no_exec_memory(filters: &mut Vec<libc::sock_filter>, syscall: u32) {
    filters.extend([
        jump(BPF_JMP_JEQ_K, syscall, 0, 4),
        statement(BPF_LD_W_ABS, argument_word(2, 0)),
        jump(BPF_JMP_JSET_K, libc::PROT_EXEC as u32, 0, 1),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_RET_K, SECCOMP_RET_ALLOW),
    ]);
}

const fn argument_word(index: u32, high_word: u32) -> u32 {
    SECCOMP_DATA_ARGS + index * 8 + high_word * 4
}

const fn statement(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::*;

    #[test]
    fn installed_policy_verifies_itself_and_allows_stdout() {
        let policy = ControlReadyPolicyCandidate::new().expect("build worker policy candidate");
        let mut descriptors = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        let read_end = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let write_end = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
        let child = unsafe { libc::fork() };
        assert!(child >= 0, "fork candidate-policy canary");
        if child == 0 {
            drop(read_end);
            if unsafe { libc::dup2(write_end.as_raw_fd(), libc::STDOUT_FILENO) } < 0
                || unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
                || !policy.install_and_verify()
            {
                unsafe { libc::_exit(120) };
            }
            let result = [1_u8];
            unsafe {
                libc::syscall(libc::SYS_write, libc::STDOUT_FILENO, result.as_ptr(), 1);
                libc::_exit(0);
            }
        }
        drop(write_end);
        let mut result = [0_u8; 1];
        std::fs::File::from(read_end)
            .read_exact(&mut result)
            .expect("read candidate-policy canary result");
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(child, &mut status, 0) }, child);
        assert!(libc::WIFEXITED(status));
        assert_eq!(libc::WEXITSTATUS(status), 0);
        assert_eq!(result, [1]);
    }

    #[test]
    fn an_unlisted_syscall_kills_instead_of_returning_a_fallback_error() {
        let policy = ControlReadyPolicyCandidate::new().expect("build worker policy candidate");
        let child = unsafe { libc::fork() };
        assert!(child >= 0, "fork default-kill canary");
        if child == 0 {
            if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
                || !policy.install_and_verify()
            {
                unsafe { libc::_exit(120) };
            }
            unsafe {
                libc::syscall(libc::SYS_getpid);
                libc::_exit(121);
            }
        }
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(child, &mut status, 0) }, child);
        assert!(libc::WIFSIGNALED(status));
        assert_eq!(libc::WTERMSIG(status), libc::SIGSYS);
    }
}
