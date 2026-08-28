use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::model::{BaselineRow, Disposition, Profile, Suite, TestIdentity};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedOutcome {
    Passed,
    Failed,
    Ignored,
}

impl ObservedOutcome {
    fn name(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedTest {
    pub name: String,
    pub outcome: ObservedOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailure {
    pub message: String,
}

impl RunFailure {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait SuiteRunner {
    fn discover(&self, root: &Path, suite: &Suite) -> Result<Vec<String>, RunFailure>;

    fn execute(
        &self,
        root: &Path,
        suite: &Suite,
        excluded: &[String],
    ) -> Result<Vec<ObservedTest>, RunFailure>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessRunner;

impl SuiteRunner for ProcessRunner {
    fn discover(&self, root: &Path, suite: &Suite) -> Result<Vec<String>, RunFailure> {
        let mut args = cargo_prefix(suite);
        args.extend([
            "--".into(),
            "--list".into(),
            "--format".into(),
            "terse".into(),
        ]);
        let capture = run_capture(root, &args, suite.timeout_seconds)?;
        if !capture.status.success() {
            return Err(RunFailure::new(format!(
                "test discovery exited with {}: {}",
                capture.status, capture.stderr
            )));
        }
        parse_test_list(&capture.stdout)
    }

    fn execute(
        &self,
        root: &Path,
        suite: &Suite,
        excluded: &[String],
    ) -> Result<Vec<ObservedTest>, RunFailure> {
        let mut args = cargo_prefix(suite);
        args.extend([
            "--".into(),
            "--test-threads=1".into(),
            "--format=pretty".into(),
        ]);
        for name in excluded {
            args.push("--skip".into());
            args.push(name.into());
        }
        let capture = run_capture(root, &args, suite.timeout_seconds)?;
        let (observed, summary) = parse_test_run(&capture.stdout)?;
        validate_summary(&observed, &summary, excluded.len())?;
        let expected_success = summary.failed == 0;
        if capture.status.success() != expected_success {
            return Err(RunFailure::new(format!(
                "test process status {} contradicts parsed result summary: {}",
                capture.status, capture.stderr
            )));
        }
        Ok(observed)
    }
}

pub fn execute_profile<R: SuiteRunner>(
    profile: &Profile,
    root: &Path,
    runner: &R,
) -> Result<Vec<BaselineRow>, RunFailure> {
    let mut rows = Vec::new();
    for suite in &profile.suites {
        let declared = tests_for_suite(profile, &suite.id);
        let declared_names: Vec<_> = declared.iter().map(|test| test.name.clone()).collect();
        let discovered = runner.discover(root, suite)?;
        if discovered != declared_names {
            return Err(RunFailure::new(identity_drift_message(
                suite,
                &declared_names,
                &discovered,
            )));
        }
        let included: Vec<_> = declared
            .iter()
            .filter(|test| test.disposition == Disposition::Include)
            .map(|test| test.name.clone())
            .collect();
        let excluded: Vec<_> = declared
            .iter()
            .filter(|test| test.disposition == Disposition::Exclude)
            .map(|test| test.name.clone())
            .collect();
        validate_skip_isolation(&suite.id, &included, &excluded)?;
        let mut observed = runner.execute(root, suite, &excluded)?;
        observed.sort_by(|left, right| left.name.cmp(&right.name));
        if observed.windows(2).any(|pair| pair[0].name == pair[1].name) {
            return Err(RunFailure::new(format!(
                "suite {} emitted duplicate per-test outcomes",
                suite.id
            )));
        }
        let observed_names: Vec<_> = observed.iter().map(|test| test.name.clone()).collect();
        if observed_names != included {
            return Err(RunFailure::new(identity_drift_message(
                suite,
                &included,
                &observed_names,
            )));
        }
        for test in observed {
            if test.outcome != ObservedOutcome::Passed {
                return Err(RunFailure::new(format!(
                    "suite {} required test {} observed {}",
                    suite.id,
                    test.name,
                    test.outcome.name()
                )));
            }
            rows.push(BaselineRow {
                suite_id: suite.id.clone(),
                test_name: test.name,
            });
        }
    }
    Ok(rows)
}

fn tests_for_suite<'a>(profile: &'a Profile, suite_id: &str) -> Vec<&'a TestIdentity> {
    profile
        .tests
        .iter()
        .filter(|test| test.suite_id == suite_id)
        .collect()
}

fn validate_skip_isolation(
    suite_id: &str,
    included: &[String],
    excluded: &[String],
) -> Result<(), RunFailure> {
    for exclusion in excluded {
        if included.iter().any(|name| name.contains(exclusion)) {
            return Err(RunFailure::new(format!(
                "suite {suite_id} exclusion {exclusion:?} also filters a required test"
            )));
        }
    }
    Ok(())
}

fn identity_drift_message(suite: &Suite, expected: &[String], actual: &[String]) -> String {
    let missing: Vec<_> = expected
        .iter()
        .filter(|name| !actual.contains(name))
        .collect();
    let added: Vec<_> = actual
        .iter()
        .filter(|name| !expected.contains(name))
        .collect();
    format!(
        "suite {} test identity drift: missing={missing:?}, added={added:?}",
        suite.id
    )
}

fn cargo_prefix(suite: &Suite) -> Vec<OsString> {
    [
        "test",
        "--locked",
        "--offline",
        "-p",
        &suite.package,
        "--test",
        &suite.target,
    ]
    .into_iter()
    .map(Into::into)
    .collect()
}

fn command(root: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(args);
    super::environment::scrub(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

struct Capture {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_capture(
    root: &Path,
    args: &[OsString],
    timeout_seconds: u64,
) -> Result<Capture, RunFailure> {
    let mut child = command(root, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| RunFailure::new(error.to_string()))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = thread::spawn(move || read_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_bounded(stderr));
    let status = wait_with_timeout(&mut child, timeout_seconds);
    let stdout = join_reader(stdout_reader);
    let stderr = join_reader(stderr_reader);
    Ok(Capture {
        status: status?,
        stdout: String::from_utf8(stdout?)
            .map_err(|error| RunFailure::new(format!("stdout is not UTF-8: {error}")))?,
        stderr: String::from_utf8_lossy(&stderr?).into_owned(),
    })
}

pub(super) fn wait_with_timeout(
    child: &mut std::process::Child,
    seconds: u64,
) -> Result<ExitStatus, RunFailure> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        match exited_status(child) {
            Ok(Some(status)) => {
                return Ok(status);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let cleanup = terminate_process_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                cleanup?;
                return Err(RunFailure::new(format!(
                    "test command exceeded {seconds} seconds; process group terminated"
                )));
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(target_os = "linux")]
fn exited_status(child: &mut std::process::Child) -> Result<Option<ExitStatus>, RunFailure> {
    let pid = i32::try_from(child.id()).map_err(|_| RunFailure::new("child PID exceeds i32"))?;
    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: waitid initializes siginfo_t for this direct child. WNOWAIT keeps
    // the exited group leader unreaped until its descendants have been swept.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(RunFailure::new(format!(
            "inspect child without reaping: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: successful waitid initialized the value above.
    let information = unsafe { information.assume_init() };
    // SAFETY: si_pid is valid for the SIGCHLD payload returned by waitid.
    if unsafe { information.si_pid() } == 0 {
        return Ok(None);
    }
    terminate_process_group(child.id())?;
    child
        .wait()
        .map(Some)
        .map_err(|error| RunFailure::new(format!("reap exited test command: {error}")))
}

#[cfg(not(target_os = "linux"))]
fn exited_status(child: &mut std::process::Child) -> Result<Option<ExitStatus>, RunFailure> {
    let status = child
        .try_wait()
        .map_err(|error| RunFailure::new(format!("wait for test command: {error}")))?;
    if status.is_some() {
        terminate_process_group(child.id())?;
    }
    Ok(status)
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) -> Result<(), RunFailure> {
    let pgid = i32::try_from(pid).map_err(|_| RunFailure::new("child PID exceeds i32"))?;
    // SAFETY: `pgid` is the positive PID returned by Child::id, negated only to
    // address the isolated process group created with CommandExt::process_group.
    let result = unsafe { libc::kill(-pgid, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(RunFailure::new(format!(
            "terminate process group {pgid}: {error}"
        )))
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_pid: u32) -> Result<(), RunFailure> {
    Ok(())
}

fn read_bounded(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    reader
        .by_ref()
        .take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut output)
        .map_err(|error| error.to_string())?;
    if output.len() as u64 > MAX_CAPTURE_BYTES {
        Err("test process output exceeds 1 MiB".to_owned())
    } else {
        Ok(output)
    }
}

fn join_reader(handle: thread::JoinHandle<Result<Vec<u8>, String>>) -> Result<Vec<u8>, RunFailure> {
    handle
        .join()
        .map_err(|_| RunFailure::new("output reader panicked"))?
        .map_err(RunFailure::new)
}

pub(super) fn parse_test_list(output: &str) -> Result<Vec<String>, RunFailure> {
    let mut names = Vec::new();
    for line in output.lines() {
        if let Some(name) = line.strip_suffix(": test") {
            names.push(name.to_owned());
        } else if line.ends_with(": benchmark") {
            return Err(RunFailure::new(
                "benchmark discovered in a regression test target",
            ));
        }
    }
    names.sort();
    if names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(RunFailure::new("duplicate discovered test identity"));
    }
    Ok(names)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Summary {
    pub(super) passed: usize,
    pub(super) failed: usize,
    pub(super) ignored: usize,
    pub(super) measured: usize,
    pub(super) filtered: usize,
}

pub(super) fn parse_test_run(output: &str) -> Result<(Vec<ObservedTest>, Summary), RunFailure> {
    let mut observed = Vec::new();
    let mut summary = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("test result: ") {
            if summary.replace(parse_summary(value)?).is_some() {
                return Err(RunFailure::new("duplicate libtest result summary"));
            }
        } else if let Some(value) = line.strip_prefix("test ") {
            let (name, outcome) = parse_result_line(value)?;
            observed.push(ObservedTest {
                name: name.to_owned(),
                outcome,
            });
        }
    }
    let summary = summary.ok_or_else(|| RunFailure::new("missing libtest result summary"))?;
    Ok((observed, summary))
}

fn parse_result_line(value: &str) -> Result<(&str, ObservedOutcome), RunFailure> {
    let (name, status) = value
        .rsplit_once(" ... ")
        .ok_or_else(|| RunFailure::new(format!("malformed libtest result line {value:?}")))?;
    let outcome = match status {
        "ok" => ObservedOutcome::Passed,
        "FAILED" => ObservedOutcome::Failed,
        value if value == "ignored" || value.starts_with("ignored, ") => ObservedOutcome::Ignored,
        _ => {
            return Err(RunFailure::new(format!(
                "unknown libtest outcome {status:?}"
            )))
        }
    };
    Ok((name, outcome))
}

fn parse_summary(value: &str) -> Result<Summary, RunFailure> {
    let fields: Vec<_> = value.split(';').map(str::trim).collect();
    if fields.len() != 6 || !fields[5].starts_with("finished in ") {
        return Err(RunFailure::new("malformed libtest result summary"));
    }
    let (label, passed) = fields[0]
        .split_once(". ")
        .ok_or_else(|| RunFailure::new("malformed libtest result label"))?;
    if !matches!(label, "ok" | "FAILED") {
        return Err(RunFailure::new("unknown libtest result label"));
    }
    Ok(Summary {
        passed: parse_metric(passed, "passed")?,
        failed: parse_metric(fields[1], "failed")?,
        ignored: parse_metric(fields[2], "ignored")?,
        measured: parse_metric(fields[3], "measured")?,
        filtered: parse_metric(fields[4], "filtered out")?,
    })
}

fn parse_metric(value: &str, label: &str) -> Result<usize, RunFailure> {
    value
        .strip_suffix(label)
        .map(str::trim)
        .and_then(|count| count.parse().ok())
        .ok_or_else(|| RunFailure::new(format!("malformed libtest {label} count")))
}

fn validate_summary(
    observed: &[ObservedTest],
    summary: &Summary,
    excluded_count: usize,
) -> Result<(), RunFailure> {
    let count = |outcome| {
        observed
            .iter()
            .filter(|test| test.outcome == outcome)
            .count()
    };
    if summary.passed != count(ObservedOutcome::Passed)
        || summary.failed != count(ObservedOutcome::Failed)
        || summary.ignored != count(ObservedOutcome::Ignored)
        || summary.measured != 0
        || summary.filtered != excluded_count
    {
        return Err(RunFailure::new(
            "libtest summary does not match parsed per-test outcomes and exclusions",
        ));
    }
    Ok(())
}
