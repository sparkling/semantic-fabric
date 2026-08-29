use super::{
    interpreter_string, map_file_range, program_policy, read_u16, read_u32, read_u64, set_once,
    slice, RuntimeElfRole, Segment, ELF_HEADER_BYTES, MAX_ELF_BYTES, MAX_PROGRAM_HEADERS, PF_W,
    PF_X, PROGRAM_HEADER_BYTES, PT_DYNAMIC, PT_GNU_EH_FRAME, PT_GNU_PROPERTY, PT_GNU_RELRO,
    PT_GNU_STACK, PT_INTERP, PT_LOAD, PT_NOTE, PT_PHDR, PT_TLS,
};

pub(super) fn parse(
    bytes: &[u8],
    role: RuntimeElfRole,
) -> Result<(Vec<Segment>, Segment, Option<String>), String> {
    validate_header(bytes, role)?;
    let table_offset = read_u64(bytes, 32)?;
    let count = read_u16(bytes, 56)? as usize;
    if !(1..=MAX_PROGRAM_HEADERS).contains(&count) {
        return Err("ELF program-header count is outside bounds".to_owned());
    }
    let table_size = count
        .checked_mul(PROGRAM_HEADER_BYTES)
        .ok_or_else(|| "ELF program-header size overflow".to_owned())?;
    let table = slice(
        bytes,
        table_offset,
        table_size as u64,
        "ELF program headers",
    )?;
    let mut loads = Vec::new();
    let mut dynamic = None;
    let mut interpreter_segment = None;
    let mut tls = None;
    let mut stack = None;
    let mut relro = None;
    let mut phdr = None;
    let mut eh_frame = None;
    let mut property = None;
    let mut notes = Vec::new();
    for header in table.chunks_exact(PROGRAM_HEADER_BYTES) {
        let kind = read_u32(header, 0)?;
        let flags = read_u32(header, 4)?;
        let alignment = read_u64(header, 48)?;
        let segment = Segment {
            offset: read_u64(header, 8)?,
            virtual_address: read_u64(header, 16)?,
            file_size: read_u64(header, 32)?,
            memory_size: read_u64(header, 40)?,
            flags,
            alignment,
        };
        validate_segment(bytes, segment)?;
        match kind {
            PT_LOAD => {
                if flags & (PF_X | PF_W) == PF_X | PF_W {
                    return Err("ELF has a writable and executable load segment".to_owned());
                }
                loads.push(segment);
            }
            PT_DYNAMIC => set_once(&mut dynamic, segment, "PT_DYNAMIC")?,
            PT_INTERP => set_once(&mut interpreter_segment, segment, "PT_INTERP")?,
            PT_NOTE => {
                if notes.len() == 4 {
                    return Err("ELF PT_NOTE count exceeds the closed bound".to_owned());
                }
                notes.push(segment);
            }
            PT_PHDR => set_once(&mut phdr, segment, "PT_PHDR")?,
            PT_TLS => set_once(&mut tls, segment, "PT_TLS")?,
            PT_GNU_EH_FRAME => set_once(&mut eh_frame, segment, "PT_GNU_EH_FRAME")?,
            PT_GNU_STACK => {
                if flags & PF_X != 0 {
                    return Err("ELF requests an executable PT_GNU_STACK".to_owned());
                }
                set_once(&mut stack, segment, "PT_GNU_STACK")?;
            }
            PT_GNU_RELRO => set_once(&mut relro, segment, "PT_GNU_RELRO")?,
            PT_GNU_PROPERTY => set_once(&mut property, segment, "PT_GNU_PROPERTY")?,
            _ => return Err(format!("ELF program-header type 0x{kind:x} is unknown")),
        }
    }
    if loads.is_empty() {
        return Err("ELF has no PT_LOAD segment".to_owned());
    }
    let dynamic = dynamic.ok_or_else(|| "ELF has no unique PT_DYNAMIC segment".to_owned())?;
    program_policy::validate(program_policy::Profile {
        loads: &loads,
        dynamic,
        interpreter: interpreter_segment,
        stack,
        relro,
        tls,
        phdr,
        eh_frame,
        property,
        notes: &notes,
        program_header_offset: table_offset,
        program_header_size: table_size as u64,
    })?;
    let interpreter = interpreter_segment
        .map(|segment| {
            if map_file_range(&loads, segment.virtual_address, segment.file_size)? != segment.offset
            {
                return Err("ELF PT_INTERP mapping does not match its file offset".to_owned());
            }
            interpreter_string(slice(
                bytes,
                segment.offset,
                segment.file_size,
                "ELF PT_INTERP",
            )?)
        })
        .transpose()?;
    Ok((loads, dynamic, interpreter))
}

fn validate_header(bytes: &[u8], role: RuntimeElfRole) -> Result<(), String> {
    let os_abi = bytes.get(7).copied();
    let allowed_os_abi = match role {
        RuntimeElfRole::RootPie => os_abi == Some(0),
        RuntimeElfRole::Loader | RuntimeElfRole::Libc | RuntimeElfRole::SharedObject => {
            matches!(os_abi, Some(0 | 3))
        }
    };
    if !(ELF_HEADER_BYTES..=MAX_ELF_BYTES).contains(&bytes.len())
        || bytes.get(0..4) != Some(b"\x7fELF")
        || bytes.get(4) != Some(&2)
        || bytes.get(5) != Some(&1)
        || bytes.get(6) != Some(&1)
        || !allowed_os_abi
        || bytes.get(8) != Some(&0)
        || bytes.get(9..16) != Some(&[0; 7])
        || read_u16(bytes, 16)? != 3
        || read_u16(bytes, 18)? != 62
        || read_u32(bytes, 20)? != 1
        || read_u16(bytes, 52)? as usize != ELF_HEADER_BYTES
        || read_u16(bytes, 54)? as usize != PROGRAM_HEADER_BYTES
    {
        Err("ELF header is not the role-specific ELF64/little/x86-64 form".to_owned())
    } else {
        Ok(())
    }
}

fn validate_segment(bytes: &[u8], segment: Segment) -> Result<(), String> {
    if segment.file_size > segment.memory_size {
        return Err("ELF segment file size exceeds memory size".to_owned());
    }
    let _ = slice(bytes, segment.offset, segment.file_size, "ELF segment")?;
    if segment.alignment > 1
        && (!segment.alignment.is_power_of_two()
            || segment.offset % segment.alignment != segment.virtual_address % segment.alignment)
    {
        return Err("ELF segment alignment is invalid".to_owned());
    }
    Ok(())
}
