//! Sandbox-logical path validation and narrow loader-alias normalization.

use std::path::{Component, Path};

const MAX_ALIAS_BYTES: usize = 4096;
const MAX_LOGICAL_PATH_BYTES: usize = 4096;
const MAX_LOGICAL_COMPONENTS: usize = 256;

pub(super) fn validate(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.len() > MAX_LOGICAL_PATH_BYTES
        || !path.is_absolute()
        || value.contains("//")
        || value.contains('\\')
        || value.ends_with('/')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._+-".contains(&byte))
        || value
            .split('/')
            .skip(1)
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || path
            .components()
            .filter(|part| matches!(part, Component::Normal(_)))
            .count()
            > MAX_LOGICAL_COMPONENTS
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        Err("runtime logical path is not absolute and normalized".to_owned())
    } else {
        Ok(())
    }
}

pub(super) fn normalize_relative_alias(logical: &str, raw: &[u8]) -> Result<String, String> {
    if raw.is_empty() || raw.len() > MAX_ALIAS_BYTES || raw[0] == b'/' {
        return Err("loader alias target must be bounded and relative".to_owned());
    }
    let target =
        std::str::from_utf8(raw).map_err(|_| "loader alias target is not UTF-8".to_owned())?;
    if target.contains('\\')
        || target.contains("//")
        || target.ends_with('/')
        || target
            .split('/')
            .any(|component| component.is_empty() || component == ".")
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._+-".contains(&byte))
    {
        return Err("loader alias target contains prohibited bytes".to_owned());
    }
    let parent = Path::new(logical)
        .parent()
        .ok_or_else(|| "loader alias has no logical parent".to_owned())?;
    let mut parts: Vec<String> = parent
        .components()
        .filter_map(|part| match part {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    for component in Path::new(target).components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::ParentDir if parts.pop().is_some() => {}
            _ => return Err("loader alias target escapes the logical root".to_owned()),
        }
    }
    if parts.is_empty() {
        return Err("loader alias target resolves to the logical root".to_owned());
    }
    let normalized = format!("/{}", parts.join("/"));
    validate(&normalized)?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_exact_component_and_normalized_alias_bounds() {
        let boundary = format!("/{}", vec!["a"; MAX_LOGICAL_COMPONENTS].join("/"));
        assert!(validate(&boundary).is_ok());
        let excess = format!("/{}", vec!["a"; MAX_LOGICAL_COMPONENTS + 1].join("/"));
        assert!(validate(&excess).is_err());

        let target = vec!["a"; MAX_LOGICAL_COMPONENTS].join("/");
        assert!(normalize_relative_alias("/lib64/alias", target.as_bytes()).is_err());
        let long_parent = format!("/{}/alias", "a".repeat(4080));
        assert!(validate(&long_parent).is_ok());
        assert!(normalize_relative_alias(&long_parent, b"child/child/xyz").is_err());
    }
}
