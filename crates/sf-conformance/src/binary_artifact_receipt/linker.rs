//! GNU linker dependency-file capture without executing a loader or artifact.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{
    authority,
    host_link_authority::HostLinkAuthority,
    model::LinkInputOrigin,
    producer_paths::{LinkMappedPath, SandboxPathMap},
};

const MAX_DEPFILE_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_INPUTS: usize = 16_384;
const MAX_ALIASES: usize = 256;
const MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_PATH_COMPONENTS: usize = 128;
const OUTPUT_PREFIX: &str = "/target/x86_64-unknown-linux-gnu/release/deps/semantic_fabric-";

#[derive(Debug)]
pub(super) struct DependencyFile {
    pub(super) sha256: String,
    pub(super) byte_length: u64,
    pub(super) raw_input_count: usize,
    /// Hashed linker output in the host-backed fresh target tree.
    pub(super) output_path: PathBuf,
    /// Canonical receipt identity for the hashed linker output.
    pub(super) receipt_output: String,
    /// Canonical, unique inventory; raw order and multiplicity remain bound by
    /// `sha256` and `raw_input_count`.
    pub(super) inputs: Vec<Input>,
    pub(super) aliases: Vec<InputAlias>,
    depfile_path: PathBuf,
    alias_authorities: Vec<HostLinkAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Input {
    pub(super) origin: LinkInputOrigin,
    pub(super) receipt_path: String,
    pub(super) sha256: String,
    pub(super) byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InputAlias {
    pub(super) alias_receipt_path: String,
    pub(super) terminal_receipt_path: String,
    pub(super) hop_count: u8,
    pub(super) resolution_sha256: String,
}

impl DependencyFile {
    pub(super) fn assert_current(&self, path_map: &SandboxPathMap) -> Result<(), String> {
        for authority in &self.alias_authorities {
            authority.assert_current()?;
        }
        let current = capture(&self.depfile_path, path_map)?;
        if self.sha256 != current.sha256
            || self.byte_length != current.byte_length
            || self.raw_input_count != current.raw_input_count
            || self.output_path != current.output_path
            || self.receipt_output != current.receipt_output
            || self.inputs != current.inputs
            || self.aliases != current.aliases
        {
            return Err("final link dependency observation changed during capture".to_owned());
        }
        for authority in &current.alias_authorities {
            authority.assert_current()?;
        }
        for authority in &self.alias_authorities {
            authority.assert_current()?;
        }
        Ok(())
    }
}

/// Reads the complete GNU ld depfile. The accepted grammar is exactly one
/// continued primary rule followed by one empty phony rule per primary input,
/// in the same normalized order (GNU `--dependency-file` output).
pub(super) fn capture(depfile: &Path, path_map: &SandboxPathMap) -> Result<DependencyFile, String> {
    let depfile_authority = authority::read(depfile, MAX_DEPFILE_BYTES, "link dependency file")?;
    let rule = parse(&depfile_authority.bytes)?;
    let output = path_map.map(&rule.output)?;
    if output.origin != LinkInputOrigin::BuildOutput {
        return Err("link dependency output is not inside logical /target".to_owned());
    }

    let mut unique = BTreeMap::<String, ObservedInput>::new();
    let mut aliases = BTreeMap::<String, InputAlias>::new();
    let mut alias_authorities = Vec::new();
    for logical_path in &rule.inputs {
        let (receipt_path, observed) = match path_map.map_link_input(logical_path)? {
            LinkMappedPath::Direct(mapped) => {
                let (sha256, byte_length) =
                    authority::digest(&mapped.backing, MAX_INPUT_BYTES, "link dependency")?;
                let receipt_path = mapped.receipt_path;
                (
                    receipt_path,
                    ObservedInput {
                        backing: mapped.backing,
                        origin: mapped.origin,
                        sha256,
                        byte_length,
                    },
                )
            }
            LinkMappedPath::Alias(mapping) => {
                let is_new = !aliases.contains_key(&mapping.alias_receipt_path);
                if is_new && aliases.len() == MAX_ALIASES {
                    return Err("host link alias count exceeds bounds".to_owned());
                }
                let authority = HostLinkAuthority::bind(mapping, MAX_INPUT_BYTES)?;
                let alias = InputAlias {
                    alias_receipt_path: authority.alias_receipt_path.clone(),
                    terminal_receipt_path: authority.terminal_receipt_path.clone(),
                    hop_count: 1,
                    resolution_sha256: authority.resolution_sha256.clone(),
                };
                let terminal_receipt_path = authority.terminal_receipt_path.clone();
                let observed = ObservedInput {
                    backing: authority.terminal_backing.clone(),
                    origin: LinkInputOrigin::HostSystem,
                    sha256: authority.terminal_sha256.clone(),
                    byte_length: authority.terminal_byte_length,
                };
                match aliases.get(&alias.alias_receipt_path) {
                    Some(prior) if prior != &alias => {
                        return Err("host link alias receipt identity collision".to_owned());
                    }
                    Some(_) => {}
                    None => {
                        aliases.insert(alias.alias_receipt_path.clone(), alias);
                        alias_authorities.push(authority);
                    }
                }
                (terminal_receipt_path, observed)
            }
        };
        match unique.get(&receipt_path) {
            Some(prior) if prior != &observed => {
                return Err("link input receipt identity collision".to_owned());
            }
            Some(_) => {}
            None => {
                unique.insert(receipt_path, observed);
            }
        }
    }
    let inputs = unique
        .into_iter()
        .map(|(receipt_path, observed)| Input {
            origin: observed.origin,
            receipt_path,
            sha256: observed.sha256,
            byte_length: observed.byte_length,
        })
        .collect();
    Ok(DependencyFile {
        sha256: depfile_authority.sha256,
        byte_length: depfile_authority.size,
        raw_input_count: rule.inputs.len(),
        output_path: output.backing,
        receipt_output: output.receipt_path,
        inputs,
        aliases: aliases.into_values().collect(),
        depfile_path: depfile.to_path_buf(),
        alias_authorities,
    })
}

#[derive(Debug, PartialEq, Eq)]
struct ObservedInput {
    backing: PathBuf,
    origin: LinkInputOrigin,
    sha256: String,
    byte_length: u64,
}

struct Rule {
    output: PathBuf,
    inputs: Vec<PathBuf>,
}

fn parse(bytes: &[u8]) -> Result<Rule, String> {
    if bytes.is_empty() || bytes.len() > MAX_DEPFILE_BYTES as usize {
        return Err("link dependency file size is outside bounds".to_owned());
    }
    let lines = logical_lines(bytes)?;
    if lines.len() < 3 || lines.len().is_multiple_of(2) {
        return Err("link dependency file has an invalid rule layout".to_owned());
    }
    let (target, input_words) = parse_rule(&lines[0])?;
    let output = normalized_path(&target, "link dependency output")?;
    validate_output(&output)?;
    if input_words.is_empty() || input_words.len() > MAX_INPUTS {
        return Err("link dependency input count is outside bounds".to_owned());
    }
    let inputs = input_words
        .iter()
        .map(|value| normalized_path(value, "link dependency input"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut phony_words = Vec::with_capacity(inputs.len());
    for pair in lines[1..].chunks_exact(2) {
        if !pair[0].is_empty() {
            return Err("link dependency phony rules require one empty separator".to_owned());
        }
        let (target, prerequisites) = parse_rule(&pair[1])?;
        if !prerequisites.is_empty() {
            return Err("link dependency phony rule has prerequisites".to_owned());
        }
        phony_words.push(target);
        if phony_words.len() > MAX_INPUTS {
            return Err("link dependency phony rule count is outside bounds".to_owned());
        }
    }
    if phony_words != input_words {
        return Err("link dependency raw phony rules do not match primary inputs".to_owned());
    }
    let phony = phony_words
        .iter()
        .map(|value| normalized_path(value, "link dependency phony target"))
        .collect::<Result<Vec<_>, _>>()?;
    if phony != inputs {
        return Err("link dependency phony rules do not match primary inputs".to_owned());
    }
    Ok(Rule { output, inputs })
}

/// Joins escaped physical continuations before any rule splitting. Every other
/// physical line must end in LF (CRLF is accepted but canonicalized in memory).
fn logical_lines(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if bytes.get(index + 1) == Some(&b'\n') => {
                current.push(b' ');
                index += 2;
            }
            b'\\'
                if bytes.get(index + 1) == Some(&b'\r') && bytes.get(index + 2) == Some(&b'\n') =>
            {
                current.push(b' ');
                index += 3;
            }
            b'\n' => {
                lines.push(std::mem::take(&mut current));
                index += 1;
            }
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                lines.push(std::mem::take(&mut current));
                index += 2;
            }
            b'\r' => return Err("link dependency file contains a bare carriage return".to_owned()),
            byte => {
                current.push(byte);
                index += 1;
            }
        }
    }
    if !current.is_empty() {
        return Err("link dependency file must end with LF".to_owned());
    }
    Ok(lines)
}

fn parse_rule(line: &[u8]) -> Result<(String, Vec<String>), String> {
    let colon = first_unescaped_colon(line)?;
    let targets = words(&line[..colon])?;
    let [target] = targets.as_slice() else {
        return Err("link dependency rule must contain exactly one target".to_owned());
    };
    Ok((target.clone(), words(&line[colon + 1..])?))
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
    Err("link dependency rule has no separator".to_owned())
}

fn words(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    let mut current = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' => {
                if !current.is_empty() {
                    values.push(finish(&mut current)?);
                }
                index += 1;
            }
            b'\\' => {
                let Some(next) = bytes.get(index + 1).copied() else {
                    return Err("link dependency rule ends with an escape".to_owned());
                };
                current.push(next);
                index += 2;
            }
            b'#' => return Err("link dependency file contains a comment".to_owned()),
            byte if byte.is_ascii_control() => {
                return Err("link dependency rule contains a control byte".to_owned());
            }
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
    if value.is_empty() || value.len() > MAX_PATH_BYTES {
        Err("link dependency path length is outside bounds".to_owned())
    } else {
        Ok(value)
    }
}

fn normalized_path(value: &str, label: &str) -> Result<PathBuf, String> {
    if !value.starts_with('/')
        || value.len() > MAX_PATH_BYTES
        || value.contains('\\')
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(format!("invalid {label}"));
    }
    let mut components = Vec::new();
    for component in value.split('/').skip(1) {
        match component {
            "" => return Err(format!("invalid {label}")),
            "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(format!("{label} escapes the sandbox root"));
                }
            }
            normal => {
                if components.len() == MAX_PATH_COMPONENTS {
                    return Err(format!("{label} has too many components"));
                }
                components.push(normal);
            }
        }
    }
    if components.is_empty() {
        return Err(format!("invalid {label}"));
    }
    Ok(PathBuf::from(format!("/{}", components.join("/"))))
}

fn validate_output(path: &Path) -> Result<(), String> {
    let value = path
        .to_str()
        .ok_or_else(|| "invalid link dependency output".to_owned())?;
    let suffix = value
        .strip_prefix(OUTPUT_PREFIX)
        .ok_or_else(|| "link dependency output does not match the Cargo binary law".to_owned())?;
    if suffix.len() != 16
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("link dependency output does not match the Cargo binary law".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str =
        "/target/x86_64-unknown-linux-gnu/release/deps/semantic_fabric-0123456789abcdef";

    fn depfile(inputs: &[&str], phony: &[&str]) -> Vec<u8> {
        let mut output = format!("{OUTPUT}: \\\n");
        for (index, input) in inputs.iter().enumerate() {
            let continuation = if index + 1 == inputs.len() {
                ""
            } else {
                " \\\n"
            };
            output.push_str(&format!("  {input}{continuation}"));
        }
        output.push_str("\n\n");
        for (index, input) in phony.iter().enumerate() {
            output.push_str(&format!("{input}:\n"));
            if index + 1 != phony.len() {
                output.push('\n');
            }
        }
        output.into_bytes()
    }

    #[test]
    fn parses_gnu_primary_and_phony_rules_with_normalization_and_repeats() {
        let raw = [
            "/usr/lib/gcc/x86_64-linux-gnu/13/../../../x86_64-linux-gnu/Scrt1.o",
            "/lib/x86_64-linux-gnu/libc.so.6",
            "/lib/x86_64-linux-gnu/libc.so.6",
        ];
        let rule = parse(&depfile(&raw, &raw)).unwrap();
        assert_eq!(rule.output, Path::new(OUTPUT));
        assert_eq!(
            rule.inputs,
            [
                PathBuf::from("/usr/lib/x86_64-linux-gnu/Scrt1.o"),
                PathBuf::from("/lib/x86_64-linux-gnu/libc.so.6"),
                PathBuf::from("/lib/x86_64-linux-gnu/libc.so.6"),
            ]
        );
    }

    #[test]
    fn rejects_missing_extra_reordered_and_nonempty_phony_rules() {
        let inputs = ["/target/a", "/target/b"];
        assert!(parse(&depfile(&inputs, &inputs[..1])).is_err());
        assert!(parse(&depfile(&inputs, &["/target/a", "/target/b", "/target/c"])).is_err());
        assert!(parse(&depfile(&inputs, &["/target/b", "/target/a"])).is_err());
        let mut malformed = depfile(&inputs, &inputs);
        let needle = b"/target/a:\n";
        let start = malformed
            .windows(needle.len())
            .position(|window| window == needle)
            .unwrap();
        malformed.splice(
            start..start + needle.len(),
            b"/target/a: /target/b\n".iter().copied(),
        );
        assert!(parse(&malformed).is_err());
        assert!(parse(&depfile(&["/target/a/../b"], &["/target/b"])).is_err());
    }

    #[test]
    fn rejects_above_root_and_output_name_near_misses() {
        assert!(normalized_path("/../../etc/passwd", "fixture").is_err());
        for invalid in [
            "/target/x86_64-unknown-linux-gnu/release/deps/semantic_fabric-0123456789abcde",
            "/target/x86_64-unknown-linux-gnu/release/deps/semantic_fabric-0123456789abcdeg",
            "/target/x86_64-unknown-linux-gnu/release/deps/semantic-fabric-0123456789abcdef",
        ] {
            assert!(validate_output(Path::new(invalid)).is_err());
        }
    }

    #[test]
    fn rejects_layout_comments_and_unterminated_content() {
        assert!(parse(format!("{OUTPUT}: /target/a\n").as_bytes()).is_err());
        assert!(parse(format!("{OUTPUT}: /target/a # x\n\n/target/a:\n").as_bytes()).is_err());
        assert!(logical_lines(b"/target/out: /target/a").is_err());
        assert!(words(b"/target/a\\").is_err());
    }
}

#[cfg(all(test, unix))]
#[path = "linker/capture_tests.rs"]
mod capture_tests;
