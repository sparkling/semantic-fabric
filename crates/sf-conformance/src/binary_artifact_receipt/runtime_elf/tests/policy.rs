use super::*;

const DT_PLTRELSZ: u64 = 2;
const DT_PLTGOT: u64 = 3;
const DT_INIT_ARRAY: u64 = 25;
const DT_FINI_ARRAY: u64 = 26;
const DT_INIT_ARRAYSZ: u64 = 27;
const DT_FINI_ARRAYSZ: u64 = 28;
const DT_RELRSZ: u64 = 35;
const DT_RELR: u64 = 36;
const DT_RELRENT: u64 = 37;
const DT_RELACOUNT: u64 = 0x6fff_fff9;
const DT_X86_64_PLT: u64 = 0x7000_0000;
const DT_X86_64_PLTSZ: u64 = 0x7000_0001;
const DT_X86_64_PLTENT: u64 = 0x7000_0003;

fn append_dynamic(bytes: &mut [u8], tag: u64, value: u64) {
    let header = ELF_HEADER_BYTES + PROGRAM_HEADER_BYTES;
    let size = read_u64(bytes, header + 32).unwrap() as usize;
    let entries = size / DYNAMIC_ENTRY_BYTES;
    let index = (0..entries)
        .find(|index| {
            read_u64(bytes, DYNAMIC_OFFSET + index * DYNAMIC_ENTRY_BYTES).unwrap() == DT_NULL
        })
        .unwrap();
    let offset = DYNAMIC_OFFSET + index * DYNAMIC_ENTRY_BYTES;
    put_u64(bytes, offset, tag);
    put_u64(bytes, offset + 8, value);
    if index + 1 == entries {
        let expanded = (size + DYNAMIC_ENTRY_BYTES) as u64;
        put_u64(bytes, header + 32, expanded);
        put_u64(bytes, header + 40, expanded);
    }
}

fn add_plt(bytes: &mut [u8]) {
    append_dynamic(bytes, DT_PLTGOT, BASE + 0x700);
    append_dynamic(bytes, DT_PLTRELSZ, 24);
    append_dynamic(bytes, DT_PLTREL, 7);
    append_dynamic(bytes, 23, BASE + 0x700);
}

#[test]
fn accepts_and_bounds_rela_plt_relacount_and_arrays() {
    let mut bytes = root_fixture();
    add_plt(&mut bytes);
    append_dynamic(&mut bytes, DT_INIT_ARRAY, BASE + 0x700);
    append_dynamic(&mut bytes, DT_INIT_ARRAYSZ, 8);
    append_dynamic(&mut bytes, DT_FINI_ARRAY, BASE + 0x708);
    append_dynamic(&mut bytes, DT_FINI_ARRAYSZ, 8);
    append_dynamic(&mut bytes, DT_RELACOUNT, 1);
    parse_runtime_elf(&bytes, RuntimeElfRole::RootPie).unwrap();

    let mut bad_plt = bytes.clone();
    replace_value(&mut bad_plt, DT_PLTRELSZ, 25);
    assert!(parse_runtime_elf(&bad_plt, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("DT_PLTRELSZ"));

    let mut bad_array = bytes.clone();
    replace_value(&mut bad_array, DT_INIT_ARRAYSZ, 7);
    assert!(parse_runtime_elf(&bad_array, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("address/size"));

    let mut bad_count = bytes;
    replace_value(&mut bad_count, DT_RELACOUNT, 2);
    assert!(parse_runtime_elf(&bad_count, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("RELACOUNT"));
}

#[test]
fn accepts_and_bounds_relr_and_x86_plt_tuples() {
    let mut loader = loader_fixture();
    append_dynamic(&mut loader, DT_RELR, BASE + 0x700);
    append_dynamic(&mut loader, DT_RELRSZ, 8);
    append_dynamic(&mut loader, DT_RELRENT, 8);
    parse_runtime_elf(&loader, RuntimeElfRole::Loader).unwrap();

    let mut bad_relr = loader;
    replace_value(&mut bad_relr, DT_RELRSZ, 9);
    assert!(parse_runtime_elf(&bad_relr, RuntimeElfRole::Loader)
        .unwrap_err()
        .contains("relocation table size"));

    let mut shared = fixture(&["libc.so.6"], Some("libexample.so.1"), None);
    append_dynamic(&mut shared, DT_X86_64_PLT, BASE + 0x700);
    append_dynamic(&mut shared, DT_X86_64_PLTSZ, 16);
    append_dynamic(&mut shared, DT_X86_64_PLTENT, 16);
    parse_runtime_elf(&shared, RuntimeElfRole::SharedObject).unwrap();

    replace_value(&mut shared, DT_X86_64_PLTENT, 8);
    assert!(parse_runtime_elf(&shared, RuntimeElfRole::SharedObject)
        .unwrap_err()
        .contains("x86-64 PLT"));
}

#[test]
fn arbitrary_bounded_bytes_never_panic() {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for length in 0..256 {
        let mut bytes = vec![0; length];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
        for role in [
            RuntimeElfRole::RootPie,
            RuntimeElfRole::Loader,
            RuntimeElfRole::Libc,
            RuntimeElfRole::SharedObject,
        ] {
            assert!(std::panic::catch_unwind(|| parse_runtime_elf(&bytes, role)).is_ok());
        }
    }
}
