//! Merge startup settings as data before handing them to the typed CLI boundary.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;

use clap::Parser;

#[path = "layered_config_input.rs"]
mod input;
#[path = "layered_config_model.rs"]
mod model;
use input::{Layer, OPTIONS};
use model::FileConfig;

const CONFIG_ENV: &str = "SEMANTIC_FABRIC_CONFIG";
const MAX_CONFIG_BYTES: u64 = 1 << 20;

#[derive(Debug)]
pub(super) enum ConfigError {
    InvalidArguments,
    Unreadable,
    TooLarge,
    InvalidDocument,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidArguments => "startup configuration arguments are invalid",
            Self::Unreadable => "startup configuration cannot be read",
            Self::TooLarge => "startup configuration exceeds the byte limit",
            Self::InvalidDocument => "startup configuration is invalid",
        })
    }
}

pub(super) fn expand(argv: Vec<OsString>) -> Result<Vec<OsString>, ConfigError> {
    expand_with_env(argv, |name| std::env::var_os(name))
}

fn expand_with_env(
    argv: Vec<OsString>,
    environment: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<OsString>, ConfigError> {
    if argv.get(1).is_none_or(|arg| arg != OsStr::new("serve")) {
        return Ok(argv);
    }
    let cli = match input::parse_cli(&argv[2..]) {
        Ok(Some(cli)) => cli,
        // Static help does not depend on configuration or source availability.
        Ok(None) => return Ok(vec![argv[0].clone(), "serve".into(), "--help".into()]),
        // Preserve CLI usage diagnostics only when Clap also rejects execution.
        Err(ConfigError::InvalidArguments) if crate::Cli::try_parse_from(&argv).is_err() => {
            return Ok(argv);
        }
        Err(error) => return Err(error),
    };
    let mut env = Layer::new();
    for name in OPTIONS {
        if let Some(value) = environment(&input::environment_name(name)) {
            env.insert((*name).to_owned(), value);
        }
    }
    let env_config = environment(CONFIG_ENV);
    let layered = cli.contains_key("config") || env_config.is_some() || !env.is_empty();
    if let Some(path) = env_config {
        env.insert("config".to_owned(), path);
    }
    input::validate_size(&env)?;
    let path = cli.get("config").or_else(|| env.get("config"));
    let mut merged = Layer::new();
    if let Some(path) = path {
        merge(&mut merged, read_config(Path::new(path))?.into_layer())?;
    }
    merge(&mut merged, env)?;
    if let Err(error) = merge(&mut merged, cli) {
        if !layered && crate::Cli::try_parse_from(&argv).is_err() {
            return Ok(argv);
        }
        return Err(error);
    }
    input::validate_size(&merged)?;

    let mut expanded = vec![argv[0].clone(), "serve".into()];
    for (name, value) in merged {
        // False is absence of the permission, not a conflicting auth selector.
        if name == "allow-unauthenticated" && value == "false" {
            continue;
        }
        let mut argument = OsString::from(format!("--{name}="));
        argument.push(value);
        expanded.push(argument);
    }
    // Validate only effective values. Parser errors can echo configuration data.
    if layered {
        crate::Cli::try_parse_from(&expanded).map_err(|_| ConfigError::InvalidDocument)?;
    }
    Ok(expanded)
}

fn read_config(path: &Path) -> Result<FileConfig, ConfigError> {
    let path = path.canonicalize().map_err(|_| ConfigError::Unreadable)?;
    let mut options = OpenOptions::new();
    options.read(true);
    // A FIFO must not hang startup before the held descriptor can be inspected.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| ConfigError::Unreadable)?;
    let metadata = file.metadata().map_err(|_| ConfigError::Unreadable)?;
    if !metadata.is_file() {
        return Err(ConfigError::Unreadable);
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    let mut document = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut document)
        .map_err(|_| ConfigError::Unreadable)?;
    if document.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    toml::from_str(&document).map_err(|_| ConfigError::InvalidDocument)
}

fn merge(lower: &mut Layer, higher: Layer) -> Result<(), ConfigError> {
    for group in [
        &["source", "source-env"][..],
        &["mapping", "direct-mapping-base"][..],
        &[
            "source-2",
            "source-env-2",
            "mapping-2",
            "direct-mapping-base-2",
        ][..],
    ] {
        if group.iter().any(|name| higher.contains_key(*name)) {
            for name in group {
                lower.remove(*name);
            }
        }
    }
    let anonymous = match higher.get("allow-unauthenticated") {
        Some(value) if value == "true" => true,
        Some(value) if value == "false" => false,
        Some(_) => return Err(ConfigError::InvalidDocument),
        None => false,
    };
    let registry = higher.contains_key("auth-subjects-env");
    let bearer = higher.contains_key("auth-token-env");
    let claims = higher.contains_key("pg-rls-context-env");
    if (registry && (bearer || claims || anonymous)) || (anonymous && (bearer || claims)) {
        return Err(ConfigError::InvalidDocument);
    }
    if anonymous || registry {
        for name in [
            "auth-subjects-env",
            "auth-token-env",
            "pg-rls-context-env",
            "allow-unauthenticated",
        ] {
            lower.remove(name);
        }
    } else if bearer {
        // Token rotation must preserve the current row policy. A registry's
        // per-subject policy cannot be replaced by a lone unrestricted token.
        if lower.contains_key("auth-subjects-env") && !claims {
            return Err(ConfigError::InvalidDocument);
        }
        lower.remove("auth-subjects-env");
        lower.remove("allow-unauthenticated");
    }
    lower.extend(higher);
    Ok(())
}

#[cfg(test)]
#[path = "layered_config_tests.rs"]
mod tests;
