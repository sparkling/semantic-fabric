use std::collections::BTreeMap;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::model::{
    validate_manifest, validate_sha256, Case, ExpectedStatus, Manifest, StandardReference, Surface,
    HASH_ALGORITHM, HEADER, REFERENCE_BINDING,
};

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_LINE_BYTES: usize = 1024;
const MAX_CASES: usize = 128;
const MAX_METADATA: usize = 16;

pub fn manifest_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn render_manifest(manifest: &Manifest) -> String {
    let rows = render_cases(&manifest.cases);
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    write_meta(&mut output, "profile-id", &manifest.profile_id);
    write_meta(&mut output, "surface", manifest.surface.name());
    write_meta(&mut output, "backend", &manifest.backend);
    write_meta(&mut output, "standard-id", &manifest.standard.id);
    write_meta(&mut output, "standard-status", &manifest.standard.status);
    write_meta(
        &mut output,
        "standard-snapshot-date",
        &manifest.standard.snapshot_date,
    );
    write_meta(
        &mut output,
        "standard-byte-length",
        &manifest.standard.byte_length.to_string(),
    );
    write_meta(&mut output, "standard-sha256", &manifest.standard.sha256);
    write_meta(&mut output, "standard-binding", REFERENCE_BINDING);
    write_meta(&mut output, "hash-algorithm", HASH_ALGORITHM);
    write_meta(
        &mut output,
        "attestation-scope",
        manifest.surface.attestation_scope(),
    );
    write_meta(&mut output, "case-count", &manifest.cases.len().to_string());
    write_meta(
        &mut output,
        "cases-sha256",
        &manifest_sha256(rows.as_bytes()),
    );
    output.push_str(&rows);
    output
}

pub fn parse_manifest(input: &str) -> Result<Manifest, String> {
    validate_text_shape(input)?;
    let mut lines = input.lines();
    if lines.next() != Some(HEADER) {
        return Err("invalid supported-surface manifest header".to_owned());
    }
    let mut metadata = BTreeMap::new();
    let mut cases = Vec::new();
    let mut records_started = false;
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if !records_started => {
                if metadata.insert(*key, *value).is_some() {
                    return Err(format!("line {number}: duplicate metadata key {key}"));
                }
                if metadata.len() > MAX_METADATA {
                    return Err(format!("line {number}: too many metadata records"));
                }
            }
            ["case", id, scenario, status, http_status, media_type, cause, result_sha256] => {
                records_started = true;
                if cases.len() == MAX_CASES {
                    return Err(format!("line {number}: too many case records"));
                }
                cases.push(Case {
                    id: (*id).to_owned(),
                    scenario: (*scenario).to_owned(),
                    expected_status: ExpectedStatus::parse(status).ok_or_else(|| {
                        format!("line {number}: invalid expected status {status:?}")
                    })?,
                    http_status: http_status
                        .parse()
                        .map_err(|_| format!("line {number}: invalid HTTP status"))?,
                    response_media_type: (*media_type).to_owned(),
                    cause: (*cause).to_owned(),
                    result_sha256: (*result_sha256).to_owned(),
                });
            }
            ["meta", ..] => return Err(format!("line {number}: metadata follows cases")),
            _ => return Err(format!("line {number}: malformed manifest record")),
        }
    }

    let recorded_count = parse_usize(take(&mut metadata, "case-count")?, "case-count")?;
    if recorded_count != cases.len() {
        return Err("manifest case-count does not match case records".to_owned());
    }
    let recorded_cases_sha256 = take(&mut metadata, "cases-sha256")?.to_owned();
    validate_sha256("manifest cases", &recorded_cases_sha256)?;
    if recorded_cases_sha256 != manifest_sha256(render_cases(&cases).as_bytes()) {
        return Err("manifest cases digest mismatch".to_owned());
    }

    let surface = Surface::parse(take(&mut metadata, "surface")?)
        .ok_or_else(|| "invalid manifest surface".to_owned())?;
    let standard_sha256 = take(&mut metadata, "standard-sha256")?.to_owned();
    validate_sha256("standard reference", &standard_sha256)?;
    let manifest = Manifest {
        profile_id: take(&mut metadata, "profile-id")?.to_owned(),
        surface,
        backend: take(&mut metadata, "backend")?.to_owned(),
        standard: StandardReference {
            id: take(&mut metadata, "standard-id")?.to_owned(),
            status: take(&mut metadata, "standard-status")?.to_owned(),
            snapshot_date: take(&mut metadata, "standard-snapshot-date")?.to_owned(),
            byte_length: parse_u64(
                take(&mut metadata, "standard-byte-length")?,
                "standard-byte-length",
            )?,
            sha256: standard_sha256,
        },
        cases,
    };
    expect(&mut metadata, "standard-binding", REFERENCE_BINDING)?;
    expect(&mut metadata, "hash-algorithm", HASH_ALGORITHM)?;
    expect(
        &mut metadata,
        "attestation-scope",
        surface.attestation_scope(),
    )?;
    if let Some(key) = metadata.keys().next() {
        return Err(format!("unknown manifest metadata key {key}"));
    }
    validate_manifest(&manifest)?;
    if render_manifest(&manifest) != input {
        return Err("supported-surface manifest is valid but not canonical".to_owned());
    }
    Ok(manifest)
}

fn render_cases(cases: &[Case]) -> String {
    let mut output = String::new();
    for case in cases {
        writeln!(
            output,
            "case\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            case.id,
            case.scenario,
            case.expected_status.name(),
            case.http_status,
            case.response_media_type,
            case.cause,
            case.result_sha256
        )
        .expect("String writes cannot fail");
    }
    output
}

fn validate_text_shape(input: &str) -> Result<(), String> {
    if input.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "supported-surface manifest exceeds {MAX_MANIFEST_BYTES} bytes"
        ));
    }
    if !input.ends_with('\n') {
        return Err("supported-surface manifest must end with a newline".to_owned());
    }
    for (index, line) in input.lines().enumerate() {
        if line.len() > MAX_LINE_BYTES {
            return Err(format!(
                "supported-surface manifest line {} exceeds {MAX_LINE_BYTES} bytes",
                index + 1
            ));
        }
    }
    Ok(())
}

fn write_meta(output: &mut String, key: &str, value: &str) {
    writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
}

fn take<'a>(metadata: &mut BTreeMap<&'a str, &'a str>, key: &str) -> Result<&'a str, String> {
    metadata
        .remove(key)
        .ok_or_else(|| format!("missing metadata key {key}"))
}

fn expect<'a>(
    metadata: &mut BTreeMap<&'a str, &'a str>,
    key: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = take(metadata, key)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("metadata {key} has an invalid fixed value"))
    }
}

fn parse_usize(value: &str, label: &str) -> Result<usize, String> {
    value.parse().map_err(|_| format!("invalid {label}"))
}

fn parse_u64(value: &str, label: &str) -> Result<u64, String> {
    value.parse().map_err(|_| format!("invalid {label}"))
}
