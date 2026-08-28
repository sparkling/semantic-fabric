use std::collections::{BTreeMap, BTreeSet};

use super::format;

const MAX_LINES: usize = 100_000;
const MAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TreePackage {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
}

pub(super) fn parse(raw: &str) -> Result<Vec<TreePackage>, String> {
    let mut packages: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (index, line) in raw.lines().enumerate() {
        let number = index + 1;
        if number > MAX_LINES {
            return Err(format!("cargo tree exceeds {MAX_LINES} lines"));
        }
        if line.len() > MAX_LINE_BYTES {
            return Err(format!(
                "cargo tree line {number} exceeds {MAX_LINE_BYTES} bytes"
            ));
        }
        let line = line.strip_suffix(" (*)").unwrap_or(line);
        let (display, features) = line
            .split_once('\t')
            .ok_or_else(|| format!("cargo tree line {number} has no feature delimiter"))?;
        if features.contains('\t') {
            return Err(format!("cargo tree line {number} has extra fields"));
        }
        let (name, version) = parse_display(display, number)?;
        let entry = packages.entry((name, version)).or_default();
        if !features.is_empty() {
            for feature in features.split(',') {
                super::validate_text("cargo tree feature", feature)?;
                entry.insert(feature.to_owned());
            }
        }
        if packages.len() > format::MAX_PACKAGES {
            return Err(format!(
                "cargo tree exceeds {} distinct packages",
                format::MAX_PACKAGES
            ));
        }
    }
    if packages.is_empty() {
        return Err("cargo tree is empty".to_owned());
    }
    Ok(packages
        .into_iter()
        .map(|((name, version), features)| TreePackage {
            name,
            version,
            features: features.into_iter().collect(),
        })
        .collect())
}

fn parse_display(display: &str, line: usize) -> Result<(String, String), String> {
    let (name, rest) = display
        .split_once(' ')
        .ok_or_else(|| format!("cargo tree line {line} has no version"))?;
    let (version, annotation) = rest.split_once(' ').unwrap_or((rest, ""));
    let version = version
        .strip_prefix('v')
        .ok_or_else(|| format!("cargo tree line {line} has an invalid version"))?;
    super::validate_text("cargo tree package name", name)?;
    super::validate_text("cargo tree package version", version)?;
    if !annotation.is_empty() {
        let annotation = annotation.strip_suffix(" (*)").unwrap_or(annotation);
        if !annotation.starts_with('(') || !annotation.ends_with(')') {
            return Err(format!("cargo tree line {line} has an invalid annotation"));
        }
        super::validate_text("cargo tree annotation", annotation)?;
    }
    Ok((name.to_owned(), version.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_root_specific_features_and_duplicate_nodes() {
        let parsed = parse(
            "alpha v1.0.0 (/checkout/alpha)\tstd,default\n\
             beta v2.0.0 (proc-macro)\tderive\n\
             alpha v1.0.0\talloc,default (*)\n",
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0].features,
            ["alloc", "default", "std"].map(str::to_owned)
        );
    }

    #[test]
    fn rejects_unbounded_or_unstructured_output() {
        assert!(parse("alpha v1.0.0 default\n").is_err());
        assert!(parse(&format!("alpha v1.0.0\t{}\n", "x".repeat(MAX_LINE_BYTES))).is_err());
    }
}
