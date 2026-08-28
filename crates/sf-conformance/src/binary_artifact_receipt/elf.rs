//! ELF inspection through one bound `readelf` implementation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::{artifact_pair::ArtifactPair, authority, process};

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
    pub(super) unix_mode: u32,
    pub(super) elf_class: String,
    pub(super) data: String,
    pub(super) os_abi: String,
    pub(super) machine: String,
    pub(super) interpreter: String,
    pub(super) needed: Vec<String>,
    pub(super) build_id: String,
}

pub(super) fn inspect(
    artifact: &ArtifactPair,
    readelf: &Path,
) -> Result<(ReadelfIdentity, Observation), String> {
    if !readelf.is_absolute() {
        return Err("readelf path must be absolute".to_owned());
    }
    artifact.assert_current()?;
    let artifact_argument = artifact
        .selected_path()
        .to_str()
        .ok_or_else(|| "artifact binary path is not UTF-8".to_owned())?;
    let artifact_sha256 = artifact.sha256().to_owned();
    let artifact_size = artifact.byte_length();
    let unix_mode = artifact.unix_mode();
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
    reject_runtime_search_paths(&dynamic)?;
    artifact.assert_current()?;
    let (tool_sha256_after, tool_size_after) =
        authority::digest(readelf, MAX_BINARY_BYTES, "readelf tool")?;
    if tool_sha256 != tool_sha256_after || tool_size != tool_size_after {
        return Err("readelf tool changed during ELF inspection".to_owned());
    }
    let identity = ReadelfIdentity {
        path: readelf.to_path_buf(),
        sha256: tool_sha256,
        size: tool_size,
        version: first_line(&version, "readelf version")?,
    };
    let elf_class = required_field(&headers, "Class:", "ELF class")?;
    let data = required_field(&headers, "Data:", "ELF data encoding")?;
    let os_abi = required_field(&headers, "OS/ABI:", "ELF OS/ABI")?;
    let machine = required_field(&headers, "Machine:", "ELF machine")?;
    enforce_fixed_facts(unix_mode, &elf_class, &data, &os_abi, &machine)?;
    let observation = Observation {
        artifact_sha256,
        artifact_size,
        unix_mode,
        elf_class,
        data,
        os_abi,
        machine,
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

fn enforce_fixed_facts(
    unix_mode: u32,
    elf_class: &str,
    data: &str,
    os_abi: &str,
    machine: &str,
) -> Result<(), String> {
    if unix_mode != 0o755 {
        return Err("artifact binary mode must be exactly 0755".to_owned());
    }
    if elf_class != "ELF64"
        || data != "2's complement, little endian"
        || os_abi != "UNIX - System V"
        || machine != "Advanced Micro Devices X86-64"
    {
        return Err("artifact ELF facts must be ELF64/little/x86-64/System-V".to_owned());
    }
    Ok(())
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
    const SECTION: &str = "Displaying notes found in: .note.gnu.build-id";
    const COLUMNS: [&str; 4] = ["Owner", "Data", "size", "Description"];
    const DESCRIPTION: [&str; 4] = ["(unique", "build", "ID", "bitstring)"];
    let invalid = || "ELF notes have no unique hexadecimal build ID".to_owned();
    if !input.ends_with('\n')
        || input.bytes().any(|byte| {
            !byte.is_ascii() || (byte.is_ascii_control() && !matches!(byte, b'\n' | b'\t'))
        })
    {
        return Err(invalid());
    }
    let lines: Vec<_> = input.split_terminator('\n').collect();
    let build_sections: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let name = line.trim().strip_prefix("Displaying notes found in: ")?;
            name.starts_with(".note.gnu.build-id")
                .then_some((index, name))
        })
        .collect();
    let [(section_index, name)] = build_sections.as_slice() else {
        return Err(invalid());
    };
    if *name != ".note.gnu.build-id" || lines[*section_index].trim() != SECTION {
        return Err(invalid());
    }
    if build_id_marker_count(input) != 1 {
        return Err(invalid());
    }
    let columns: Vec<_> = lines
        .get(section_index + 1)
        .ok_or_else(&invalid)?
        .split_ascii_whitespace()
        .collect();
    if columns != COLUMNS {
        return Err(invalid());
    }
    let descriptor = lines.get(section_index + 2).ok_or_else(&invalid)?;
    if descriptor.trim_end_matches([' ', '\t']) != *descriptor {
        return Err(invalid());
    }
    let tokens: Vec<_> = descriptor.split_ascii_whitespace().collect();
    if tokens.len() < 7
        || tokens[0] != "GNU"
        || tokens[2] != "NT_GNU_BUILD_ID"
        || tokens[3..7] != DESCRIPTION
    {
        return Err(invalid());
    }
    let size = note_size(tokens[1]).ok_or_else(&invalid)?;
    let (digest, consumed) = match tokens.get(7..) {
        Some(["Build", "ID:", digest]) => (*digest, 3),
        Some([]) => {
            let continuation = lines.get(section_index + 3).ok_or_else(&invalid)?;
            if continuation.trim_end_matches([' ', '\t']) != *continuation {
                return Err(invalid());
            }
            let continuation: Vec<_> = continuation.split_ascii_whitespace().collect();
            let ["Build", "ID:", digest] = continuation.as_slice() else {
                return Err(invalid());
            };
            (*digest, 4)
        }
        _ => return Err(invalid()),
    };
    if !(16..=128).contains(&digest.len())
        || digest.len() % 2 != 0
        || size.checked_mul(2) != Some(digest.len())
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid());
    }
    for line in lines.iter().skip(section_index + consumed) {
        if is_notes_header(line) {
            break;
        }
        if !line.trim().is_empty() {
            return Err(invalid());
        }
    }
    Ok(digest.to_owned())
}

fn build_id_marker_count(input: &str) -> usize {
    input
        .lines()
        .map(|line| {
            let tokens: Vec<_> = line.split_ascii_whitespace().collect();
            tokens
                .windows(2)
                .filter(|tokens| matches!(*tokens, ["Build", "ID:"]))
                .count()
        })
        .sum()
}

fn note_size(value: &str) -> Option<usize> {
    let digits = value.strip_prefix("0x")?;
    if digits.is_empty()
        || digits.len() > 16
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let size = usize::from_str_radix(digits, 16).ok()?;
    (size <= 64).then_some(size)
}

fn is_notes_header(line: &str) -> bool {
    line.trim()
        .strip_prefix("Displaying notes found in: ")
        .is_some_and(|name| {
            !name.is_empty() && !name.bytes().any(|byte| byte.is_ascii_whitespace())
        })
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
    fn accepts_structured_inline_and_wrapped_build_ids() {
        let id = "0123456789abcdef0123456789abcdef01234567";
        let inline = format!(
            "\nDisplaying notes found in: .note.gnu.property\n  Owner Data size Description\n\nDisplaying notes found in: .note.gnu.build-id\n  Owner                Data size \tDescription\n  GNU 0x00000014 NT_GNU_BUILD_ID (unique build ID bitstring) Build ID: {id}\n\nDisplaying notes found in: .note.ABI-tag\n  Owner Data size Description\n"
        );
        assert_eq!(build_id(&inline).unwrap(), id);
        let wrapped = format!(
            "Displaying notes found in: .note.gnu.build-id\n\tOwner\tData\tsize\tDescription\n\tGNU\t0x14\tNT_GNU_BUILD_ID\t(unique build ID bitstring)\n    Build\tID:\t{id}\n"
        );
        assert_eq!(build_id(&wrapped).unwrap(), id);
    }

    #[test]
    fn rejects_unstructured_spoofed_or_duplicate_build_ids() {
        let id = "0123456789abcdef0123456789abcdef01234567";
        let valid = format!(
            "Displaying notes found in: .note.gnu.build-id\nOwner Data size Description\nGNU 0x14 NT_GNU_BUILD_ID (unique build ID bitstring) Build ID: {id}\n"
        );
        let invalid = [
            format!("Build ID: {id}\n"),
            valid.replace("\nGNU ", "\nLLVM "),
            valid.replace("NT_GNU_BUILD_ID", "NT_GNU_BUILD_IDX"),
            valid.replace("(unique build ID bitstring)", "(GNU build ID)"),
            valid.replace("Owner Data size Description", "Owner Size Description"),
            valid.replace(".note.gnu.build-id", ".note.gnu.build-id.evil"),
            format!("{valid}{valid}"),
            format!("Displaying notes found in: .note.fake\nOwner Data size Description\nGNU 0x14 NT_FAKE (description) Build ID: {id}\n\n{valid}"),
            valid.replace(id, &format!("{id} trailing")),
            valid.replace(id, &id.to_ascii_uppercase()),
            valid.replace("0x14", "0x13"),
            valid.replace("0x14", "0X14"),
            valid.replace("0x14", "0xffffffffffffffff"),
            valid
                .replace("0x14", "0x7")
                .replace(id, "0123456789abcd"),
            valid
                .replace("0x14", "0x41")
                .replace(id, &"a".repeat(130)),
            valid.replace(id, "0123456789abcdef0"),
            valid.replace(id, "0123456789abcdef0123456789abcdef0123456g"),
            valid.replace("Owner", "Own\0er"),
            valid.replace('\n', "\r\n"),
            valid.trim_end().to_owned(),
            valid.replace(&format!("{id}\n"), &format!("{id} \n")),
            valid.replace(&format!("Build ID: {id}"), &format!("Build ID: {id}\nextra row")),
        ];
        for notes in invalid {
            assert!(
                build_id(&notes).is_err(),
                "accepted invalid notes: {notes:?}"
            );
        }
    }

    #[test]
    fn rejects_intervening_or_competing_wrapped_build_id_rows() {
        let id = "0123456789abcdef0123456789abcdef01234567";
        let prefix = "Displaying notes found in: .note.gnu.build-id\nOwner Data size Description\nGNU 0x14 NT_GNU_BUILD_ID (unique build ID bitstring)";
        for suffix in [
            format!("\n\nBuild ID: {id}\n"),
            format!("\nBuild ID: {id}\nBuild ID: {id}\n"),
            format!(" Build ID: {id}\nBuild ID: {id}\n"),
            format!("\nBuild ID:{id}\n"),
        ] {
            assert!(build_id(&format!("{prefix}{suffix}")).is_err());
        }
    }

    #[test]
    fn rejects_rpath_and_runpath() {
        assert!(reject_runtime_search_paths("0x1 (RPATH) Library rpath: [/tmp]\n").is_err());
        assert!(reject_runtime_search_paths("0x1 (RUNPATH) Library runpath: [/tmp]\n").is_err());
    }

    #[test]
    fn enforces_the_fixed_receipt_elf_facts() {
        assert!(enforce_fixed_facts(
            0o755,
            "ELF64",
            "2's complement, little endian",
            "UNIX - System V",
            "Advanced Micro Devices X86-64"
        )
        .is_ok());
        assert!(enforce_fixed_facts(
            0o755,
            "ELF64",
            "2's complement, big endian",
            "UNIX - System V",
            "Advanced Micro Devices X86-64"
        )
        .is_err());
        for mode in [0o4755, 0o2755, 0o1755] {
            assert!(enforce_fixed_facts(
                mode,
                "ELF64",
                "2's complement, little endian",
                "UNIX - System V",
                "Advanced Micro Devices X86-64"
            )
            .is_err());
        }
    }
}
