//! GNU/LLD dependency-file capture without executing an artifact or loader.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::authority;

const MAX_DEPFILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INPUTS: usize = 20_000;
const MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DependencyFile {
    pub(super) sha256: String,
    pub(super) byte_length: u64,
    pub(super) inputs: Vec<Input>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Input {
    pub(super) logical_path: PathBuf,
    pub(super) sha256: String,
    pub(super) byte_length: u64,
}

/// Maps a linker's sandbox-visible path to a non-symlink host backing path.
#[derive(Debug, Clone, Copy)]
pub(super) struct Root<'a> {
    pub(super) logical: &'a Path,
    pub(super) backing: &'a Path,
}

/// Reads the dependency file emitted by the final GNU-style linker invocation.
/// Every recorded input must be a non-writable, non-linked regular file below a
/// caller-supplied trusted root. A missing or malformed depfile is an error.
pub(super) fn capture(depfile: &Path, roots: &[Root<'_>]) -> Result<DependencyFile, String> {
    if roots.is_empty() {
        return Err("link dependency capture has no trusted roots".to_owned());
    }
    let authority = authority::read(depfile, MAX_DEPFILE_BYTES, "link dependency file")?;
    let paths = parse(&authority.bytes)?;
    if paths.is_empty() || paths.len() > MAX_INPUTS {
        return Err(format!(
            "link dependency input count is outside bounds: {}",
            paths.len()
        ));
    }
    let mut seen = BTreeSet::new();
    let mut inputs = Vec::with_capacity(paths.len());
    for logical_path in paths {
        if !logical_path.is_absolute() {
            return Err("link dependency path is not absolute".to_owned());
        }
        if !seen.insert(logical_path.clone()) {
            return Err(format!(
                "duplicate link dependency path {}",
                logical_path.display()
            ));
        }
        let root = roots
            .iter()
            .find(|root| logical_path.starts_with(root.logical))
            .ok_or_else(|| {
                format!(
                    "link dependency path escapes trusted roots: {}",
                    logical_path.display()
                )
            })?;
        let relative = logical_path
            .strip_prefix(root.logical)
            .map_err(|_| "link dependency root mapping changed".to_owned())?;
        let backing = root.backing.join(relative);
        authority::validate_beneath(root.backing, &backing, "link dependency")?;
        let (sha256, byte_length) =
            authority::digest(&backing, MAX_INPUT_BYTES, "link dependency")?;
        inputs.push(Input {
            logical_path,
            sha256,
            byte_length,
        });
    }
    Ok(DependencyFile {
        sha256: authority.sha256,
        byte_length: authority.size,
        inputs,
    })
}

fn parse(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    let colon = first_unescaped_colon(bytes)?;
    let target = words(&bytes[..colon])?;
    if target.len() != 1 {
        return Err("link dependency file must contain exactly one output target".to_owned());
    }
    words(&bytes[colon + 1..]).and_then(|values| {
        values
            .into_iter()
            .map(|value| {
                if value.is_empty()
                    || value.len() > 16 * 1024
                    || value.bytes().any(|byte| byte.is_ascii_control())
                {
                    Err("invalid link dependency path".to_owned())
                } else {
                    Ok(PathBuf::from(value))
                }
            })
            .collect()
    })
}

fn first_unescaped_colon(bytes: &[u8]) -> Result<usize, String> {
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b':' {
            return Ok(index);
        }
    }
    Err("link dependency file has no rule separator".to_owned())
}

fn words(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    let mut current = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                if !current.is_empty() {
                    values.push(finish(&mut current)?);
                }
                index += 1;
            }
            b'\\' => {
                let Some(next) = bytes.get(index + 1).copied() else {
                    return Err("link dependency file ends with an escape".to_owned());
                };
                if next == b'\n' {
                    index += 2;
                } else if next == b'\r' && bytes.get(index + 2) == Some(&b'\n') {
                    index += 3;
                } else {
                    current.push(next);
                    index += 2;
                }
            }
            b'#' => return Err("link dependency file contains an unsupported comment".to_owned()),
            byte => {
                current.push(byte);
                index += 1;
            }
        }
    }
    if !current.is_empty() {
        values.push(finish(&mut current)?);
    }
    Ok(values)
}

fn finish(bytes: &mut Vec<u8>) -> Result<String, String> {
    let value = String::from_utf8(std::mem::take(bytes))
        .map_err(|error| format!("link dependency path is not UTF-8: {error}"))?;
    if value.is_empty() {
        Err("empty link dependency path".to_owned())
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnu_style_continuations_and_escapes() {
        let values = parse(b"/tmp/out: /tmp/first \\\n /tmp/with\\ space\n").unwrap();
        assert_eq!(
            values,
            [
                PathBuf::from("/tmp/first"),
                PathBuf::from("/tmp/with space")
            ]
        );
    }

    #[test]
    fn rejects_ambiguous_or_malformed_rules() {
        assert!(parse(b"/tmp/out /tmp/input\n").is_err());
        assert!(parse(b"/tmp/a /tmp/b: /tmp/input\n").is_err());
        assert!(parse(b"/tmp/out: /tmp/input # comment\n").is_err());
    }
}
