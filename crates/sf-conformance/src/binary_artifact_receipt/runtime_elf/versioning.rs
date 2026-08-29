use std::collections::{BTreeMap, BTreeSet};

use super::{dynamic_string, map_file_range, read_u16, read_u32, slice, Segment};

const DT_VERDEF: u64 = 0x6fff_fffc;
const DT_VERDEFNUM: u64 = 0x6fff_fffd;
const DT_VERNEED: u64 = 0x6fff_fffe;
const DT_VERNEEDNUM: u64 = 0x6fff_ffff;
const MAX_VERSION_RECORDS: u64 = 256;
const MAX_VERSION_AUXILIARIES: u16 = 256;
const MAX_TOTAL_VERSION_RECORDS: usize = 4_096;

#[derive(Default)]
struct VersionState {
    ranges: BTreeMap<u64, u64>,
    indices: BTreeSet<u16>,
    files: BTreeSet<String>,
}

impl VersionState {
    fn reserve(&mut self, address: u64, size: u64, label: &str) -> Result<(), String> {
        if self.ranges.len() >= MAX_TOTAL_VERSION_RECORDS {
            return Err("ELF version record total is outside bounds".to_owned());
        }
        let end = address
            .checked_add(size)
            .ok_or_else(|| format!("ELF {label} range overflow"))?;
        let overlaps_predecessor = self
            .ranges
            .range(..=address)
            .next_back()
            .is_some_and(|(_, existing_end)| *existing_end > address);
        let overlaps_successor = self
            .ranges
            .range(address..)
            .next()
            .is_some_and(|(start, _)| *start < end);
        if overlaps_predecessor || overlaps_successor {
            return Err(format!("ELF {label} overlaps another version record"));
        }
        self.ranges.insert(address, end);
        Ok(())
    }
}

pub(super) fn validate(
    bytes: &[u8],
    loads: &[Segment],
    strings: &[u8],
    entries: &[(u64, u64)],
    needed: &[String],
) -> Result<(), String> {
    let mut state = VersionState::default();
    if let Some((address, count)) = pair(entries, DT_VERNEED, DT_VERNEEDNUM)? {
        validate_verneed(bytes, loads, strings, needed, address, count, &mut state)?;
    }
    match pair(entries, DT_VERDEF, DT_VERDEFNUM)? {
        Some((address, count)) => {
            validate_verdef(bytes, loads, strings, address, count, &mut state)
        }
        None => Ok(()),
    }
}

fn validate_verneed(
    bytes: &[u8],
    loads: &[Segment],
    strings: &[u8],
    needed: &[String],
    address: u64,
    count: u64,
    state: &mut VersionState,
) -> Result<(), String> {
    validate_count(count, "VERNEED")?;
    let mut current = address;
    let mut auxiliaries = Vec::new();
    for index in 0..count {
        state.reserve(current, 16, "VERNEED record")?;
        let row = mapped(bytes, loads, current, 16, "ELF VERNEED record")?;
        let version = read_u16(row, 0)?;
        let auxiliary_count = read_u16(row, 2)?;
        let file_offset = read_u32(row, 4)? as u64;
        let auxiliary_offset = read_u32(row, 8)? as u64;
        let next = read_u32(row, 12)? as u64;
        if version != 1
            || !(1..=MAX_VERSION_AUXILIARIES).contains(&auxiliary_count)
            || auxiliary_offset < 16
            || !auxiliary_offset.is_multiple_of(4)
        {
            return Err("ELF VERNEED record is malformed".to_owned());
        }
        let file = dynamic_string(strings, file_offset, "VERNEED file")?;
        validate_name(&file, "VERNEED file")?;
        if !needed.iter().any(|candidate| candidate == &file) {
            return Err("ELF VERNEED file is not a DT_NEEDED object".to_owned());
        }
        if !state.files.insert(file) {
            return Err("ELF VERNEED repeats a dependency file".to_owned());
        }
        auxiliaries.push((
            add(current, auxiliary_offset, "VERNEED auxiliary")?,
            auxiliary_count,
        ));
        current = next_record(current, next, index, count, "VERNEED")?;
    }
    for (address, count) in auxiliaries {
        validate_vernaux(bytes, loads, strings, address, count, state)?;
    }
    Ok(())
}

fn validate_vernaux(
    bytes: &[u8],
    loads: &[Segment],
    strings: &[u8],
    mut current: u64,
    count: u16,
    state: &mut VersionState,
) -> Result<(), String> {
    for index in 0..u64::from(count) {
        state.reserve(current, 16, "VERNAUX record")?;
        let row = mapped(bytes, loads, current, 16, "ELF VERNAUX record")?;
        let hash = read_u32(row, 0)?;
        let flags = read_u16(row, 4)?;
        let other = read_u16(row, 6)? & 0x7fff;
        let name_offset = read_u32(row, 8)? as u64;
        let next = read_u32(row, 12)? as u64;
        if flags & !2 != 0 || other < 2 {
            return Err("ELF VERNAUX record has invalid flags or index".to_owned());
        }
        if !state.indices.insert(other) {
            return Err("ELF version index is duplicated".to_owned());
        }
        let name = dynamic_string(strings, name_offset, "VERNAUX name")?;
        validate_name(&name, "VERNAUX name")?;
        if hash != elf_hash(name.as_bytes()) {
            return Err("ELF VERNAUX hash does not match its name".to_owned());
        }
        current = next_record(current, next, index, u64::from(count), "VERNAUX")?;
    }
    Ok(())
}

fn validate_verdef(
    bytes: &[u8],
    loads: &[Segment],
    strings: &[u8],
    address: u64,
    count: u64,
    state: &mut VersionState,
) -> Result<(), String> {
    validate_count(count, "VERDEF")?;
    let mut current = address;
    let mut auxiliaries = Vec::new();
    for index in 0..count {
        state.reserve(current, 20, "VERDEF record")?;
        let row = mapped(bytes, loads, current, 20, "ELF VERDEF record")?;
        let version = read_u16(row, 0)?;
        let flags = read_u16(row, 2)?;
        let definition_index = read_u16(row, 4)? & 0x7fff;
        let auxiliary_count = read_u16(row, 6)?;
        let hash = read_u32(row, 8)?;
        let auxiliary_offset = read_u32(row, 12)? as u64;
        let next = read_u32(row, 16)? as u64;
        if version != 1
            || flags & !3 != 0
            || definition_index == 0
            || !(1..=MAX_VERSION_AUXILIARIES).contains(&auxiliary_count)
            || auxiliary_offset < 20
            || !auxiliary_offset.is_multiple_of(4)
        {
            return Err("ELF VERDEF record is malformed".to_owned());
        }
        if !state.indices.insert(definition_index) {
            return Err("ELF version index is duplicated".to_owned());
        }
        auxiliaries.push((
            add(current, auxiliary_offset, "VERDEF auxiliary")?,
            auxiliary_count,
            hash,
        ));
        current = next_record(current, next, index, count, "VERDEF")?;
    }
    for (address, count, hash) in auxiliaries {
        validate_verdaux(bytes, loads, strings, address, count, hash, state)?;
    }
    Ok(())
}

fn validate_verdaux(
    bytes: &[u8],
    loads: &[Segment],
    strings: &[u8],
    mut current: u64,
    count: u16,
    expected_hash: u32,
    state: &mut VersionState,
) -> Result<(), String> {
    for index in 0..u64::from(count) {
        state.reserve(current, 8, "VERDAUX record")?;
        let row = mapped(bytes, loads, current, 8, "ELF VERDAUX record")?;
        let name_offset = read_u32(row, 0)? as u64;
        let next = read_u32(row, 4)? as u64;
        let name = dynamic_string(strings, name_offset, "VERDAUX name")?;
        validate_name(&name, "VERDAUX name")?;
        if index == 0 && expected_hash != elf_hash(name.as_bytes()) {
            return Err("ELF VERDEF hash does not match its name".to_owned());
        }
        current = next_record(current, next, index, u64::from(count), "VERDAUX")?;
    }
    Ok(())
}

fn pair(
    entries: &[(u64, u64)],
    address_tag: u64,
    count_tag: u64,
) -> Result<Option<(u64, u64)>, String> {
    let address = value(entries, address_tag)?;
    let count = value(entries, count_tag)?;
    match (address, count) {
        (None, None) => Ok(None),
        (Some(address), Some(count)) => Ok(Some((address, count))),
        _ => Err("ELF version-table tag pair is incomplete".to_owned()),
    }
}

fn value(entries: &[(u64, u64)], tag: u64) -> Result<Option<u64>, String> {
    let mut values = entries
        .iter()
        .filter_map(|(candidate, value)| (*candidate == tag).then_some(*value));
    let first = values.next();
    if values.next().is_some() {
        Err(format!("duplicate ELF version dynamic tag 0x{tag:x}"))
    } else {
        Ok(first)
    }
}

fn validate_count(count: u64, label: &str) -> Result<(), String> {
    if (1..=MAX_VERSION_RECORDS).contains(&count) {
        Ok(())
    } else {
        Err(format!("ELF {label} count is outside bounds"))
    }
}

fn next_record(
    current: u64,
    next: u64,
    index: u64,
    count: u64,
    label: &str,
) -> Result<u64, String> {
    let last = index + 1 == count;
    if (last && next != 0) || (!last && (next == 0 || !next.is_multiple_of(4))) {
        return Err(format!("ELF {label} chain length does not match its count"));
    }
    if last {
        Ok(current)
    } else {
        add(current, next, label)
    }
}

fn mapped<'a>(
    bytes: &'a [u8],
    loads: &[Segment],
    address: u64,
    size: u64,
    label: &str,
) -> Result<&'a [u8], String> {
    let offset = map_file_range(loads, address, size)?;
    slice(bytes, offset, size, label)
}

fn add(left: u64, right: u64, label: &str) -> Result<u64, String> {
    left.checked_add(right)
        .ok_or_else(|| format!("ELF {label} address overflow"))
}

fn validate_name(value: &str, label: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if value.is_empty()
        || value.len() > 128
        || !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(byte))
    {
        Err(format!("invalid ELF {label}"))
    } else {
        Ok(())
    }
}

pub(super) fn elf_hash(bytes: &[u8]) -> u32 {
    let mut hash = 0u32;
    for byte in bytes {
        hash = hash.wrapping_shl(4).wrapping_add(u32::from(*byte));
        let high = hash & 0xf000_0000;
        if high != 0 {
            hash ^= high >> 24;
            hash &= !high;
        }
    }
    hash
}

#[cfg(test)]
mod internal_tests {
    use super::*;

    #[test]
    fn ordered_ranges_reject_overlap_and_bound_total_work() {
        let mut state = VersionState::default();
        for index in 0..MAX_TOTAL_VERSION_RECORDS {
            state
                .reserve((index as u64) * 32, 8, "test record")
                .unwrap();
        }
        assert!(state
            .reserve((MAX_TOTAL_VERSION_RECORDS as u64) * 32, 8, "test record")
            .unwrap_err()
            .contains("total is outside bounds"));

        let mut overlapping = VersionState::default();
        overlapping.reserve(32, 16, "test record").unwrap();
        assert!(overlapping
            .reserve(40, 16, "test record")
            .unwrap_err()
            .contains("overlaps"));
        assert!(overlapping
            .reserve(24, 16, "test record")
            .unwrap_err()
            .contains("overlaps"));
    }
}
