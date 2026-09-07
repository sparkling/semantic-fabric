use std::collections::{BTreeMap, BTreeSet};

use super::tree::TreePackage;

const MAX_LINES: usize = 500_000;
const MAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ContextKind {
    Target,
    Host,
}

impl ContextKind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Host => "host",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "target" => Ok(Self::Target),
            "host" => Ok(Self::Host),
            _ => Err(format!("invalid Cargo feature context {value:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ContextPackage {
    pub name: String,
    pub version: String,
    pub context: ContextKind,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum EdgeKind {
    Normal,
    Build,
}

impl EdgeKind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Build => "build",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ContextEdge {
    pub parent_name: String,
    pub parent_version: String,
    pub parent_context: ContextKind,
    pub child_name: String,
    pub child_version: String,
    pub child_context: ContextKind,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone)]
struct StackPackage {
    name: String,
    version: String,
    context: ContextKind,
}

#[derive(Debug)]
pub(super) struct ParsedTree {
    pub aggregate: Vec<TreePackage>,
    pub contexts: Vec<ContextPackage>,
    pub edges: Vec<ContextEdge>,
}

pub(super) fn parse(raw: &str) -> Result<ParsedTree, String> {
    let mut context_stack: Vec<StackPackage> = Vec::new();
    let mut build_depths = BTreeSet::new();
    let mut pending_build_depths = BTreeSet::new();
    let mut contexts = BTreeMap::new();
    let mut aggregate: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut edges = BTreeSet::new();
    let mut saw_root = false;

    for (index, line) in raw.lines().enumerate() {
        let number = index + 1;
        if number > MAX_LINES {
            return Err(format!("context Cargo tree exceeds {MAX_LINES} lines"));
        }
        if line.is_empty() || line.len() > MAX_LINE_BYTES {
            return Err(format!(
                "context Cargo tree line {number} is empty or exceeds {MAX_LINE_BYTES} bytes"
            ));
        }
        if let Some(depth) = build_heading_depth(line, number)? {
            if depth == 0 || context_stack.len() < depth {
                return Err(format!(
                    "context Cargo tree line {number} has an orphan build-dependency heading"
                ));
            }
            build_depths.retain(|existing| *existing <= depth);
            pending_build_depths.retain(|existing| *existing <= depth);
            if !build_depths.insert(depth) || !pending_build_depths.insert(depth) {
                return Err(format!(
                    "context Cargo tree line {number} repeats a build-dependency heading"
                ));
            }
            continue;
        }

        let (depth, display, features) = package_line(line, number)?;
        if depth == 0 {
            if saw_root || !contexts.is_empty() {
                return Err("context Cargo tree has more than one root".to_owned());
            }
            saw_root = true;
            context_stack.clear();
            build_depths.clear();
            pending_build_depths.clear();
        } else {
            if !saw_root || context_stack.len() < depth {
                return Err(format!(
                    "context Cargo tree line {number} skips its parent depth"
                ));
            }
            build_depths.retain(|existing| *existing <= depth);
            pending_build_depths.retain(|existing| *existing <= depth);
        }
        context_stack.truncate(depth);

        let (name, version, proc_macro) = parse_display(display, number)?;
        let features = parse_features(features, number)?;
        let build_edge = build_depths.contains(&depth);
        let context = if depth == 0 {
            ContextKind::Target
        } else if context_stack[depth - 1].context == ContextKind::Host || build_edge || proc_macro
        {
            ContextKind::Host
        } else {
            ContextKind::Target
        };
        pending_build_depths.remove(&depth);
        if depth > 0 {
            let parent = &context_stack[depth - 1];
            edges.insert(ContextEdge {
                parent_name: parent.name.clone(),
                parent_version: parent.version.clone(),
                parent_context: parent.context,
                child_name: name.clone(),
                child_version: version.clone(),
                child_context: context,
                kind: if build_edge {
                    EdgeKind::Build
                } else {
                    EdgeKind::Normal
                },
            });
        }
        context_stack.push(StackPackage {
            name: name.clone(),
            version: version.clone(),
            context,
        });

        let key = (name.clone(), version.clone(), context);
        if let Some(previous) = contexts.insert(key.clone(), features.clone()) {
            if previous != features {
                return Err(format!(
                    "context Cargo tree reports different {} feature sets for {} {}",
                    context.name(),
                    name,
                    version
                ));
            }
        }
        aggregate
            .entry((name, version))
            .or_default()
            .extend(features);
    }
    if !saw_root || contexts.is_empty() {
        return Err("context Cargo tree is empty".to_owned());
    }
    if let Some(depth) = pending_build_depths.first() {
        return Err(format!(
            "context Cargo tree build-dependency heading at depth {depth} has no child"
        ));
    }

    Ok(ParsedTree {
        aggregate: aggregate
            .into_iter()
            .map(|((name, version), features)| TreePackage {
                name,
                version,
                features: features.into_iter().collect(),
            })
            .collect(),
        contexts: contexts
            .into_iter()
            .map(|((name, version, context), features)| ContextPackage {
                name,
                version,
                context,
                features,
            })
            .collect(),
        edges: edges.into_iter().collect(),
    })
}

fn build_heading_depth(line: &str, number: usize) -> Result<Option<usize>, String> {
    let Some(prefix) = line.strip_suffix("[build-dependencies]") else {
        if line.trim_start().starts_with('[') {
            return Err(format!(
                "context Cargo tree line {number} has an unsupported section heading"
            ));
        }
        return Ok(None);
    };
    parse_indent(prefix, number).map(|depth| Some(depth + 1))
}

fn package_line(line: &str, number: usize) -> Result<(usize, &str, &str), String> {
    let (depth, body) = if line.starts_with("|-- ") || line.starts_with("`-- ") {
        (1, &line[4..])
    } else if line.starts_with("|   ") || line.starts_with("    ") {
        let mut offset = 0;
        while matches!(line.get(offset..offset + 4), Some("|   " | "    ")) {
            offset += 4;
        }
        match line.get(offset..offset + 4) {
            Some("|-- " | "`-- ") => (offset / 4 + 1, &line[offset + 4..]),
            _ => {
                return Err(format!(
                    "context Cargo tree line {number} has invalid indentation"
                ))
            }
        }
    } else {
        (0, line)
    };
    let (display, features) = body
        .split_once('\t')
        .ok_or_else(|| format!("context Cargo tree line {number} has no feature delimiter"))?;
    if features.contains('\t') {
        return Err(format!(
            "context Cargo tree line {number} has extra feature fields"
        ));
    }
    Ok((depth, display, features))
}

fn parse_indent(prefix: &str, number: usize) -> Result<usize, String> {
    if !prefix.len().is_multiple_of(4)
        || prefix
            .as_bytes()
            .chunks_exact(4)
            .any(|chunk| chunk != b"|   " && chunk != b"    ")
    {
        return Err(format!(
            "context Cargo tree line {number} has invalid heading indentation"
        ));
    }
    Ok(prefix.len() / 4)
}

fn parse_display(display: &str, line: usize) -> Result<(String, String, bool), String> {
    if display.ends_with(" (*)") {
        return Err(format!(
            "context Cargo tree line {line} is deduplicated; --no-dedupe is required"
        ));
    }
    let (name, rest) = display
        .split_once(' ')
        .ok_or_else(|| format!("context Cargo tree line {line} has no version"))?;
    let (version, annotation) = rest.split_once(' ').unwrap_or((rest, ""));
    let version = version
        .strip_prefix('v')
        .ok_or_else(|| format!("context Cargo tree line {line} has an invalid version"))?;
    super::validate_text("context Cargo tree package name", name)?;
    super::validate_text("context Cargo tree package version", version)?;
    if !annotation.is_empty() && (!annotation.starts_with('(') || !annotation.ends_with(')')) {
        return Err(format!(
            "context Cargo tree line {line} has an invalid annotation"
        ));
    }
    if !annotation.is_empty() {
        super::validate_text("context Cargo tree annotation", annotation)?;
    }
    Ok((
        name.to_owned(),
        version.to_owned(),
        annotation == "(proc-macro)",
    ))
}

fn parse_features(features: &str, line: usize) -> Result<Vec<String>, String> {
    let mut parsed = BTreeSet::new();
    if !features.is_empty() {
        for feature in features.split(',') {
            super::validate_text("context Cargo tree feature", feature)?;
            if !parsed.insert(feature.to_owned()) {
                return Err(format!(
                    "context Cargo tree line {line} repeats feature {feature:?}"
                ));
            }
        }
    }
    Ok(parsed.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_target_proc_macro_and_build_contexts() {
        let parsed = parse(concat!(
            "root v1.0.0\tparser-worker-evidence\n",
            "|-- target v1.0.0\tstd\n",
            "|   `-- shared v1.0.0\tstd\n",
            "|-- derive v1.0.0 (proc-macro)\t\n",
            "|   `-- shared v1.0.0\t\n",
            "`-- native v1.0.0\t\n",
            "    [build-dependencies]\n",
            "    `-- shared v1.0.0\t\n",
        ))
        .unwrap();

        assert!(parsed.contexts.iter().any(|package| {
            package.name == "shared"
                && package.context == ContextKind::Target
                && package.features == ["std"]
        }));
        assert!(parsed.contexts.iter().any(|package| {
            package.name == "shared"
                && package.context == ContextKind::Host
                && package.features.is_empty()
        }));
        let shared = parsed
            .aggregate
            .iter()
            .find(|package| package.name == "shared")
            .unwrap();
        assert_eq!(shared.features, ["std"]);
        assert!(parsed.edges.iter().any(|edge| {
            edge.parent_name == "native"
                && edge.child_name == "shared"
                && edge.kind == EdgeKind::Build
                && edge.child_context == ContextKind::Host
        }));
    }

    #[test]
    fn rejects_context_collapse_and_malformed_sections() {
        let collapsed = "root v1.0.0\t\n|-- shared v1.0.0\tstd\n`-- shared v1.0.0\talloc\n";
        assert!(parse(collapsed).unwrap_err().contains("different target"));
        assert!(parse("root v1.0.0\t\n    [dev-dependencies]\n").is_err());
        assert!(parse("root v1.0.0\t\n    [build-dependencies]\n").is_err());
        assert!(parse("root v1.0.0\t\n`-- shared v1.0.0 (*)\t\n").is_err());
    }
}
