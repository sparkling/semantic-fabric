//! Exact bubblewrap boundary for the current `sf-cli` artifact observation.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{authority, cargo, process};

const LINKER: &str = "/usr/bin/x86_64-linux-gnu-gcc-13";

const SYSTEM_ROOTS: &[(&str, bool)] = &[
    ("/usr/bin", true),
    ("/usr/lib", true),
    ("/usr/libexec", false),
    ("/usr/include", false),
    ("/usr/share", false),
    ("/etc/alternatives", false),
    ("/usr/lib64", false),
];

#[derive(Clone, Copy)]
pub(super) struct Request<'a> {
    pub(super) bwrap: &'a Path,
    pub(super) source: &'a Path,
    pub(super) toolchain: &'a Path,
    pub(super) cargo_registry: &'a Path,
    pub(super) target: &'a Path,
    pub(super) source_date_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    pub(super) executable: PathBuf,
    pub(super) argv: Vec<OsString>,
    pub(super) bwrap_version: String,
    pub(super) bwrap_sha256: String,
    pub(super) bwrap_byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    source: PathBuf,
    destination: &'static str,
}

pub(super) fn plan(request: &Request<'_>) -> Result<Plan, String> {
    validate_request(request)?;
    let executable = cargo::identify(request.bwrap, "bubblewrap executable")?;
    Ok(Plan {
        executable: request.bwrap.to_path_buf(),
        argv: arguments(request, &system_mounts()?),
        bwrap_version: executable.version,
        bwrap_sha256: executable.sha256,
        bwrap_byte_length: executable.byte_length,
    })
}

pub(super) fn execute(request: &Request<'_>) -> Result<(process::Output, Plan), String> {
    let plan = plan(request)?;
    let before = (plan.bwrap_sha256.clone(), plan.bwrap_byte_length);
    let mut command = Command::new(&plan.executable);
    command.current_dir("/").env_clear().args(&plan.argv);
    let result = process::run(
        command,
        "controlled bubblewrap cargo build",
        64 * 1024 * 1024,
        16 * 1024 * 1024,
        Duration::from_secs(20 * 60),
    );
    let after = executable_identity(request.bwrap, "bubblewrap executable")?;
    if after != before {
        return Err("bubblewrap executable changed during execution".to_owned());
    }
    Ok((result?, plan))
}

fn validate_request(request: &Request<'_>) -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    return Err("bubblewrap artifact capture requires Linux".to_owned());

    let bwrap = canonical_file(request.bwrap, "bubblewrap executable")?;
    if request.source_date_epoch == 0 || request.source_date_epoch > i64::MAX as u64 {
        return Err("SOURCE_DATE_EPOCH is outside supported Unix timestamp bounds".to_owned());
    }
    let source = canonical_directory(request.source, "source repository")?;
    let toolchain = canonical_directory(request.toolchain, "toolchain")?;
    let registry = canonical_directory(request.cargo_registry, "Cargo registry")?;
    let target = canonical_directory(request.target, "fresh target directory")?;
    if fs::read_dir(&target)
        .map_err(|error| format!("read fresh target directory: {error}"))?
        .next()
        .transpose()
        .map_err(|error| format!("enumerate fresh target directory: {error}"))?
        .is_some()
    {
        return Err("fresh target directory is not empty".to_owned());
    }
    let roots = [&source, &toolchain, &registry, &target];
    for (index, left) in roots.iter().enumerate() {
        for right in &roots[index + 1..] {
            if overlaps(left, right) {
                return Err("sandbox mount roots overlap".to_owned());
            }
        }
    }
    if roots.iter().any(|root| bwrap.starts_with(root)) {
        return Err("bubblewrap executable overlaps a sandbox mount root".to_owned());
    }
    for (path, label) in [
        (toolchain.join("bin/cargo"), "controlled Cargo executable"),
        (toolchain.join("bin/rustc"), "controlled rustc executable"),
        (PathBuf::from(LINKER), "controlled linker executable"),
    ] {
        canonical_file(&path, label)?;
    }
    Ok(())
}

fn arguments(request: &Request<'_>, system: &[Mount]) -> Vec<OsString> {
    let mounts = [
        Mount {
            source: request.source.to_path_buf(),
            destination: "/workspace",
        },
        Mount {
            source: request.toolchain.to_path_buf(),
            destination: "/toolchain",
        },
        Mount {
            source: request.cargo_registry.to_path_buf(),
            destination: "/cargo-home/registry",
        },
    ];
    let mut destinations: Vec<&str> = mounts
        .iter()
        .map(|mount| mount.destination)
        .chain(system.iter().map(|mount| mount.destination))
        .chain(["/target", "/proc", "/dev", "/tmp", "/home/harness"])
        .collect();
    destinations.sort_unstable();
    destinations.dedup();

    let mut argv = strings(&[
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--unshare-net",
        "--clearenv",
        "--tmpfs",
        "/",
        "--cap-drop",
        "ALL",
    ]);
    for directory in parent_directories(&destinations) {
        push_pair(&mut argv, "--dir", directory);
    }
    argv.extend(strings(&[
        "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp",
    ]));
    for mount in &mounts {
        push_mount(&mut argv, "--ro-bind", mount);
    }
    argv.push("--bind".into());
    argv.push(request.target.as_os_str().to_owned());
    argv.push("/target".into());
    for mount in system {
        push_mount(&mut argv, "--ro-bind", mount);
    }
    for (name, value) in [
        ("CARGO_HOME", "/cargo-home".to_owned()),
        ("CARGO_INCREMENTAL", "0".to_owned()),
        ("CARGO_NET_OFFLINE", "true".to_owned()),
        ("HOME", "/home/harness".to_owned()),
        ("LC_ALL", "C".to_owned()),
        ("PATH", "/toolchain/bin:/usr/bin".to_owned()),
        ("RUSTC", "/toolchain/bin/rustc".to_owned()),
        ("RUSTUP_HOME", "/toolchain".to_owned()),
        ("SOURCE_DATE_EPOCH", request.source_date_epoch.to_string()),
        ("TMPDIR", "/tmp".to_owned()),
        ("TZ", "UTC".to_owned()),
    ] {
        argv.extend(strings(&["--setenv", name]));
        argv.push(value.into());
    }
    argv.extend(strings(&[
        "--chdir",
        "/workspace",
        "--",
        "/toolchain/bin/cargo",
        "rustc",
        "--locked",
        "--offline",
        "--release",
        "-p",
        "sf-cli",
        "--bin",
        "semantic-fabric",
        "--target",
        "x86_64-unknown-linux-gnu",
        "--target-dir",
        "/target",
        "--message-format=json-render-diagnostics",
        "--",
        "-C",
        "linker=/usr/bin/x86_64-linux-gnu-gcc-13",
        "-C",
        "link-arg=-Wl,--dependency-file=/target/final-link.d",
    ]));
    argv
}

fn system_mounts() -> Result<Vec<Mount>, String> {
    let mut mounts = Vec::new();
    for (value, required) in SYSTEM_ROOTS {
        let path = Path::new(value);
        if !path.exists() {
            if *required {
                return Err(format!("required sandbox system root is missing: {value}"));
            }
            continue;
        }
        let source = canonical_directory(path, "sandbox system root")?;
        mounts.push(Mount {
            source,
            destination: value,
        });
    }
    for (logical, backing) in [
        ("/bin", "/usr/bin"),
        ("/lib", "/usr/lib"),
        ("/lib64", "/usr/lib64"),
    ] {
        let logical = Path::new(logical);
        if !logical.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(logical).map_err(|error| {
            format!("inspect merged-usr mapping {}: {error}", logical.display())
        })?;
        if !metadata.file_type().is_symlink() {
            continue;
        }
        let canonical = fs::canonicalize(logical).map_err(|error| {
            format!("resolve merged-usr mapping {}: {error}", logical.display())
        })?;
        if canonical != Path::new(backing) {
            return Err(format!(
                "unexpected merged-usr mapping for {}",
                logical.display()
            ));
        }
        let source = mounts
            .iter()
            .find(|mount| mount.destination == backing)
            .ok_or_else(|| format!("merged-usr backing root is unavailable: {backing}"))?
            .source
            .clone();
        mounts.push(Mount {
            source,
            destination: logical.to_str().expect("fixed logical path"),
        });
    }
    Ok(mounts)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_absolute_normal(path, label)?;
    authority::validate_directory(path, label)?;
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
    if canonical != path {
        return Err(format!("{label} is not canonical"));
    }
    Ok(canonical)
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_absolute_normal(path, label)?;
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
    if canonical != path {
        return Err(format!("{label} is not canonical"));
    }
    let _ = executable_identity(path, label)?;
    Ok(canonical)
}

fn executable_identity(path: &Path, label: &str) -> Result<(String, u64), String> {
    let (sha256, byte_length) = authority::digest(path, 2 * 1024 * 1024 * 1024, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path)
            .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
        if metadata.mode() & 0o111 == 0 {
            return Err(format!("{label} is not executable"));
        }
    }
    Ok((sha256, byte_length))
}

fn validate_absolute_normal(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(format!("{label} path must be absolute and normalized"));
    }
    Ok(())
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn parent_directories<'a>(destinations: &'a [&'a str]) -> Vec<&'a str> {
    let mut directories = BTreeSet::new();
    for destination in destinations {
        let mut current = Path::new(destination);
        while current != Path::new("/") {
            directories.insert(current.to_str().expect("fixed sandbox destination"));
            current = current.parent().expect("absolute destination has a parent");
        }
    }
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort_by_key(|path| (Path::new(path).components().count(), *path));
    directories
}

fn push_mount(argv: &mut Vec<OsString>, flag: &str, mount: &Mount) {
    argv.push(flag.into());
    argv.push(mount.source.as_os_str().to_owned());
    argv.push(mount.destination.into());
}

fn push_pair(argv: &mut Vec<OsString>, flag: &str, value: &str) {
    argv.extend(strings(&[flag, value]));
}

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(argv: &[OsString]) -> Vec<String> {
        argv.iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn argv_has_exact_isolation_environment_and_cargo_law() {
        let request = Request {
            bwrap: Path::new("/host/bwrap"),
            source: Path::new("/host/source"),
            toolchain: Path::new("/host/toolchain"),
            cargo_registry: Path::new("/host/registry"),
            target: Path::new("/host/target"),
            source_date_epoch: 1_700_000_000,
        };
        let system = [
            Mount {
                source: "/usr/bin".into(),
                destination: "/usr/bin",
            },
            Mount {
                source: "/usr/lib".into(),
                destination: "/usr/lib",
            },
            Mount {
                source: "/usr/bin".into(),
                destination: "/bin",
            },
            Mount {
                source: "/usr/lib".into(),
                destination: "/lib",
            },
        ];
        let argv = text(&arguments(&request, &system));
        assert_eq!(argv[..9].join("\0"),
            "--die-with-parent\0--new-session\0--unshare-all\0--unshare-net\0--clearenv\0--tmpfs\0/\0--cap-drop\0ALL");
        assert_eq!(argv[argv.len() - 22..].join("\0"),
            "--chdir\0/workspace\0--\0/toolchain/bin/cargo\0rustc\0--locked\0--offline\0--release\0-p\0sf-cli\0--bin\0semantic-fabric\0--target\0x86_64-unknown-linux-gnu\0--target-dir\0/target\0--message-format=json-render-diagnostics\0--\0-C\0linker=/usr/bin/x86_64-linux-gnu-gcc-13\0-C\0link-arg=-Wl,--dependency-file=/target/final-link.d");
        let environment = argv
            .windows(3)
            .filter(|window| window[0] == "--setenv")
            .map(|window| format!("{}={}", window[1], window[2]))
            .collect::<Vec<_>>()
            .join("|");
        assert_eq!(environment, "CARGO_HOME=/cargo-home|CARGO_INCREMENTAL=0|CARGO_NET_OFFLINE=true|HOME=/home/harness|LC_ALL=C|PATH=/toolchain/bin:/usr/bin|RUSTC=/toolchain/bin/rustc|RUSTUP_HOME=/toolchain|SOURCE_DATE_EPOCH=1700000000|TMPDIR=/tmp|TZ=UTC");
    }

    #[test]
    fn mounts_are_allowlisted_and_never_broad() {
        let request = Request {
            bwrap: Path::new("/host/bwrap"),
            source: Path::new("/host/source"),
            toolchain: Path::new("/host/toolchain"),
            cargo_registry: Path::new("/host/registry"),
            target: Path::new("/host/target"),
            source_date_epoch: 1,
        };
        let system = [
            ("/usr/bin", "/usr/bin"),
            ("/usr/lib", "/usr/lib"),
            ("/usr/libexec", "/usr/libexec"),
            ("/usr/include", "/usr/include"),
            ("/usr/share", "/usr/share"),
            ("/etc/alternatives", "/etc/alternatives"),
            ("/usr/lib64", "/usr/lib64"),
        ]
        .map(|(source, destination)| Mount {
            source: source.into(),
            destination,
        });
        let argv = text(&arguments(&request, &system));
        let mounts: Vec<_> = argv
            .windows(3)
            .filter(|window| window[0] == "--ro-bind" || window[0] == "--bind")
            .map(|window| window.join("="))
            .collect();
        let joined = mounts.join("|");
        assert_eq!(joined, "--ro-bind=/host/source=/workspace|--ro-bind=/host/toolchain=/toolchain|--ro-bind=/host/registry=/cargo-home/registry|--bind=/host/target=/target|--ro-bind=/usr/bin=/usr/bin|--ro-bind=/usr/lib=/usr/lib|--ro-bind=/usr/libexec=/usr/libexec|--ro-bind=/usr/include=/usr/include|--ro-bind=/usr/share=/usr/share|--ro-bind=/etc/alternatives=/etc/alternatives|--ro-bind=/usr/lib64=/usr/lib64");
        assert!(!["=/=", "=/home=", "=/etc="]
            .iter()
            .any(|value| joined.contains(value)));
        assert!(!argv
            .iter()
            .any(|value| value.contains("strace") || value.contains("ldd")));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_relative_symlink_writable_overlap_and_nonempty_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = std::env::temp_dir().join(format!(
            "semantic-fabric-artifact-sandbox-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        for directory in ["source", "toolchain", "toolchain/bin", "registry", "target"] {
            let path = root.join(directory);
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for executable in ["bwrap", "toolchain/bin/cargo", "toolchain/bin/rustc"] {
            let path = root.join(executable);
            fs::write(&path, b"fixture").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let request = Request {
            bwrap: &root.join("bwrap"),
            source: &root.join("source"),
            toolchain: &root.join("toolchain"),
            cargo_registry: &root.join("registry"),
            target: &root.join("target"),
            source_date_epoch: 1,
        };
        validate_request(&request).unwrap();
        let relative = Request {
            bwrap: Path::new("bwrap"),
            ..request
        };
        assert!(validate_request(&relative)
            .unwrap_err()
            .contains("absolute"));
        let alias = root.join("bwrap-link");
        symlink(request.bwrap, &alias).unwrap();
        let linked = Request {
            bwrap: &alias,
            ..request
        };
        assert!(validate_request(&linked).unwrap_err().contains("canonical"));
        fs::set_permissions(request.source, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_request(&request).unwrap_err().contains("writable"));
        fs::set_permissions(request.source, fs::Permissions::from_mode(0o755)).unwrap();
        let nested = request.source.join("nested-target");
        fs::create_dir(&nested).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o755)).unwrap();
        let overlap = Request {
            target: &nested,
            ..request
        };
        assert!(validate_request(&overlap).unwrap_err().contains("overlap"));
        fs::write(request.target.join("occupied"), b"x").unwrap();
        assert!(validate_request(&request)
            .unwrap_err()
            .contains("not empty"));
        fs::remove_dir_all(root).unwrap();
    }
}
