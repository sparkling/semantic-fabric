use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::digest::sha256_file;
use super::model::{SourceBinding, SourceTree};
use super::paths::RepositoryLayout;
use super::subprocess::{canonical_program, require_success, BoundedCommand};
use super::workload_runner::workload_sha256;

const MAX_GIT_OUTPUT_BYTES: usize = 1_048_576;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError(pub String);

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SourceError {}

#[derive(Debug, Clone)]
pub struct InspectedSource {
    pub binding: SourceBinding,
    pub executable: PathBuf,
}

pub fn inspect_source(
    layout: &RepositoryLayout,
    manifest: &[u8],
) -> Result<InspectedSource, SourceError> {
    let commit_output = git(layout.root(), &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let commit = std::str::from_utf8(&commit_output)
        .map_err(|_| SourceError("git commit output is not UTF-8".into()))?
        .trim();
    if commit_output.iter().filter(|byte| **byte == b'\n').count() != 1 {
        return Err(SourceError("git commit output is not canonical".into()));
    }
    let status = git(
        layout.root(),
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let tree = if status.is_empty() {
        SourceTree::Clean
    } else {
        SourceTree::Dirty
    };
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| SourceError(format!("resolve current executable: {error}")))?;
    layout
        .validate_contained_file(&executable)
        .map_err(|error| SourceError(error.to_string()))?;
    let artifact_sha256 =
        sha256_file(&executable).map_err(|error| SourceError(error.to_string()))?;
    let workload_sha256 =
        workload_sha256(manifest).map_err(|error| SourceError(error.to_string()))?;
    let binding = SourceBinding::new(commit, tree, &artifact_sha256, &workload_sha256)
        .map_err(|error| SourceError(error.to_string()))?;
    Ok(InspectedSource {
        binding,
        executable,
    })
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, SourceError> {
    let program = canonical_program(Path::new("/usr/bin/git"))
        .map_err(|error| SourceError(error.to_string()))?;
    let output = BoundedCommand {
        program,
        args: args.iter().map(|value| (*value).to_owned()).collect(),
        current_dir: root.to_owned(),
        stdin: Vec::new(),
        timeout: GIT_TIMEOUT,
        maximum_output: MAX_GIT_OUTPUT_BYTES,
        observe_linux_identity: false,
    }
    .run()
    .map_err(|error| SourceError(error.to_string()))?;
    require_success(&output, "git source inspection")
        .map_err(|error| SourceError(error.to_string()))?;
    Ok(output.stdout)
}
