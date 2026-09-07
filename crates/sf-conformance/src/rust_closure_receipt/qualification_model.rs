use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::context_tree::{ContextEdge, ContextKind, ContextPackage};
use super::qualification::{QualifiedContext, QualifiedEdge, QualifiedPackage, ROOT_FEATURE};
use super::{Edge, PackageRecord};

pub(super) fn bind_contexts(
    package_records: Vec<PackageRecord>,
    contexts: Vec<ContextPackage>,
    edges: Vec<ContextEdge>,
) -> Result<Vec<QualifiedPackage>, String> {
    let mut identities = BTreeMap::new();
    for package in &package_records {
        let identity = (package.name.clone(), package.version.clone());
        if identities.insert(identity, package).is_some() {
            return Err("qualified closure has an ambiguous name/version identity".to_owned());
        }
    }
    let mut feature_contexts = BTreeMap::new();
    for context in contexts {
        let package = identities
            .get(&(context.name.clone(), context.version.clone()))
            .ok_or_else(|| format!("context package {} has no package record", context.name))?;
        if feature_contexts
            .insert((package.key.clone(), context.context), context.features)
            .is_some()
        {
            return Err("duplicate package feature context".to_owned());
        }
    }
    let mut context_edges: BTreeMap<(String, ContextKind), BTreeSet<QualifiedEdge>> =
        BTreeMap::new();
    let mut represented = BTreeSet::new();
    for occurrence in edges {
        let parent = identities
            .get(&(occurrence.parent_name, occurrence.parent_version))
            .ok_or_else(|| "context edge parent has no package record".to_owned())?;
        let child = identities
            .get(&(occurrence.child_name, occurrence.child_version))
            .ok_or_else(|| "context edge child has no package record".to_owned())?;
        let kind = occurrence.kind.name();
        let candidates: Vec<_> = parent
            .edges
            .iter()
            .filter(|edge| edge.package_key == child.key && edge.kind == kind)
            .collect();
        if candidates.is_empty() {
            return Err(format!(
                "context edge {} -> {} has no metadata match",
                parent.name, child.name,
            ));
        }
        for edge in candidates {
            let qualified = QualifiedEdge {
                alias: edge.alias.clone(),
                package_key: child.key.clone(),
                context: occurrence.child_context,
                kind: edge.kind.clone(),
                target: edge.target.clone(),
            };
            context_edges
                .entry((parent.key.clone(), occurrence.parent_context))
                .or_default()
                .insert(qualified);
            represented.insert(global_edge_key(parent, edge));
        }
    }
    for package in &package_records {
        for edge in &package.edges {
            if !represented.contains(&global_edge_key(package, edge)) {
                return Err(format!(
                    "metadata edge {} -> {} is absent from the expanded context tree",
                    package.name, edge.package_key
                ));
            }
        }
    }

    let mut packages = Vec::with_capacity(package_records.len());
    for package in package_records {
        let context_keys: Vec<_> = feature_contexts
            .keys()
            .filter(|(key, _)| key == &package.key)
            .cloned()
            .collect();
        if context_keys.is_empty() {
            return Err(format!("package {} has no feature context", package.name));
        }
        let mut union = BTreeSet::new();
        let mut qualified_contexts = Vec::with_capacity(context_keys.len());
        for key in context_keys {
            let features = feature_contexts.remove(&key).expect("context key exists");
            union.extend(features.iter().cloned());
            let edges = context_edges
                .remove(&key)
                .unwrap_or_default()
                .into_iter()
                .collect();
            qualified_contexts.push(QualifiedContext {
                kind: key.1,
                features,
                edges,
            });
        }
        qualified_contexts.sort();
        if union.into_iter().collect::<Vec<_>>() != package.features {
            return Err(format!(
                "feature contexts do not reproduce package features for {}",
                package.name
            ));
        }
        packages.push(QualifiedPackage {
            key: package.key,
            name: package.name,
            version: package.version,
            origin_kind: package.origin_kind,
            origin: package.origin,
            contexts: qualified_contexts,
        });
    }
    packages.sort_by(|left, right| left.key.cmp(&right.key));
    if !feature_contexts.is_empty() || !context_edges.is_empty() {
        return Err("qualification context records were not consumed".to_owned());
    }
    validate_packages(&packages)?;
    Ok(packages)
}

fn global_edge_key(
    package: &PackageRecord,
    edge: &Edge,
) -> (String, String, String, String, String) {
    (
        package.key.clone(),
        edge.alias.clone(),
        edge.package_key.clone(),
        edge.kind.clone(),
        edge.target.clone().unwrap_or_else(|| "-".to_owned()),
    )
}

pub(super) fn validate_packages(packages: &[QualifiedPackage]) -> Result<(), String> {
    if packages.is_empty() || packages.len() > super::qualification_format::MAX_PACKAGES {
        return Err("qualification package count is outside bounds".to_owned());
    }
    let keys: BTreeSet<_> = packages.iter().map(|package| &package.key).collect();
    if keys.len() != packages.len() || !packages.windows(2).all(|pair| pair[0].key < pair[1].key) {
        return Err("qualification package keys are duplicate or unsorted".to_owned());
    }
    for package in packages {
        super::validate_sha256("qualification package key", &package.key)?;
        super::validate_text("qualification package name", &package.name)?;
        super::validate_text("qualification package version", &package.version)?;
        super::validate_text("qualification package origin", &package.origin)?;
        let identity = PackageRecord {
            key: String::new(),
            name: package.name.clone(),
            version: package.version.clone(),
            origin_kind: package.origin_kind,
            origin: package.origin.clone(),
            features: Vec::new(),
            edges: Vec::new(),
        };
        if identity.computed_key() != package.key {
            return Err(format!(
                "qualification package key mismatch for {}",
                package.name
            ));
        }
        if package.contexts.is_empty() || !package.contexts.windows(2).all(|pair| pair[0] < pair[1])
        {
            return Err(format!("invalid contexts for {}", package.name));
        }
        for context in &package.contexts {
            super::validate_sorted("qualification features", &context.features)?;
            if !context.edges.windows(2).all(|pair| pair[0] < pair[1]) {
                return Err(format!("unsorted context edges for {}", package.name));
            }
            for edge in &context.edges {
                super::validate_text("qualification dependency alias", &edge.alias)?;
                super::validate_sha256("qualification dependency key", &edge.package_key)?;
                if !keys.contains(&edge.package_key) {
                    return Err(format!(
                        "qualification edge from {} leaves closure",
                        package.name
                    ));
                }
                if !matches!(edge.kind.as_str(), "normal" | "build") {
                    return Err("qualification dependency kind is invalid".to_owned());
                }
                if edge.kind == "build" && edge.context != ContextKind::Host {
                    return Err(
                        "qualification build dependency does not enter host context".to_owned()
                    );
                }
                if context.kind == ContextKind::Host && edge.context != ContextKind::Host {
                    return Err(
                        "qualification host dependency escapes into target context".to_owned()
                    );
                }
                if let Some(target) = &edge.target {
                    super::validate_text("qualification dependency target", target)?;
                }
            }
        }
    }
    for package in packages {
        for context in &package.contexts {
            for edge in &context.edges {
                let target = packages
                    .iter()
                    .find(|package| package.key == edge.package_key)
                    .expect("edge target key was checked");
                if !target
                    .contexts
                    .iter()
                    .any(|context| context.kind == edge.context)
                {
                    return Err(format!(
                        "qualification edge targets a missing context for {}",
                        target.name
                    ));
                }
            }
        }
    }
    require_root_features(packages)
}

fn require_root_features(packages: &[QualifiedPackage]) -> Result<(), String> {
    for name in [super::ROOT_PACKAGE, "sf-sparql"] {
        let matches: Vec<_> = packages
            .iter()
            .filter(|package| package.name == name)
            .collect();
        let [package] = matches.as_slice() else {
            return Err(format!(
                "qualification closure has no unique {name} package"
            ));
        };
        let target: Vec<_> = package
            .contexts
            .iter()
            .filter(|context| context.kind == ContextKind::Target)
            .collect();
        let [target] = target.as_slice() else {
            return Err(format!(
                "qualification closure has no unique target context for {name}"
            ));
        };
        if name == super::ROOT_PACKAGE && package.contexts.len() != 1 {
            return Err("qualification root package has a non-target context".to_owned());
        }
        if target.features.as_slice() != [ROOT_FEATURE] {
            return Err(format!(
                "qualification root feature set for {name} is not exact"
            ));
        }
    }
    Ok(())
}

pub(super) fn closure_digest(packages: &[QualifiedPackage]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"semantic-fabric-parser-worker-context-closure-v1\0");
    for package in packages {
        for value in [
            package.key.as_str(),
            package.name.as_str(),
            package.version.as_str(),
            package.origin_kind.name(),
            package.origin.as_str(),
        ] {
            digest.update(value.as_bytes());
            digest.update([0]);
        }
        for context in &package.contexts {
            digest.update(context.kind.name().as_bytes());
            digest.update([0]);
            for feature in &context.features {
                digest.update(b"feature\0");
                digest.update(feature.as_bytes());
                digest.update([0]);
            }
            for edge in &context.edges {
                digest.update(b"edge\0");
                for value in [
                    edge.alias.as_str(),
                    edge.package_key.as_str(),
                    edge.context.name(),
                    edge.kind.as_str(),
                    edge.target.as_deref().unwrap_or("-"),
                ] {
                    digest.update(value.as_bytes());
                    digest.update([0]);
                }
            }
        }
    }
    format!("{:x}", digest.finalize())
}
