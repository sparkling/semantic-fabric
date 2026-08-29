use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{
    domain_sha256, fixed_metadata, sha256, validate_sha256, BwrapHostResolutionInventory,
    BwrapHostResolutionView, RecordedBwrapHostResolution, HEADER, INVENTORY_DOMAIN,
    STDOUT_CHUNK_BYTES,
};
use crate::binary_artifact_receipt::runtime_linkage::{
    parse_runtime_linkage_view, MAX_LOADER_OUTPUT_BYTES, MAX_RESOLVED_OBJECTS,
};

const MAX_INVENTORY_BYTES: usize = 2 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_METADATA: usize = 8 + super::NONCLAIM_KEYS.len();
const MAX_STDOUT_CHUNKS: usize = MAX_LOADER_OUTPUT_BYTES.div_ceil(STDOUT_CHUNK_BYTES);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Tool,
    RuntimeElf,
    DirectNeeded,
    Loader,
    Resolved,
    Virtual,
    Stdout,
    StdoutChunk,
}

pub(super) fn render(inventory: &BwrapHostResolutionInventory) -> Result<String, String> {
    let mut output = unsigned_inventory(inventory)?;
    let digest = domain_sha256(INVENTORY_DOMAIN, output.as_bytes());
    writeln!(output, "inventory-sha256\t{digest}").expect("String writes cannot fail");
    validate_text_shape(&output)?;
    Ok(output)
}

pub(super) fn unsigned_inventory(
    inventory: &BwrapHostResolutionInventory,
) -> Result<String, String> {
    inventory.validate()?;
    let record = &inventory.record;
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in fixed_metadata() {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    writeln!(
        output,
        "tool\tbubblewrap\t{}\t{}\t{}\t{}",
        record.bwrap_path,
        record.bwrap_byte_length,
        record.bwrap_sha256,
        record.bwrap_executable_policy
    )
    .expect("String writes cannot fail");
    writeln!(
        output,
        "runtime-elf\t{}\t{}\troot-pie\t{}",
        record.runtime_elf_policy,
        record.runtime_elf_policy_sha256,
        record.view.elf_interpreter()
    )
    .expect("String writes cannot fail");
    for needed in record.view.direct_needed() {
        writeln!(output, "direct-needed\t{needed}").expect("String writes cannot fail");
    }
    writeln!(output, "loader\t{}", record.view.loader_path()).expect("String writes cannot fail");
    for object in record.view.resolved_objects() {
        writeln!(
            output,
            "resolved\t{}\t{}",
            object.soname(),
            object.resolved_path()
        )
        .expect("String writes cannot fail");
    }
    for object in record.view.virtual_objects() {
        writeln!(output, "virtual\t{}", object.name()).expect("String writes cannot fail");
    }
    let chunk_count = record.stdout.len().div_ceil(STDOUT_CHUNK_BYTES);
    writeln!(
        output,
        "stdout\t{}\t{}\t{}\t{}",
        record.stdout.len(),
        record.stdout_sha256,
        chunk_count,
        STDOUT_CHUNK_BYTES
    )
    .expect("String writes cannot fail");
    for (index, chunk) in record.stdout.chunks(STDOUT_CHUNK_BYTES).enumerate() {
        writeln!(output, "stdout-chunk\t{index:08}\t{}", encode_hex(chunk))
            .expect("String writes cannot fail");
    }
    validate_text_shape(&output)?;
    Ok(output)
}

pub(super) fn parse(input: &str) -> Result<BwrapHostResolutionInventory, String> {
    validate_text_shape(input)?;
    if input.contains('\r') || !input.ends_with('\n') {
        return Err("bubblewrap inventory must use LF and end with one LF".to_owned());
    }
    let body = &input[..input.len() - 1];
    let trailer_start = body
        .rfind('\n')
        .ok_or_else(|| "bubblewrap inventory has no digest trailer".to_owned())?;
    let unsigned = &input[..=trailer_start];
    let trailer: Vec<_> = body[trailer_start + 1..].split('\t').collect();
    let expected_digest = match trailer.as_slice() {
        ["inventory-sha256", digest] => *digest,
        _ => return Err("bubblewrap inventory digest trailer is malformed".to_owned()),
    };
    validate_sha256("bubblewrap inventory", expected_digest)?;
    if domain_sha256(INVENTORY_DOMAIN, unsigned.as_bytes()) != expected_digest {
        return Err("bubblewrap inventory digest drift".to_owned());
    }

    let mut lines = unsigned.lines().enumerate();
    if lines.next().map(|(_, line)| line) != Some(HEADER) {
        return Err("invalid bubblewrap host-resolution inventory header".to_owned());
    }
    let mut metadata = BTreeMap::new();
    let mut stage = None;
    let mut tool = None;
    let mut runtime_elf = None;
    let mut direct_needed = Vec::new();
    let mut loader = None;
    let mut resolved = Vec::new();
    let mut virtual_objects = Vec::new();
    let mut stdout_meta = None;
    let mut stdout_chunks = Vec::new();
    for (index, line) in lines {
        let number = index + 1;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if stage.is_none() => {
                if metadata.len() == MAX_METADATA || metadata.insert(*key, *value).is_some() {
                    return Err(format!(
                        "line {number}: duplicate or excess inventory metadata"
                    ));
                }
            }
            ["tool", "bubblewrap", path, length, digest, policy] => {
                advance(&mut stage, Stage::Tool, number)?;
                if tool
                    .replace((
                        *path,
                        parse_u64(length, number, "bubblewrap byte length")?,
                        *digest,
                        *policy,
                    ))
                    .is_some()
                {
                    return Err(format!("line {number}: duplicate bubblewrap tool"));
                }
            }
            ["runtime-elf", policy, digest, "root-pie", interpreter] => {
                advance(&mut stage, Stage::RuntimeElf, number)?;
                if runtime_elf
                    .replace((*policy, *digest, *interpreter))
                    .is_some()
                {
                    return Err(format!("line {number}: duplicate runtime ELF record"));
                }
            }
            ["direct-needed", soname] => {
                advance(&mut stage, Stage::DirectNeeded, number)?;
                if direct_needed.len() == MAX_RESOLVED_OBJECTS {
                    return Err(format!("line {number}: too many direct-needed records"));
                }
                direct_needed.push((*soname).to_owned());
            }
            ["loader", path] => {
                advance(&mut stage, Stage::Loader, number)?;
                if loader.replace(*path).is_some() {
                    return Err(format!("line {number}: duplicate loader record"));
                }
            }
            ["resolved", soname, path] => {
                advance(&mut stage, Stage::Resolved, number)?;
                if resolved.len() == MAX_RESOLVED_OBJECTS {
                    return Err(format!("line {number}: too many resolved records"));
                }
                resolved.push(((*soname).to_owned(), (*path).to_owned()));
            }
            ["virtual", name] => {
                advance(&mut stage, Stage::Virtual, number)?;
                if virtual_objects.len() == MAX_RESOLVED_OBJECTS {
                    return Err(format!("line {number}: too many virtual records"));
                }
                virtual_objects.push((*name).to_owned());
            }
            ["stdout", length, digest, count, chunk_bytes] => {
                advance(&mut stage, Stage::Stdout, number)?;
                if *chunk_bytes != STDOUT_CHUNK_BYTES.to_string() {
                    return Err(format!("line {number}: stdout chunk policy drift"));
                }
                if stdout_meta
                    .replace((
                        parse_usize(length, number, "stdout byte length")?,
                        *digest,
                        parse_usize(count, number, "stdout chunk count")?,
                    ))
                    .is_some()
                {
                    return Err(format!("line {number}: duplicate stdout metadata"));
                }
            }
            ["stdout-chunk", chunk_index, encoded] => {
                advance(&mut stage, Stage::StdoutChunk, number)?;
                if stdout_chunks.len() == MAX_STDOUT_CHUNKS
                    || *chunk_index != format!("{:08}", stdout_chunks.len())
                {
                    return Err(format!("line {number}: stdout chunk index or count drift"));
                }
                let chunk = decode_hex(encoded, number)?;
                if chunk.is_empty() || chunk.len() > STDOUT_CHUNK_BYTES {
                    return Err(format!(
                        "line {number}: stdout chunk size is outside bounds"
                    ));
                }
                stdout_chunks.push(chunk);
            }
            ["meta", ..] => {
                return Err(format!("line {number}: metadata follows inventory records"));
            }
            _ => {
                return Err(format!(
                    "line {number}: unknown or malformed inventory record"
                ))
            }
        }
    }
    enforce_fixed_metadata(&mut metadata)?;
    if let Some(key) = metadata.keys().next() {
        return Err(format!("unknown bubblewrap inventory metadata key {key}"));
    }
    let (path, byte_length, bwrap_sha256, executable_policy) =
        tool.ok_or_else(|| "missing bubblewrap tool record".to_owned())?;
    let (runtime_policy, runtime_policy_sha256, interpreter) =
        runtime_elf.ok_or_else(|| "missing runtime ELF record".to_owned())?;
    let explicit_loader = loader.ok_or_else(|| "missing loader record".to_owned())?;
    let (stdout_length, stdout_sha256, stdout_count) =
        stdout_meta.ok_or_else(|| "missing stdout metadata".to_owned())?;
    let stdout = assemble_stdout(stdout_chunks, stdout_length, stdout_count, stdout_sha256)?;
    let replayed = parse_runtime_linkage_view(bwrap_sha256, interpreter, &direct_needed, &stdout)?;
    let replayed_resolved: Vec<_> = replayed
        .resolved_objects()
        .iter()
        .map(|object| {
            (
                object.soname().to_owned(),
                object.resolved_path().to_owned(),
            )
        })
        .collect();
    let replayed_virtual: Vec<_> = replayed
        .virtual_objects()
        .iter()
        .map(|object| object.name().to_owned())
        .collect();
    if explicit_loader != replayed.loader_path()
        || resolved != replayed_resolved
        || virtual_objects != replayed_virtual
    {
        return Err("recorded resolution names or paths differ from raw stdout".to_owned());
    }
    let inventory = BwrapHostResolutionInventory::from_recorded(RecordedBwrapHostResolution {
        bwrap_path: path.to_owned(),
        bwrap_sha256: bwrap_sha256.to_owned(),
        bwrap_byte_length: byte_length,
        bwrap_executable_policy: executable_policy.to_owned(),
        runtime_elf_policy: runtime_policy.to_owned(),
        runtime_elf_policy_sha256: runtime_policy_sha256.to_owned(),
        view: BwrapHostResolutionView::from_runtime(replayed),
        stdout,
        stdout_sha256: stdout_sha256.to_owned(),
    })?;
    if render(&inventory)? != input {
        return Err("bubblewrap inventory is not in canonical generated form".to_owned());
    }
    Ok(inventory)
}

fn enforce_fixed_metadata<'a>(metadata: &mut BTreeMap<&'a str, &'a str>) -> Result<(), String> {
    for (key, expected) in fixed_metadata() {
        let actual = metadata
            .remove(key)
            .ok_or_else(|| format!("missing bubblewrap inventory metadata key {key}"))?;
        if actual != expected {
            return Err(format!("bubblewrap inventory metadata {key} drift"));
        }
    }
    Ok(())
}

fn assemble_stdout(
    chunks: Vec<Vec<u8>>,
    expected_length: usize,
    expected_count: usize,
    expected_sha256: &str,
) -> Result<Vec<u8>, String> {
    if expected_length == 0
        || expected_length > MAX_LOADER_OUTPUT_BYTES
        || expected_count == 0
        || expected_count > MAX_STDOUT_CHUNKS
        || chunks.len() != expected_count
        || expected_count != expected_length.div_ceil(STDOUT_CHUNK_BYTES)
    {
        return Err("inventory stdout count or length is outside bounds".to_owned());
    }
    let mut stdout = Vec::with_capacity(expected_length);
    for (index, chunk) in chunks.into_iter().enumerate() {
        if index + 1 < expected_count && chunk.len() != STDOUT_CHUNK_BYTES {
            return Err("inventory has a short non-final stdout chunk".to_owned());
        }
        stdout.extend(chunk);
    }
    validate_sha256("inventory stdout", expected_sha256)?;
    if stdout.len() != expected_length || sha256(&stdout) != expected_sha256 {
        return Err("inventory stdout bytes differ from metadata".to_owned());
    }
    Ok(stdout)
}

fn advance(stage: &mut Option<Stage>, next: Stage, line: usize) -> Result<(), String> {
    if stage.is_some_and(|current| current > next) {
        return Err(format!(
            "line {line}: inventory records are not in canonical order"
        ));
    }
    *stage = Some(next);
    Ok(())
}

fn parse_u64(value: &str, line: usize, label: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("line {line}: invalid {label}"))
}

fn parse_usize(value: &str, line: usize, label: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("line {line}: invalid {label}"))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str, line: usize) -> Result<Vec<u8>, String> {
    if value.len() > STDOUT_CHUNK_BYTES * 2 || !value.len().is_multiple_of(2) {
        return Err(format!("line {line}: stdout chunk hex length is invalid"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|()| format!("line {line}: stdout chunk is not lowercase hexadecimal"))
}

fn hex_digit(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(()),
    }
}

fn validate_text_shape(input: &str) -> Result<(), String> {
    if input.is_empty() || input.len() > MAX_INVENTORY_BYTES {
        return Err("bubblewrap inventory size is outside bounds".to_owned());
    }
    if input.lines().any(|line| line.len() > MAX_LINE_BYTES) {
        return Err(format!(
            "bubblewrap inventory line exceeds {MAX_LINE_BYTES} bytes"
        ));
    }
    if input
        .bytes()
        .any(|byte| (byte.is_ascii_control() && !matches!(byte, b'\n' | b'\t')) || byte == 0x7f)
    {
        return Err("bubblewrap inventory contains a prohibited control byte".to_owned());
    }
    Ok(())
}
