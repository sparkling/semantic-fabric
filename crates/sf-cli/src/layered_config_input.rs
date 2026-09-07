//! Closed input vocabulary and option/value separation for startup layers.

use std::collections::BTreeMap;
use std::ffi::OsString;

use super::{ConfigError, MAX_CONFIG_BYTES};

pub(super) type Layer = BTreeMap<String, OsString>;

pub(super) const OPTIONS: &[&str] = &[
    "source-tls-roots-env",
    "source-tls-roots-env-2",
    "source",
    "source-env",
    "mapping",
    "direct-mapping-base",
    "source-2",
    "source-env-2",
    "mapping-2",
    "direct-mapping-base-2",
    "ontology",
    "auth-subjects-env",
    "auth-token-env",
    "pg-rls-context-env",
    "allow-unauthenticated",
    "bind",
    "log-level",
    "metrics",
    "timeout-secs",
    "max-query-len",
    "max-concurrent-requests",
    "max-source-work",
    "max-result-items",
    "max-order-rows",
    "max-order-bytes",
    "max-serialized-bytes",
    "pg-pool-size",
    "pg-pool-wait-secs",
    "sqlite-pool-size",
    "shutdown-timeout-secs",
];

pub(super) fn environment_name(name: &str) -> String {
    format!(
        "SEMANTIC_FABRIC_{}",
        name.replace('-', "_").to_ascii_uppercase()
    )
}

pub(super) fn validate_size(layer: &Layer) -> Result<(), ConfigError> {
    let mut total = 0usize;
    for (key, value) in layer {
        let text = value.to_str().ok_or(ConfigError::InvalidDocument)?;
        if text.contains('\0') {
            return Err(ConfigError::InvalidDocument);
        }
        total = total
            .checked_add(key.len())
            .and_then(|n| n.checked_add(text.len()))
            .ok_or(ConfigError::TooLarge)?;
        if total > MAX_CONFIG_BYTES as usize {
            return Err(ConfigError::TooLarge);
        }
    }
    Ok(())
}

/// Values are consumed once, never scanned again as flags. None means a real
/// help option, not a string supplied as another option's value.
pub(super) fn parse_cli(args: &[OsString]) -> Result<Option<Layer>, ConfigError> {
    let mut layer = Layer::new();
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        let arg = arg.to_str().ok_or(ConfigError::InvalidArguments)?;
        if matches!(arg, "--help" | "-h") {
            return Ok(None);
        }
        if arg == "--" {
            if args.next().is_some() {
                return Err(ConfigError::InvalidArguments);
            }
            break;
        }
        let text = arg
            .strip_prefix("--")
            .ok_or(ConfigError::InvalidArguments)?;
        let (name, inline) = match text.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (text, None),
        };
        if name != "config" && !OPTIONS.contains(&name) {
            return Err(ConfigError::InvalidArguments);
        }
        let is_bool = matches!(name, "metrics" | "allow-unauthenticated");
        let value = if let Some(value) = inline {
            OsString::from(value)
        } else if is_bool
            && args
                .peek()
                .is_none_or(|next| next.to_string_lossy().starts_with('-'))
        {
            OsString::from("true")
        } else {
            let value = args.next().ok_or(ConfigError::InvalidArguments)?;
            if value.to_string_lossy().starts_with('-') {
                return Err(ConfigError::InvalidArguments);
            }
            value.clone()
        };
        layer.insert(name.to_owned(), value);
    }
    validate_size(&layer)?;
    Ok(Some(layer))
}
