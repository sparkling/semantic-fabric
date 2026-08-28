use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use super::bounded_io;
use super::digest::sha256_hex;

const MAGIC: &str = "sf-performance-runner-profile-v1";
pub const MAX_PROFILE_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError(pub String);

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProfileError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerProfile {
    pub profile_id: String,
    pub controlled: bool,
    pub os: String,
    pub architecture: String,
    pub kernel_release: String,
    pub cpu_model: String,
    pub online_cpus: String,
    pub allowed_cpus: String,
    pub isolated_cpus: String,
    pub scaling_governor: String,
    pub turbo: String,
    pub swap_total_kib: u64,
    pub mem_total_kib: u64,
    pub load1_limit_milli: u64,
    pub build_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSnapshot {
    pub os: String,
    pub architecture: String,
    pub kernel_release: String,
    pub cpu_model: String,
    pub online_cpus: String,
    pub allowed_cpus: String,
    pub isolated_cpus: String,
    pub scaling_governor: String,
    pub turbo: String,
    pub swap_total_kib: u64,
    pub mem_total_kib: u64,
    pub load1_milli: u64,
    pub build_profile: String,
}

pub trait RunnerProbe {
    type Error: fmt::Display;

    fn probe(&self) -> Result<RunnerSnapshot, Self::Error>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxRunnerProbe;

impl RunnerProbe for LinuxRunnerProbe {
    type Error = ProfileError;

    fn probe(&self) -> Result<RunnerSnapshot, Self::Error> {
        probe_linux()
    }
}

impl RunnerProfile {
    pub fn digest(&self) -> Result<String, ProfileError> {
        Ok(sha256_hex(render_profile(self)?.as_bytes()))
    }

    pub fn validate<P: RunnerProbe>(&self, probe: &P) -> Result<RunnerSnapshot, ProfileError> {
        if !self.controlled {
            return Err(ProfileError(
                "runner profile is explicitly uncontrolled; operator review is required".into(),
            ));
        }
        if self.os != "linux"
            || self.scaling_governor != "performance"
            || self.turbo != "disabled"
            || self.swap_total_kib != 0
            || self.build_profile != "release"
        {
            return Err(ProfileError(
                "controlled profile must require Linux, release build, performance governor, disabled turbo, and zero swap"
                    .into(),
            ));
        }
        let snapshot = probe
            .probe()
            .map_err(|error| ProfileError(format!("probe runner: {error}")))?;
        compare_static(self, &snapshot)?;
        let allowed = parse_cpu_list(&snapshot.allowed_cpus)?;
        let isolated = parse_cpu_list(&snapshot.isolated_cpus)?;
        let online = parse_cpu_list(&snapshot.online_cpus)?;
        if allowed.is_empty() || !allowed.is_subset(&online) || !allowed.is_subset(&isolated) {
            return Err(ProfileError(
                "allowed CPU set must be non-empty, online, and wholly isolated".into(),
            ));
        }
        if snapshot.load1_milli > self.load1_limit_milli {
            return Err(ProfileError(format!(
                "runner load1 {} milli exceeds profile limit {} milli",
                snapshot.load1_milli, self.load1_limit_milli
            )));
        }
        Ok(snapshot)
    }
}

pub fn render_profile(profile: &RunnerProfile) -> Result<String, ProfileError> {
    validate_text("profile id", &profile.profile_id, true)?;
    for (label, value) in [
        ("os", profile.os.as_str()),
        ("architecture", profile.architecture.as_str()),
        ("kernel release", profile.kernel_release.as_str()),
        ("cpu model", profile.cpu_model.as_str()),
        ("online CPUs", profile.online_cpus.as_str()),
        ("allowed CPUs", profile.allowed_cpus.as_str()),
        ("scaling governor", profile.scaling_governor.as_str()),
        ("turbo", profile.turbo.as_str()),
        ("build profile", profile.build_profile.as_str()),
    ] {
        validate_text(label, value, false)?;
    }
    if profile.isolated_cpus.len() > 512 || profile.isolated_cpus.contains(['\t', '\n', '\r']) {
        return Err(ProfileError("invalid isolated CPUs".into()));
    }
    let output = format!(
        "{MAGIC}\nprofile-id\t{}\ncontrolled\t{}\nos\t{}\narchitecture\t{}\nkernel-release\t{}\ncpu-model\t{}\nonline-cpus\t{}\nallowed-cpus\t{}\nisolated-cpus\t{}\nscaling-governor\t{}\nturbo\t{}\nswap-total-kib\t{}\nmem-total-kib\t{}\nload1-limit-milli\t{}\nbuild-profile\t{}\n",
        profile.profile_id,
        profile.controlled,
        profile.os,
        profile.architecture,
        profile.kernel_release,
        profile.cpu_model,
        profile.online_cpus,
        profile.allowed_cpus,
        profile.isolated_cpus,
        profile.scaling_governor,
        profile.turbo,
        profile.swap_total_kib,
        profile.mem_total_kib,
        profile.load1_limit_milli,
        profile.build_profile,
    );
    if output.len() > MAX_PROFILE_BYTES {
        return Err(ProfileError("runner profile exceeds byte bound".into()));
    }
    Ok(output)
}

pub fn parse_profile(bytes: &[u8]) -> Result<RunnerProfile, ProfileError> {
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err(ProfileError("runner profile exceeds byte bound".into()));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ProfileError("runner profile is not UTF-8".into()))?;
    if !text.ends_with('\n') || text.contains('\r') {
        return Err(ProfileError(
            "runner profile must use canonical LF termination".into(),
        ));
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != 16 || lines[0] != MAGIC {
        return Err(ProfileError(
            "invalid runner profile header or field count".into(),
        ));
    }
    let profile = RunnerProfile {
        profile_id: field(&lines, 1, "profile-id")?.to_owned(),
        controlled: parse_field(&lines, 2, "controlled")?,
        os: field(&lines, 3, "os")?.to_owned(),
        architecture: field(&lines, 4, "architecture")?.to_owned(),
        kernel_release: field(&lines, 5, "kernel-release")?.to_owned(),
        cpu_model: field(&lines, 6, "cpu-model")?.to_owned(),
        online_cpus: field(&lines, 7, "online-cpus")?.to_owned(),
        allowed_cpus: field(&lines, 8, "allowed-cpus")?.to_owned(),
        isolated_cpus: field(&lines, 9, "isolated-cpus")?.to_owned(),
        scaling_governor: field(&lines, 10, "scaling-governor")?.to_owned(),
        turbo: field(&lines, 11, "turbo")?.to_owned(),
        swap_total_kib: parse_field(&lines, 12, "swap-total-kib")?,
        mem_total_kib: parse_field(&lines, 13, "mem-total-kib")?,
        load1_limit_milli: parse_field(&lines, 14, "load1-limit-milli")?,
        build_profile: field(&lines, 15, "build-profile")?.to_owned(),
    };
    if render_profile(&profile)? != text {
        return Err(ProfileError("runner profile is not canonical".into()));
    }
    Ok(profile)
}

pub fn render_uncontrolled_template(snapshot: &RunnerSnapshot) -> Result<String, ProfileError> {
    render_profile(&RunnerProfile {
        profile_id: "operator-must-name-profile".into(),
        controlled: false,
        os: snapshot.os.clone(),
        architecture: snapshot.architecture.clone(),
        kernel_release: snapshot.kernel_release.clone(),
        cpu_model: snapshot.cpu_model.clone(),
        online_cpus: snapshot.online_cpus.clone(),
        allowed_cpus: snapshot.allowed_cpus.clone(),
        isolated_cpus: snapshot.isolated_cpus.clone(),
        scaling_governor: snapshot.scaling_governor.clone(),
        turbo: snapshot.turbo.clone(),
        swap_total_kib: snapshot.swap_total_kib,
        mem_total_kib: snapshot.mem_total_kib,
        load1_limit_milli: 0,
        build_profile: snapshot.build_profile.clone(),
    })
}

fn compare_static(profile: &RunnerProfile, actual: &RunnerSnapshot) -> Result<(), ProfileError> {
    let fields = [
        ("os", profile.os.as_str(), actual.os.as_str()),
        (
            "architecture",
            profile.architecture.as_str(),
            actual.architecture.as_str(),
        ),
        (
            "kernel-release",
            profile.kernel_release.as_str(),
            actual.kernel_release.as_str(),
        ),
        (
            "cpu-model",
            profile.cpu_model.as_str(),
            actual.cpu_model.as_str(),
        ),
        (
            "online-cpus",
            profile.online_cpus.as_str(),
            actual.online_cpus.as_str(),
        ),
        (
            "allowed-cpus",
            profile.allowed_cpus.as_str(),
            actual.allowed_cpus.as_str(),
        ),
        (
            "isolated-cpus",
            profile.isolated_cpus.as_str(),
            actual.isolated_cpus.as_str(),
        ),
        (
            "scaling-governor",
            profile.scaling_governor.as_str(),
            actual.scaling_governor.as_str(),
        ),
        ("turbo", profile.turbo.as_str(), actual.turbo.as_str()),
        (
            "build-profile",
            profile.build_profile.as_str(),
            actual.build_profile.as_str(),
        ),
    ];
    for (label, expected, observed) in fields {
        if expected != observed {
            return Err(ProfileError(format!(
                "runner {label} mismatch: expected {expected:?}, observed {observed:?}"
            )));
        }
    }
    if profile.swap_total_kib != actual.swap_total_kib
        || profile.mem_total_kib != actual.mem_total_kib
    {
        return Err(ProfileError(
            "runner memory or swap profile mismatch".into(),
        ));
    }
    Ok(())
}

fn probe_linux() -> Result<RunnerSnapshot, ProfileError> {
    if std::env::consts::OS != "linux" {
        return Err(ProfileError(
            "controlled performance capture requires Linux".into(),
        ));
    }
    let status = read_text("/proc/self/status", MAX_PROFILE_BYTES)?;
    let allowed_cpus = proc_field(&status, "Cpus_allowed_list")?;
    let online_cpus = read_trimmed("/sys/devices/system/cpu/online")?;
    let isolated_cpus = read_trimmed("/sys/devices/system/cpu/isolated")?;
    let governors = probe_governors(&allowed_cpus)?;
    let meminfo = read_text("/proc/meminfo", MAX_PROFILE_BYTES)?;
    Ok(RunnerSnapshot {
        os: "linux".into(),
        architecture: std::env::consts::ARCH.into(),
        kernel_release: read_trimmed("/proc/sys/kernel/osrelease")?,
        cpu_model: probe_cpu_model()?,
        online_cpus,
        allowed_cpus,
        isolated_cpus,
        scaling_governor: governors,
        turbo: probe_turbo()?,
        swap_total_kib: meminfo_kib(&meminfo, "SwapTotal")?,
        mem_total_kib: meminfo_kib(&meminfo, "MemTotal")?,
        load1_milli: parse_decimal_milli(
            read_trimmed("/proc/loadavg")?
                .split_whitespace()
                .next()
                .ok_or_else(|| ProfileError("missing load average".into()))?,
        )?,
        build_profile: if cfg!(debug_assertions) {
            "debug".into()
        } else {
            "release".into()
        },
    })
}

fn probe_cpu_model() -> Result<String, ProfileError> {
    let cpuinfo = read_text("/proc/cpuinfo", 4 * 1024 * 1024)?;
    let mut models = BTreeSet::new();
    for line in cpuinfo.lines() {
        if let Some((_, value)) = line.split_once(':') {
            if line.starts_with("model name") || line.starts_with("Hardware") {
                models.insert(normalize_space(value));
            }
        }
    }
    if models.is_empty() {
        return Err(ProfileError("CPU model is unavailable".into()));
    }
    Ok(models.into_iter().collect::<Vec<_>>().join(" | "))
}

fn probe_governors(cpu_list: &str) -> Result<String, ProfileError> {
    let mut governors = BTreeSet::new();
    for cpu in parse_cpu_list(cpu_list)? {
        let path = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor");
        if !Path::new(&path).exists() {
            return Ok("unavailable".into());
        }
        governors.insert(read_trimmed(&path)?);
    }
    if governors.is_empty() {
        return Ok("unavailable".into());
    }
    Ok(governors.into_iter().collect::<Vec<_>>().join(","))
}

fn probe_turbo() -> Result<String, ProfileError> {
    let probes = [
        ("/sys/devices/system/cpu/intel_pstate/no_turbo", "1"),
        ("/sys/devices/system/cpu/cpufreq/boost", "0"),
    ];
    let mut found = false;
    for (path, disabled_value) in probes {
        if Path::new(path).exists() {
            found = true;
            if read_trimmed(path)? != disabled_value {
                return Ok("enabled".into());
            }
        }
    }
    if !found {
        return Ok("unavailable".into());
    }
    Ok("disabled".into())
}

fn parse_cpu_list(value: &str) -> Result<BTreeSet<u32>, ProfileError> {
    let mut cpus = BTreeSet::new();
    if value.is_empty() {
        return Ok(cpus);
    }
    for part in value.split(',') {
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (parse_cpu(start)?, parse_cpu(end)?),
            None => {
                let cpu = parse_cpu(part)?;
                (cpu, cpu)
            }
        };
        if start > end || end > 65_535 {
            return Err(ProfileError("invalid CPU list range".into()));
        }
        cpus.extend(start..=end);
    }
    Ok(cpus)
}

fn parse_cpu(value: &str) -> Result<u32, ProfileError> {
    value
        .parse()
        .map_err(|_| ProfileError("invalid CPU list".into()))
}

fn read_trimmed(path: &str) -> Result<String, ProfileError> {
    let text = read_text(path, MAX_PROFILE_BYTES)?;
    Ok(normalize_space(text.trim()))
}

fn read_text(path: &str, maximum: usize) -> Result<String, ProfileError> {
    let bytes = bounded_io::read(Path::new(path), maximum)
        .map_err(|error| ProfileError(error.to_string()))?;
    String::from_utf8(bytes).map_err(|_| ProfileError(format!("{path} is not UTF-8")))
}

fn proc_field(text: &str, key: &str) -> Result<String, ProfileError> {
    let prefix = format!("{key}:");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| normalize_space(value.trim()))
        .ok_or_else(|| ProfileError(format!("missing {key} in proc status")))
}

fn meminfo_kib(text: &str, key: &str) -> Result<u64, ProfileError> {
    let value = proc_field(text, key)?;
    let mut fields = value.split_whitespace();
    let kib = fields
        .next()
        .ok_or_else(|| ProfileError(format!("missing {key} value")))?
        .parse()
        .map_err(|_| ProfileError(format!("invalid {key} value")))?;
    if fields.next() != Some("kB") || fields.next().is_some() {
        return Err(ProfileError(format!("invalid {key} unit")));
    }
    Ok(kib)
}

fn parse_decimal_milli(value: &str) -> Result<u64, ProfileError> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ProfileError("invalid load average".into()));
    }
    let whole: u64 = whole
        .parse()
        .map_err(|_| ProfileError("invalid load average".into()))?;
    let mut padded = fraction.to_owned();
    while padded.len() < 3 {
        padded.push('0');
    }
    let fractional = if padded.is_empty() {
        0
    } else {
        padded
            .parse::<u64>()
            .map_err(|_| ProfileError("invalid load average".into()))?
    };
    whole
        .checked_mul(1_000)
        .and_then(|value| value.checked_add(fractional))
        .ok_or_else(|| ProfileError("load average overflow".into()))
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_text(label: &str, value: &str, id: bool) -> Result<(), ProfileError> {
    let valid = !value.is_empty()
        && value.len() <= 512
        && !value.contains(['\t', '\n', '\r'])
        && (!id
            || value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            }));
    if !valid {
        return Err(ProfileError(format!("invalid {label}")));
    }
    Ok(())
}

fn field<'a>(lines: &'a [&str], index: usize, key: &str) -> Result<&'a str, ProfileError> {
    let mut parts = lines[index].split('\t');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(actual), Some(value), None) if actual == key => Ok(value),
        _ => Err(ProfileError(format!("invalid {key} field"))),
    }
}

fn parse_field<T: std::str::FromStr>(
    lines: &[&str],
    index: usize,
    key: &str,
) -> Result<T, ProfileError> {
    field(lines, index, key)?
        .parse()
        .map_err(|_| ProfileError(format!("invalid {key}")))
}
