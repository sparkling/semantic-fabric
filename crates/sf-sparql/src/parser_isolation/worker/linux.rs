//! Post-exec worker state repair, verification, and control handshake.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;

use super::policy_candidate::ControlReadyPolicyCandidate;
use crate::parser_isolation::build_identity;
use crate::parser_isolation::profile::{
    control_ready_profile_candidate_digest, v1_limits, V1_CANDIDATE_RLIMIT_FSIZE_BYTES,
    V1_LIMIT_VALUES,
};
use crate::parser_isolation::protocol::{BuildIdentityDigest, HelloFrame, FRAME_LEN};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WorkerFailure;

pub(super) fn run() -> Result<(), WorkerFailure> {
    let stable_parent = repair_and_verify_kernel_envelope()?;
    let build_identity = observe_current_executable()?;
    close_unintended_descriptors()?;
    verify_standard_descriptors()?;
    verify_parent_is_stable(stable_parent)?;

    let parser_profile_candidate = control_ready_profile_candidate_digest();
    let observed_limits = v1_limits();
    let policy = ControlReadyPolicyCandidate::new().ok_or(WorkerFailure)?;
    if !policy.install_and_verify() {
        return Err(WorkerFailure);
    }

    let mut encoded_hello = [0_u8; FRAME_LEN];
    read_exact(libc::STDIN_FILENO, &mut encoded_hello)?;
    let hello = HelloFrame::decode(&encoded_hello).map_err(|_| WorkerFailure)?;
    let ready = hello
        .acknowledge_verified(build_identity, parser_profile_candidate, observed_limits)
        .map_err(|_| WorkerFailure)?;
    write_all(libc::STDOUT_FILENO, &ready.encode())?;

    // QueryV1 is not implemented. Remaining alive until the trusted parent
    // closes its pipe proves a real control-ready transition without accepting
    // an unframed byte as a parser request. This does not qualify the candidate
    // syscall surface for parser execution.
    require_parent_eof()
}

fn repair_and_verify_kernel_envelope() -> Result<libc::pid_t, WorkerFailure> {
    close_unintended_descriptors()?;
    verify_standard_descriptors()?;

    let process = unsafe { libc::getpid() };
    let thread = unsafe { libc::syscall(libc::SYS_gettid) };
    let process_group = unsafe { libc::getpgrp() };
    if process <= 1 || thread != libc::c_long::from(process) || process_group != process {
        return Err(WorkerFailure);
    }
    let parent = unsafe { libc::getppid() };
    if parent <= 1 {
        return Err(WorkerFailure);
    }

    let mut death_signal = 0;
    if unsafe { libc::prctl(libc::PR_GET_PDEATHSIG, &mut death_signal, 0, 0, 0) } != 0
        || death_signal != libc::SIGKILL
        || unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) } != 1
        || unsafe { libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) }
            != libc::SECCOMP_MODE_FILTER as libc::c_int
    {
        return Err(WorkerFailure);
    }

    verify_limits_and_stage_one()?;
    reset_and_verify_signals()?;
    let previous_umask = unsafe { libc::umask(0o077) };
    if previous_umask != 0o077
        || unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0
        || unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) } != 0
    {
        return Err(WorkerFailure);
    }
    verify_parent_is_stable(parent)?;
    Ok(parent)
}

fn verify_parent_is_stable(expected: libc::pid_t) -> Result<(), WorkerFailure> {
    if unsafe { libc::getppid() } == expected {
        Ok(())
    } else {
        Err(WorkerFailure)
    }
}

fn close_unintended_descriptors() -> Result<(), WorkerFailure> {
    let result = unsafe { libc::close_range(3, u32::MAX, 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(WorkerFailure)
    }
}

fn verify_standard_descriptors() -> Result<(), WorkerFailure> {
    let input = verify_descriptor(libc::STDIN_FILENO, libc::O_RDONLY, libc::S_IFIFO, None)?;
    let output = verify_descriptor(libc::STDOUT_FILENO, libc::O_WRONLY, libc::S_IFIFO, None)?;
    if input == output {
        return Err(WorkerFailure);
    }
    verify_descriptor(
        libc::STDERR_FILENO,
        libc::O_WRONLY,
        libc::S_IFCHR,
        Some(libc::makedev(1, 3)),
    )?;
    Ok(())
}

fn verify_descriptor(
    descriptor: libc::c_int,
    access_mode: libc::c_int,
    file_type: libc::mode_t,
    expected_device: Option<libc::dev_t>,
) -> Result<(libc::dev_t, libc::ino_t), WorkerFailure> {
    let descriptor_flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    let status_flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    if descriptor_flags != 0
        || status_flags < 0
        || status_flags & libc::O_ACCMODE != access_mode
        || status_flags & libc::O_NONBLOCK != 0
        || unsafe { libc::fstat(descriptor, &mut metadata) } != 0
        || metadata.st_mode & libc::S_IFMT != file_type
        || expected_device.is_some_and(|device| metadata.st_rdev != device)
    {
        return Err(WorkerFailure);
    }
    Ok((metadata.st_dev, metadata.st_ino))
}

fn verify_limits_and_stage_one() -> Result<(), WorkerFailure> {
    // `max_processes` is a policy count, not RLIMIT_NPROC: that limit is shared
    // by the real UID and privileged processes may be exempt. The parent-side
    // stage-one policy's tested fork/clone denial and this worker's later
    // default-kill candidate enforce the one-process contract instead.
    if V1_LIMIT_VALUES.max_processes != 1 {
        return Err(WorkerFailure);
    }
    for (resource, value) in [
        (libc::RLIMIT_CORE, 0),
        (libc::RLIMIT_AS, V1_LIMIT_VALUES.address_space_bytes),
        (libc::RLIMIT_CPU, V1_LIMIT_VALUES.cpu_time_millis / 1_000),
        (libc::RLIMIT_FSIZE, V1_CANDIDATE_RLIMIT_FSIZE_BYTES),
        (libc::RLIMIT_NOFILE, V1_LIMIT_VALUES.max_open_fds),
        (libc::RLIMIT_STACK, V1_LIMIT_VALUES.stack_bytes),
    ] {
        let observed = query_limit(resource)?;
        if observed.rlim_cur != value || observed.rlim_max != value {
            return Err(WorkerFailure);
        }
    }

    // Setting an already-equal value is harmless without stage one, but the
    // inherited launch policy must reject every mutation with EPERM.
    let core = query_limit(libc::RLIMIT_CORE)?;
    let mutation = unsafe {
        libc::syscall(
            libc::SYS_prlimit64,
            0,
            libc::RLIMIT_CORE,
            &core as *const libc::rlimit,
            std::ptr::null_mut::<libc::rlimit>(),
        )
    };
    if mutation != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
        return Err(WorkerFailure);
    }
    Ok(())
}

fn query_limit(resource: libc::__rlimit_resource_t) -> Result<libc::rlimit, WorkerFailure> {
    let mut observed = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let result = unsafe {
        libc::syscall(
            libc::SYS_prlimit64,
            0,
            resource,
            std::ptr::null::<libc::rlimit>(),
            &mut observed as *mut libc::rlimit,
        )
    };
    if result == 0 {
        Ok(observed)
    } else {
        Err(WorkerFailure)
    }
}

fn reset_and_verify_signals() -> Result<(), WorkerFailure> {
    let mut inherited = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    if unsafe { libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut inherited) } != 0 {
        return Err(WorkerFailure);
    }
    reset_glibc_internal_signal_dispositions()?;
    for signal in 1..=libc::SIGRTMAX() {
        if signal == libc::SIGKILL || signal == libc::SIGSTOP {
            continue;
        }
        // glibc reserves kernel signal numbers 32 and 33 for its threading
        // implementation and rejects public sigaction calls for them. They are
        // reset and verified through the raw x86-64 kernel ABI above.
        if signal == 32 || signal == 33 {
            continue;
        }
        if unsafe { libc::sigismember(&inherited, signal) } != 1 {
            return Err(WorkerFailure);
        }
        let action = libc::sigaction {
            sa_sigaction: libc::SIG_DFL,
            sa_mask: unsafe { std::mem::zeroed() },
            sa_flags: 0,
            sa_restorer: None,
        };
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(WorkerFailure);
        }
        let mut observed = unsafe { std::mem::zeroed::<libc::sigaction>() };
        if unsafe { libc::sigaction(signal, std::ptr::null(), &mut observed) } != 0
            || observed.sa_sigaction != libc::SIG_DFL
        {
            return Err(WorkerFailure);
        }
    }
    let mut pending = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    if unsafe { libc::sigpending(&mut pending) } != 0 {
        return Err(WorkerFailure);
    }
    for signal in 1..=libc::SIGRTMAX() {
        if unsafe { libc::sigismember(&pending, signal) } == 1 {
            return Err(WorkerFailure);
        }
    }
    // Rust's Unix runtime installs a process alt-stack before `main`. The
    // private entry deliberately removes that application-runtime handler
    // surface after restoring default dispositions.
    let disabled = libc::stack_t {
        ss_sp: std::ptr::null_mut(),
        ss_flags: libc::SS_DISABLE,
        ss_size: 0,
    };
    let mut alternate = unsafe { std::mem::zeroed::<libc::stack_t>() };
    if unsafe { libc::sigaltstack(&disabled, std::ptr::null_mut()) } != 0
        || unsafe { libc::sigaltstack(std::ptr::null(), &mut alternate) } != 0
        || alternate.ss_flags & libc::SS_DISABLE == 0
    {
        return Err(WorkerFailure);
    }

    // Unblock only after dispositions, pending state, and the runtime alt-stack
    // have reached their final contract. This avoids delivering an inherited
    // signal while the envelope is half repaired; fatal synchronous signals
    // and RLIMIT_CPU's SIGXCPU are then able to terminate normally.
    let mut unblocked = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    let mut observed_mask = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    if unsafe { libc::sigemptyset(&mut unblocked) } != 0
        || unsafe { libc::sigprocmask(libc::SIG_SETMASK, &unblocked, std::ptr::null_mut()) } != 0
        || unsafe { libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut observed_mask) }
            != 0
    {
        return Err(WorkerFailure);
    }
    for signal in 1..=libc::SIGRTMAX() {
        if unsafe { libc::sigismember(&observed_mask, signal) } != 0 {
            return Err(WorkerFailure);
        }
    }
    Ok(())
}

#[repr(C)]
struct KernelSignalAction {
    handler: usize,
    flags: libc::c_ulong,
    restorer: usize,
    mask: u64,
}

fn reset_glibc_internal_signal_dispositions() -> Result<(), WorkerFailure> {
    // The x86-64 kernel ABI uses one 64-bit signal-set word. Calling glibc's
    // sigaction wrapper cannot address its two NPTL-reserved signals, while an
    // inherited SIG_IGN disposition would survive exec. Reset and read back
    // both through rt_sigaction instead of assuming exec repaired them.
    let requested = KernelSignalAction {
        handler: libc::SIG_DFL,
        flags: 0,
        restorer: 0,
        mask: 0,
    };
    for signal in [32, 33] {
        let mut observed = KernelSignalAction {
            handler: usize::MAX,
            flags: usize::MAX as libc::c_ulong,
            restorer: usize::MAX,
            mask: u64::MAX,
        };
        let set = unsafe {
            libc::syscall(
                libc::SYS_rt_sigaction,
                signal,
                &requested as *const KernelSignalAction,
                std::ptr::null_mut::<KernelSignalAction>(),
                std::mem::size_of::<u64>(),
            )
        };
        let query = unsafe {
            libc::syscall(
                libc::SYS_rt_sigaction,
                signal,
                std::ptr::null::<KernelSignalAction>(),
                &mut observed as *mut KernelSignalAction,
                std::mem::size_of::<u64>(),
            )
        };
        if set != 0 || query != 0 || observed.handler != libc::SIG_DFL {
            return Err(WorkerFailure);
        }
    }
    Ok(())
}

fn observe_current_executable() -> Result<BuildIdentityDigest, WorkerFailure> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open("/proc/self/exe")
        .map_err(|_| WorkerFailure)?;
    let identity = build_identity::observe(&file).ok_or(WorkerFailure)?;
    drop(file);
    Ok(identity)
}

fn read_exact(descriptor: libc::c_int, output: &mut [u8]) -> Result<(), WorkerFailure> {
    let mut offset = 0;
    while offset < output.len() {
        let count = unsafe {
            libc::read(
                descriptor,
                output[offset..].as_mut_ptr().cast(),
                output.len() - offset,
            )
        };
        if count > 0 {
            offset += count as usize;
        } else if count < 0
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
        {
            continue;
        } else {
            return Err(WorkerFailure);
        }
    }
    Ok(())
}

fn write_all(descriptor: libc::c_int, input: &[u8]) -> Result<(), WorkerFailure> {
    let mut offset = 0;
    while offset < input.len() {
        let count = unsafe {
            libc::write(
                descriptor,
                input[offset..].as_ptr().cast(),
                input.len() - offset,
            )
        };
        if count > 0 {
            offset += count as usize;
        } else if count < 0
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
        {
            continue;
        } else {
            return Err(WorkerFailure);
        }
    }
    Ok(())
}

fn require_parent_eof() -> Result<(), WorkerFailure> {
    let mut unexpected = [0_u8; 1];
    loop {
        let count = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                unexpected.as_mut_ptr().cast(),
                unexpected.len(),
            )
        };
        if count == 0 {
            return Ok(());
        }
        if count > 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(WorkerFailure);
        }
    }
}
