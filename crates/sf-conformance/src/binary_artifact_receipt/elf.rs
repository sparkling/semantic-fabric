//! ELF inspection through one bound `readelf` implementation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{authority, process};

const MAX_BINARY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TOOL_OUTPUT: u64 = 512 * 1024;
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadelfIdentity {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) size: u64,
    pub(super) version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Observation {
    pub(super) artifact_sha256: String,
    pub(super) artifact_size: u64,
    pub(super) elf_class: String,
    pub(super) data: String,
    pub(super) file_type: String,
    pub(super) machine: String,
    pub(super) interpreter: String,
    pub(super) needed: Vec<String>,
    pub(super) build_id: String,
    pub(super) comment: Vec<String>,
    pub(super) header_sha256: String,
    pub(super) dynamic_sha256: String,
    pub(super) notes_sha256: String,
    pub(super) program_headers_sha256: String,
}

pub(super) fn inspect(
    binary_path: &Path,
    readelf: &Path,
) -> Result<(ReadelfIdentity, Observation), String> {
    if !readelf.is_absolute() {
        return Err("readelf path must be absolute".to_owned());
    }
    let artifact_argument = binary_path
        .to_str()
        .ok_or_else(|| "artifact binary path is not UTF-8".to_owned())?;
    let (artifact_sha256, artifact_size) =
        authority::digest(binary_path, MAX_BINARY_BYTES, "artifact binary")?;
    let (tool_sha256, tool_size) = authority::digest(readelf, MAX_BINARY_BYTES, "readelf tool")?;
    let version = utf8(&invoke(readelf, &["--version"])?, "readelf version")?;
    let headers = utf8(
        &invoke(readelf, &["-h", "-W", artifact_argument])?,
        "ELF header",
    )?;
    let program = utf8(
        &invoke(readelf, &["-l", "-W", artifact_argument])?,
        "ELF program headers",
    )?;
    let dynamic = utf8(
        &invoke(readelf, &["-d", "-W", artifact_argument])?,
        "ELF dynamic section",
    )?;
    let notes = utf8(
        &invoke(readelf, &["-n", "-W", artifact_argument])?,
        "ELF notes",
    )?;
    let comment = utf8(
        &invoke(readelf, &["-p", ".comment", artifact_argument])?,
        "ELF comment",
    )?;
    reject_runtime_search_paths(&dynamic)?;
    let (artifact_sha256_after, artifact_size_after) =
        authority::digest(binary_path, MAX_BINARY_BYTES, "artifact binary")?;
    let (tool_sha256_after, tool_size_after) =
        authority::digest(readelf, MAX_BINARY_BYTES, "readelf tool")?;
    if artifact_sha256 != artifact_sha256_after || artifact_size != artifact_size_after {
        return Err("artifact binary changed during ELF inspection".to_owned());
    }
    if tool_sha256 != tool_sha256_after || tool_size != tool_size_after {
        return Err("readelf tool changed during ELF inspection".to_owned());
    }
    let identity = ReadelfIdentity {
        path: readelf.to_path_buf(),
        sha256: tool_sha256,
        size: tool_size,
        version: first_line(&version, "readelf version")?,
    };
    let observation = Observation {
        artifact_sha256,
        artifact_size,
        elf_class: required_field(&headers, "Class:", "ELF class")?,
        data: required_field(&headers, "Data:", "ELF data encoding")?,
        file_type: required_field(&headers, "Type:", "ELF type")?,
        machine: required_field(&headers, "Machine:", "ELF machine")?,
        interpreter: program
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("[Requesting program interpreter: ")
            })
            .and_then(|value| value.strip_suffix(']'))
            .ok_or_else(|| "ELF program headers have no interpreter".to_owned())?
            .to_owned(),
        needed: needed(&dynamic)?,
        build_id: build_id(&notes)?,
        comment: comments(&comment)?,
        header_sha256: digest(&headers),
        dynamic_sha256: digest(&dynamic),
        notes_sha256: digest(&notes),
        program_headers_sha256: digest(&program),
    };
    Ok((identity, observation))
}

fn invoke(readelf: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(readelf);
    command.env_clear().env("LC_ALL", "C").args(arguments);
    let output = process::run(
        command,
        "readelf",
        MAX_TOOL_OUTPUT,
        MAX_TOOL_OUTPUT,
        TOOL_TIMEOUT,
    )?;
    if !output.stderr.is_empty() {
        return Err(format!(
            "readelf wrote stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

fn utf8(bytes: &[u8], label: &str) -> Result<String, String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| format!("{label} is not UTF-8: {error}"))
}

fn first_line(value: &str, label: &str) -> Result<String, String> {
    let line = value.lines().next().unwrap_or_default().trim();
    if line.is_empty() || line.len() > 4096 {
        Err(format!("invalid {label}"))
    } else {
        Ok(line.to_owned())
    }
}

fn required_field(input: &str, prefix: &str, label: &str) -> Result<String, String> {
    let values: Vec<_> = input
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix(prefix))
        .map(str::trim)
        .collect();
    match values.as_slice() {
        [value] if !value.is_empty() && value.len() <= 4096 => Ok((*value).to_owned()),
        _ => Err(format!("{label} has no unique value")),
    }
}

fn needed(input: &str) -> Result<Vec<String>, String> {
    let mut names = BTreeSet::new();
    for line in input.lines() {
        let Some(value) = line.split("Shared library: [").nth(1) else {
            continue;
        };
        let Some(name) = value.strip_suffix(']') else {
            return Err("malformed ELF DT_NEEDED record".to_owned());
        };
        if name.is_empty() || name.len() > 4096 || name.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err("invalid ELF DT_NEEDED library name".to_owned());
        }
        if !names.insert(name.to_owned()) {
            return Err("duplicate ELF DT_NEEDED library name".to_owned());
        }
    }
    if names.is_empty() {
        return Err("ELF dynamic section has no DT_NEEDED libraries".to_owned());
    }
    Ok(names.into_iter().collect())
}

fn reject_runtime_search_paths(input: &str) -> Result<(), String> {
    if input
        .lines()
        .any(|line| line.contains("(RPATH)") || line.contains("(RUNPATH)"))
    {
        Err("ELF dynamic section declares RPATH or RUNPATH".to_owned())
    } else {
        Ok(())
    }
}

fn build_id(input: &str) -> Result<String, String> {
    let values: Vec<_> = input
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Build ID: "))
        .collect();
    match values.as_slice() {
        [value]
            if (16..=128).contains(&value.len())
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            Ok((*value).to_owned())
        }
        _ => Err("ELF notes have no unique SHA-1 build ID".to_owned()),
    }
}

fn comments(input: &str) -> Result<Vec<String>, String> {
    let values: Vec<_> = input
        .lines()
        .filter_map(|line| line.split_once(']').map(|(_, value)| value.trim()))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    if values.is_empty()
        || values
            .iter()
            .any(|value| value.len() > 4096 || value.bytes().any(|byte| byte.is_ascii_control()))
    {
        return Err("ELF comment section is empty or invalid".to_owned());
    }
    Ok(values)
}

fn digest(input: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dynamic_libraries_in_canonical_order() {
        let input = " 0x1 (NEEDED) Shared library: [libz.so.1]\n 0x1 (NEEDED) Shared library: [liba.so.1]\n";
        assert_eq!(needed(input).unwrap(), ["liba.so.1", "libz.so.1"]);
    }

    #[test]
    fn rejects_duplicate_or_malformed_libraries() {
        assert!(needed("Shared library: [liba.so]\nShared library: [liba.so]\n").is_err());
        assert!(needed("Shared library: [liba.so\n").is_err());
    }

    #[test]
    fn accepts_unique_build_id() {
        let id = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(build_id(&format!("Build ID: {id}\n")).unwrap(), id);
        assert!(build_id("Build ID: not-a-digest\n").is_err());
    }

    #[test]
    fn rejects_rpath_and_runpath() {
        assert!(reject_runtime_search_paths("0x1 (RPATH) Library rpath: [/tmp]\n").is_err());
        assert!(reject_runtime_search_paths("0x1 (RUNPATH) Library runpath: [/tmp]\n").is_err());
    }
}
