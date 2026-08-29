//! Strict semantic parser for glibc `ld.so --list` output.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    validate_runtime_file_path, validate_soname, ResolvedRuntimeObject, VirtualRuntimeObject,
    MAX_LOADER_OUTPUT_BYTES, MAX_RESOLVED_OBJECTS,
};

const VIRTUAL_OBJECT: &str = "linux-vdso.so.1";

pub(super) struct ParsedLoaderOutput {
    pub(super) loader_path: String,
    pub(super) resolved_objects: Vec<ResolvedRuntimeObject>,
    pub(super) virtual_objects: Vec<VirtualRuntimeObject>,
}

pub(super) fn parse(output: &[u8]) -> Result<ParsedLoaderOutput, String> {
    if output.is_empty() || output.len() > MAX_LOADER_OUTPUT_BYTES || !output.ends_with(b"\n") {
        return Err("runtime loader output size or termination is invalid".to_owned());
    }
    if output
        .iter()
        .any(|byte| !byte.is_ascii() || (byte.is_ascii_control() && !matches!(byte, b'\n' | b'\t')))
    {
        return Err("runtime loader output has an invalid byte".to_owned());
    }
    let text = std::str::from_utf8(output)
        .map_err(|error| format!("runtime loader output is not UTF-8: {error}"))?;
    let mut loader_path = None;
    let mut resolved = BTreeMap::new();
    let mut resolved_paths = BTreeSet::new();
    let mut virtual_objects = BTreeSet::new();
    for line in text.split_terminator('\n') {
        let body = indented_body(line)?;
        let (record, _address) = address_record(body)?;
        if record == VIRTUAL_OBJECT {
            if !virtual_objects.insert(record.to_owned()) {
                return Err("runtime loader output has a duplicate virtual object".to_owned());
            }
        } else if let Some((soname, path)) = record.split_once(" => ") {
            if path.contains(" => ") || path == "not found" {
                return Err(
                    "runtime loader output has an unresolved or ambiguous object".to_owned(),
                );
            }
            validate_soname(soname)?;
            validate_runtime_file_path(path, "resolved runtime object path")?;
            let basename = super::linux_basename(path)
                .ok_or_else(|| "resolved runtime object path has no file name".to_owned())?;
            if basename != soname {
                return Err("runtime object name and resolved-path basename differ".to_owned());
            }
            if resolved.len() == MAX_RESOLVED_OBJECTS {
                return Err("resolved runtime object count exceeds bounds".to_owned());
            }
            if resolved
                .insert(soname.to_owned(), path.to_owned())
                .is_some()
                || !resolved_paths.insert(path.to_owned())
            {
                return Err("runtime loader output has a duplicate object identity".to_owned());
            }
        } else if record.starts_with('/') {
            validate_runtime_file_path(record, "runtime loader path")?;
            if loader_path.replace(record.to_owned()).is_some() {
                return Err("runtime loader output has multiple loader paths".to_owned());
            }
        } else {
            return Err("runtime loader output has an unknown record".to_owned());
        }
    }
    let loader_path = loader_path.ok_or_else(|| "runtime loader path is missing".to_owned())?;
    if resolved_paths.contains(&loader_path) {
        return Err("runtime loader path duplicates a resolved object path".to_owned());
    }
    let loader_name = super::linux_basename(&loader_path)
        .ok_or_else(|| "runtime loader path has no file name".to_owned())?;
    if resolved.contains_key(VIRTUAL_OBJECT) || resolved.contains_key(loader_name) {
        return Err("runtime loader object identity sets overlap".to_owned());
    }
    if virtual_objects.len() != 1 || !virtual_objects.contains(VIRTUAL_OBJECT) {
        return Err("runtime loader virtual-object set is not exact".to_owned());
    }
    if resolved.is_empty() {
        return Err("runtime loader resolved-object set is empty".to_owned());
    }
    let resolved_objects = resolved
        .into_iter()
        .map(|(soname, resolved_path)| ResolvedRuntimeObject {
            soname,
            resolved_path,
        })
        .collect();
    let virtual_objects = virtual_objects
        .into_iter()
        .map(|name| VirtualRuntimeObject { name })
        .collect();
    Ok(ParsedLoaderOutput {
        loader_path,
        resolved_objects,
        virtual_objects,
    })
}

fn indented_body(line: &str) -> Result<&str, String> {
    let body = line
        .strip_prefix('\t')
        .ok_or_else(|| "runtime loader output line indentation is invalid".to_owned())?;
    if body.is_empty()
        || body
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        || body.trim() != body
    {
        Err("runtime loader output line indentation is invalid".to_owned())
    } else {
        Ok(body)
    }
}

fn address_record(line: &str) -> Result<(&str, &str), String> {
    let (record, address) = line
        .rsplit_once(" (0x")
        .ok_or_else(|| "runtime loader output address is missing".to_owned())?;
    let digits = address
        .strip_suffix(')')
        .ok_or_else(|| "runtime loader output address is malformed".to_owned())?;
    if record.is_empty()
        || record.ends_with(' ')
        || digits.is_empty()
        || digits.len() > 16
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("runtime loader output address is not canonical".to_owned());
    }
    Ok((record, digits))
}
