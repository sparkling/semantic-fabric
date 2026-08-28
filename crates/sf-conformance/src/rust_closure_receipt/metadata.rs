use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use super::{
    platform::TargetContext, Edge, PackageRecord, ROOT_BINARY, ROOT_MANIFEST, ROOT_PACKAGE,
};
use serde::Deserialize;

const MAX_PACKAGES: usize = 10_000;
const MAX_EDGES: usize = 100_000;
const MAX_FEATURES: usize = 100_000;
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    workspace_default_members: Vec<String>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
    id: String,
    source: Option<String>,
    manifest_path: String,
    targets: Vec<Target>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    rename: Option<String>,
    optional: bool,
    kind: Option<String>,
    target: Option<String>,
}

#[derive(Deserialize)]
struct Target {
    kind: Vec<String>,
    name: String,
    src_path: String,
    #[serde(rename = "required-features", default)]
    required_features: Vec<String>,
}

#[derive(Deserialize)]
struct Resolve {
    root: Option<String>,
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Deserialize)]
struct NodeDep {
    name: String,
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
    kind: Option<String>,
    target: Option<String>,
}

pub(super) fn parse(
    raw: &str,
    raw_tree: &str,
    repo_root: &Path,
    target: &TargetContext,
) -> Result<Vec<PackageRecord>, String> {
    let input: Metadata =
        serde_json::from_str(raw).map_err(|error| format!("parse cargo metadata: {error}"))?;
    if input.packages.is_empty()
        || input.packages.len() > MAX_PACKAGES
        || input.resolve.nodes.len() > MAX_PACKAGES
    {
        return Err("cargo metadata package/node count is outside bounds".to_owned());
    }
    let root_id = input
        .resolve
        .root
        .as_deref()
        .ok_or_else(|| "cargo metadata has no unique resolve root".to_owned())?;
    if input.workspace_default_members.as_slice() != [root_id] {
        return Err("cargo metadata default member does not equal its root".to_owned());
    }
    let workspace_members: BTreeSet<_> =
        input.workspace_members.iter().map(String::as_str).collect();
    if workspace_members.len() != input.workspace_members.len()
        || !workspace_members.contains(root_id)
    {
        return Err("cargo metadata workspace membership is invalid".to_owned());
    }
    let packages = index_packages(input.packages)?;
    let nodes = index_nodes(input.resolve.nodes)?;
    let tree_packages = bind_tree_packages(super::tree::parse(raw_tree)?, &packages)?;
    validate_root(root_id, &packages, repo_root, &tree_packages)?;
    let reachable = reachable_nodes(root_id, &nodes, &packages, &tree_packages, target)?;
    let mut keys = BTreeMap::new();
    for id in &reachable {
        let package = packages
            .get(id)
            .ok_or_else(|| format!("resolved package {id:?} has no package record"))?;
        let (origin_kind, origin) = super::origin::classify(
            &package.name,
            package.source.as_deref(),
            &package.manifest_path,
            repo_root,
            workspace_members.contains(id.as_str()),
        )?;
        let mut record = PackageRecord {
            key: String::new(),
            name: package.name.clone(),
            version: package.version.clone(),
            origin_kind,
            origin,
            features: Vec::new(),
            edges: Vec::new(),
        };
        record.key = record.computed_key();
        if keys.insert(id.clone(), record.key.clone()).is_some() {
            return Err("duplicate reachable package id".to_owned());
        }
    }
    let mut records = Vec::with_capacity(reachable.len());
    let mut total_features = 0usize;
    let mut total_edges = 0usize;
    for id in reachable {
        let package = packages.get(&id).expect("reachable packages were checked");
        let node = nodes
            .get(&id)
            .ok_or_else(|| format!("resolved package {id:?} has no node"))?;
        let (origin_kind, origin) = super::origin::classify(
            &package.name,
            package.source.as_deref(),
            &package.manifest_path,
            repo_root,
            workspace_members.contains(id.as_str()),
        )?;
        let features = tree_packages[&id].clone();
        total_features = total_features
            .checked_add(features.len())
            .ok_or_else(|| "feature count overflow".to_owned())?;
        if total_features > MAX_FEATURES {
            return Err("cargo metadata feature count exceeds bounds".to_owned());
        }
        let edges = edges(node, package, &tree_packages[&id], &packages, &keys, target)?;
        total_edges = total_edges
            .checked_add(edges.len())
            .ok_or_else(|| "edge count overflow".to_owned())?;
        if total_edges > MAX_EDGES {
            return Err("cargo metadata edge count exceeds bounds".to_owned());
        }
        records.push(PackageRecord {
            key: keys[&id].clone(),
            name: package.name.clone(),
            version: package.version.clone(),
            origin_kind,
            origin,
            features,
            edges,
        });
    }
    records.sort_by(|left, right| left.key.cmp(&right.key));
    super::validate_packages(&records)?;
    Ok(records)
}

fn index_packages(packages: Vec<Package>) -> Result<BTreeMap<String, Package>, String> {
    let mut result = BTreeMap::new();
    for package in packages {
        super::validate_text("cargo package id", &package.id)?;
        super::validate_text("cargo package name", &package.name)?;
        super::validate_text("cargo package version", &package.version)?;
        if result.insert(package.id.clone(), package).is_some() {
            return Err("duplicate cargo metadata package id".to_owned());
        }
    }
    Ok(result)
}

fn index_nodes(nodes: Vec<Node>) -> Result<BTreeMap<String, Node>, String> {
    let mut result = BTreeMap::new();
    for node in nodes {
        super::validate_text("cargo node id", &node.id)?;
        if result.insert(node.id.clone(), node).is_some() {
            return Err("duplicate cargo metadata node id".to_owned());
        }
    }
    Ok(result)
}

fn bind_tree_packages(
    tree: Vec<super::tree::TreePackage>,
    packages: &BTreeMap<String, Package>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut result = BTreeMap::new();
    for tree_package in tree {
        let matches: Vec<_> = packages
            .values()
            .filter(|package| {
                package.name == tree_package.name && package.version == tree_package.version
            })
            .collect();
        let [package] = matches.as_slice() else {
            return Err(format!(
                "cargo tree package {} {} has {} metadata matches",
                tree_package.name,
                tree_package.version,
                matches.len()
            ));
        };
        result.insert(package.id.clone(), tree_package.features);
    }
    Ok(result)
}

fn validate_root(
    root_id: &str,
    packages: &BTreeMap<String, Package>,
    repo_root: &Path,
    tree_packages: &BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    let root = packages
        .get(root_id)
        .ok_or_else(|| "cargo metadata root package is missing".to_owned())?;
    if root.name != ROOT_PACKAGE || root.source.is_some() {
        return Err(format!("cargo metadata root is not {ROOT_PACKAGE}"));
    }
    let expected_manifest = repo_root.join(ROOT_MANIFEST);
    if Path::new(&root.manifest_path) != expected_manifest {
        return Err("cargo metadata root manifest binding mismatch".to_owned());
    }
    let binaries: Vec<_> = root
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| kind == "bin"))
        .collect();
    if binaries.len() != 1 || binaries[0].name != ROOT_BINARY {
        return Err(format!(
            "root package must expose exactly one binary named {ROOT_BINARY}"
        ));
    }
    let expected_source = repo_root.join("crates/sf-cli/src/main.rs");
    if Path::new(&binaries[0].src_path) != expected_source {
        return Err("root binary source binding mismatch".to_owned());
    }
    let active_features: BTreeSet<_> = tree_packages
        .get(root_id)
        .ok_or_else(|| "cargo tree does not contain the root package".to_owned())?
        .iter()
        .map(String::as_str)
        .collect();
    if binaries[0]
        .required_features
        .iter()
        .any(|feature| !active_features.contains(feature.as_str()))
    {
        return Err("root binary required features are not active".to_owned());
    }
    Ok(())
}

fn reachable_nodes(
    root: &str,
    nodes: &BTreeMap<String, Node>,
    packages: &BTreeMap<String, Package>,
    tree_packages: &BTreeMap<String, Vec<String>>,
    target: &TargetContext,
) -> Result<BTreeSet<String>, String> {
    if !tree_packages.contains_key(root) {
        return Err("cargo tree does not contain the root package".to_owned());
    }
    let mut reached = BTreeSet::new();
    let mut queue = VecDeque::from([root.to_owned()]);
    while let Some(id) = queue.pop_front() {
        if !reached.insert(id.clone()) {
            continue;
        }
        let node = nodes
            .get(&id)
            .ok_or_else(|| format!("resolved node {id:?} is missing"))?;
        let package = packages
            .get(&id)
            .ok_or_else(|| format!("resolved package {id:?} is missing"))?;
        let active_optional =
            super::features::active_optional_aliases(&package.features, &tree_packages[&id]);
        for dependency in &node.deps {
            if dependency.dep_kinds.is_empty() {
                return Err(format!("dependency {} has no kind", dependency.name));
            }
            let target_package = packages
                .get(&dependency.pkg)
                .ok_or_else(|| format!("dependency {} has no package record", dependency.name))?;
            let active_kind = dependency
                .dep_kinds
                .iter()
                .try_fold(false, |active, kind| {
                    Ok::<_, String>(
                        active
                            || (is_closure_kind(kind) && target.matches(kind.target.as_deref())?),
                    )
                })?;
            if !active_kind
                || !dependency_is_active(
                    package,
                    target_package,
                    dependency,
                    &active_optional,
                    target,
                )?
            {
                continue;
            }
            if !tree_packages.contains_key(&dependency.pkg) {
                return Err(format!(
                    "active dependency {} of {} is absent from cargo tree",
                    dependency.name, package.name
                ));
            }
            queue.push_back(dependency.pkg.clone());
        }
    }
    if reached.len() != tree_packages.len() {
        return Err("cargo tree contains a package unreachable in metadata".to_owned());
    }
    Ok(reached)
}

fn edges(
    node: &Node,
    package: &Package,
    active_features: &[String],
    packages: &BTreeMap<String, Package>,
    keys: &BTreeMap<String, String>,
    target: &TargetContext,
) -> Result<Vec<Edge>, String> {
    let mut result = Vec::new();
    let active_optional =
        super::features::active_optional_aliases(&package.features, active_features);
    for dependency in &node.deps {
        super::validate_text("dependency alias", &dependency.name)?;
        let active_kinds: Vec<_> = dependency
            .dep_kinds
            .iter()
            .filter_map(|kind| {
                if !is_closure_kind(kind) {
                    return None;
                }
                match target.matches(kind.target.as_deref()) {
                    Ok(true) => Some(Ok(kind)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect::<Result<_, _>>()?;
        if active_kinds.is_empty() {
            continue;
        }
        if !keys.contains_key(&dependency.pkg) {
            continue;
        }
        let target_package = packages
            .get(&dependency.pkg)
            .ok_or_else(|| format!("dependency {} has no package record", dependency.name))?;
        if !dependency_is_active(
            package,
            target_package,
            dependency,
            &active_optional,
            target,
        )? {
            continue;
        }
        for kind in active_kinds {
            let name = match kind.kind.as_deref() {
                None => "normal",
                Some("build") => "build",
                Some("dev") => continue,
                Some(other) => return Err(format!("unknown cargo dependency kind {other:?}")),
            };
            let package_key = keys.get(&dependency.pkg).ok_or_else(|| {
                format!(
                    "closure dependency {} has no reachable package",
                    dependency.name
                )
            })?;
            result.push(Edge {
                alias: dependency.name.clone(),
                package_key: package_key.clone(),
                kind: name.to_owned(),
                target: kind.target.clone(),
            });
        }
    }
    result.sort();
    if result.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("duplicate canonical dependency edge".to_owned());
    }
    Ok(result)
}

fn dependency_is_active(
    package: &Package,
    target_package: &Package,
    dependency: &NodeDep,
    active_optional: &BTreeSet<String>,
    target: &TargetContext,
) -> Result<bool, String> {
    let mut by_package = Vec::new();
    for declared in &package.dependencies {
        if declared.name == target_package.name
            && matches!(declared.kind.as_deref(), None | Some("build"))
            && target.matches(declared.target.as_deref())?
        {
            by_package.push(declared);
        }
    }
    if by_package.is_empty() {
        return Err(format!(
            "resolved dependency {} of {} has no declaration",
            dependency.name, package.name
        ));
    }
    let by_alias: Vec<_> = by_package
        .iter()
        .copied()
        .filter(|declared| dependency_alias(declared) == dependency.name)
        .collect();
    let declarations = if by_alias.is_empty() {
        if by_package.len() != 1 {
            return Err(format!(
                "resolved dependency {} of {} has ambiguous declarations",
                dependency.name, package.name
            ));
        }
        by_package
    } else {
        by_alias
    };
    Ok(declarations.iter().any(|declared| !declared.optional)
        || declarations
            .iter()
            .any(|declared| active_optional.contains(&dependency_alias(declared))))
}

fn dependency_alias(dependency: &Dependency) -> String {
    super::features::dependency_alias(&dependency.name, dependency.rename.as_deref())
}

fn is_closure_kind(kind: &DepKind) -> bool {
    matches!(kind.kind.as_deref(), None | Some("build"))
}
