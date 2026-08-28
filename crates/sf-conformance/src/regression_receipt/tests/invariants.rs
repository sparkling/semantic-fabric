use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::super::format;
use super::super::model::{InputBinding, Surface};
use super::super::runner::{ObservedTest, RunFailure, SuiteRunner};
use super::super::{run_at_root, validate_w3c_inventory, Mode, ProfileSpec, W3cInventorySpec};
use super::{passing_runner, profile_with_tests, required_test, FakeRunner};

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        loop {
            let serial = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "semantic-fabric-baseline-{label}-{}-{serial}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create isolated scratch directory: {error}"),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct MutatingRunner {
    inner: FakeRunner,
    input: PathBuf,
    executions: RefCell<usize>,
}

impl SuiteRunner for MutatingRunner {
    fn discover(
        &self,
        root: &Path,
        suite: &super::super::model::Suite,
    ) -> Result<Vec<String>, RunFailure> {
        self.inner.discover(root, suite)
    }

    fn execute(
        &self,
        root: &Path,
        suite: &super::super::model::Suite,
        excluded: &[String],
    ) -> Result<Vec<ObservedTest>, RunFailure> {
        fs::write(&self.input, b"mutated").expect("mutate bound input");
        *self.executions.borrow_mut() += 1;
        self.inner.execute(root, suite, excluded)
    }
}

#[test]
fn generate_then_check_replays_exact_rows_without_mutating_baseline() {
    let scratch = Scratch::new("round-trip");
    let spec = write_minimal_profile(scratch.path());
    let runner = passing_runner();

    let generated = run_at_root(spec, Mode::Generate, &runner, scratch.path()).unwrap();
    let baseline_path = scratch.path().join(spec.baseline_relative);
    let before = fs::read(&baseline_path).unwrap();
    let checked = run_at_root(spec, Mode::Check, &runner, scratch.path()).unwrap();

    assert_eq!(
        (checked, fs::read(baseline_path).unwrap()),
        (generated, before)
    );
}

#[test]
fn generation_rejects_bound_input_mutated_by_runner() {
    let scratch = Scratch::new("input-mutation");
    let spec = write_minimal_profile(scratch.path());
    let runner = MutatingRunner {
        inner: passing_runner(),
        input: scratch.path().join("input.txt"),
        executions: RefCell::new(0),
    };

    let error = run_at_root(spec, Mode::Generate, &runner, scratch.path()).unwrap_err();

    assert!(
        error.contains("bound input changed during execution"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn fixed_read_rejects_symlinked_parent_even_when_target_stays_inside_root() {
    use std::os::unix::fs::symlink;

    let scratch = Scratch::new("symlink-parent");
    let actual = scratch.path().join("actual");
    fs::create_dir(&actual).unwrap();
    let spec = write_minimal_profile(&actual);
    symlink(&actual, scratch.path().join("linked")).unwrap();
    let linked_spec = ProfileSpec {
        inventory_relative: "linked/inventory.tsv",
        baseline_relative: "linked/baselines/baseline.tsv",
        ..spec
    };

    let error = run_at_root(
        linked_spec,
        Mode::Generate,
        &passing_runner(),
        scratch.path(),
    )
    .unwrap_err();

    assert!(error.contains("symlink component"), "{error}");
}

#[cfg(unix)]
#[test]
fn authority_read_rejects_symlink_and_hard_link_files() {
    use std::os::unix::fs::symlink;

    let scratch = Scratch::new("authority-links");
    let target = scratch.path().join("target.tsv");
    let symlink_path = scratch.path().join("symlink.tsv");
    let hard_link = scratch.path().join("hardlink.tsv");
    fs::write(&target, b"authority\n").unwrap();
    symlink(&target, &symlink_path).unwrap();

    let symlink_error = super::super::read_authority(&symlink_path, 64).unwrap_err();
    fs::hard_link(&target, &hard_link).unwrap();
    let hard_link_error = super::super::read_authority(&target, 64).unwrap_err();

    assert!(symlink_error.contains("non-symlink"), "{symlink_error}");
    assert!(hard_link_error.contains("hard link"), "{hard_link_error}");
}

#[cfg(unix)]
#[test]
fn atomic_generation_rejects_symlink_and_hard_link_targets() {
    use std::os::unix::fs::symlink;

    let scratch = Scratch::new("atomic-targets");
    let referent = scratch.path().join("referent.tsv");
    fs::write(&referent, b"old\n").unwrap();
    symlink(&referent, scratch.path().join("symlink.tsv")).unwrap();
    fs::hard_link(&referent, scratch.path().join("hardlink.tsv")).unwrap();

    assert_eq!(
        (
            super::super::atomic::replace_fixed(
                scratch.path(),
                "symlink.tsv",
                "symlink.tsv",
                b"new\n",
            ),
            super::super::atomic::replace_fixed(
                scratch.path(),
                "hardlink.tsv",
                "hardlink.tsv",
                b"new\n",
            ),
        ),
        (
            Err("fixed path contains symlink component ".to_owned()
                + &scratch.path().join("symlink.tsv").display().to_string()),
            Err("baseline target is a hard link".to_owned()),
        )
    );
}

#[test]
fn query_w3c_binding_rejects_deleted_case_tree_file() {
    let scratch = Scratch::new("w3c-deletion");
    let suite = scratch.path().join("tests/w3c/rdb2rdf");
    copy_tree(&source_w3c_suite(), &suite);
    let spec = ProfileSpec {
        profile_id: "w3c-test",
        surface: Surface::SparqlQuery,
        inventory_relative: "unused.tsv",
        baseline_relative: "unused-baseline.tsv",
        baseline_name: "unused-baseline.tsv",
        w3c: Some(W3cInventorySpec {
            suite_relative: "tests/w3c/rdb2rdf",
            inventory_relative: "tests/w3c/rdb2rdf/inventory.tsv",
        }),
    };
    validate_w3c_inventory(scratch.path(), spec).expect("copied inventory is valid");
    fs::remove_file(suite.join("cases/D000-1table1column0rows/create.sql")).unwrap();

    let error = validate_w3c_inventory(scratch.path(), spec).unwrap_err();

    assert!(error.contains("case-tree files differ"), "{error}");
}

#[cfg(target_os = "linux")]
#[test]
fn timeout_terminates_descendant_in_isolated_process_group() {
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};

    let scratch = Scratch::new("process-group");
    let pid_file = scratch.path().join("descendant.pid");
    let mut child = std::process::Command::new("sh");
    child
        .arg("-c")
        .arg("sleep 30 & echo $! > \"$1\"; wait")
        .arg("sh")
        .arg(&pid_file)
        .process_group(0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = child.spawn().expect("spawn isolated process group");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !pid_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let descendant: u32 = fs::read_to_string(&pid_file)
        .expect("shell wrote descendant PID")
        .trim()
        .parse()
        .unwrap();

    let error = super::super::runner::wait_with_timeout(&mut child, 0).unwrap_err();
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_is_running(descendant) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        error.message.contains("process group terminated")
            && child.try_wait().unwrap().is_some()
            && !process_is_running(descendant)
    );
}

#[cfg(target_os = "linux")]
fn process_is_running(pid: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.chars().next())
        .is_some_and(|state| state != 'Z')
}

fn write_minimal_profile(root: &Path) -> ProfileSpec {
    fs::create_dir(root.join("baselines")).unwrap();
    fs::write(root.join("input.txt"), b"source").unwrap();
    let mut profile = profile_with_tests(vec![
        required_test("required_case"),
        super::excluded_test("resource_case"),
    ]);
    profile.inputs = vec![InputBinding {
        path: "input.txt".to_owned(),
        byte_length: 6,
        sha256: format::sha256(b"source"),
    }];
    fs::write(root.join("inventory.tsv"), format::render_profile(&profile)).unwrap();
    ProfileSpec {
        profile_id: "test-profile",
        surface: Surface::SparqlQuery,
        inventory_relative: "inventory.tsv",
        baseline_relative: "baselines/baseline.tsv",
        baseline_name: "baseline.tsv",
        w3c: None,
    }
}

fn source_w3c_suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    let mut entries: Vec<_> = fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type().unwrap();
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination);
        } else if kind.is_file() {
            fs::copy(entry.path(), destination).unwrap();
        } else {
            panic!("unexpected source tree entry {}", entry.path().display());
        }
    }
}
