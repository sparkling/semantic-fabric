//! Linux-only descriptor-exact child setup for the prepared runtime probe.

use std::collections::BTreeSet;
use std::ffi::{CString, OsString};
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

use super::{MAX_EXECVEAT_ARGUMENTS, MAX_EXECVEAT_ARGUMENT_BYTES};

const MAX_CHILD_FD: RawFd = 1023;
const MAX_ADDRESS_SPACE: libc::rlim_t = 1024 * 1024 * 1024;
const MAX_CPU_SECONDS: libc::rlim_t = 10;
const MAX_FILE_BYTES: libc::rlim_t = 64 * 1024 * 1024;
const MAX_OPEN_FILES: libc::rlim_t = 1024;
const MAX_STACK_BYTES: libc::rlim_t = 16 * 1024 * 1024;
const REQUIRED_SEALS: libc::c_int =
    libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;

#[derive(Debug)]
pub(in crate::binary_artifact_receipt) struct Request {
    pub(in crate::binary_artifact_receipt) executable_fd: RawFd,
    pub(in crate::binary_artifact_receipt) argv: Vec<OsString>,
    pub(in crate::binary_artifact_receipt) data_fds: Vec<RawFd>,
}

pub(super) fn install(command: &mut Command, request: Request) -> Result<(), String> {
    validate(&request)?;
    let arguments = c_arguments(&request.argv)?;
    let mut pointers: Vec<usize> = arguments
        .iter()
        .map(|argument| argument.as_ptr() as usize)
        .collect();
    pointers.push(0);
    let executable_fd = request.executable_fd;
    let data_fds = request.data_fds;
    // SAFETY: getpid has no preconditions and identifies the spawning process.
    let expected_parent = unsafe { libc::getpid() };
    // SAFETY: the closure invokes only async-signal-safe syscalls after fork.
    // `arguments` owns every byte addressed by `pointers` until execveat.
    unsafe {
        command.pre_exec(move || {
            let _arguments_live = &arguments;
            child_setup(executable_fd, &data_fds, expected_parent)?;
            let empty_environment = [0usize];
            libc::execveat(
                executable_fd,
                c"".as_ptr(),
                pointers.as_ptr().cast(),
                empty_environment.as_ptr().cast(),
                libc::AT_EMPTY_PATH,
            );
            Err(std::io::Error::last_os_error())
        });
    }
    Ok(())
}

fn validate(request: &Request) -> Result<(), String> {
    if request.argv.is_empty()
        || request.argv.len() > MAX_EXECVEAT_ARGUMENTS
        || request.data_fds.is_empty()
        || request.executable_fd < 3
        || request.executable_fd > MAX_CHILD_FD
    {
        return Err("invalid descriptor-exact process request".to_owned());
    }
    require_cloexec(request.executable_fd, "prepared executable")?;
    let mut descriptors = BTreeSet::new();
    descriptors.insert(request.executable_fd);
    for descriptor in &request.data_fds {
        if *descriptor < 3
            || *descriptor > MAX_CHILD_FD
            || !descriptors.insert(*descriptor)
            || seals(*descriptor)? != REQUIRED_SEALS
        {
            return Err("invalid prepared data descriptor allowlist".to_owned());
        }
        require_cloexec(*descriptor, "prepared data descriptor")?;
    }
    let argument_bytes = request.argv.iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value.as_os_str().as_bytes().len() + 1)
            .ok_or_else(|| "prepared argument byte count overflow".to_owned())
    })?;
    if argument_bytes > MAX_EXECVEAT_ARGUMENT_BYTES {
        return Err("prepared argument bytes exceed the process bound".to_owned());
    }
    Ok(())
}

fn c_arguments(arguments: &[OsString]) -> Result<Vec<CString>, String> {
    arguments
        .iter()
        .map(|argument| {
            CString::new(argument.as_os_str().as_bytes())
                .map_err(|_| "prepared argument contains NUL".to_owned())
        })
        .collect()
}

unsafe fn child_setup(
    executable_fd: RawFd,
    data_fds: &[RawFd],
    expected_parent: libc::pid_t,
) -> std::io::Result<()> {
    libc::umask(0o022);
    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if libc::getppid() != expected_parent {
        return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
    }
    if libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0
        || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    set_limit(libc::RLIMIT_CORE, 0)?;
    set_limit(libc::RLIMIT_AS, MAX_ADDRESS_SPACE)?;
    set_limit(libc::RLIMIT_CPU, MAX_CPU_SECONDS)?;
    set_limit(libc::RLIMIT_FSIZE, MAX_FILE_BYTES)?;
    set_limit(libc::RLIMIT_NOFILE, MAX_OPEN_FILES)?;
    set_limit(libc::RLIMIT_STACK, MAX_STACK_BYTES)?;
    if libc::close_range(3, u32::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int) != 0 {
        return Err(std::io::Error::last_os_error());
    }
    require_child_cloexec(executable_fd)?;
    for descriptor in data_fds {
        let descriptor_seals = libc::fcntl(*descriptor, libc::F_GET_SEALS);
        if descriptor_seals < 0 || libc::lseek(*descriptor, 0, libc::SEEK_SET) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if descriptor_seals != REQUIRED_SEALS {
            return Err(std::io::Error::from_raw_os_error(libc::EPERM));
        }
        let flags = libc::fcntl(*descriptor, libc::F_GETFD);
        if flags < 0 || libc::fcntl(*descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

unsafe fn set_limit(
    resource: libc::__rlimit_resource_t,
    value: libc::rlim_t,
) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    if libc::setrlimit(resource, &limit) == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn require_cloexec(descriptor: RawFd, label: &str) -> Result<(), String> {
    // SAFETY: fcntl only inspects the supplied live descriptor.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
        Err(format!("{label} is not close-on-exec"))
    } else {
        Ok(())
    }
}

unsafe fn require_child_cloexec(descriptor: RawFd) -> std::io::Result<()> {
    let flags = libc::fcntl(descriptor, libc::F_GETFD);
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if flags & libc::FD_CLOEXEC == 0 {
        return Err(std::io::Error::from_raw_os_error(libc::EPERM));
    }
    Ok(())
}

fn seals(descriptor: RawFd) -> Result<libc::c_int, String> {
    // SAFETY: fcntl only inspects the supplied live descriptor.
    let value = unsafe { libc::fcntl(descriptor, libc::F_GET_SEALS) };
    if value < 0 {
        Err(format!(
            "inspect prepared data seals: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::time::Duration;

    #[test]
    fn child_policy_inverts_cloexec_only_for_the_data_allowlist() {
        let executable = sealed_file(b"tool");
        let data = sealed_file(b"data");
        let ambient = sealed_file(b"ambient");
        let (read_end, write_end) = pipe();
        // SAFETY: forked child performs raw syscalls only, then _exit.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            unsafe {
                let ambient_flags = libc::fcntl(ambient.as_raw_fd(), libc::F_GETFD);
                libc::fcntl(
                    ambient.as_raw_fd(),
                    libc::F_SETFD,
                    ambient_flags & !libc::FD_CLOEXEC,
                );
                let status =
                    child_setup(executable.as_raw_fd(), &[data.as_raw_fd()], libc::getppid())
                        .and_then(|_| {
                            let tool = libc::fcntl(executable.as_raw_fd(), libc::F_GETFD);
                            let allowed = libc::fcntl(data.as_raw_fd(), libc::F_GETFD);
                            let other = libc::fcntl(ambient.as_raw_fd(), libc::F_GETFD);
                            if tool & libc::FD_CLOEXEC != 0
                                && allowed & libc::FD_CLOEXEC == 0
                                && other & libc::FD_CLOEXEC != 0
                            {
                                Ok(())
                            } else {
                                Err(std::io::Error::other("unexpected descriptor flags"))
                            }
                        })
                        .is_ok() as u8;
                libc::write(write_end.as_raw_fd(), &status as *const u8 as *const _, 1);
                libc::_exit(0);
            }
        }
        drop(write_end);
        let mut status = [0u8; 1];
        use std::io::Read;
        let mut reader = &read_end;
        reader.read_exact(&mut status).unwrap();
        let mut child_status = 0;
        // SAFETY: pid is the direct child returned by fork.
        assert_eq!(unsafe { libc::waitpid(pid, &mut child_status, 0) }, pid);
        assert_eq!(status, [1]);
    }

    #[test]
    fn request_validation_rejects_ambiguous_or_mutable_descriptors() {
        let executable = sealed_file(b"tool");
        let data = sealed_file(b"data");
        let valid = || Request {
            executable_fd: executable.as_raw_fd(),
            argv: vec!["tool".into()],
            data_fds: vec![data.as_raw_fd()],
        };
        validate(&valid()).unwrap();

        let mut duplicate = valid();
        duplicate.data_fds.push(data.as_raw_fd());
        assert!(validate(&duplicate).is_err());

        let mut executable_alias = valid();
        executable_alias.data_fds = vec![executable.as_raw_fd()];
        assert!(validate(&executable_alias).is_err());

        let mutable = unsealed_file(b"mutable");
        let mut mutable_request = valid();
        mutable_request.data_fds = vec![mutable.as_raw_fd()];
        assert!(validate(&mutable_request).is_err());

        let mut ambient = valid();
        ambient.data_fds = vec![2];
        assert!(validate(&ambient).is_err());

        let mut missing = valid();
        missing.data_fds.clear();
        assert!(validate(&missing).is_err());
    }

    #[test]
    fn request_validation_rejects_cloexec_and_argument_drift() {
        let executable = sealed_file(b"tool");
        let data = sealed_file(b"data");
        // SAFETY: fcntl mutates flags on the live test descriptor.
        assert_eq!(
            unsafe { libc::fcntl(data.as_raw_fd(), libc::F_SETFD, 0) },
            0
        );
        let request = Request {
            executable_fd: executable.as_raw_fd(),
            argv: vec!["tool".into()],
            data_fds: vec![data.as_raw_fd()],
        };
        assert!(validate(&request).unwrap_err().contains("close-on-exec"));
        assert!(c_arguments(&[OsString::from("embedded\0nul")]).is_err());
        let oversized = OsString::from("x".repeat(MAX_EXECVEAT_ARGUMENT_BYTES + 1));
        let sealed = sealed_file(b"sealed");
        let oversized_request = Request {
            executable_fd: executable.as_raw_fd(),
            argv: vec![oversized],
            data_fds: vec![sealed.as_raw_fd()],
        };
        assert!(validate(&oversized_request).is_err());
    }

    #[test]
    fn descriptor_exact_runner_enforces_capture_bounds_and_cleanup() {
        let output = run_shell("printf ok", 16, Duration::from_secs(1)).unwrap();
        assert_eq!(output.stdout, b"ok");
        assert!(run_shell("printf 12345", 4, Duration::from_secs(1))
            .unwrap_err()
            .contains("exceeds"));
        assert!(run_shell("sleep 5", 4, Duration::from_millis(50))
            .unwrap_err()
            .contains("timed out"));
    }

    #[test]
    fn descriptor_exact_exec_rewinds_data_closes_ambient_fds_and_starts_env_empty() {
        let cat = File::open("/bin/cat").unwrap();
        let rewound = sealed_file(b"rewound");
        let output = run_request(
            &cat,
            &rewound,
            vec![
                "cat".into(),
                format!("/proc/self/fd/{}", rewound.as_raw_fd()).into(),
            ],
        )
        .unwrap();
        assert_eq!(output.stdout, b"rewound");

        let environment_probe = sealed_file(b"placeholder");
        let output = run_request(
            &cat,
            &environment_probe,
            vec!["cat".into(), "/proc/self/environ".into()],
        )
        .unwrap();
        assert!(output.stdout.is_empty());

        let shell = File::open("/bin/sh").unwrap();
        let allowed = sealed_file(b"allowed");
        let ambient = sealed_file(b"ambient");
        // SAFETY: the test intentionally makes this descriptor inheritable so
        // child setup, rather than the fixture, must close it at exec.
        assert_eq!(
            unsafe { libc::fcntl(ambient.as_raw_fd(), libc::F_SETFD, 0) },
            0
        );
        let script = format!(
            "test ! -e /proc/self/fd/{} && test ! -e /proc/self/fd/{} && test -e /proc/self/fd/{} && printf ok",
            shell.as_raw_fd(),
            ambient.as_raw_fd(),
            allowed.as_raw_fd(),
        );
        let output = run_request(
            &shell,
            &allowed,
            vec!["sh".into(), "-c".into(), script.into()],
        )
        .unwrap();
        assert_eq!(output.stdout, b"ok");
    }

    fn run_request(
        executable: &File,
        data: &File,
        argv: Vec<OsString>,
    ) -> Result<super::super::Output, String> {
        super::super::run_execveat(
            Request {
                executable_fd: executable.as_raw_fd(),
                argv,
                data_fds: vec![data.as_raw_fd()],
            },
            "descriptor-exact contract fixture",
            64,
            64,
            Duration::from_secs(1),
        )
    }

    fn run_shell(
        script: &str,
        max_stdout: u64,
        timeout: Duration,
    ) -> Result<super::super::Output, String> {
        let executable = File::open("/bin/sh").unwrap();
        let data = sealed_file(b"placeholder");
        super::super::run_execveat(
            Request {
                executable_fd: executable.as_raw_fd(),
                argv: vec!["sh".into(), "-c".into(), script.into()],
                data_fds: vec![data.as_raw_fd()],
            },
            "descriptor-exact fixture",
            max_stdout,
            16,
            timeout,
        )
    }

    fn sealed_file(bytes: &[u8]) -> File {
        let file = unsealed_file(bytes);
        let seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        // SAFETY: fcntl applies seals to the owned memfd.
        assert_eq!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) },
            0
        );
        file
    }

    fn unsealed_file(bytes: &[u8]) -> File {
        // SAFETY: the name is NUL terminated and the returned fd is owned.
        let descriptor = unsafe {
            libc::memfd_create(
                c"semantic-fabric-execveat-test".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        assert!(descriptor >= 0);
        // SAFETY: ownership of the new descriptor transfers to File.
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        file.write_all(bytes).unwrap();
        file
    }

    fn pipe() -> (File, File) {
        let mut descriptors = [0; 2];
        // SAFETY: pipe2 initializes both descriptors on success.
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        // SAFETY: ownership of both new descriptors transfers to Files.
        unsafe {
            (
                File::from_raw_fd(descriptors[0]),
                File::from_raw_fd(descriptors[1]),
            )
        }
    }
}
