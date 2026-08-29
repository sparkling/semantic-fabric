//! Bounded, data-only ELF preflight for the runtime-closure lane.
//!
//! Parsing bytes proves neither their filesystem authority nor that a loader
//! consumed them. It also does not validate every loader semantic, including
//! symbol/hash tables, relocation payloads, or segment permission topology.
//! The collector must bind authority and execution separately before this
//! preflight can contribute to admission.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

mod policy;
mod program;
mod program_policy;
mod versioning;

pub const RUNTIME_ELF_POLICY: &str = "elf64-le-x86_64-closed-dynamic-tags-safe-search-flags-v1";

/// Hashes the policy identifier and exact implementation source files that define it.
pub fn runtime_elf_policy_sha256() -> String {
    let mut digest = Sha256::new();
    digest.update(b"semantic-fabric:runtime-elf-policy:v1\0");
    digest.update(RUNTIME_ELF_POLICY.as_bytes());
    for (name, source) in [
        (
            "runtime_elf.rs",
            include_bytes!("runtime_elf.rs").as_slice(),
        ),
        (
            "runtime_elf/policy.rs",
            include_bytes!("runtime_elf/policy.rs").as_slice(),
        ),
        (
            "runtime_elf/program.rs",
            include_bytes!("runtime_elf/program.rs").as_slice(),
        ),
        (
            "runtime_elf/program_policy.rs",
            include_bytes!("runtime_elf/program_policy.rs").as_slice(),
        ),
        (
            "runtime_elf/versioning.rs",
            include_bytes!("runtime_elf/versioning.rs").as_slice(),
        ),
    ] {
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update((source.len() as u64).to_le_bytes());
        digest.update(source);
    }
    format!("{:x}", digest.finalize())
}

const MAX_ELF_BYTES: usize = 2 * 1024 * 1024 * 1024;
const MAX_PROGRAM_HEADERS: usize = 128;
const MAX_DYNAMIC_ENTRIES: usize = 256;
const MAX_STRING_TABLE_BYTES: usize = 1024 * 1024;
const MAX_NEEDED: usize = 256;
const ELF_HEADER_BYTES: usize = 64;
const PROGRAM_HEADER_BYTES: usize = 56;
const DYNAMIC_ENTRY_BYTES: usize = 16;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_NOTE: u32 = 4;
const PT_PHDR: u32 = 6;
const PT_TLS: u32 = 7;
const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PT_GNU_RELRO: u32 = 0x6474_e552;
const PT_GNU_PROPERTY: u32 = 0x6474_e553;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;
const DT_SONAME: u64 = 14;
const DT_PLTREL: u64 = 20;
const DT_FLAGS: u64 = 30;
const DT_FLAGS_1: u64 = 0x6fff_fffb;
const DF_BIND_NOW: u64 = 0x8;
const DF_STATIC_TLS: u64 = 0x10;
const DF_1_NOW: u64 = 0x1;
const DF_1_PIE: u64 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeElfRole {
    RootPie,
    Loader,
    Libc,
    SharedObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeElfView {
    interpreter: Option<String>,
    needed: Vec<String>,
    soname: Option<String>,
    flags: Option<u64>,
    flags_1: Option<u64>,
    dynamic_tags: Vec<u64>,
}

impl RuntimeElfView {
    pub fn interpreter(&self) -> Option<&str> {
        self.interpreter.as_deref()
    }

    pub fn needed(&self) -> &[String] {
        &self.needed
    }

    pub fn soname(&self) -> Option<&str> {
        self.soname.as_deref()
    }

    pub fn flags(&self) -> Option<u64> {
        self.flags
    }

    pub fn flags_1(&self) -> Option<u64> {
        self.flags_1
    }

    pub fn dynamic_tags(&self) -> &[u64] {
        &self.dynamic_tags
    }
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    offset: u64,
    virtual_address: u64,
    file_size: u64,
    memory_size: u64,
    flags: u32,
    alignment: u64,
}

/// Parses one exact byte sequence under the closed M0 runtime ELF policy.
///
/// Unknown dynamic tags, resolution-changing flags, malformed names, ambiguous
/// virtual mappings, and executable+writable load segments fail closed.
pub fn parse_runtime_elf(bytes: &[u8], role: RuntimeElfRole) -> Result<RuntimeElfView, String> {
    let (loads, dynamic, interpreter) = program::parse(bytes, role)?;
    let entries = dynamic_entries(bytes, dynamic)?;
    let (string_address, string_size) = required_string_table(&entries)?;
    let string_size = usize::try_from(string_size)
        .map_err(|_| "ELF dynamic string table size does not fit memory".to_owned())?;
    if !(1..=MAX_STRING_TABLE_BYTES).contains(&string_size) {
        return Err("ELF dynamic string table size is outside bounds".to_owned());
    }
    let string_offset = map_file_range(&loads, string_address, string_size as u64)?;
    let strings = slice(
        bytes,
        string_offset,
        string_size as u64,
        "ELF dynamic string table",
    )?;
    if strings.first() != Some(&0) {
        return Err("ELF dynamic string table does not start with NUL".to_owned());
    }

    let mut needed = Vec::new();
    let mut needed_set = BTreeSet::new();
    let mut soname = None;
    let mut tags = BTreeSet::new();
    let mut flags = None;
    let mut flags_1 = None;
    for &(tag, value) in &entries {
        if !policy::allows(role, tag) {
            return Err(format!(
                "ELF dynamic tag 0x{tag:x} is prohibited or unknown"
            ));
        }
        if tag != DT_NEEDED && !tags.insert(tag) {
            return Err(format!("duplicate ELF dynamic tag 0x{tag:x}"));
        }
        match tag {
            DT_NEEDED => {
                if needed.len() == MAX_NEEDED {
                    return Err("ELF DT_NEEDED count exceeds bounds".to_owned());
                }
                let name = dynamic_string(strings, value, "DT_NEEDED")?;
                validate_object_name(&name, "DT_NEEDED")?;
                if !needed_set.insert(name.clone()) {
                    return Err("duplicate ELF DT_NEEDED library name".to_owned());
                }
                needed.push(name);
            }
            DT_SONAME => {
                let name = dynamic_string(strings, value, "DT_SONAME")?;
                validate_object_name(&name, "DT_SONAME")?;
                soname = Some(name);
            }
            DT_FLAGS => {
                if value & !(DF_BIND_NOW | DF_STATIC_TLS) != 0 {
                    return Err("ELF DT_FLAGS enables a prohibited loader behavior".to_owned());
                }
                flags = Some(value);
            }
            DT_FLAGS_1 => {
                if value & !(DF_1_NOW | DF_1_PIE) != 0 {
                    return Err("ELF DT_FLAGS_1 enables a prohibited loader behavior".to_owned());
                }
                flags_1 = Some(value);
            }
            DT_PLTREL if !matches!(value, 7 | 17) => {
                return Err("ELF DT_PLTREL is neither RELA nor REL".to_owned());
            }
            9 if value != 24 => return Err("ELF DT_RELAENT must be 24".to_owned()),
            11 if value != 24 => return Err("ELF DT_SYMENT must be 24".to_owned()),
            19 if value != 16 => return Err("ELF DT_RELENT must be 16".to_owned()),
            37 if value != 8 => return Err("ELF DT_RELRENT must be 8".to_owned()),
            _ => {}
        }
    }
    policy::validate(
        role,
        &entries,
        &loads,
        interpreter.as_deref(),
        &needed,
        soname.as_deref(),
    )?;
    versioning::validate(bytes, &loads, strings, &entries, &needed)?;
    tags.extend(entries.iter().map(|(tag, _)| *tag));
    Ok(RuntimeElfView {
        interpreter,
        needed,
        soname,
        flags,
        flags_1,
        dynamic_tags: tags.into_iter().collect(),
    })
}

fn dynamic_entries(bytes: &[u8], dynamic: Segment) -> Result<Vec<(u64, u64)>, String> {
    if dynamic.file_size == 0
        || !dynamic.file_size.is_multiple_of(DYNAMIC_ENTRY_BYTES as u64)
        || dynamic.file_size / DYNAMIC_ENTRY_BYTES as u64 > MAX_DYNAMIC_ENTRIES as u64
    {
        return Err("ELF PT_DYNAMIC size is outside bounds".to_owned());
    }
    let table = slice(bytes, dynamic.offset, dynamic.file_size, "ELF PT_DYNAMIC")?;
    let mut entries = Vec::new();
    let mut terminated = false;
    for entry in table.chunks_exact(DYNAMIC_ENTRY_BYTES) {
        let tag = read_u64(entry, 0)?;
        let value = read_u64(entry, 8)?;
        if terminated {
            if tag != DT_NULL || value != 0 {
                return Err("ELF PT_DYNAMIC has data after DT_NULL".to_owned());
            }
        } else if tag == DT_NULL {
            if value != 0 {
                return Err("ELF DT_NULL has a nonzero value".to_owned());
            }
            terminated = true;
        } else {
            entries.push((tag, value));
        }
    }
    if !terminated {
        return Err("ELF PT_DYNAMIC has no DT_NULL terminator".to_owned());
    }
    Ok(entries)
}

fn required_string_table(entries: &[(u64, u64)]) -> Result<(u64, u64), String> {
    let address = unique_value(entries, DT_STRTAB, "DT_STRTAB")?;
    let size = unique_value(entries, DT_STRSZ, "DT_STRSZ")?;
    Ok((address, size))
}

fn unique_value(entries: &[(u64, u64)], tag: u64, label: &str) -> Result<u64, String> {
    let values: Vec<_> = entries
        .iter()
        .filter_map(|(candidate, value)| (*candidate == tag).then_some(*value))
        .collect();
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(format!("ELF has no unique {label}")),
    }
}

fn map_file_range(loads: &[Segment], address: u64, size: u64) -> Result<u64, String> {
    let end = address
        .checked_add(size)
        .ok_or_else(|| "ELF virtual-address range overflow".to_owned())?;
    let mut offsets = Vec::new();
    for load in loads {
        let load_end = load
            .virtual_address
            .checked_add(load.file_size)
            .ok_or_else(|| "ELF PT_LOAD range overflow".to_owned())?;
        if load.flags & PF_R != 0 && address >= load.virtual_address && end <= load_end {
            offsets.push(
                load.offset
                    .checked_add(address - load.virtual_address)
                    .ok_or_else(|| "ELF mapped file offset overflow".to_owned())?,
            );
        }
    }
    match offsets.as_slice() {
        [offset] => Ok(*offset),
        _ => Err("ELF virtual range does not have one file-backed mapping".to_owned()),
    }
}

fn dynamic_string(strings: &[u8], offset: u64, label: &str) -> Result<String, String> {
    let start = usize::try_from(offset).map_err(|_| format!("ELF {label} offset overflow"))?;
    let tail = strings
        .get(start..)
        .ok_or_else(|| format!("ELF {label} offset is outside the string table"))?;
    if start == 0 || strings.get(start - 1) != Some(&0) {
        return Err(format!("ELF {label} does not start at a string boundary"));
    }
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| format!("ELF {label} string is unterminated"))?;
    String::from_utf8(tail[..end].to_vec()).map_err(|_| format!("ELF {label} is not UTF-8"))
}

fn validate_object_name(value: &str, label: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if !(1..=128).contains(&value.len()) {
        return Err(format!("ELF {label} object name is outside bounds"));
    }
    if value.contains("..") {
        return Err(format!("ELF {label} object name contains dot-dot"));
    }
    if !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(format!(
            "ELF {label} object name is not anchored by alphanumeric bytes"
        ));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        return Err(format!(
            "ELF {label} object name contains a prohibited byte"
        ));
    }
    Ok(())
}

fn interpreter_string(bytes: &[u8]) -> Result<String, String> {
    if !(2..=513).contains(&bytes.len())
        || bytes.last() != Some(&0)
        || bytes[..bytes.len() - 1].contains(&0)
    {
        return Err("ELF PT_INTERP is not one bounded NUL-terminated string".to_owned());
    }
    let value = std::str::from_utf8(&bytes[..bytes.len() - 1])
        .map_err(|_| "ELF PT_INTERP is not UTF-8".to_owned())?;
    let path = Path::new(value);
    if !path.is_absolute()
        || value.contains("//")
        || value.contains('\\')
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !b"/._+-".contains(&byte))
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err("ELF PT_INTERP is not a normalized absolute path".to_owned());
    }
    Ok(value.to_owned())
}

fn set_once<T>(slot: &mut Option<T>, value: T, label: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("ELF has duplicate {label}"))
    } else {
        Ok(())
    }
}

fn slice<'a>(bytes: &'a [u8], offset: u64, size: u64, label: &str) -> Result<&'a [u8], String> {
    let start = usize::try_from(offset).map_err(|_| format!("{label} offset overflow"))?;
    let length = usize::try_from(size).map_err(|_| format!("{label} size overflow"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| format!("{label} range overflow"))?;
    bytes
        .get(start..end)
        .ok_or_else(|| format!("{label} is outside the ELF bytes"))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "ELF integer range overflow".to_owned())?;
    let value: [u8; 2] = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated ELF integer".to_owned())?
        .try_into()
        .map_err(|_| "invalid ELF integer".to_owned())?;
    Ok(u16::from_le_bytes(value))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "ELF integer range overflow".to_owned())?;
    let value: [u8; 4] = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated ELF integer".to_owned())?
        .try_into()
        .map_err(|_| "invalid ELF integer".to_owned())?;
    Ok(u32::from_le_bytes(value))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| "ELF integer range overflow".to_owned())?;
    let value: [u8; 8] = bytes
        .get(offset..end)
        .ok_or_else(|| "truncated ELF integer".to_owned())?
        .try_into()
        .map_err(|_| "invalid ELF integer".to_owned())?;
    Ok(u64::from_le_bytes(value))
}

#[cfg(test)]
pub(super) mod tests;
