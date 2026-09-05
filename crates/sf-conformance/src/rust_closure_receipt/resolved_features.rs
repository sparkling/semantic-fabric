use std::collections::BTreeMap;

use serde::Deserialize;

use super::PackageRecord;

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    features: Vec<String>,
}

pub(super) fn validate(raw: &str, packages: &[PackageRecord]) -> Result<(), String> {
    let metadata: Metadata = serde_json::from_str(raw)
        .map_err(|error| format!("parse Cargo feature metadata: {error}"))?;
    let mut ids = BTreeMap::new();
    for package in metadata.packages {
        if ids
            .insert((package.name.clone(), package.version.clone()), package.id)
            .is_some()
        {
            return Err(format!(
                "Cargo metadata repeats package identity {} {}",
                package.name, package.version
            ));
        }
    }
    let mut nodes = BTreeMap::new();
    for node in metadata.resolve.nodes {
        if nodes.insert(node.id.clone(), node.features).is_some() {
            return Err(format!("Cargo metadata repeats resolved node {}", node.id));
        }
    }
    for package in packages {
        let id = ids
            .get(&(package.name.clone(), package.version.clone()))
            .ok_or_else(|| {
                format!(
                    "qualified package {} {} has no unique Cargo metadata identity",
                    package.name, package.version
                )
            })?;
        let mut features = nodes
            .get(id)
            .ok_or_else(|| format!("qualified package {} has no resolved node", package.name))?
            .clone();
        features.sort();
        if features.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(format!(
                "Cargo metadata repeats a resolved feature for {}",
                package.name
            ));
        }
        // `cargo metadata` exposes the workspace-wide resolved feature union,
        // while `cargo tree -p sf-cli` is root-specific.  The reverse
        // containment would therefore admit features activated only by other
        // workspace roots into this qualification input.
        if package
            .features
            .iter()
            .any(|feature| features.binary_search(feature).is_err())
        {
            return Err(format!(
                "Cargo metadata omits a root-specific context-tree feature for {} {}",
                package.name, package.version
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rust_closure_receipt::OriginKind;

    fn package(features: &[&str]) -> PackageRecord {
        let mut package = PackageRecord {
            key: String::new(),
            name: "alpha".to_owned(),
            version: "1.0.0".to_owned(),
            origin_kind: OriginKind::Registry,
            origin: "registry+https://github.com/rust-lang/crates.io-index".to_owned(),
            features: features.iter().map(|value| (*value).to_owned()).collect(),
            edges: Vec::new(),
        };
        package.key = package.computed_key();
        package
    }

    #[test]
    fn requires_root_specific_tree_features_to_exist_in_workspace_metadata() {
        let raw = r#"{"packages":[{"id":"alpha 1.0.0","name":"alpha","version":"1.0.0"}],"resolve":{"nodes":[{"id":"alpha 1.0.0","features":["std"]}]}}"#;

        validate(raw, &[package(&["std"])]).unwrap();
        validate(raw, &[package(&[])]).unwrap();
        assert!(validate(raw, &[package(&["alloc"])]).is_err());
    }
}
