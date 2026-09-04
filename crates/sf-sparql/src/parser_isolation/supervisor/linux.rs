//! Descriptor-exact spawn and pre-exec controls.

use std::ffi::CString;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::executable::PreparedParserExecutable;
use super::io::BoundedWorkerIo;
use super::lifecycle::{open_pidfd, terminate_unbound_child, ParserWorkerProcess};
use super::seccomp::StageOnePolicy;
use super::SupervisorError;
use crate::parser_isolation::protocol::ParserWorkerLimits;
use crate::parser_isolation::worker::{PRIVATE_WORKER_MODE, PRIVATE_WORKER_NAME};

const MIN_DYNAMIC_LOADER_FDS: u64 = 16;
const MAX_FIXTURE_ARGUMENTS: usize = 8;
const MAX_FIXTURE_ARGUMENT_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy)]
struct LimitSpec {
    resource: libc::__rlimit_resource_t,
    value: libc::rlim_t,
}

#[derive(Clone, Copy)]
struct ValidatedLimits {
    specs: [LimitSpec; 6],
    open_fd_ceiling: i32,
    wall_time: Duration,
}

impl ValidatedLimits {
    fn new(limits: ParserWorkerLimits) -> Result<Self, SupervisorError> {
        let values = limits.values();
        if !values.cpu_time_millis.is_multiple_of(1_000) {
            return Err(SupervisorError::InvalidLimits(
                "CPU milliseconds are not exactly representable by RLIMIT_CPU",
            ));
        }
        if values.max_processes != 1 {
            return Err(SupervisorError::InvalidLimits(
                "stage-one containment requires exactly one process",
            ));
        }
        if values.max_open_fds < MIN_DYNAMIC_LOADER_FDS {
            return Err(SupervisorError::InvalidLimits(
                "RLIMIT_NOFILE is below the dynamic-loader qualification floor",
            ));
        }
        let open_fd_ceiling = i32::try_from(values.max_open_fds).map_err(|_| {
            SupervisorError::InvalidLimits("RLIMIT_NOFILE exceeds the launch fd range")
        })?;
        let cpu_seconds = values.cpu_time_millis / 1_000;
        let wall_time = Duration::from_millis(values.wall_time_millis);
        Instant::now()
            .checked_add(wall_time)
            .ok_or(SupervisorError::InvalidLimits(
                "wall timeout overflows Instant",
            ))?;
        Ok(Self {
            specs: [
                limit(libc::RLIMIT_CORE, 0)?,
                limit(libc::RLIMIT_AS, values.address_space_bytes)?,
                limit(libc::RLIMIT_CPU, cpu_seconds)?,
                // RLIMIT_FSIZE does not bound pipe output. BoundedWorkerIo
                // independently enforces cumulative `max_output_bytes`.
                limit(libc::RLIMIT_FSIZE, values.max_output_bytes)?,
                limit(libc::RLIMIT_NOFILE, values.max_open_fds)?,
                limit(libc::RLIMIT_STACK, values.stack_bytes)?,
            ],
            open_fd_ceiling,
            wall_time,
        })
    }
}

fn limit(resource: libc::__rlimit_resource_t, value: u64) -> Result<LimitSpec, SupervisorError> {
    let value = libc::rlim_t::try_from(value)
        .map_err(|_| SupervisorError::InvalidLimits("rlimit value does not fit the platform"))?;
    Ok(LimitSpec { resource, value })
}

pub(super) fn spawn_private(
    executable: &PreparedParserExecutable,
    limits: ParserWorkerLimits,
) -> Result<ParserWorkerProcess, SupervisorError> {
    spawn(
        executable,
        limits,
        &[
            PRIVATE_WORKER_NAME.as_bytes(),
            PRIVATE_WORKER_MODE.as_bytes(),
        ],
    )
}

#[cfg(test)]
pub(super) fn spawn_fixture(
    executable: &PreparedParserExecutable,
    limits: ParserWorkerLimits,
    arguments: &[&[u8]],
) -> Result<ParserWorkerProcess, SupervisorError> {
    spawn(executable, limits, arguments)
}

fn spawn(
    executable: &PreparedParserExecutable,
    limits: ParserWorkerLimits,
    arguments: &[&[u8]],
) -> Result<ParserWorkerProcess, SupervisorError> {
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
    {
        let _ = (executable, limits, arguments);
        return Err(SupervisorError::UnsupportedPlatform);
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    {
        require_parent_signal_contracts()?;
        if arguments.is_empty() || arguments.len() > MAX_FIXTURE_ARGUMENTS {
            return Err(SupervisorError::InvalidState(
                "worker argv count is outside the fixed launch bound",
            ));
        }
        let argument_bytes = arguments.iter().try_fold(0_usize, |total, argument| {
            total
                .checked_add(argument.len() + 1)
                .ok_or(SupervisorError::InvalidState(
                    "worker argv byte count overflow",
                ))
        })?;
        if argument_bytes > MAX_FIXTURE_ARGUMENT_BYTES {
            return Err(SupervisorError::InvalidState(
                "worker argv bytes exceed the fixed launch bound",
            ));
        }
        let io_limits = limits.values();
        let limits = ValidatedLimits::new(limits)?;
        let wall_deadline =
            Instant::now()
                .checked_add(limits.wall_time)
                .ok_or(SupervisorError::InvalidLimits(
                    "wall timeout overflows Instant",
                ))?;
        let launch_fd = executable.duplicate_for_launch(limits.open_fd_ceiling)?;
        if launch_fd.as_raw_fd() < limits.open_fd_ceiling {
            return Err(SupervisorError::InvalidState(
                "launch descriptor is below RLIMIT_NOFILE",
            ));
        }
        let c_arguments = arguments
            .iter()
            .map(|argument| {
                CString::new(*argument)
                    .map_err(|_| SupervisorError::InvalidState("private worker argv contains NUL"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut argument_pointers = c_arguments
            .iter()
            .map(|argument| argument.as_ptr() as usize)
            .collect::<Vec<_>>();
        argument_pointers.push(0);
        // Box both pointer arrays before deriving the addresses enforced by
        // seccomp. Moving the closure cannot move either backing allocation.
        let argument_pointers = argument_pointers.into_boxed_slice();
        let empty_environment = Box::new([0_usize]);
        let empty_path = c"".as_ptr() as usize;
        let policy = StageOnePolicy::new(
            launch_fd.as_raw_fd(),
            empty_path,
            argument_pointers.as_ptr() as usize,
            empty_environment.as_ptr() as usize,
        )?;
        let launch_fd_raw = launch_fd.as_raw_fd();
        let limit_specs = limits.specs;
        let mut blocked_signal_mask = unsafe { std::mem::zeroed::<libc::sigset_t>() };
        // SAFETY: sigfillset initializes this parent-owned POD before fork.
        if unsafe { libc::sigfillset(&mut blocked_signal_mask) } != 0 {
            return Err(SupervisorError::operation(
                "build blocked worker signal mask",
            )(std::io::Error::last_os_error()));
        }
        // SAFETY: getpid has no preconditions and is run before fork.
        let expected_parent = unsafe { libc::getpid() };
        // Command supplies only fork, stdio, and process-group plumbing. Its
        // dummy path and configured environment are deliberately bypassed: the
        // final hook uses raw execveat with the held fd and explicit empty envp,
        // or returns an error so Command never attempts its configured path.
        let mut command = Command::new("/__sf_descriptor_exec_only__");
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        // SAFETY: all allocations and pointer construction occur above. The
        // boxed argv/envp arrays keep the exact seccomp-bound addresses stable.
        // The closure performs only fixed control flow over prebuilt POD and
        // raw or POSIX async-signal-safe syscalls, then execs or returns an OS
        // error.
        unsafe {
            command.pre_exec(move || {
                let _arguments_live = &c_arguments;
                child_setup(
                    launch_fd_raw,
                    limits.open_fd_ceiling,
                    expected_parent,
                    &limit_specs,
                    &blocked_signal_mask,
                    &policy,
                )?;
                let result = libc::syscall(
                    libc::SYS_execveat,
                    launch_fd_raw,
                    empty_path as *const libc::c_char,
                    argument_pointers.as_ptr().cast::<*const libc::c_char>(),
                    empty_environment.as_ptr().cast::<*const libc::c_char>(),
                    libc::AT_EMPTY_PATH,
                );
                let _ = result;
                Err(std::io::Error::last_os_error())
            });
        }
        let mut child = command
            .spawn()
            .map_err(SupervisorError::operation("spawn held parser executable"))?;
        drop(launch_fd);
        let pidfd = match open_pidfd(child.id()) {
            Ok(pidfd) => pidfd,
            Err(error) => {
                let cleanup = terminate_unbound_child(&mut child);
                return Err(cleanup.err().unwrap_or(error));
            }
        };
        let process_group = match i32::try_from(child.id()) {
            Ok(process_group) => process_group,
            Err(_) => {
                let primary =
                    SupervisorError::InvalidState("worker PID exceeds process-group range");
                let cleanup = terminate_unbound_child(&mut child);
                return Err(cleanup.err().unwrap_or(primary));
            }
        };
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let primary = SupervisorError::InvalidState("spawned worker has no stdin pipe");
                let cleanup = terminate_unbound_child(&mut child);
                return Err(cleanup.err().unwrap_or(primary));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                drop(stdin);
                let primary = SupervisorError::InvalidState("spawned worker has no stdout pipe");
                let cleanup = terminate_unbound_child(&mut child);
                return Err(cleanup.err().unwrap_or(primary));
            }
        };
        let io = match BoundedWorkerIo::new(
            stdin,
            stdout,
            io_limits.max_input_bytes,
            io_limits.max_output_bytes,
        ) {
            Ok(io) => io,
            Err(primary) => {
                let cleanup = terminate_unbound_child(&mut child);
                return Err(cleanup.err().unwrap_or(primary));
            }
        };
        Ok(ParserWorkerProcess {
            child: Some(child),
            pidfd,
            process_group,
            io,
            wall_deadline,
        })
    }
}

unsafe fn child_setup(
    launch_fd: RawFd,
    open_fd_ceiling: i32,
    expected_parent: libc::pid_t,
    limits: &[LimitSpec; 6],
    blocked_signal_mask: &libc::sigset_t,
    policy: &StageOnePolicy,
) -> std::io::Result<()> {
    // SAFETY: every operation is a raw/async-signal-safe syscall over prebuilt
    // POD. No formatting, allocation, locking, or destructor runs here.
    unsafe {
        libc::umask(0o077);
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
            || libc::getppid() != expected_parent
        {
            return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
        }
        // DUMPABLE may reset across exec; worker entry sets and verifies it
        // again before Ready. NO_NEW_PRIVS persists. All catchable signals
        // stay blocked until that entry resets and verifies their dispositions;
        // SIGKILL cleanup and PDEATHSIG remain effective.
        if libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0
            || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::sigprocmask(libc::SIG_SETMASK, blocked_signal_mask, std::ptr::null_mut()) != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        for limit in limits {
            set_and_verify_limit(*limit)?;
        }
        if launch_fd < open_fd_ceiling {
            return Err(std::io::Error::from_raw_os_error(libc::EPERM));
        }
        if libc::close_range(3, u32::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let flags = libc::fcntl(launch_fd, libc::F_GETFD);
        if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
            return Err(std::io::Error::from_raw_os_error(libc::EPERM));
        }
        policy.install_in_child()?;
        Ok(())
    }
}

unsafe fn set_and_verify_limit(limit: LimitSpec) -> std::io::Result<()> {
    let requested = libc::rlimit {
        rlim_cur: limit.value,
        rlim_max: limit.value,
    };
    // SAFETY: pointers address fixed stack POD and pid 0 means this child.
    if unsafe {
        libc::syscall(
            libc::SYS_prlimit64,
            0,
            limit.resource,
            &requested as *const libc::rlimit,
            std::ptr::null_mut::<libc::rlimit>(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut observed = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe {
        libc::syscall(
            libc::SYS_prlimit64,
            0,
            limit.resource,
            std::ptr::null::<libc::rlimit>(),
            &mut observed as *mut libc::rlimit,
        )
    } != 0
        || observed.rlim_cur != limit.value
        || observed.rlim_max != limit.value
    {
        return Err(std::io::Error::from_raw_os_error(libc::EPERM));
    }
    Ok(())
}

fn require_parent_signal_contracts() -> Result<(), SupervisorError> {
    // Post-spawn pidfd_open is valid only while SIGCHLD creates a zombie and no
    // competing wait-any reaper consumes this child. The disposition is checked
    // here; exclusive reaping remains an explicit integration precondition.
    let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
    // SAFETY: a null new action queries the process-wide disposition.
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) } != 0 {
        return Err(SupervisorError::operation("inspect SIGCHLD disposition")(
            std::io::Error::last_os_error(),
        ));
    }
    if action.sa_sigaction == libc::SIG_IGN || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
        return Err(SupervisorError::InvalidState(
            "SIGCHLD disposition cannot preserve an unreaped child",
        ));
    }
    // The nonblocking parent writer uses a pipe write after poll. A concurrent
    // close can still race that syscall, so SIGPIPE must remain ignored and
    // EPIPE must be reported as an ordinary contained I/O failure. Rust's
    // standard runtime establishes this disposition; future integration must
    // exclude process-wide signal mutation while supervisors can be live.
    let mut pipe_action = unsafe { std::mem::zeroed::<libc::sigaction>() };
    // SAFETY: a null new action queries the process-wide disposition.
    if unsafe { libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut pipe_action) } != 0 {
        return Err(SupervisorError::operation("inspect SIGPIPE disposition")(
            std::io::Error::last_os_error(),
        ));
    }
    if pipe_action.sa_sigaction != libc::SIG_IGN {
        return Err(SupervisorError::InvalidState(
            "SIGPIPE disposition cannot make worker pipe failures recoverable",
        ));
    }
    Ok(())
}
