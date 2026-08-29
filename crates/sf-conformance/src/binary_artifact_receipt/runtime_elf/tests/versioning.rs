use super::*;

const DT_VERDEF: u64 = 0x6fff_fffc;
const DT_VERDEFNUM: u64 = 0x6fff_fffd;
const DT_VERNEED: u64 = 0x6fff_fffe;
const DT_VERNEEDNUM: u64 = 0x6fff_ffff;
const DT_VERSYM: u64 = 0x6fff_fff0;

fn string_offset(bytes: &[u8], value: &[u8]) -> u32 {
    bytes[STRINGS_OFFSET..]
        .windows(value.len() + 1)
        .position(|window| window.starts_with(value) && window[value.len()] == 0)
        .unwrap() as u32
}

fn multi_verneed_fixture() -> Vec<u8> {
    let mut bytes = fixture(&["libc.so.6", "libm.so.6"], None, Some(INTERPRETER));
    let libc = string_offset(&bytes, b"libc.so.6");
    let libm = string_offset(&bytes, b"libm.so.6");
    let name = string_offset(&bytes, b"GLIBC_2.2.5");
    let hash = super::super::versioning::elf_hash(b"GLIBC_2.2.5");

    put_u16(&mut bytes, 0x502, 2);
    put_u32(&mut bytes, 0x504, libc);
    put_u32(&mut bytes, 0x508, 16);
    put_u32(&mut bytes, 0x50c, 0x30);
    put_u32(&mut bytes, 0x510, hash);
    put_u16(&mut bytes, 0x516, 2);
    put_u32(&mut bytes, 0x518, name);
    put_u32(&mut bytes, 0x51c, 16);
    put_u32(&mut bytes, 0x520, hash);
    put_u16(&mut bytes, 0x526, 3);
    put_u32(&mut bytes, 0x528, name);
    put_u32(&mut bytes, 0x52c, 0);

    put_u16(&mut bytes, 0x530, 1);
    put_u16(&mut bytes, 0x532, 1);
    put_u32(&mut bytes, 0x534, libm);
    put_u32(&mut bytes, 0x538, 16);
    put_u32(&mut bytes, 0x53c, 0);
    put_u32(&mut bytes, 0x540, hash);
    put_u16(&mut bytes, 0x546, 4);
    put_u32(&mut bytes, 0x548, name);
    put_u32(&mut bytes, 0x54c, 0);
    replace_value(&mut bytes, DT_VERNEEDNUM, 2);
    bytes
}

fn multi_verdef_fixture() -> Vec<u8> {
    let mut bytes = loader_fixture();
    let name = string_offset(&bytes, b"GLIBC_2.2.5");
    let hash = super::super::versioning::elf_hash(b"GLIBC_2.2.5");

    put_u16(&mut bytes, 0x506, 2);
    put_u32(&mut bytes, 0x508, hash);
    put_u32(&mut bytes, 0x50c, 20);
    put_u32(&mut bytes, 0x510, 0x30);
    put_u32(&mut bytes, 0x514, name);
    put_u32(&mut bytes, 0x518, 8);
    put_u32(&mut bytes, 0x51c, name);
    put_u32(&mut bytes, 0x520, 0);

    put_u16(&mut bytes, 0x530, 1);
    put_u16(&mut bytes, 0x534, 2);
    put_u16(&mut bytes, 0x536, 1);
    put_u32(&mut bytes, 0x538, hash);
    put_u32(&mut bytes, 0x53c, 20);
    put_u32(&mut bytes, 0x540, 0);
    put_u32(&mut bytes, 0x544, name);
    put_u32(&mut bytes, 0x548, 0);
    replace_value(&mut bytes, DT_VERDEFNUM, 2);
    bytes
}

fn combined_version_fixture() -> Vec<u8> {
    let mut bytes = libc_fixture();
    let name = string_offset(&bytes, b"GLIBC_2.2.5");
    let hash = super::super::versioning::elf_hash(b"GLIBC_2.2.5");

    replace_tag(&mut bytes, 4, DT_VERDEF);
    replace_tag(&mut bytes, DT_VERSYM, DT_VERDEFNUM);
    replace_value(&mut bytes, DT_VERDEFNUM, 1);
    put_u16(&mut bytes, 0x520, 1);
    put_u16(&mut bytes, 0x522, 1);
    put_u16(&mut bytes, 0x524, 3);
    put_u16(&mut bytes, 0x526, 1);
    put_u32(&mut bytes, 0x528, hash);
    put_u32(&mut bytes, 0x52c, 20);
    put_u32(&mut bytes, 0x530, 0);
    put_u32(&mut bytes, 0x534, name);
    put_u32(&mut bytes, 0x538, 0);
    bytes
}

#[test]
fn rejects_malformed_verneed_records_and_auxiliaries() {
    let mut version = root_fixture();
    put_u16(&mut version, 0x500, 2);
    assert!(parse_runtime_elf(&version, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("VERNEED"));

    let mut count = root_fixture();
    replace_value(&mut count, DT_VERNEEDNUM, 2);
    assert!(parse_runtime_elf(&count, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("chain length"));

    let mut auxiliary_next = root_fixture();
    put_u32(&mut auxiliary_next, 0x51c, 4);
    assert!(parse_runtime_elf(&auxiliary_next, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("VERNAUX"));

    let mut middle_file = root_fixture();
    put_u32(&mut middle_file, 0x504, 2);
    assert!(parse_runtime_elf(&middle_file, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("string boundary"));

    let mut bad_index = root_fixture();
    put_u16(&mut bad_index, 0x516, 1);
    assert!(parse_runtime_elf(&bad_index, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("flags or index"));

    let mut bad_hash = root_fixture();
    put_u32(&mut bad_hash, 0x510, 1);
    assert!(parse_runtime_elf(&bad_hash, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("hash does not match"));
}

#[test]
fn rejects_unmapped_or_unbounded_version_tables() {
    let mut unmapped = root_fixture();
    replace_value(&mut unmapped, DT_VERNEED, BASE + 0x900);
    assert!(parse_runtime_elf(&unmapped, RuntimeElfRole::RootPie).is_err());

    let mut unbounded = root_fixture();
    replace_value(&mut unbounded, DT_VERNEEDNUM, 257);
    assert!(parse_runtime_elf(&unbounded, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("outside bounds"));

    let mut incomplete = root_fixture();
    replace_tag(&mut incomplete, DT_VERNEEDNUM, 12);
    replace_value(&mut incomplete, 12, BASE);
    assert!(parse_runtime_elf(&incomplete, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("version-table dynamic-tag pair is incomplete"));
}

#[test]
fn rejects_malformed_verdef_records_and_auxiliaries() {
    let mut version = loader_fixture();
    put_u16(&mut version, 0x500, 2);
    assert!(parse_runtime_elf(&version, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("VERDEF"));

    let mut duplicate_index = loader_fixture();
    replace_value(&mut duplicate_index, DT_VERDEFNUM, 2);
    assert!(parse_runtime_elf(&duplicate_index, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("chain length"));

    let mut bad_auxiliary = loader_fixture();
    put_u32(&mut bad_auxiliary, 0x518, 4);
    assert!(parse_runtime_elf(&bad_auxiliary, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("VERDAUX"));

    let mut bad_hash = loader_fixture();
    put_u32(&mut bad_hash, 0x508, 1);
    assert!(parse_runtime_elf(&bad_hash, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("hash does not match"));

    let mut missing_count = loader_fixture();
    replace_tag(&mut missing_count, DT_VERDEFNUM, 4);
    replace_value(&mut missing_count, 4, BASE);
    assert!(parse_runtime_elf(&missing_count, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("version-table dynamic-tag pair is incomplete"));

    let mut missing_address = loader_fixture();
    replace_tag(&mut missing_address, DT_VERDEF, 4);
    replace_value(&mut missing_address, 4, BASE);
    assert!(parse_runtime_elf(&missing_address, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("version-table dynamic-tag pair is incomplete"));
}

#[test]
fn accepts_multi_record_and_multi_auxiliary_version_chains() {
    parse_runtime_elf(&multi_verneed_fixture(), RuntimeElfRole::RootPie).unwrap();
    parse_runtime_elf(&multi_verdef_fixture(), RuntimeElfRole::Loader).unwrap();
    parse_runtime_elf(&combined_version_fixture(), RuntimeElfRole::Libc).unwrap();
}

#[test]
fn rejects_overlapping_or_duplicate_version_authority() {
    let mut overlapping_need = multi_verneed_fixture();
    put_u32(&mut overlapping_need, 0x50c, 4);
    assert!(
        parse_runtime_elf(&overlapping_need, RuntimeElfRole::RootPie)
            .unwrap_err()
            .contains("overlaps")
    );

    let mut shared_auxiliary = multi_verneed_fixture();
    put_u16(&mut shared_auxiliary, 0x502, 3);
    put_u32(&mut shared_auxiliary, 0x52c, 16);
    let error = parse_runtime_elf(&shared_auxiliary, RuntimeElfRole::RootPie).unwrap_err();
    assert!(error.contains("overlaps"), "{error}");

    let mut duplicate_file = multi_verneed_fixture();
    let first_file = read_u32(&duplicate_file, 0x504).unwrap();
    put_u32(&mut duplicate_file, 0x534, first_file);
    assert!(parse_runtime_elf(&duplicate_file, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("repeats a dependency file"));

    let mut duplicate_index = multi_verneed_fixture();
    put_u16(&mut duplicate_index, 0x546, 2);
    assert!(parse_runtime_elf(&duplicate_index, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("version index is duplicated"));

    let mut overlapping_def = multi_verdef_fixture();
    put_u32(&mut overlapping_def, 0x510, 4);
    assert!(parse_runtime_elf(&overlapping_def, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("overlaps"));

    let mut duplicate_definition = multi_verdef_fixture();
    put_u16(&mut duplicate_definition, 0x534, 1);
    assert!(
        parse_runtime_elf(&duplicate_definition, RuntimeElfRole::Loader)
            .unwrap_err()
            .contains("version index is duplicated")
    );

    let mut cross_table_overlap = combined_version_fixture();
    replace_value(&mut cross_table_overlap, DT_VERDEF, BASE + 0x500);
    assert!(
        parse_runtime_elf(&cross_table_overlap, RuntimeElfRole::Libc)
            .unwrap_err()
            .contains("overlaps")
    );

    let mut cross_table_index = combined_version_fixture();
    put_u16(&mut cross_table_index, 0x524, 2);
    assert!(parse_runtime_elf(&cross_table_index, RuntimeElfRole::Libc)
        .unwrap_err()
        .contains("version index is duplicated"));
}
