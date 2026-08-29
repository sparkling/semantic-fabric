use super::*;

#[test]
fn rejects_wrong_headers_wx_loads_and_truncation() {
    let mut wrong_machine = root_fixture();
    put_u16(&mut wrong_machine, 18, 3);
    assert!(parse_runtime_elf(&wrong_machine, RuntimeElfRole::RootPie).is_err());

    let mut writable_executable = root_fixture();
    put_u32(&mut writable_executable, ELF_HEADER_BYTES + 4, PF_W | PF_X);
    assert!(parse_runtime_elf(&writable_executable, RuntimeElfRole::RootPie).is_err());

    let mut memory_tail = root_fixture();
    let file_size = read_u64(&memory_tail, ELF_HEADER_BYTES + 32).unwrap();
    put_u64(&mut memory_tail, ELF_HEADER_BYTES + 40, file_size + 0x1000);
    parse_runtime_elf(&memory_tail, RuntimeElfRole::RootPie).unwrap();

    let mut file_larger_than_memory = root_fixture();
    put_u64(
        &mut file_larger_than_memory,
        ELF_HEADER_BYTES + 40,
        file_size - 1,
    );
    assert!(
        parse_runtime_elf(&file_larger_than_memory, RuntimeElfRole::RootPie)
            .unwrap_err()
            .contains("file size exceeds memory size")
    );

    let mut executable_stack = root_fixture();
    let interp_header = ELF_HEADER_BYTES + 2 * PROGRAM_HEADER_BYTES;
    let stack_header = ELF_HEADER_BYTES + 3 * PROGRAM_HEADER_BYTES;
    put_u32(&mut executable_stack, stack_header + 4, PF_R | PF_W | PF_X);
    assert!(
        parse_runtime_elf(&executable_stack, RuntimeElfRole::RootPie)
            .unwrap_err()
            .contains("executable PT_GNU_STACK")
    );

    let mut mismapped_interpreter = fixture(&["libc.so.6"], None, Some(INTERPRETER));
    put_u64(
        &mut mismapped_interpreter,
        interp_header + 16,
        BASE + INTERP_OFFSET as u64 + 1,
    );
    assert!(
        parse_runtime_elf(&mismapped_interpreter, RuntimeElfRole::RootPie)
            .unwrap_err()
            .contains("PT_INTERP")
    );

    let valid = root_fixture();
    assert!(parse_runtime_elf(&valid[..100], RuntimeElfRole::RootPie).is_err());
}

#[test]
fn accepts_gnu_osabi_and_aligned_interpreter_for_libc_only() {
    let interpreter_header = ELF_HEADER_BYTES + 2 * PROGRAM_HEADER_BYTES;
    let mut current_libc = libc_fixture();
    current_libc[7] = 3;
    put_u64(&mut current_libc, interpreter_header + 48, 16);

    let view = parse_runtime_elf(&current_libc, RuntimeElfRole::Libc).unwrap();
    assert_eq!(view.interpreter(), Some(INTERPRETER));
    assert_eq!(view.soname(), Some("libc.so.6"));
    assert!(parse_runtime_elf(&current_libc, RuntimeElfRole::SharedObject).is_err());

    let mut forbidden_root_osabi = root_fixture();
    forbidden_root_osabi[7] = 3;
    assert!(
        parse_runtime_elf(&forbidden_root_osabi, RuntimeElfRole::RootPie)
            .unwrap_err()
            .contains("role-specific ELF64")
    );

    let mut unknown_osabi = current_libc.clone();
    unknown_osabi[7] = 4;
    assert!(parse_runtime_elf(&unknown_osabi, RuntimeElfRole::Libc)
        .unwrap_err()
        .contains("role-specific ELF64"));

    let mut unsupported_alignment = current_libc;
    put_u64(&mut unsupported_alignment, interpreter_header + 48, 8);
    assert!(
        parse_runtime_elf(&unsupported_alignment, RuntimeElfRole::Libc)
            .unwrap_err()
            .contains("PT_INTERP")
    );
}

#[test]
fn rejects_missing_duplicate_unknown_or_overlapping_program_headers() {
    let stack_index = 3;
    let relro_index = 4;

    let mut missing_stack = root_fixture();
    program_header(
        &mut missing_stack,
        stack_index,
        ph(
            PT_NOTE,
            PF_R,
            INTERP_OFFSET as u64,
            BASE + INTERP_OFFSET as u64,
            8,
            4,
        ),
    );
    assert!(parse_runtime_elf(&missing_stack, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("PT_GNU_STACK"));

    let mut missing_relro = root_fixture();
    program_header(
        &mut missing_relro,
        relro_index,
        ph(
            PT_NOTE,
            PF_R,
            INTERP_OFFSET as u64,
            BASE + INTERP_OFFSET as u64,
            8,
            4,
        ),
    );
    assert!(parse_runtime_elf(&missing_relro, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("PT_GNU_RELRO"));

    let mut duplicate_stack = root_fixture();
    program_header(
        &mut duplicate_stack,
        relro_index,
        ph(PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 16),
    );
    assert!(parse_runtime_elf(&duplicate_stack, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("duplicate PT_GNU_STACK"));

    let mut unknown = root_fixture();
    put_u32(
        &mut unknown,
        ELF_HEADER_BYTES + relro_index * PROGRAM_HEADER_BYTES,
        0x1234_5678,
    );
    assert!(parse_runtime_elf(&unknown, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("unknown"));

    let mut overlapping = root_fixture();
    program_header(
        &mut overlapping,
        relro_index,
        ph(
            PT_LOAD,
            PF_R,
            DYNAMIC_OFFSET as u64,
            BASE + DYNAMIC_OFFSET as u64,
            DYNAMIC_ENTRY_BYTES as u64,
            0x1000,
        ),
    );
    assert!(parse_runtime_elf(&overlapping, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("overlap"));
}

#[test]
fn phdr_must_describe_the_actual_program_header_table() {
    let mut valid = root_fixture();
    put_u16(&mut valid, 56, 6);
    program_header(
        &mut valid,
        5,
        ph(
            PT_PHDR,
            PF_R,
            ELF_HEADER_BYTES as u64,
            BASE + ELF_HEADER_BYTES as u64,
            (6 * PROGRAM_HEADER_BYTES) as u64,
            8,
        ),
    );
    parse_runtime_elf(&valid, RuntimeElfRole::RootPie).unwrap();

    let mut wrong_size = valid;
    put_u64(
        &mut wrong_size,
        ELF_HEADER_BYTES + 5 * PROGRAM_HEADER_BYTES + 32,
        (5 * PROGRAM_HEADER_BYTES) as u64,
    );
    put_u64(
        &mut wrong_size,
        ELF_HEADER_BYTES + 5 * PROGRAM_HEADER_BYTES + 40,
        (5 * PROGRAM_HEADER_BYTES) as u64,
    );
    assert!(parse_runtime_elf(&wrong_size, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("actual program-header table"));
}

#[test]
fn optional_program_headers_have_closed_mapped_shapes() {
    for (kind, alignment, label) in [
        (PT_TLS, 8, "PT_TLS"),
        (PT_GNU_EH_FRAME, 4, "PT_GNU_EH_FRAME"),
        (PT_GNU_PROPERTY, 8, "PT_GNU_PROPERTY"),
        (PT_NOTE, 4, "PT_NOTE"),
    ] {
        let mut valid = root_fixture();
        put_u16(&mut valid, 56, 6);
        program_header(
            &mut valid,
            5,
            ph(kind, PF_R, 0x700, BASE + 0x700, 8, alignment),
        );
        if kind == PT_TLS {
            put_u64(
                &mut valid,
                ELF_HEADER_BYTES + 5 * PROGRAM_HEADER_BYTES + 40,
                16,
            );
        }
        parse_runtime_elf(&valid, RuntimeElfRole::RootPie).unwrap();

        let mut wrong_flags = valid.clone();
        put_u32(
            &mut wrong_flags,
            ELF_HEADER_BYTES + 5 * PROGRAM_HEADER_BYTES + 4,
            PF_R | PF_W,
        );
        let error = parse_runtime_elf(&wrong_flags, RuntimeElfRole::RootPie).unwrap_err();
        assert!(error.contains(label), "{label}: {error}");

        let mut wrong_mapping = valid;
        put_u64(
            &mut wrong_mapping,
            ELF_HEADER_BYTES + 5 * PROGRAM_HEADER_BYTES + 8,
            0x710,
        );
        let error = parse_runtime_elf(&wrong_mapping, RuntimeElfRole::RootPie).unwrap_err();
        assert!(error.contains(label), "{label}: {error}");
    }
}

#[test]
fn relro_allows_a_memory_tail_but_requires_exact_load_mappings() {
    let relro_header = ELF_HEADER_BYTES + 4 * PROGRAM_HEADER_BYTES;
    let mut valid = root_fixture();
    let file_size = read_u64(&valid, relro_header + 32).unwrap();
    put_u64(&mut valid, relro_header + 40, file_size + 8);
    parse_runtime_elf(&valid, RuntimeElfRole::RootPie).unwrap();

    let mut wrong_file_mapping = valid.clone();
    put_u64(
        &mut wrong_file_mapping,
        relro_header + 8,
        DYNAMIC_OFFSET as u64 + 1,
    );
    assert!(
        parse_runtime_elf(&wrong_file_mapping, RuntimeElfRole::RootPie)
            .unwrap_err()
            .contains("PT_GNU_RELRO file mapping is inconsistent")
    );

    let mut outside_load = valid;
    put_u64(&mut outside_load, relro_header + 40, 0x1000);
    assert!(parse_runtime_elf(&outside_load, RuntimeElfRole::RootPie)
        .unwrap_err()
        .contains("one readable PT_LOAD mapping"));
}
