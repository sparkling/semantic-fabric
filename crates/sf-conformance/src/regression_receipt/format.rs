use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::model::{
    validate_relative_path, validate_sha256, validate_token, BaselineRow, Disposition,
    ExpectedBaseline, InputBinding, Profile, Suite, Surface, TestIdentity, ATTESTATION_SCOPE,
    BASELINE_HEADER, HASH_ALGORITHM, PROFILE_HEADER,
};

pub const MAX_INVENTORY_BYTES: u64 = 256 * 1024;
pub const MAX_BASELINE_BYTES: u64 = 256 * 1024;
const MAX_LINE_BYTES: usize = 1024;
const MAX_INPUTS: usize = 16;
const MAX_SUITES: usize = 8;
const MAX_TESTS: usize = 400;

pub fn render_profile(profile: &Profile) -> String {
    let mut output = String::new();
    writeln!(output, "{PROFILE_HEADER}").expect("String writes cannot fail");
    write_meta(&mut output, "profile-id", &profile.profile_id);
    write_meta(&mut output, "surface", profile.surface.name());
    write_meta(&mut output, "backend", &profile.backend);
    write_meta(&mut output, "hash-algorithm", HASH_ALGORITHM);
    write_meta(&mut output, "attestation-scope", ATTESTATION_SCOPE);
    for input in &profile.inputs {
        writeln!(
            output,
            "input\t{}\t{}\t{}",
            input.path, input.byte_length, input.sha256
        )
        .expect("String writes cannot fail");
    }
    for suite in &profile.suites {
        writeln!(
            output,
            "suite\t{}\t{}\t{}\t{}",
            suite.id, suite.package, suite.target, suite.timeout_seconds
        )
        .expect("String writes cannot fail");
    }
    for test in &profile.tests {
        writeln!(
            output,
            "test\t{}\t{}\t{}\t{}",
            test.suite_id,
            test.name,
            test.disposition.name(),
            test.reason
        )
        .expect("String writes cannot fail");
    }
    output
}

pub fn parse_profile(input: &str) -> Result<Profile, String> {
    validate_text_shape("profile inventory", input, MAX_INVENTORY_BYTES)?;
    let mut lines = input.lines();
    if lines.next() != Some(PROFILE_HEADER) {
        return Err("invalid regression profile header".to_owned());
    }
    let mut metadata = BTreeMap::new();
    let mut inputs = Vec::new();
    let mut suites = Vec::new();
    let mut tests = Vec::new();
    let mut records_started = false;
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if !records_started => {
                insert_meta(&mut metadata, key, value, number, 5)?;
            }
            ["input", path, length, digest] => {
                records_started = true;
                if inputs.len() == MAX_INPUTS {
                    return Err(format!("line {number}: too many input bindings"));
                }
                validate_relative_path(path)?;
                validate_sha256(path, digest)?;
                inputs.push(InputBinding {
                    path: (*path).to_owned(),
                    byte_length: parse_u64("input byte length", length, number)?,
                    sha256: (*digest).to_owned(),
                });
            }
            ["suite", id, package, target, timeout] => {
                records_started = true;
                if suites.len() == MAX_SUITES {
                    return Err(format!("line {number}: too many suites"));
                }
                for (label, value) in [("suite id", id), ("package", package), ("target", target)] {
                    validate_token(label, value)?;
                }
                let timeout_seconds = parse_u64("suite timeout", timeout, number)?;
                if !(1..=3600).contains(&timeout_seconds) {
                    return Err(format!("line {number}: suite timeout is outside 1..=3600"));
                }
                suites.push(Suite {
                    id: (*id).to_owned(),
                    package: (*package).to_owned(),
                    target: (*target).to_owned(),
                    timeout_seconds,
                });
            }
            ["test", suite_id, name, disposition, reason] => {
                records_started = true;
                if tests.len() == MAX_TESTS {
                    return Err(format!("line {number}: too many test identities"));
                }
                validate_token("test suite id", suite_id)?;
                validate_token("test identity", name)?;
                validate_token("test disposition reason", reason)?;
                tests.push(TestIdentity {
                    suite_id: (*suite_id).to_owned(),
                    name: (*name).to_owned(),
                    disposition: Disposition::parse(disposition).ok_or_else(|| {
                        format!("line {number}: invalid disposition {disposition:?}")
                    })?,
                    reason: (*reason).to_owned(),
                });
            }
            ["meta", ..] => return Err(format!("line {number}: metadata follows records")),
            _ => return Err(format!("line {number}: malformed profile record")),
        }
    }
    let profile = Profile {
        profile_id: take(&mut metadata, "profile-id")?.to_owned(),
        surface: Surface::parse(take(&mut metadata, "surface")?)
            .ok_or_else(|| "invalid profile surface".to_owned())?,
        backend: take(&mut metadata, "backend")?.to_owned(),
        inputs,
        suites,
        tests,
    };
    expect(&mut metadata, "hash-algorithm", HASH_ALGORITHM)?;
    expect(&mut metadata, "attestation-scope", ATTESTATION_SCOPE)?;
    reject_unknown_metadata(&metadata, "profile")?;
    validate_profile(&profile)?;
    Ok(profile)
}

pub fn render_baseline(baseline: &ExpectedBaseline) -> String {
    let mut output = String::new();
    writeln!(output, "{BASELINE_HEADER}").expect("String writes cannot fail");
    write_meta(&mut output, "profile-id", &baseline.profile_id);
    write_meta(&mut output, "surface", baseline.surface.name());
    write_meta(&mut output, "backend", &baseline.backend);
    write_meta(&mut output, "inventory-path", &baseline.inventory_path);
    write_meta(&mut output, "inventory-sha256", &baseline.inventory_sha256);
    write_meta(&mut output, "hash-algorithm", HASH_ALGORITHM);
    write_meta(&mut output, "attestation-scope", ATTESTATION_SCOPE);
    write_meta(
        &mut output,
        "suite-count",
        &baseline.suite_count.to_string(),
    );
    write_meta(&mut output, "test-count", &baseline.rows.len().to_string());
    write_meta(
        &mut output,
        "excluded-count",
        &baseline.excluded_count.to_string(),
    );
    write_meta(&mut output, "outcomes-sha256", &baseline.outcomes_sha256);
    output.push_str(&row_records(&baseline.rows));
    output
}

pub fn parse_baseline(input: &str) -> Result<ExpectedBaseline, String> {
    validate_text_shape("expected regression baseline", input, MAX_BASELINE_BYTES)?;
    let mut lines = input.lines();
    if lines.next() != Some(BASELINE_HEADER) {
        return Err("invalid expected regression baseline header".to_owned());
    }
    let mut metadata = BTreeMap::new();
    let mut rows = Vec::new();
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if rows.is_empty() => {
                insert_meta(&mut metadata, key, value, number, 11)?;
            }
            ["test", suite_id, test_name, "passed"] => {
                if rows.len() == MAX_TESTS {
                    return Err(format!("line {number}: too many baseline rows"));
                }
                validate_token("baseline suite id", suite_id)?;
                validate_token("baseline test identity", test_name)?;
                rows.push(BaselineRow {
                    suite_id: (*suite_id).to_owned(),
                    test_name: (*test_name).to_owned(),
                });
            }
            ["test", ..] => {
                return Err(format!(
                    "line {number}: baseline outcome must be exactly passed"
                ));
            }
            ["meta", ..] => return Err(format!("line {number}: metadata follows test rows")),
            _ => return Err(format!("line {number}: malformed baseline record")),
        }
    }
    let recorded_test_count = parse_usize("test-count", take(&mut metadata, "test-count")?, 1)?;
    if recorded_test_count != rows.len() {
        return Err("baseline test-count does not match test rows".to_owned());
    }
    let inventory_sha256 = take(&mut metadata, "inventory-sha256")?.to_owned();
    let outcomes_sha256 = take(&mut metadata, "outcomes-sha256")?.to_owned();
    validate_sha256("inventory", &inventory_sha256)?;
    validate_sha256("outcomes", &outcomes_sha256)?;
    let baseline = ExpectedBaseline {
        profile_id: take(&mut metadata, "profile-id")?.to_owned(),
        surface: Surface::parse(take(&mut metadata, "surface")?)
            .ok_or_else(|| "invalid baseline surface".to_owned())?,
        backend: take(&mut metadata, "backend")?.to_owned(),
        inventory_path: take(&mut metadata, "inventory-path")?.to_owned(),
        inventory_sha256,
        outcomes_sha256,
        suite_count: parse_usize("suite-count", take(&mut metadata, "suite-count")?, 1)?,
        excluded_count: parse_usize("excluded-count", take(&mut metadata, "excluded-count")?, 1)?,
        rows,
    };
    expect(&mut metadata, "hash-algorithm", HASH_ALGORITHM)?;
    expect(&mut metadata, "attestation-scope", ATTESTATION_SCOPE)?;
    reject_unknown_metadata(&metadata, "baseline")?;
    validate_baseline(&baseline)?;
    Ok(baseline)
}

pub fn outcomes_digest(rows: &[BaselineRow]) -> String {
    sha256(row_records(rows).as_bytes())
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_profile(profile: &Profile) -> Result<(), String> {
    validate_token("profile id", &profile.profile_id)?;
    if profile.backend != "sqlite" {
        return Err("regression profiles are SQLite-only".to_owned());
    }
    if profile.inputs.is_empty() || profile.suites.is_empty() || profile.tests.is_empty() {
        return Err("profile must bind inputs, suites, and tests".to_owned());
    }
    require_strict_order(
        profile.inputs.iter().map(|item| item.path.as_str()),
        "input",
    )?;
    require_strict_order(profile.suites.iter().map(|item| item.id.as_str()), "suite")?;
    let suite_ids: BTreeSet<_> = profile
        .suites
        .iter()
        .map(|suite| suite.id.as_str())
        .collect();
    let mut last = None;
    let mut included = BTreeSet::new();
    for test in &profile.tests {
        if !suite_ids.contains(test.suite_id.as_str()) {
            return Err(format!("test references unknown suite {}", test.suite_id));
        }
        let key = (test.suite_id.as_str(), test.name.as_str());
        if last.is_some_and(|previous| previous >= key) {
            return Err("test identities are not strictly ordered".to_owned());
        }
        last = Some(key);
        match test.disposition {
            Disposition::Include if test.reason == "required" => {
                included.insert(test.suite_id.as_str());
            }
            Disposition::Exclude
                if matches!(
                    test.reason.as_str(),
                    "live-provider-optional" | "timing-performance"
                ) => {}
            _ => return Err(format!("invalid disposition reason for {}", test.name)),
        }
    }
    if included.len() != suite_ids.len() {
        return Err("every suite must contain at least one required test".to_owned());
    }
    Ok(())
}

fn validate_baseline(baseline: &ExpectedBaseline) -> Result<(), String> {
    validate_token("baseline profile id", &baseline.profile_id)?;
    validate_relative_path(&baseline.inventory_path)?;
    validate_sha256("baseline inventory", &baseline.inventory_sha256)?;
    validate_sha256("baseline outcomes", &baseline.outcomes_sha256)?;
    if baseline.rows.is_empty() {
        return Err("expected regression baseline has no test rows".to_owned());
    }
    require_strict_pair_order(&baseline.rows)?;
    let suite_count = baseline
        .rows
        .iter()
        .map(|row| row.suite_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    if baseline.suite_count != suite_count {
        return Err("baseline suite-count does not match test rows".to_owned());
    }
    if baseline.outcomes_sha256 != outcomes_digest(&baseline.rows) {
        return Err("baseline outcomes digest mismatch".to_owned());
    }
    Ok(())
}

fn row_records(rows: &[BaselineRow]) -> String {
    let mut output = String::new();
    for row in rows {
        writeln!(output, "test\t{}\t{}\tpassed", row.suite_id, row.test_name)
            .expect("String writes cannot fail");
    }
    output
}

fn validate_text_shape(label: &str, input: &str, limit: u64) -> Result<(), String> {
    if input.len() as u64 > limit {
        return Err(format!("{label} exceeds {limit} bytes"));
    }
    if !input.ends_with('\n') {
        return Err(format!("{label} must end with a newline"));
    }
    for (index, line) in input.lines().enumerate() {
        if line.len() > MAX_LINE_BYTES {
            return Err(format!(
                "{label} line {} exceeds {MAX_LINE_BYTES} bytes",
                index + 1
            ));
        }
    }
    Ok(())
}

fn require_strict_order<'a>(
    values: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<(), String> {
    let mut previous = None;
    for value in values {
        if previous.is_some_and(|prior| prior >= value) {
            return Err(format!("{label} records are not strictly ordered"));
        }
        previous = Some(value);
    }
    Ok(())
}

fn require_strict_pair_order(rows: &[BaselineRow]) -> Result<(), String> {
    let mut previous = None;
    for row in rows {
        let key = (row.suite_id.as_str(), row.test_name.as_str());
        if previous.is_some_and(|prior| prior >= key) {
            return Err("baseline test rows are not strictly ordered".to_owned());
        }
        previous = Some(key);
    }
    Ok(())
}

fn write_meta(output: &mut String, key: &str, value: &str) {
    writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
}

fn insert_meta<'a>(
    metadata: &mut BTreeMap<&'a str, &'a str>,
    key: &'a str,
    value: &'a str,
    line: usize,
    maximum: usize,
) -> Result<(), String> {
    if metadata.insert(key, value).is_some() {
        return Err(format!("line {line}: duplicate metadata key {key}"));
    }
    if metadata.len() > maximum {
        return Err(format!("line {line}: too many metadata records"));
    }
    Ok(())
}

fn parse_u64(label: &str, value: &str, line: usize) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("line {line}: invalid {label}"))
}

fn parse_usize(label: &str, value: &str, line: usize) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("line {line}: invalid {label}"))
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
        Err(format!(
            "metadata {key} is {actual:?}, expected {expected:?}"
        ))
    }
}

fn reject_unknown_metadata(metadata: &BTreeMap<&str, &str>, label: &str) -> Result<(), String> {
    if let Some(key) = metadata.keys().next() {
        Err(format!("unknown {label} metadata key {key}"))
    } else {
        Ok(())
    }
}
