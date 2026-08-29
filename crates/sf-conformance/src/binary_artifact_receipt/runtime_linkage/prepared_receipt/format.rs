use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{
    domain_sha256, fixed_metadata, parse_role, role_name, sha256, validate_absolute_path,
    validate_sha256, PreparedRuntimeReceipt, RecordedBindingIdentity, RecordedObjectIdentity,
    RecordedPreparedRuntimeObservation, HEADER, RECEIPT_DOMAIN, STDOUT_CHUNK_BYTES,
};
use crate::binary_artifact_receipt::runtime_linkage::{
    parse_runtime_linkage_view, MAX_LOADER_OUTPUT_BYTES, MAX_RESOLVED_OBJECTS,
    MAX_RUNTIME_OBJECT_BYTES,
};

const MAX_PREPARED_RECEIPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_BINDINGS: usize = MAX_RESOLVED_OBJECTS + 2;
const MAX_METADATA: usize = 10 + super::NONCLAIM_KEYS.len() + 1;
const MAX_STDOUT_CHUNKS: usize = MAX_LOADER_OUTPUT_BYTES.div_ceil(STDOUT_CHUNK_BYTES);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    View,
    Tool,
    DirectNeeded,
    Binding,
    Stdout,
    StdoutChunk,
}

pub(in super::super) fn render(receipt: &PreparedRuntimeReceipt) -> Result<String, String> {
    let mut output = unsigned_receipt(receipt)?;
    let digest = domain_sha256(RECEIPT_DOMAIN, output.as_bytes());
    writeln!(output, "receipt-sha256\t{digest}").expect("String writes cannot fail");
    validate_text_shape(&output)?;
    Ok(output)
}

pub(super) fn unsigned_receipt(receipt: &PreparedRuntimeReceipt) -> Result<String, String> {
    receipt.validate()?;
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in fixed_metadata() {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    writeln!(
        output,
        "meta\tobservation-record-sha256\t{}",
        receipt.record_sha256()
    )
    .expect("String writes cannot fail");
    output.push_str(&observation_records(&receipt.record));
    validate_text_shape(&output)?;
    Ok(output)
}

pub(super) fn observation_records(record: &RecordedPreparedRuntimeObservation) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "view\t{}\t{}",
        record.view.artifact_sha256(),
        record.view.elf_interpreter()
    )
    .expect("String writes cannot fail");
    writeln!(
        output,
        "tool\tbubblewrap\t{}\t{}\t{}\t{}",
        record.bwrap_path,
        record.bwrap_byte_length,
        record.bwrap_sha256,
        record.bwrap_executable_policy
    )
    .expect("String writes cannot fail");
    for needed in record.view.direct_needed() {
        writeln!(output, "direct-needed\t{needed}").expect("String writes cannot fail");
    }
    for binding in &record.bindings {
        let (soname_kind, soname) = match binding.object.soname.as_deref() {
            Some(value) => ("some", value),
            None => ("none", ""),
        };
        writeln!(
            output,
            "binding\t{}\t{}\t{}\t{}\t{}\t{:04o}\t{}\t{}\t{}\t{}",
            role_name(binding.object.role),
            binding.object.logical_path,
            soname_kind,
            soname,
            binding.destination,
            binding.mode,
            binding.object.device,
            binding.object.inode,
            binding.object.byte_length,
            binding.object.sha256
        )
        .expect("String writes cannot fail");
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
    output
}

pub(in super::super) fn parse(input: &str) -> Result<PreparedRuntimeReceipt, String> {
    validate_text_shape(input)?;
    if input.contains('\r') {
        return Err("prepared receipt must use LF line endings".to_owned());
    }
    if !input.ends_with('\n') {
        return Err("prepared receipt must end with one LF".to_owned());
    }
    let without_final_lf = &input[..input.len() - 1];
    let trailer_start = without_final_lf
        .rfind('\n')
        .ok_or_else(|| "prepared receipt has no digest trailer".to_owned())?;
    let unsigned = &input[..=trailer_start];
    let trailer = &without_final_lf[trailer_start + 1..];
    let fields: Vec<_> = trailer.split('\t').collect();
    let expected_receipt_sha256 = match fields.as_slice() {
        ["receipt-sha256", digest] => *digest,
        _ => return Err("prepared receipt digest trailer is malformed".to_owned()),
    };
    validate_sha256("prepared receipt", expected_receipt_sha256)?;
    if domain_sha256(RECEIPT_DOMAIN, unsigned.as_bytes()) != expected_receipt_sha256 {
        return Err("prepared receipt digest drift".to_owned());
    }

    let mut lines = unsigned.lines().enumerate();
    if lines.next().map(|(_, line)| line) != Some(HEADER) {
        return Err("invalid prepared runtime observation header".to_owned());
    }
    let mut metadata = BTreeMap::new();
    let mut stage = None;
    let mut view = None;
    let mut tool = None;
    let mut direct_needed = Vec::new();
    let mut bindings = Vec::new();
    let mut stdout_meta = None;
    let mut stdout_chunks = Vec::new();
    for (index, line) in lines {
        let number = index + 1;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if stage.is_none() => {
                if metadata.len() == MAX_METADATA {
                    return Err(format!(
                        "line {number}: too many prepared receipt metadata records"
                    ));
                }
                if metadata.insert(*key, *value).is_some() {
                    return Err(format!(
                        "line {number}: duplicate prepared receipt metadata"
                    ));
                }
            }
            ["view", artifact_sha256, interpreter] => {
                advance(&mut stage, Stage::View, number)?;
                if view.replace((*artifact_sha256, *interpreter)).is_some() {
                    return Err(format!("line {number}: duplicate prepared receipt view"));
                }
            }
            ["tool", "bubblewrap", path, byte_length, digest, executable_policy] => {
                advance(&mut stage, Stage::Tool, number)?;
                let parsed = (
                    *path,
                    parse_u64(byte_length, number, "tool byte length")?,
                    *digest,
                    *executable_policy,
                );
                if tool.replace(parsed).is_some() {
                    return Err(format!("line {number}: duplicate prepared receipt tool"));
                }
            }
            ["direct-needed", soname] => {
                advance(&mut stage, Stage::DirectNeeded, number)?;
                if direct_needed.len() == MAX_RESOLVED_OBJECTS {
                    return Err(format!("line {number}: too many direct-needed records"));
                }
                direct_needed.push((*soname).to_owned());
            }
            ["binding", role, logical_path, soname_kind, soname, destination, mode, device, inode, byte_length, digest] =>
            {
                advance(&mut stage, Stage::Binding, number)?;
                if bindings.len() == MAX_BINDINGS {
                    return Err(format!("line {number}: too many prepared bindings"));
                }
                bindings.push(parse_binding(
                    role,
                    logical_path,
                    soname_kind,
                    soname,
                    destination,
                    mode,
                    device,
                    inode,
                    byte_length,
                    digest,
                    number,
                )?);
            }
            ["stdout", byte_length, digest, chunk_count, chunk_bytes] => {
                advance(&mut stage, Stage::Stdout, number)?;
                if *chunk_bytes != STDOUT_CHUNK_BYTES.to_string() {
                    return Err(format!("line {number}: stdout chunk policy drift"));
                }
                let parsed = (
                    parse_usize(byte_length, number, "stdout byte length")?,
                    *digest,
                    parse_usize(chunk_count, number, "stdout chunk count")?,
                );
                if stdout_meta.replace(parsed).is_some() {
                    return Err(format!("line {number}: duplicate stdout metadata"));
                }
            }
            ["stdout-chunk", chunk_index, encoded] => {
                advance(&mut stage, Stage::StdoutChunk, number)?;
                if stdout_chunks.len() == MAX_STDOUT_CHUNKS {
                    return Err(format!("line {number}: too many stdout chunks"));
                }
                let expected_index = format!("{:08}", stdout_chunks.len());
                if *chunk_index != expected_index {
                    return Err(format!("line {number}: stdout chunk index drift"));
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
                return Err(format!(
                    "line {number}: metadata follows observation records"
                ));
            }
            _ => {
                return Err(format!(
                    "line {number}: unknown or malformed receipt record"
                ))
            }
        }
    }
    enforce_fixed_metadata(&mut metadata)?;
    let expected_record_sha256 = take(&mut metadata, "observation-record-sha256")?;
    validate_sha256("prepared observation record", expected_record_sha256)?;
    if let Some(key) = metadata.keys().next() {
        return Err(format!("unknown prepared receipt metadata key {key}"));
    }
    let (artifact_sha256, interpreter) =
        view.ok_or_else(|| "missing prepared receipt view".to_owned())?;
    let (bwrap_path, bwrap_byte_length, bwrap_sha256, bwrap_executable_policy) =
        tool.ok_or_else(|| "missing prepared receipt tool".to_owned())?;
    let (stdout_byte_length, stdout_sha256, stdout_chunk_count) =
        stdout_meta.ok_or_else(|| "missing prepared receipt stdout metadata".to_owned())?;
    let stdout = assemble_stdout(
        stdout_chunks,
        stdout_byte_length,
        stdout_chunk_count,
        stdout_sha256,
    )?;
    let view = parse_runtime_linkage_view(artifact_sha256, interpreter, &direct_needed, &stdout)?;
    let receipt = PreparedRuntimeReceipt::from_recorded(RecordedPreparedRuntimeObservation {
        view,
        bindings,
        bwrap_sha256: bwrap_sha256.to_owned(),
        bwrap_byte_length,
        bwrap_path: bwrap_path.to_owned(),
        bwrap_executable_policy: bwrap_executable_policy.to_owned(),
        stdout,
        stdout_sha256: stdout_sha256.to_owned(),
    })?;
    if receipt.record_sha256() != expected_record_sha256 {
        return Err("prepared observation record digest drift".to_owned());
    }
    if render(&receipt)? != input {
        return Err("prepared receipt is not in canonical generated form".to_owned());
    }
    Ok(receipt)
}

#[allow(clippy::too_many_arguments)]
fn parse_binding(
    role: &str,
    logical_path: &str,
    soname_kind: &str,
    soname: &str,
    destination: &str,
    mode: &str,
    device: &str,
    inode: &str,
    byte_length: &str,
    digest: &str,
    line: usize,
) -> Result<RecordedBindingIdentity, String> {
    let role =
        parse_role(role).ok_or_else(|| format!("line {line}: unknown prepared binding role"))?;
    let mode = match mode {
        "0444" => 0o444,
        "0555" => 0o555,
        _ => return Err(format!("line {line}: prepared binding mode drift")),
    };
    let soname = match (soname_kind, soname) {
        ("none", "") => None,
        ("some", value) => Some(value.to_owned()),
        _ => {
            return Err(format!(
                "line {line}: prepared binding SONAME option is malformed"
            ))
        }
    };
    validate_absolute_path(logical_path, "receipt object path")?;
    validate_absolute_path(destination, "receipt binding destination")?;
    let identity = RecordedObjectIdentity {
        logical_path: logical_path.to_owned(),
        role,
        soname,
        device: parse_u64(device, line, "binding device")?,
        inode: parse_u64(inode, line, "binding inode")?,
        byte_length: parse_u64(byte_length, line, "binding byte length")?,
        sha256: digest.to_owned(),
    };
    if identity.byte_length == 0 || identity.byte_length > MAX_RUNTIME_OBJECT_BYTES {
        return Err(format!(
            "line {line}: binding byte length is outside bounds"
        ));
    }
    Ok(RecordedBindingIdentity {
        object: identity,
        destination: destination.to_owned(),
        mode,
    })
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
        return Err("prepared receipt stdout count or length is outside bounds".to_owned());
    }
    let mut stdout = Vec::with_capacity(expected_length);
    for (index, chunk) in chunks.into_iter().enumerate() {
        if index + 1 < expected_count && chunk.len() != STDOUT_CHUNK_BYTES {
            return Err("prepared receipt has a short non-final stdout chunk".to_owned());
        }
        stdout.extend(chunk);
    }
    validate_sha256("prepared stdout", expected_sha256)?;
    if stdout.len() != expected_length || sha256(&stdout) != expected_sha256 {
        return Err("prepared receipt stdout bytes differ from metadata".to_owned());
    }
    Ok(stdout)
}

fn enforce_fixed_metadata<'a>(metadata: &mut BTreeMap<&'a str, &'a str>) -> Result<(), String> {
    for (key, expected) in fixed_metadata() {
        let actual = take(metadata, key)?;
        if actual != expected {
            return Err(format!(
                "prepared receipt metadata {key} is {actual:?}, expected {expected:?}"
            ));
        }
    }
    Ok(())
}

fn advance(stage: &mut Option<Stage>, next: Stage, line: usize) -> Result<(), String> {
    if stage.is_some_and(|current| current > next) {
        return Err(format!(
            "line {line}: receipt records are not in canonical order"
        ));
    }
    *stage = Some(next);
    Ok(())
}

fn take<'a>(metadata: &mut BTreeMap<&'a str, &'a str>, key: &str) -> Result<&'a str, String> {
    metadata
        .remove(key)
        .ok_or_else(|| format!("missing prepared receipt metadata key {key}"))
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
    if input.is_empty() || input.len() > MAX_PREPARED_RECEIPT_BYTES {
        return Err("prepared receipt size is outside bounds".to_owned());
    }
    if input.lines().any(|line| line.len() > MAX_LINE_BYTES) {
        return Err(format!(
            "prepared receipt line exceeds {MAX_LINE_BYTES} bytes"
        ));
    }
    if input
        .bytes()
        .any(|byte| (byte.is_ascii_control() && !matches!(byte, b'\n' | b'\t')) || byte == 0x7f)
    {
        return Err("prepared receipt contains a prohibited control byte".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
