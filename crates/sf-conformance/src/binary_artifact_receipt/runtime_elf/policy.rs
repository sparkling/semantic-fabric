use super::{map_file_range, RuntimeElfRole, Segment};

const DT_NEEDED: u64 = 1;
const DT_PLTRELSZ: u64 = 2;
const DT_PLTGOT: u64 = 3;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_INIT: u64 = 12;
const DT_FINI: u64 = 13;
const DT_SONAME: u64 = 14;
const DT_PLTREL: u64 = 20;
const DT_DEBUG: u64 = 21;
const DT_JMPREL: u64 = 23;
const DT_INIT_ARRAY: u64 = 25;
const DT_FINI_ARRAY: u64 = 26;
const DT_INIT_ARRAYSZ: u64 = 27;
const DT_FINI_ARRAYSZ: u64 = 28;
const DT_FLAGS: u64 = 30;
const DT_RELRSZ: u64 = 35;
const DT_RELR: u64 = 36;
const DT_RELRENT: u64 = 37;
const DT_GNU_HASH: u64 = 0x6fff_fef5;
const DT_VERSYM: u64 = 0x6fff_fff0;
const DT_RELACOUNT: u64 = 0x6fff_fff9;
const DT_FLAGS_1: u64 = 0x6fff_fffb;
const DT_VERDEF: u64 = 0x6fff_fffc;
const DT_VERDEFNUM: u64 = 0x6fff_fffd;
const DT_VERNEED: u64 = 0x6fff_fffe;
const DT_VERNEEDNUM: u64 = 0x6fff_ffff;
const DT_X86_64_PLT: u64 = 0x7000_0000;
const DT_X86_64_PLTSZ: u64 = 0x7000_0001;
const DT_X86_64_PLTENT: u64 = 0x7000_0003;

const ROOT_TAGS: &[u64] = &[
    DT_NEEDED,
    DT_PLTRELSZ,
    DT_PLTGOT,
    DT_STRTAB,
    DT_SYMTAB,
    DT_RELA,
    DT_RELASZ,
    DT_RELAENT,
    DT_STRSZ,
    DT_SYMENT,
    DT_INIT,
    DT_FINI,
    DT_PLTREL,
    DT_DEBUG,
    DT_JMPREL,
    DT_INIT_ARRAY,
    DT_FINI_ARRAY,
    DT_INIT_ARRAYSZ,
    DT_FINI_ARRAYSZ,
    DT_FLAGS,
    DT_GNU_HASH,
    DT_VERSYM,
    DT_RELACOUNT,
    DT_FLAGS_1,
    DT_VERNEED,
    DT_VERNEEDNUM,
];

const LOADER_TAGS: &[u64] = &[
    DT_HASH,
    DT_STRTAB,
    DT_SYMTAB,
    DT_RELA,
    DT_RELASZ,
    DT_RELAENT,
    DT_STRSZ,
    DT_SYMENT,
    DT_SONAME,
    DT_FLAGS,
    DT_RELRSZ,
    DT_RELR,
    DT_RELRENT,
    DT_GNU_HASH,
    DT_VERSYM,
    DT_FLAGS_1,
    DT_VERDEF,
    DT_VERDEFNUM,
];

const SHARED_TAGS: &[u64] = &[
    DT_NEEDED,
    DT_PLTRELSZ,
    DT_PLTGOT,
    DT_HASH,
    DT_STRTAB,
    DT_SYMTAB,
    DT_RELA,
    DT_RELASZ,
    DT_RELAENT,
    DT_STRSZ,
    DT_SYMENT,
    DT_INIT,
    DT_FINI,
    DT_SONAME,
    DT_PLTREL,
    DT_JMPREL,
    DT_INIT_ARRAY,
    DT_FINI_ARRAY,
    DT_INIT_ARRAYSZ,
    DT_FINI_ARRAYSZ,
    DT_FLAGS,
    DT_RELRSZ,
    DT_RELR,
    DT_RELRENT,
    DT_GNU_HASH,
    DT_VERSYM,
    DT_RELACOUNT,
    DT_FLAGS_1,
    DT_VERDEF,
    DT_VERDEFNUM,
    DT_VERNEED,
    DT_VERNEEDNUM,
    DT_X86_64_PLT,
    DT_X86_64_PLTSZ,
    DT_X86_64_PLTENT,
];

pub(super) fn allows(role: RuntimeElfRole, tag: u64) -> bool {
    tags(role).contains(&tag)
}

fn tags(role: RuntimeElfRole) -> &'static [u64] {
    match role {
        RuntimeElfRole::RootPie => ROOT_TAGS,
        RuntimeElfRole::Loader => LOADER_TAGS,
        RuntimeElfRole::Libc | RuntimeElfRole::SharedObject => SHARED_TAGS,
    }
}

pub(super) fn validate(
    role: RuntimeElfRole,
    entries: &[(u64, u64)],
    loads: &[Segment],
    interpreter: Option<&str>,
    needed: &[String],
    soname: Option<&str>,
) -> Result<(), String> {
    required(entries, DT_STRTAB, "DT_STRTAB")?;
    required(entries, DT_STRSZ, "DT_STRSZ")?;
    required(entries, DT_SYMTAB, "DT_SYMTAB")?;
    if required(entries, DT_SYMENT, "DT_SYMENT")? != 24 {
        return Err("ELF DT_SYMENT must be 24".to_owned());
    }
    required(entries, DT_GNU_HASH, "DT_GNU_HASH")?;
    validate_role(role, entries, interpreter, needed, soname)?;
    validate_relocations(entries, loads)?;
    validate_pointer(entries, loads, DT_SYMTAB, 24)?;
    validate_pointer(entries, loads, DT_GNU_HASH, 4)?;
    validate_optional_pointer(entries, loads, DT_HASH, 8)?;
    validate_optional_pointer(entries, loads, DT_INIT, 1)?;
    validate_optional_pointer(entries, loads, DT_FINI, 1)?;
    validate_optional_pointer(entries, loads, DT_PLTGOT, 8)?;
    validate_optional_pointer(entries, loads, DT_VERSYM, 2)?;
    validate_pair(entries, loads, DT_INIT_ARRAY, DT_INIT_ARRAYSZ, 8)?;
    validate_pair(entries, loads, DT_FINI_ARRAY, DT_FINI_ARRAYSZ, 8)?;
    validate_version_pair(entries, loads, DT_VERDEF, DT_VERDEFNUM)?;
    validate_version_pair(entries, loads, DT_VERNEED, DT_VERNEEDNUM)?;
    validate_x86_plt(entries, loads)
}

fn validate_role(
    role: RuntimeElfRole,
    entries: &[(u64, u64)],
    interpreter: Option<&str>,
    needed: &[String],
    soname: Option<&str>,
) -> Result<(), String> {
    let flags = optional(entries, DT_FLAGS)?;
    let flags_1 = optional(entries, DT_FLAGS_1)?;
    match role {
        RuntimeElfRole::RootPie
            if interpreter == Some("/lib64/ld-linux-x86-64.so.2")
                && soname.is_none()
                && !needed.is_empty()
                && flags == Some(0x8)
                && flags_1 == Some(0x0800_0001)
                && optional(entries, DT_DEBUG)? == Some(0) =>
        {
            Ok(())
        }
        RuntimeElfRole::Loader
            if interpreter.is_none()
                && needed.is_empty()
                && soname == Some("ld-linux-x86-64.so.2")
                && flags == Some(0x8)
                && flags_1 == Some(0x1) =>
        {
            Ok(())
        }
        RuntimeElfRole::Libc
            if interpreter == Some("/lib64/ld-linux-x86-64.so.2")
                && needed == ["ld-linux-x86-64.so.2"]
                && soname == Some("libc.so.6")
                && flags == Some(0x18)
                && flags_1 == Some(0x1) =>
        {
            Ok(())
        }
        RuntimeElfRole::SharedObject
            if interpreter.is_none()
                && soname.is_some()
                && !matches!(soname, Some("ld-linux-x86-64.so.2" | "libc.so.6"))
                && matches!(flags, None | Some(0x8) | Some(0x18))
                && matches!(flags_1, None | Some(0x1)) =>
        {
            Ok(())
        }
        _ => Err("ELF role-specific dynamic policy does not match".to_owned()),
    }
}

fn validate_relocations(entries: &[(u64, u64)], loads: &[Segment]) -> Result<(), String> {
    validate_triplet(entries, loads, DT_RELA, DT_RELASZ, DT_RELAENT, 24)?;
    validate_triplet(entries, loads, DT_RELR, DT_RELRSZ, DT_RELRENT, 8)?;
    let plt = [DT_PLTGOT, DT_PLTRELSZ, DT_PLTREL, DT_JMPREL];
    let present = plt.iter().filter(|tag| has(entries, **tag)).count();
    if present != 0 && present != plt.len() {
        return Err("ELF PLT relocation tuple is incomplete".to_owned());
    }
    if present != 0 {
        let size = required(entries, DT_PLTRELSZ, "DT_PLTRELSZ")?;
        if required(entries, DT_PLTREL, "DT_PLTREL")? != DT_RELA {
            return Err("ELF DT_PLTREL must be DT_RELA".to_owned());
        }
        if size == 0 || !size.is_multiple_of(24) {
            return Err("ELF DT_PLTRELSZ is not a nonempty RELA table".to_owned());
        }
        validate_pointer(entries, loads, DT_JMPREL, size)?;
    }
    if let Some(count) = optional(entries, DT_RELACOUNT)? {
        let size = required(entries, DT_RELASZ, "DT_RELASZ")?;
        let entry = required(entries, DT_RELAENT, "DT_RELAENT")?;
        let table_entries = size
            .checked_div(entry)
            .ok_or_else(|| "ELF DT_RELAENT cannot be zero".to_owned())?;
        if count == 0 || count > table_entries {
            return Err("ELF DT_RELACOUNT exceeds the RELA table".to_owned());
        }
    }
    Ok(())
}

fn validate_triplet(
    entries: &[(u64, u64)],
    loads: &[Segment],
    address_tag: u64,
    size_tag: u64,
    entry_tag: u64,
    expected_entry: u64,
) -> Result<(), String> {
    let present = [address_tag, size_tag, entry_tag]
        .iter()
        .filter(|tag| has(entries, **tag))
        .count();
    if present == 0 {
        return Ok(());
    }
    if present != 3 || required(entries, entry_tag, "relocation entry size")? != expected_entry {
        return Err("ELF relocation tuple is incomplete or has the wrong entry size".to_owned());
    }
    let size = required(entries, size_tag, "relocation table size")?;
    if size == 0 || !size.is_multiple_of(expected_entry) {
        return Err("ELF relocation table size is invalid".to_owned());
    }
    validate_pointer(entries, loads, address_tag, size)
}

fn validate_pair(
    entries: &[(u64, u64)],
    loads: &[Segment],
    address_tag: u64,
    size_tag: u64,
    alignment: u64,
) -> Result<(), String> {
    match (
        optional(entries, address_tag)?,
        optional(entries, size_tag)?,
    ) {
        (None, None) => Ok(()),
        (Some(address), Some(size)) if size > 0 && size.is_multiple_of(alignment) => {
            map_file_range(loads, address, size).map(|_| ())
        }
        _ => Err("ELF address/size dynamic-tag pair is incomplete or invalid".to_owned()),
    }
}

fn validate_version_pair(
    entries: &[(u64, u64)],
    loads: &[Segment],
    address_tag: u64,
    count_tag: u64,
) -> Result<(), String> {
    match (
        optional(entries, address_tag)?,
        optional(entries, count_tag)?,
    ) {
        (None, None) => Ok(()),
        (Some(address), Some(count)) if count > 0 => map_file_range(loads, address, 1).map(|_| ()),
        _ => Err("ELF version-table dynamic-tag pair is incomplete or invalid".to_owned()),
    }
}

fn validate_x86_plt(entries: &[(u64, u64)], loads: &[Segment]) -> Result<(), String> {
    let tags = [DT_X86_64_PLT, DT_X86_64_PLTSZ, DT_X86_64_PLTENT];
    let present = tags.iter().filter(|tag| has(entries, **tag)).count();
    if present == 0 {
        return Ok(());
    }
    if present != 3 || required(entries, DT_X86_64_PLTENT, "DT_X86_64_PLTENT")? != 16 {
        return Err("ELF x86-64 PLT tuple is incomplete or invalid".to_owned());
    }
    let size = required(entries, DT_X86_64_PLTSZ, "DT_X86_64_PLTSZ")?;
    if size == 0 || !size.is_multiple_of(16) {
        return Err("ELF x86-64 PLT size is invalid".to_owned());
    }
    validate_pointer(entries, loads, DT_X86_64_PLT, size)
}

fn validate_pointer(
    entries: &[(u64, u64)],
    loads: &[Segment],
    tag: u64,
    size: u64,
) -> Result<(), String> {
    let address = required(entries, tag, "dynamic pointer")?;
    map_file_range(loads, address, size).map(|_| ())
}

fn validate_optional_pointer(
    entries: &[(u64, u64)],
    loads: &[Segment],
    tag: u64,
    size: u64,
) -> Result<(), String> {
    if let Some(address) = optional(entries, tag)? {
        map_file_range(loads, address, size)?;
    }
    Ok(())
}

fn required(entries: &[(u64, u64)], tag: u64, label: &str) -> Result<u64, String> {
    optional(entries, tag)?.ok_or_else(|| format!("ELF is missing {label}"))
}

fn optional(entries: &[(u64, u64)], tag: u64) -> Result<Option<u64>, String> {
    let mut values = entries
        .iter()
        .filter_map(|(candidate, value)| (*candidate == tag).then_some(*value));
    let first = values.next();
    if values.next().is_some() {
        Err(format!("ELF has duplicate dynamic tag 0x{tag:x}"))
    } else {
        Ok(first)
    }
}

fn has(entries: &[(u64, u64)], tag: u64) -> bool {
    entries.iter().any(|(candidate, _)| *candidate == tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loads() -> [Segment; 1] {
        [Segment {
            offset: 0,
            virtual_address: 0x1000,
            file_size: 0x1000,
            memory_size: 0x1000,
            flags: 4,
            alignment: 0x1000,
        }]
    }

    fn plt(size: u64) -> Vec<(u64, u64)> {
        vec![
            (DT_PLTGOT, 0x1000),
            (DT_PLTRELSZ, size),
            (DT_PLTREL, DT_RELA),
            (DT_JMPREL, 0x1000),
        ]
    }

    #[test]
    fn plt_relocations_require_a_nonempty_whole_rela_table() {
        validate_relocations(&plt(24), &loads()).unwrap();
        for size in [0, 1, 23, 25, 47] {
            let error = validate_relocations(&plt(size), &loads()).unwrap_err();
            assert!(error.contains("DT_PLTRELSZ"), "size={size}: {error}");
        }
    }
}
