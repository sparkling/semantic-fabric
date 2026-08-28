//! Exact Git-blob verification after filter-capable index materialization.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{authority, process};

const BATCH_SIZE: usize = 256;
const MAX_BATCH_OUTPUT: u64 = 128 * 1024;

pub(super) fn validate(
    git: &Path,
    repository: &Path,
    materialized_root: &Path,
    expected: &BTreeMap<String, String>,
    command_factory: fn(&Path, &Path) -> Command,
) -> Result<(), String> {
    if expected.is_empty() {
        return Err("Git blob authority is empty".to_owned());
    }
    let entries: Vec<_> = expected.iter().collect();
    for batch in entries.chunks(BATCH_SIZE) {
        let paths = checked_paths(materialized_root, batch)?;
        let mut command = command_factory(git, repository);
        command.args(["hash-object", "--no-filters", "--"]);
        command.args(&paths);
        let output = process::run(
            command,
            "bound Git blob verification",
            MAX_BATCH_OUTPUT,
            MAX_BATCH_OUTPUT,
            Duration::from_secs(30),
        )?;
        if !output.stderr.is_empty() {
            return Err("Git blob verification wrote stderr".to_owned());
        }
        let observed = parse_hashes(&output.stdout, batch.len())?;
        for ((path, expected_object), observed_object) in batch.iter().zip(observed) {
            if expected_object.as_str() != observed_object {
                return Err(format!(
                    "materialized source bytes differ from Git blob authority: {path}"
                ));
            }
        }
    }
    Ok(())
}

fn checked_paths(root: &Path, entries: &[(&String, &String)]) -> Result<Vec<PathBuf>, String> {
    entries
        .iter()
        .map(|(relative, _)| {
            let path = root.join(relative);
            authority::validate_beneath(root, &path, "materialized Git blob")?;
            Ok(path)
        })
        .collect()
}

fn parse_hashes(bytes: &[u8], count: usize) -> Result<Vec<&str>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("Git blob hashes are not UTF-8: {error}"))?;
    if !text.ends_with('\n') || text.contains('\r') {
        return Err("Git blob hashes are not canonical LF records".to_owned());
    }
    let values: Vec<_> = text.lines().collect();
    if values.len() != count
        || values.iter().any(|value| {
            !matches!(value.len(), 40 | 64)
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err("Git blob verification returned invalid object IDs".to_owned());
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_exact_canonical_object_records() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            parse_hashes(format!("{sha}\n").as_bytes(), 1).unwrap(),
            [sha]
        );
        assert!(parse_hashes(format!("{sha}\r\n").as_bytes(), 1).is_err());
        assert!(parse_hashes(format!("{sha}\n{sha}\n").as_bytes(), 1).is_err());
    }
}
