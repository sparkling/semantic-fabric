//! Private tests for the Linux supervisor foundation.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux_tests {
    use std::fs::{self, File};
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::ExitStatusExt;

    use sha2::{Digest, Sha256};

    use super::super::executable::{
        executable_with_len, write_executable, PreparedParserExecutable, TempDirectory,
        MAX_EXECUTABLE_BYTES,
    };
    use super::super::linux::spawn_fixture;
    use super::super::seccomp::StageOnePolicy;
    use super::super::{v1_limits, SupervisorError};
    use crate::parser_isolation::protocol::{ParserWorkerLimitValues, ParserWorkerLimits};

    #[test]
    fn current_executable_identity_and_fingerprint_are_observed_from_one_inode() {
        let prepared = PreparedParserExecutable::current().expect("prepare current executable");
        let identity = prepared.identity();
        let mut independent = File::open("/proc/self/exe").expect("open current executable");
        let metadata = independent.metadata().expect("stat current executable");

        assert_eq!(identity.device(), metadata.dev());
        assert_eq!(identity.inode(), metadata.ino());
        assert_eq!(identity.byte_len(), metadata.len());
        assert_eq!(identity.mode(), metadata.mode());
        assert_eq!(
            identity.fingerprint().bytes(),
            sha256_reader(&mut independent)
        );
        assert_eq!(
            format!("{:?}", identity.fingerprint()),
            "ObservedExecutableFingerprint(<non-authoritative>)"
        );
    }

    #[test]
    fn held_fingerprint_survives_path_replacement_without_reopening() {
        let directory = TempDirectory::new();
        let path = directory.path().join("worker");
        write_executable(&path, b"original held bytes");
        let held_file = File::open(&path).expect("open original fixture");
        // Unlink before preparation so the deliberate namespace replacement
        // cannot also create a post-snapshot ctime drift on the held inode.
        fs::remove_file(&path).expect("unlink held fixture path");
        let prepared = PreparedParserExecutable::from_file_for_test(held_file, false)
            .expect("prepare original fixture");
        let original_identity = prepared.identity();

        write_executable(&path, b"replacement bytes");
        let replacement_metadata = fs::metadata(&path).expect("stat replacement");

        assert_ne!(original_identity.inode(), replacement_metadata.ino());
        let expected: [u8; 32] = Sha256::digest(b"original held bytes").into();
        assert_eq!(original_identity.fingerprint().bytes(), expected);
        let duplicate = prepared
            .duplicate_for_launch(64)
            .expect("duplicate held inode");
        assert!(duplicate.as_raw_fd() >= 64);
    }

    #[test]
    fn executable_preparation_rejects_unbounded_and_inheritable_descriptors() {
        let directory = TempDirectory::new();
        let oversized = directory.path().join("oversized");
        let file = executable_with_len(&oversized, MAX_EXECUTABLE_BYTES + 1);
        assert!(matches!(
            PreparedParserExecutable::from_file_for_test(file, false),
            Err(SupervisorError::InvalidExecutable(
                "descriptor exceeds the fixed fingerprint byte ceiling"
            ))
        ));

        let cat = File::open("/bin/cat").expect("open cat fixture");
        // SAFETY: this test intentionally clears CLOEXEC on its owned fd.
        assert_eq!(unsafe { libc::fcntl(cat.as_raw_fd(), libc::F_SETFD, 0) }, 0);
        assert!(matches!(
            PreparedParserExecutable::from_file_for_test(cat, true),
            Err(SupervisorError::InvalidExecutable(
                "descriptor is not close-on-exec"
            ))
        ));
    }

    #[test]
    fn linux_limit_projection_rejects_inexact_or_ineffective_values_before_spawn() {
        let executable = prepared_cat();
        let mut values = v1_limits().values();
        values.cpu_time_millis = 1_500;
        assert_invalid_limits(&executable, values, "CPU milliseconds");

        let mut values = v1_limits().values();
        values.max_processes = 2;
        assert_invalid_limits(&executable, values, "exactly one process");

        let mut values = v1_limits().values();
        values.max_open_fds = 15;
        assert_invalid_limits(&executable, values, "dynamic-loader qualification floor");
    }

    #[test]
    fn descriptor_launch_applies_exact_limits_and_closes_environment_and_fds() {
        const EXPECTED: &[u8] = b"64\n10\n131072\n16384\n1048576\n0\nOK\n";
        let shell = PreparedParserExecutable::from_file_for_test(
            File::open("/bin/sh").expect("open shell fixture"),
            true,
        )
        .expect("prepare shell fixture");
        let ambient_source = File::open("/dev/null").expect("open ambient fixture fd");
        // SAFETY: F_DUPFD creates an intentionally inheritable fd at or above
        // 128, beyond the child NOFILE ceiling, for the close_range canary.
        let ambient_fd = unsafe { libc::fcntl(ambient_source.as_raw_fd(), libc::F_DUPFD, 128) };
        assert!(ambient_fd >= 128);
        // SAFETY: F_GETFD only inspects the descriptor just created above.
        let ambient_flags = unsafe { libc::fcntl(ambient_fd, libc::F_GETFD) };
        assert!(ambient_flags >= 0);
        assert_eq!(ambient_flags & libc::FD_CLOEXEC, 0);
        // SAFETY: ownership of the newly duplicated descriptor transfers here.
        let ambient_fd = unsafe { OwnedFd::from_raw_fd(ambient_fd) };
        let script = format!(
            "n=3; while [ \"$n\" -le 64 ]; do [ ! -e /proc/self/fd/$n ] || exit 40; n=$((n+1)); done; [ ! -e /proc/self/fd/{} ] || exit 42; [ ! -s /proc/self/environ ] || exit 41; ulimit -n; ulimit -t; ulimit -f; ulimit -s; ulimit -v; ulimit -c; printf 'OK\\n'",
            ambient_fd.as_raw_fd()
        );
        let mut child = spawn_fixture(&shell, v1_limits(), &[b"sh", b"-c", script.as_bytes()])
            .expect("launch shell fixture");
        drop(ambient_fd);
        let mut output = [0_u8; EXPECTED.len()];
        child
            .read_exact_until_deadline(&mut output)
            .expect("read exact fixture output before the canonical deadline");
        let status = child.wait_until_deadline().expect("reap shell fixture");
        assert!(status.success(), "fixture status: {status}");
        assert_eq!(&output, EXPECTED);
    }

    #[test]
    fn stage_one_filter_denies_creation_escape_and_wrong_exec_but_allows_observation() {
        let executable = prepared_cat();
        let launch_fd = executable
            .duplicate_for_launch(64)
            .expect("create high launch descriptor");
        assert!(launch_fd.as_raw_fd() >= 64);
        let empty_path = c"".as_ptr() as usize;
        let argv = Box::new([c"true".as_ptr(), std::ptr::null()]);
        let envp = Box::new([std::ptr::null::<libc::c_char>()]);
        let policy = StageOnePolicy::new(
            launch_fd.as_raw_fd(),
            empty_path,
            argv.as_ptr() as usize,
            envp.as_ptr() as usize,
        )
        .expect("build stage-one policy");
        let (read_end, write_end) = pipe();

        // SAFETY: child uses only inherited POD, raw syscalls, then _exit.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork canary process");
        if pid == 0 {
            drop(read_end);
            let passed = unsafe {
                run_stage_one_canary(
                    &policy,
                    launch_fd.as_raw_fd(),
                    empty_path as *const libc::c_char,
                    &*argv,
                    &*envp,
                )
            };
            let result = [u8::from(passed)];
            // SAFETY: write_end is live and the byte has a stable address.
            unsafe {
                libc::write(write_end.as_raw_fd(), result.as_ptr().cast(), result.len());
                libc::_exit(0);
            }
        }
        drop(write_end);
        let mut result = [0_u8; 1];
        File::from(read_end)
            .read_exact(&mut result)
            .expect("read canary barrier");
        let mut status = 0;
        // SAFETY: pid is the unreaped direct child returned by fork.
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
        assert!(libc::WIFEXITED(status));
        assert_eq!(result, [1]);
    }

    #[test]
    fn pidfd_group_lifecycle_reaps_and_a_clean_next_launch_succeeds() {
        let executable = prepared_cat();
        let mut child =
            spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"]).expect("launch held cat");
        barrier(&mut child);
        let pidfd = child.duplicate_pidfd().expect("duplicate pidfd");
        assert!(pidfd_targets_live_process(&pidfd));
        let status = child.terminate_and_reap().expect("terminate and reap");
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        assert!(!pidfd_targets_live_process(&pidfd));

        successful_round_trip(&executable);
    }

    #[test]
    fn drop_fallback_and_canonical_deadline_both_reap_before_relaunch() {
        let executable = prepared_cat();
        let pidfd = {
            let mut child = spawn_fixture(&executable, v1_limits(), &[b"cat", b"-"])
                .expect("launch drop fixture");
            barrier(&mut child);
            let pidfd = child.duplicate_pidfd().expect("duplicate drop pidfd");
            drop(child);
            pidfd
        };
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&executable);

        let mut values = v1_limits().values();
        values.wall_time_millis = 1;
        let limits = ParserWorkerLimits::new(values).expect("short fixture limits");
        let mut child =
            spawn_fixture(&executable, limits, &[b"cat", b"-"]).expect("launch deadline fixture");
        assert!(matches!(
            child.wait_until_deadline(),
            Err(SupervisorError::DeadlineExceeded)
        ));
        successful_round_trip(&executable);
    }

    #[test]
    fn failed_descriptor_exec_does_not_poison_the_next_launch() {
        let directory = TempDirectory::new();
        let invalid = directory.path().join("not-elf");
        write_executable(&invalid, b"not an ELF image");
        let invalid = PreparedParserExecutable::from_file_for_test(
            File::open(&invalid).expect("open invalid fixture"),
            false,
        )
        .expect("prepare explicit invalid fixture");
        assert!(spawn_fixture(&invalid, v1_limits(), &[b"invalid"]).is_err());
        successful_round_trip(&prepared_cat());
    }

    fn prepared_cat() -> PreparedParserExecutable {
        PreparedParserExecutable::from_file_for_test(
            File::open("/bin/cat").expect("open cat fixture"),
            true,
        )
        .expect("prepare cat fixture")
    }

    fn assert_invalid_limits(
        executable: &PreparedParserExecutable,
        values: ParserWorkerLimitValues,
        message: &str,
    ) {
        let limits = ParserWorkerLimits::new(values).expect("wire-level limits remain valid");
        let error = match spawn_fixture(executable, limits, &[b"cat", b"-"]) {
            Ok(mut child) => {
                let _ = child.terminate_and_reap();
                panic!("Linux projection accepted invalid limits")
            }
            Err(error) => error,
        };
        assert!(error.to_string().contains(message), "{error}");
    }

    fn barrier(child: &mut super::super::lifecycle::ParserWorkerProcess) {
        child
            .write_all_until_deadline(b"B")
            .expect("write barrier byte");
        let mut observed = [0_u8; 1];
        child
            .read_exact_until_deadline(&mut observed)
            .expect("read barrier byte");
        assert_eq!(observed, [b'B']);
    }

    fn successful_round_trip(executable: &PreparedParserExecutable) {
        let mut child =
            spawn_fixture(executable, v1_limits(), &[b"cat", b"-"]).expect("launch next fixture");
        barrier(&mut child);
        child.close_stdin();
        assert!(child
            .wait_until_deadline()
            .expect("reap next fixture")
            .success());
    }

    fn pidfd_targets_live_process(pidfd: &OwnedFd) -> bool {
        // SAFETY: signal zero probes the process identified by this pidfd.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            true
        } else {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            false
        }
    }

    unsafe fn run_stage_one_canary(
        policy: &StageOnePolicy,
        launch_fd: i32,
        empty_path: *const libc::c_char,
        argv: &[*const libc::c_char],
        envp: &[*const libc::c_char],
    ) -> bool {
        let wrong_argv = [c"true".as_ptr(), std::ptr::null()];
        let wrong_envp = [std::ptr::null::<libc::c_char>()];
        // SAFETY: the canary child has not created threads and sets NNP before
        // installing the inherited, parent-built filter.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
            || unsafe { policy.install_in_child() }.is_err()
        {
            return false;
        }
        let thread_flags = libc::CLONE_VM | libc::CLONE_SIGHAND | libc::CLONE_THREAD;
        let mut denied = unsafe { is_eperm_now(libc::syscall(libc::SYS_fork)) };
        denied &= unsafe { is_eperm_now(libc::syscall(libc::SYS_vfork)) };
        denied &= unsafe { is_eperm_now(libc::syscall(libc::SYS_clone, thread_flags, 0, 0, 0, 0)) };
        denied &=
            unsafe { is_eperm_now(libc::syscall(libc::SYS_clone3, std::ptr::null::<u8>(), 0)) };
        denied &= unsafe {
            is_eperm_now(libc::syscall(
                libc::SYS_execve,
                c"/bin/true".as_ptr(),
                argv.as_ptr(),
                envp.as_ptr(),
            ))
        };
        denied &= unsafe { is_eperm_now(libc::syscall(libc::SYS_setsid)) };
        denied &= unsafe { is_eperm_now(libc::syscall(libc::SYS_setpgid, 0, 0)) };
        denied &= unsafe {
            execveat_is_denied(
                launch_fd,
                c"/bin/true".as_ptr(),
                argv,
                envp,
                libc::AT_EMPTY_PATH,
            )
        };
        denied &= unsafe { execveat_is_denied(0, empty_path, argv, envp, libc::AT_EMPTY_PATH) };
        denied &= unsafe { execveat_is_denied(launch_fd, empty_path, argv, envp, 0) };
        denied &= unsafe {
            execveat_is_denied(
                launch_fd,
                empty_path,
                &wrong_argv,
                envp,
                libc::AT_EMPTY_PATH,
            )
        };
        denied &= unsafe {
            execveat_is_denied(
                launch_fd,
                empty_path,
                argv,
                &wrong_envp,
                libc::AT_EMPTY_PATH,
            )
        };
        if !denied {
            return false;
        }
        let mut observed = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let query = unsafe {
            libc::syscall(
                libc::SYS_prlimit64,
                0,
                libc::RLIMIT_NOFILE,
                std::ptr::null::<libc::rlimit>(),
                &mut observed,
            )
        };
        let self_pid = unsafe { libc::syscall(libc::SYS_getpid) };
        let other_query_denied = unsafe {
            is_eperm_now(libc::syscall(
                libc::SYS_prlimit64,
                self_pid,
                libc::RLIMIT_NOFILE,
                std::ptr::null::<libc::rlimit>(),
                &mut observed,
            ))
        };
        let mutation = unsafe {
            libc::syscall(
                libc::SYS_prlimit64,
                0,
                libc::RLIMIT_NOFILE,
                &observed,
                std::ptr::null_mut::<libc::rlimit>(),
            )
        };
        let mutation_denied = is_eperm(mutation);
        let setrlimit_denied = unsafe {
            is_eperm_now(libc::syscall(
                libc::SYS_setrlimit,
                libc::RLIMIT_NOFILE,
                &observed,
            ))
        };
        query == 0
            && other_query_denied
            && mutation_denied
            && setrlimit_denied
            // A representative allowed syscall proves this is deliberately not
            // a default-kill general sandbox.
            && unsafe { libc::syscall(libc::SYS_getpid) } > 0
    }

    fn is_eperm(result: libc::c_long) -> bool {
        result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    unsafe fn is_eperm_now(result: libc::c_long) -> bool {
        is_eperm(result)
    }

    unsafe fn execveat_is_denied(
        fd: i32,
        path: *const libc::c_char,
        argv: &[*const libc::c_char],
        envp: &[*const libc::c_char],
        flags: i32,
    ) -> bool {
        // SAFETY: every pointer addresses a live, null-terminated canary array.
        unsafe {
            is_eperm_now(libc::syscall(
                libc::SYS_execveat,
                fd,
                path,
                argv.as_ptr(),
                envp.as_ptr(),
                flags,
            ))
        }
    }

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut descriptors = [0; 2];
        // SAFETY: pipe2 initializes two new owned descriptors on success.
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        // SAFETY: ownership of both returned descriptors transfers here.
        unsafe {
            (
                OwnedFd::from_raw_fd(descriptors[0]),
                OwnedFd::from_raw_fd(descriptors[1]),
            )
        }
    }

    fn sha256_reader(reader: &mut File) -> [u8; 32] {
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = reader.read(&mut buffer).expect("read digest fixture");
            if count == 0 {
                return digest.finalize().into();
            }
            digest.update(&buffer[..count]);
        }
    }
}
