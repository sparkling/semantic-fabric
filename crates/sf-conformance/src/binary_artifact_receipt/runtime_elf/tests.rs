use super::*;

const BASE: u64 = 0x400000;
const INTERP_OFFSET: usize = 0x200;
const DYNAMIC_OFFSET: usize = 0x300;
const STRINGS_OFFSET: usize = 0x600;
const INTERPRETER: &str = "/lib64/ld-linux-x86-64.so.2";

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], program_count: u16) {
    bytes[..8].copy_from_slice(b"\x7fELF\x02\x01\x01\x00");
    put_u16(bytes, 16, 3);
    put_u16(bytes, 18, 62);
    put_u32(bytes, 20, 1);
    put_u64(bytes, 32, ELF_HEADER_BYTES as u64);
    put_u16(bytes, 52, ELF_HEADER_BYTES as u16);
    put_u16(bytes, 54, PROGRAM_HEADER_BYTES as u16);
    put_u16(bytes, 56, program_count);
}

#[derive(Clone, Copy)]
struct ProgramHeader {
    kind: u32,
    flags: u32,
    offset: u64,
    virtual_address: u64,
    size: u64,
    alignment: u64,
}

fn ph(
    kind: u32,
    flags: u32,
    offset: u64,
    virtual_address: u64,
    size: u64,
    alignment: u64,
) -> ProgramHeader {
    ProgramHeader {
        kind,
        flags,
        offset,
        virtual_address,
        size,
        alignment,
    }
}

fn program_header(bytes: &mut [u8], index: usize, header: ProgramHeader) {
    let start = ELF_HEADER_BYTES + index * PROGRAM_HEADER_BYTES;
    put_u32(bytes, start, header.kind);
    put_u32(bytes, start + 4, header.flags);
    put_u64(bytes, start + 8, header.offset);
    put_u64(bytes, start + 16, header.virtual_address);
    put_u64(bytes, start + 32, header.size);
    put_u64(bytes, start + 40, header.size);
    put_u64(bytes, start + 48, header.alignment);
}

fn string(strings: &mut Vec<u8>, value: &str) -> u64 {
    let offset = strings.len() as u64;
    strings.extend_from_slice(value.as_bytes());
    strings.push(0);
    offset
}

fn fixture(needed: &[&str], soname: Option<&str>, interpreter: Option<&str>) -> Vec<u8> {
    let mut strings = vec![0];
    let needed: Vec<_> = needed
        .iter()
        .map(|name| string(&mut strings, name))
        .collect();
    let soname = soname.map(|name| string(&mut strings, name));
    let version_name = string(&mut strings, "GLIBC_2.2.5");
    let mut dynamic = Vec::new();
    dynamic.extend(needed.iter().map(|offset| (DT_NEEDED, *offset)));
    if let Some(offset) = soname {
        dynamic.push((DT_SONAME, offset));
    }
    if interpreter.is_some() {
        dynamic.push((21, 0));
    }
    dynamic.extend([
        (DT_STRTAB, BASE + STRINGS_OFFSET as u64),
        (DT_STRSZ, strings.len() as u64),
        (6, BASE + 0x580),
        (11, 24),
        (7, BASE + 0x560),
        (8, 24),
        (9, 24),
        (0x6fff_fef5, BASE + 0x540),
        (0x6fff_fff0, BASE + 0x530),
        (0x6fff_fffe, BASE + 0x500),
        (0x6fff_ffff, 1),
        (DT_FLAGS, DF_BIND_NOW),
        (
            DT_FLAGS_1,
            DF_1_NOW | (u64::from(interpreter.is_some()) * DF_1_PIE),
        ),
        (DT_NULL, 0),
        (DT_NULL, 0),
    ]);
    let dynamic_size = dynamic.len() * DYNAMIC_ENTRY_BYTES;
    let mut bytes = vec![0; 0x800];
    let program_count = if interpreter.is_some() { 5 } else { 4 };
    header(&mut bytes, program_count);
    let bytes_len = bytes.len() as u64;
    program_header(&mut bytes, 0, ph(PT_LOAD, 5, 0, BASE, bytes_len, 0x1000));
    program_header(
        &mut bytes,
        1,
        ph(
            PT_DYNAMIC,
            PF_R | PF_W,
            DYNAMIC_OFFSET as u64,
            BASE + DYNAMIC_OFFSET as u64,
            dynamic_size as u64,
            8,
        ),
    );
    if let Some(interpreter) = interpreter {
        let mut value = interpreter.as_bytes().to_vec();
        value.push(0);
        bytes[INTERP_OFFSET..INTERP_OFFSET + value.len()].copy_from_slice(&value);
        program_header(
            &mut bytes,
            2,
            ph(
                PT_INTERP,
                4,
                INTERP_OFFSET as u64,
                BASE + INTERP_OFFSET as u64,
                value.len() as u64,
                1,
            ),
        );
    }
    let stack_index = if interpreter.is_some() { 3 } else { 2 };
    program_header(
        &mut bytes,
        stack_index,
        ph(PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 16),
    );
    program_header(
        &mut bytes,
        stack_index + 1,
        ph(
            PT_GNU_RELRO,
            PF_R,
            DYNAMIC_OFFSET as u64,
            BASE + DYNAMIC_OFFSET as u64,
            dynamic_size as u64,
            1,
        ),
    );
    for (index, (tag, value)) in dynamic.into_iter().enumerate() {
        let start = DYNAMIC_OFFSET + index * DYNAMIC_ENTRY_BYTES;
        put_u64(&mut bytes, start, tag);
        put_u64(&mut bytes, start + 8, value);
    }
    if let Some(file) = needed.first() {
        write_verneed(&mut bytes, *file, version_name);
    }
    bytes[STRINGS_OFFSET..STRINGS_OFFSET + strings.len()].copy_from_slice(&strings);
    bytes
}

pub(crate) fn loader_fixture() -> Vec<u8> {
    let mut bytes = fixture(&[], Some("ld-linux-x86-64.so.2"), None);
    replace_tag(&mut bytes, 0x6fff_fffe, 0x6fff_fffc);
    replace_tag(&mut bytes, 0x6fff_ffff, 0x6fff_fffd);
    let version_name = bytes[STRINGS_OFFSET..]
        .windows(b"GLIBC_2.2.5\0".len())
        .position(|window| window == b"GLIBC_2.2.5\0")
        .unwrap() as u64;
    write_verdef(&mut bytes, version_name);
    bytes
}

fn write_verneed(bytes: &mut [u8], file: u64, name: u64) {
    put_u16(bytes, 0x500, 1);
    put_u16(bytes, 0x502, 1);
    put_u32(bytes, 0x504, file as u32);
    put_u32(bytes, 0x508, 16);
    put_u32(bytes, 0x50c, 0);
    put_u32(bytes, 0x510, super::versioning::elf_hash(b"GLIBC_2.2.5"));
    put_u16(bytes, 0x514, 0);
    put_u16(bytes, 0x516, 2);
    put_u32(bytes, 0x518, name as u32);
    put_u32(bytes, 0x51c, 0);
}

fn write_verdef(bytes: &mut [u8], name: u64) {
    bytes[0x500..0x520].fill(0);
    put_u16(bytes, 0x500, 1);
    put_u16(bytes, 0x502, 1);
    put_u16(bytes, 0x504, 1);
    put_u16(bytes, 0x506, 1);
    put_u32(bytes, 0x508, super::versioning::elf_hash(b"GLIBC_2.2.5"));
    put_u32(bytes, 0x50c, 20);
    put_u32(bytes, 0x510, 0);
    put_u32(bytes, 0x514, name as u32);
    put_u32(bytes, 0x518, 0);
}

pub(crate) fn libc_fixture() -> Vec<u8> {
    let mut bytes = fixture(
        &["ld-linux-x86-64.so.2"],
        Some("libc.so.6"),
        Some(INTERPRETER),
    );
    replace_tag(&mut bytes, 21, 4);
    replace_value(&mut bytes, 4, BASE + 0x520);
    replace_value(&mut bytes, DT_FLAGS, DF_BIND_NOW | DF_STATIC_TLS);
    replace_value(&mut bytes, DT_FLAGS_1, DF_1_NOW);
    bytes
}

pub(crate) fn root_fixture() -> Vec<u8> {
    root_fixture_with_needed(&["libc.so.6"])
}

pub(crate) fn root_fixture_with_needed(needed: &[&str]) -> Vec<u8> {
    let bytes = fixture(needed, None, Some(INTERPRETER));
    parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).unwrap();
    bytes
}

pub(crate) fn shared_fixture(needed: &[&str], soname: &str) -> Vec<u8> {
    fixture(needed, Some(soname), None)
}

fn replace_tag(bytes: &mut [u8], old: u64, new: u64) {
    let count = read_u64(bytes, ELF_HEADER_BYTES + PROGRAM_HEADER_BYTES + 32).unwrap() as usize
        / DYNAMIC_ENTRY_BYTES;
    for index in 0..count {
        let offset = DYNAMIC_OFFSET + index * DYNAMIC_ENTRY_BYTES;
        if read_u64(bytes, offset).unwrap() == old {
            put_u64(bytes, offset, new);
            return;
        }
    }
    panic!("tag 0x{old:x} not found");
}

fn replace_value(bytes: &mut [u8], tag: u64, value: u64) {
    let count = read_u64(bytes, ELF_HEADER_BYTES + PROGRAM_HEADER_BYTES + 32).unwrap() as usize
        / DYNAMIC_ENTRY_BYTES;
    for index in 0..count {
        let offset = DYNAMIC_OFFSET + index * DYNAMIC_ENTRY_BYTES;
        if read_u64(bytes, offset).unwrap() == tag {
            put_u64(bytes, offset + 8, value);
            return;
        }
    }
    panic!("tag 0x{tag:x} not found");
}

#[test]
fn parses_a_closed_root_view_and_preserves_needed_order() {
    let bytes = fixture(
        &["libz.so.1", "libc.so.6"],
        None,
        Some("/lib64/ld-linux-x86-64.so.2"),
    );
    let view = parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).unwrap();
    assert_eq!(view.interpreter(), Some("/lib64/ld-linux-x86-64.so.2"));
    assert_eq!(view.needed(), ["libz.so.1", "libc.so.6"]);
    assert_eq!(view.soname(), None);
    assert_eq!(view.flags(), Some(DF_BIND_NOW));
    assert_eq!(view.flags_1(), Some(DF_1_NOW | DF_1_PIE));
    assert!(view.dynamic_tags().windows(2).all(|pair| pair[0] < pair[1]));
    let policy = runtime_elf_policy_sha256();
    assert_eq!(
        policy,
        "cd23f2d883c1e99b655395284e7d803e6d00b9eaf90a417560efca7ffde50b0a"
    );
    assert_eq!(policy.len(), 64);
    assert!(policy
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
}

#[test]
fn parses_a_shared_object_soname_without_an_interpreter() {
    let bytes = fixture(&["libc.so.6"], Some("libexample.so.1"), None);
    let view = parse_runtime_elf(&bytes, RuntimeElfRole::SharedObject).unwrap();
    assert_eq!(view.interpreter(), None);
    assert_eq!(view.soname(), Some("libexample.so.1"));
    assert_eq!(view.flags_1(), Some(DF_1_NOW));
}

#[test]
fn parses_only_the_exact_loader_role() {
    let view = parse_runtime_elf(&loader_fixture(), RuntimeElfRole::Loader).unwrap();
    assert_eq!(view.soname(), Some("ld-linux-x86-64.so.2"));
    assert!(view.needed().is_empty());
    assert!(parse_runtime_elf(&loader_fixture(), RuntimeElfRole::SharedObject).is_err());
}

#[test]
fn rejects_every_search_active_or_unknown_tag() {
    for tag in [
        15,
        16,
        17,
        18,
        19,
        22,
        24,
        29,
        32,
        33,
        34,
        0x6fff_fdfc,
        0x6fff_fdfd,
        0x6fff_fefa,
        0x6fff_fefb,
        0x6fff_fefc,
        0x6fff_fef6,
        0x6fff_fef7,
        0x7fff_fffd,
        0x7fff_ffff,
        0x1234_5678,
    ] {
        let mut bytes = root_fixture();
        replace_tag(&mut bytes, 0x6fff_fef5, tag);
        let error = parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).unwrap_err();
        assert!(
            error.contains("prohibited or unknown"),
            "tag 0x{tag:x}: {error}"
        );
    }
}

#[test]
fn rejects_resolution_changing_flags() {
    for bit in 0..64 {
        let value = DF_BIND_NOW ^ (1u64 << bit);
        let mut bytes = root_fixture();
        replace_value(&mut bytes, DT_FLAGS, value);
        assert!(
            parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).is_err(),
            "DT_FLAGS 0x{value:x}"
        );
    }
    for bit in 0..64 {
        let value = (DF_1_NOW | DF_1_PIE) ^ (1u64 << bit);
        let mut bytes = root_fixture();
        replace_value(&mut bytes, DT_FLAGS_1, value);
        assert!(
            parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).is_err(),
            "DT_FLAGS_1 0x{value:x}"
        );
    }
}

#[test]
fn rejects_unsafe_needed_names_before_execution() {
    let _ = root_fixture();
    for (name, reason) in [
        ("/tmp/lib.so", "anchored"),
        ("$ORIGIN.so", "anchored"),
        ("lib evil.so", "prohibited byte"),
        ("../lib.so", "dot-dot"),
        ("lib..so", "dot-dot"),
        ("lib\n.so", "prohibited byte"),
    ] {
        let bytes = fixture(&[name], None, Some(INTERPRETER));
        let error = parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).unwrap_err();
        assert!(error.contains(reason), "{name:?}: {error}");
    }
    let long = format!("lib{}.so", "a".repeat(128));
    let error = parse_runtime_elf(
        &fixture(&[&long], None, Some(INTERPRETER)),
        RuntimeElfRole::RootPie,
    )
    .unwrap_err();
    assert!(error.contains("outside bounds"), "{error}");
}

#[test]
fn rejects_duplicate_names_tags_and_missing_termination() {
    assert!(parse_runtime_elf(
        &fixture(
            &["libc.so.6", "libc.so.6"],
            None,
            Some("/lib64/ld-linux-x86-64.so.2")
        ),
        RuntimeElfRole::RootPie
    )
    .is_err());
    let mut duplicate = root_fixture();
    replace_tag(&mut duplicate, 0x6fff_fef5, DT_STRSZ);
    assert!(parse_runtime_elf(&duplicate, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("unique DT_STRSZ"));

    let mut missing_null = root_fixture();
    let count = read_u64(&missing_null, ELF_HEADER_BYTES + PROGRAM_HEADER_BYTES + 32).unwrap()
        as usize
        / DYNAMIC_ENTRY_BYTES;
    put_u64(
        &mut missing_null,
        DYNAMIC_OFFSET + (count - 2) * DYNAMIC_ENTRY_BYTES,
        12,
    );
    put_u64(
        &mut missing_null,
        DYNAMIC_OFFSET + (count - 2) * DYNAMIC_ENTRY_BYTES + 8,
        BASE,
    );
    put_u64(
        &mut missing_null,
        DYNAMIC_OFFSET + (count - 1) * DYNAMIC_ENTRY_BYTES,
        13,
    );
    put_u64(
        &mut missing_null,
        DYNAMIC_OFFSET + (count - 1) * DYNAMIC_ENTRY_BYTES + 8,
        BASE,
    );
    assert!(parse_runtime_elf(&missing_null, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("no DT_NULL terminator"));

    let mut after_null = root_fixture();
    let count = read_u64(&after_null, ELF_HEADER_BYTES + PROGRAM_HEADER_BYTES + 32).unwrap()
        as usize
        / DYNAMIC_ENTRY_BYTES;
    put_u64(
        &mut after_null,
        DYNAMIC_OFFSET + (count - 1) * DYNAMIC_ENTRY_BYTES + 8,
        1,
    );
    assert!(parse_runtime_elf(&after_null, RuntimeElfRole::RootPie).is_err());
}

#[test]
fn rejects_unmapped_strings_and_noncanonical_interpreters() {
    let mut unmapped = root_fixture();
    replace_value(&mut unmapped, DT_STRTAB, BASE + 0x900);
    assert!(parse_runtime_elf(&unmapped, RuntimeElfRole::RootPie).is_err());

    for interpreter in ["lib64/ld.so", "/lib64/../ld.so", "/lib64//ld.so"] {
        assert!(parse_runtime_elf(
            &fixture(&["libc.so.6"], None, Some(interpreter)),
            RuntimeElfRole::RootPie
        )
        .is_err());
    }
}

#[test]
fn rejects_mid_string_needed_offsets_and_incomplete_tuples() {
    let mut middle = fixture(&["libc.so.6"], None, Some("/lib64/ld-linux-x86-64.so.2"));
    let needed_offset = (DYNAMIC_OFFSET..STRINGS_OFFSET)
        .step_by(DYNAMIC_ENTRY_BYTES)
        .find(|offset| read_u64(&middle, *offset).unwrap() == DT_NEEDED)
        .unwrap();
    let value = read_u64(&middle, needed_offset + 8).unwrap();
    put_u64(&mut middle, needed_offset + 8, value + 1);
    assert!(parse_runtime_elf(&middle, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("does not start at a string boundary"));

    let mut incomplete = fixture(&["libc.so.6"], None, Some("/lib64/ld-linux-x86-64.so.2"));
    replace_tag(&mut incomplete, 9, 12);
    replace_value(&mut incomplete, 12, BASE);
    assert!(parse_runtime_elf(&incomplete, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("relocation tuple is incomplete"));
}

#[test]
fn scalar_readers_reject_offset_overflow_without_panicking() {
    for error in [
        read_u16(&[], usize::MAX).unwrap_err(),
        read_u32(&[], usize::MAX).unwrap_err(),
        read_u64(&[], usize::MAX).unwrap_err(),
    ] {
        assert!(error.contains("range overflow"), "{error}");
    }
}

mod policy;
mod program;
mod versioning;
