use super::{map_file_range, Segment, PF_R, PF_W};

pub(super) struct Profile<'a> {
    pub(super) loads: &'a [Segment],
    pub(super) dynamic: Segment,
    pub(super) interpreter: Option<Segment>,
    pub(super) stack: Option<Segment>,
    pub(super) relro: Option<Segment>,
    pub(super) tls: Option<Segment>,
    pub(super) phdr: Option<Segment>,
    pub(super) eh_frame: Option<Segment>,
    pub(super) property: Option<Segment>,
    pub(super) notes: &'a [Segment],
    pub(super) program_header_offset: u64,
    pub(super) program_header_size: u64,
}

pub(super) fn validate(profile: Profile<'_>) -> Result<(), String> {
    let loads = profile.loads;
    validate_loads(loads)?;
    validate_exact_file_segment(loads, profile.dynamic, PF_R | PF_W, 8, "PT_DYNAMIC")?;
    if profile.dynamic.file_size != profile.dynamic.memory_size {
        return Err("ELF PT_DYNAMIC file and memory sizes differ".to_owned());
    }
    if let Some(interpreter) = profile.interpreter {
        validate_exact_file_segment_any_alignment(loads, interpreter, PF_R, &[1, 16], "PT_INTERP")?;
    }
    let stack = profile
        .stack
        .ok_or_else(|| "ELF has no unique PT_GNU_STACK".to_owned())?;
    if stack.offset != 0
        || stack.virtual_address != 0
        || stack.file_size != 0
        || stack.memory_size != 0
        || stack.flags != PF_R | PF_W
        || !matches!(stack.alignment, 0 | 16)
    {
        return Err("ELF PT_GNU_STACK does not match the closed non-executable shape".to_owned());
    }
    let relro = profile
        .relro
        .ok_or_else(|| "ELF has no unique PT_GNU_RELRO".to_owned())?;
    if relro.file_size == 0 || relro.flags != PF_R || relro.alignment != 1 {
        return Err("ELF PT_GNU_RELRO does not match the closed shape".to_owned());
    }
    validate_memory_mapping(loads, relro)?;
    if map_file_range(loads, relro.virtual_address, relro.file_size)? != relro.offset {
        return Err("ELF PT_GNU_RELRO file mapping is inconsistent".to_owned());
    }
    if let Some(tls) = profile.tls {
        if tls.memory_size == 0
            || tls.flags != PF_R
            || tls.alignment < 8
            || !tls.alignment.is_power_of_two()
        {
            return Err("ELF PT_TLS does not match the closed shape".to_owned());
        }
        validate_memory_mapping(loads, tls)?;
        if tls.file_size > 0
            && map_file_range(loads, tls.virtual_address, tls.file_size)? != tls.offset
        {
            return Err("ELF PT_TLS file mapping is inconsistent".to_owned());
        }
    }
    if let Some(phdr) = profile.phdr {
        validate_exact_file_segment(loads, phdr, PF_R, 8, "PT_PHDR")?;
        if phdr.offset != profile.program_header_offset
            || phdr.file_size != profile.program_header_size
        {
            return Err("ELF PT_PHDR does not describe the actual program-header table".to_owned());
        }
    }
    if let Some(eh_frame) = profile.eh_frame {
        validate_exact_file_segment(loads, eh_frame, PF_R, 4, "PT_GNU_EH_FRAME")?;
    }
    if let Some(property) = profile.property {
        validate_exact_file_segment(loads, property, PF_R, 8, "PT_GNU_PROPERTY")?;
    }
    for note in profile.notes {
        validate_exact_file_segment_any_alignment(loads, *note, PF_R, &[4, 8], "PT_NOTE")?;
    }
    Ok(())
}

fn validate_loads(loads: &[Segment]) -> Result<(), String> {
    if loads.len() > 8 {
        return Err("ELF PT_LOAD count exceeds the closed bound".to_owned());
    }
    for load in loads {
        if load.file_size == 0
            || load.memory_size == 0
            || ![PF_R, PF_R | 1, PF_R | 2].contains(&load.flags)
            || load.alignment < 4096
            || !load.alignment.is_power_of_two()
        {
            return Err("ELF PT_LOAD does not match the closed shape".to_owned());
        }
    }
    let mut virtual_ranges: Vec<_> = loads
        .iter()
        .map(|load| {
            let end = load
                .virtual_address
                .checked_add(load.memory_size)
                .ok_or_else(|| "ELF PT_LOAD memory range overflow".to_owned())?;
            Ok((load.virtual_address, end))
        })
        .collect::<Result<_, String>>()?;
    virtual_ranges.sort_unstable();
    if virtual_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("ELF PT_LOAD memory ranges overlap".to_owned());
    }
    let mut file_ranges: Vec<_> = loads
        .iter()
        .filter(|load| load.file_size > 0)
        .map(|load| {
            let end = load
                .offset
                .checked_add(load.file_size)
                .ok_or_else(|| "ELF PT_LOAD file range overflow".to_owned())?;
            Ok((load.offset, end))
        })
        .collect::<Result<_, String>>()?;
    file_ranges.sort_unstable();
    if file_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("ELF PT_LOAD file ranges overlap".to_owned());
    }
    Ok(())
}

fn validate_exact_file_segment(
    loads: &[Segment],
    segment: Segment,
    flags: u32,
    alignment: u64,
    label: &str,
) -> Result<(), String> {
    validate_exact_file_segment_any_alignment(loads, segment, flags, &[alignment], label)
}

fn validate_exact_file_segment_any_alignment(
    loads: &[Segment],
    segment: Segment,
    flags: u32,
    alignments: &[u64],
    label: &str,
) -> Result<(), String> {
    if segment.file_size == 0
        || segment.file_size != segment.memory_size
        || segment.flags != flags
        || !alignments.contains(&segment.alignment)
        || map_file_range(loads, segment.virtual_address, segment.file_size)? != segment.offset
    {
        Err(format!(
            "ELF {label} does not match the closed mapped shape"
        ))
    } else {
        Ok(())
    }
}

fn validate_memory_mapping(loads: &[Segment], segment: Segment) -> Result<(), String> {
    let end = segment
        .virtual_address
        .checked_add(segment.memory_size)
        .ok_or_else(|| "ELF program memory range overflow".to_owned())?;
    let count = loads
        .iter()
        .filter(|load| {
            let load_end = load.virtual_address.checked_add(load.memory_size);
            load.flags & PF_R != 0
                && segment.virtual_address >= load.virtual_address
                && load_end.is_some_and(|load_end| end <= load_end)
        })
        .count();
    if count == 1 {
        Ok(())
    } else {
        Err("ELF program range does not have one readable PT_LOAD mapping".to_owned())
    }
}
