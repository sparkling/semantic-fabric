//! GNU/LLD dependency-file capture without executing a loader or artifact.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::authority;

const MAX_DEPFILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INPUTS: usize = 16_384;
const MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DependencyFile {
    pub(super) sha256: String,
    pub(super) byte_length: u64,
    /// Logical target emitted before the depfile colon.
    pub(super) logical_output: PathBuf,
    pub(super) inputs: Vec<Input>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Input {
    /// Preserved for producer conversion into a schema logical path and origin.
    pub(super) logical_path: PathBuf,
    pub(super) sha256: String,
    pub(super) byte_length: u64,
}

/// Maps a sandbox-visible path to a non-symlink host backing path.
#[derive(Debug, Clone, Copy)]
pub(super) struct Root<'a> {
    pub(super) logical: &'a Path,
    pub(super) backing: &'a Path,
}

/// Reads the final linker depfile and requires its target to be the exact
/// sandbox-logical binary. Every dependency maps through one unambiguous,
/// longest trusted root to a hardened host backing file.
pub(super) fn capture(
    depfile: &Path,
    expected_output: &Path,
    roots: &[Root<'_>],
) -> Result<DependencyFile, String> {
    validate_expected_output(expected_output)?;
    validate_roots(roots)?;
    let authority = authority::read(depfile, MAX_DEPFILE_BYTES, "link dependency file")?;
    let rule = parse(&authority.bytes)?;
    if rule.output != expected_output {
        return Err(format!(
            "link dependency output must be exactly {}",
            expected_output.display()
        ));
    }
    if rule.inputs.is_empty() || rule.inputs.len() > MAX_INPUTS {
        return Err(format!(
            "link dependency input count is outside bounds: {}",
            rule.inputs.len()
        ));
    }
    let mut seen = BTreeSet::new();
    let mut inputs = Vec::with_capacity(rule.inputs.len());
    for logical_path in rule.inputs {
        if !seen.insert(logical_path.clone()) {
            return Err(format!(
                "duplicate link dependency path {}",
                logical_path.display()
            ));
        }
        let root = longest_root(&logical_path, roots)?;
        let relative = logical_path
            .strip_prefix(root.logical)
            .map_err(|_| "link dependency root mapping changed".to_owned())?;
        if relative.as_os_str().is_empty() {
            return Err("link dependency path cannot equal a trusted root".to_owned());
        }
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
        logical_output: rule.output,
        inputs,
    })
}

fn validate_roots(roots: &[Root<'_>]) -> Result<(), String> {
    if roots.is_empty() {
        return Err("link dependency capture has no trusted roots".to_owned());
    }
    for root in roots {
        validate_logical(root.logical, "trusted link root")?;
        if !root.backing.is_absolute() {
            return Err("trusted link root backing path must be absolute".to_owned());
        }
        authority::validate_directory(root.backing, "trusted link root backing")?;
    }
    Ok(())
}

fn validate_expected_output(path: &Path) -> Result<(), String> {
    validate_logical(path, "expected link output")?;
    Ok(())
}

fn longest_root<'a>(logical_path: &Path, roots: &'a [Root<'a>]) -> Result<&'a Root<'a>, String> {
    let mut best: Option<(&Root<'_>, usize)> = None;
    for root in roots
        .iter()
        .filter(|root| logical_path.starts_with(root.logical))
    {
        let length = root.logical.components().count();
        match best {
            None => best = Some((root, length)),
            Some((_, best_length)) if length > best_length => best = Some((root, length)),
            Some((_, best_length)) if length == best_length => {
                return Err(format!(
                    "link dependency path has ambiguous trusted roots: {}",
                    logical_path.display()
                ));
            }
            _ => {}
        }
    }
    best.map(|(root, _)| root).ok_or_else(|| {
        format!(
            "link dependency path escapes trusted roots: {}",
            logical_path.display()
        )
    })
}

struct Rule {
    output: PathBuf,
    inputs: Vec<PathBuf>,
}

fn parse(bytes: &[u8]) -> Result<Rule, String> {
    let colon = first_unescaped_colon(bytes)?;
    let targets = words(&bytes[..colon])?;
    let [target] = targets.as_slice() else {
        return Err("link dependency file must contain exactly one output target".to_owned());
    };
    let output = logical_path(target, "link dependency output")?;
    let inputs = words(&bytes[colon + 1..])?
        .into_iter()
        .map(|value| logical_path(&value, "link dependency input"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Rule { output, inputs })
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

fn logical_path(value: &str, label: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.len() > 16 * 1024
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(format!("invalid {label}"));
    }
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || value.contains("//")
        || value.contains('\\')
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        Err(format!("invalid {label}"))
    } else {
        Ok(path)
    }
}

fn validate_logical(path: &Path, label: &str) -> Result<(), String> {
    let value = path.to_str().ok_or_else(|| format!("invalid {label}"))?;
    if logical_path(value, label)? != path {
        return Err(format!("invalid {label}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_output_and_parses_gnu_continuations() {
        let rule = parse(
            b"/target/x86_64-unknown-linux-gnu/release/semantic-fabric: /target/one \\\n+ /target/with\\ space\n",
        )
        .unwrap();
        assert_eq!(
            rule.output,
            Path::new("/target/x86_64-unknown-linux-gnu/release/semantic-fabric")
        );
        assert_eq!(
            rule.inputs,
            [
                PathBuf::from("/target/one"),
                PathBuf::from("/target/with space")
            ]
        );
    }

    #[test]
    fn rejects_bad_targets_and_paths() {
        assert!(parse(b"/target/a /target/b: /target/input\n").is_err());
        assert!(parse(b"relative: /target/input\n").is_err());
        assert!(parse(b"/target/out: /target/../escape\n").is_err());
    }

    #[test]
    fn longest_trusted_root_wins_and_ties_fail() {
        let top = Root {
            logical: Path::new("/target"),
            backing: Path::new("/a"),
        };
        let nested = Root {
            logical: Path::new("/target/build"),
            backing: Path::new("/b"),
        };
        assert_eq!(
            longest_root(Path::new("/target/build/x"), &[top, nested])
                .unwrap()
                .logical,
            Path::new("/target/build")
        );
        let tie = Root {
            logical: Path::new("/target/build"),
            backing: Path::new("/c"),
        };
        assert!(longest_root(Path::new("/target/build/x"), &[nested, tie]).is_err());
    }
}
