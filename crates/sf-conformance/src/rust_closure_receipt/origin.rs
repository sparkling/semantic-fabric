use std::path::{Component, Path};

use super::OriginKind;

pub(super) fn classify(
    package_name: &str,
    source: Option<&str>,
    manifest_path: &str,
    repo_root: &Path,
    workspace_member: bool,
) -> Result<(OriginKind, String), String> {
    match source {
        None => workspace_origin(package_name, manifest_path, repo_root, workspace_member),
        Some(source) if source.starts_with("registry+") => {
            super::validate_text("registry source", source)?;
            Ok((OriginKind::Registry, source.to_owned()))
        }
        Some(source) if source.starts_with("git+") && source.contains('#') => {
            super::validate_text("git source", source)?;
            Ok((OriginKind::Git, source.to_owned()))
        }
        Some(source) => Err(format!("unsupported package source {source:?}")),
    }
}

fn workspace_origin(
    package_name: &str,
    manifest_path: &str,
    repo_root: &Path,
    workspace_member: bool,
) -> Result<(OriginKind, String), String> {
    if !workspace_member {
        return Err(format!(
            "path dependency {package_name} is outside repository workspace"
        ));
    }
    let relative = Path::new(manifest_path)
        .strip_prefix(repo_root)
        .map_err(|_| format!("path dependency {package_name} is outside repository"))?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "package {package_name} has a non-normalized manifest path"
        ));
    }
    let relative = relative
        .to_str()
        .ok_or_else(|| format!("package {package_name} manifest path is not UTF-8"))?;
    Ok((OriginKind::Workspace, relative.replace('\\', "/")))
}
