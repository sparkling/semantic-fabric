//! Bounded, typed startup configuration layering for `serve` (ADR-0011).

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    source: SourceConfig,
    mappings: MappingConfig,
    graphs: GraphConfig,
    governance: GovernanceConfig,
    observability: ObservabilityConfig,
    serve: ServeConfig,
    security: SecurityConfig,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SourceConfig {
    source: Option<String>,
    source_env: Option<String>,
    source_2: Option<String>,
    source_env_2: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MappingConfig {
    mapping: Option<String>,
    direct_mapping_base: Option<String>,
    mapping_2: Option<String>,
    direct_mapping_base_2: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GraphConfig {
    ontology: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GovernanceConfig {
    timeout_secs: Option<u64>,
    max_query_len: Option<usize>,
    max_concurrent_requests: Option<usize>,
    max_source_work: Option<u64>,
    max_result_items: Option<u64>,
    max_order_rows: Option<usize>,
    max_order_bytes: Option<u64>,
    max_serialized_bytes: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ObservabilityConfig {
    log_level: Option<String>,
    metrics: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ServeConfig {
    bind: Option<String>,
    pg_pool_size: Option<usize>,
    pg_pool_wait_secs: Option<u64>,
    sqlite_pool_size: Option<usize>,
    shutdown_timeout_secs: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SecurityConfig {
    auth_subjects_env: Option<String>,
    auth_token_env: Option<String>,
    pg_rls_context_env: Option<String>,
    allow_unauthenticated: Option<bool>,
}

pub(super) fn expand(argv: Vec<OsString>) -> Result<Vec<OsString>, ConfigError> {
    expand_with_env(argv, |name| std::env::var_os(name))
}

fn expand_with_env(
    argv: Vec<OsString>,
    environment: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<OsString>, ConfigError> {
    if argv.len() < 2 || argv[1] != OsStr::new("serve") {
        return Ok(argv);
    }

    let cli_config = option_value(&argv[2..], "--config")?;
    let config_path = cli_config
        .clone()
        .or_else(|| environment(CONFIG_ENV).map(PathBuf::from));
    let config = match config_path.as_deref() {
        Some(path) => read_config(path)?,
        None => FileConfig::default(),
    };

    let cli_flags = &argv[2..];
    let mut injected = Vec::new();
    if cli_config.is_none() {
        if let Some(path) = config_path {
            push_os(&mut injected, "--config", path.into_os_string());
        }
    }

    inject_file_groups(&mut injected, &config, cli_flags, &environment);
    inject_file_scalars(&mut injected, &config);
    inject_environment(&mut injected, cli_flags, &environment);

    let mut expanded = Vec::with_capacity(argv.len() + injected.len());
    expanded.extend(argv[..2].iter().cloned());
    expanded.extend(injected);
    expanded.extend(argv[2..].iter().cloned());
    Ok(expanded)
}

fn read_config(path: &Path) -> Result<FileConfig, ConfigError> {
    let path = path.canonicalize().map_err(|_| ConfigError::Unreadable)?;
    let file = File::open(path).map_err(|_| ConfigError::Unreadable)?;
    if file.metadata().map_err(|_| ConfigError::Unreadable)?.len() > MAX_CONFIG_BYTES {
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

fn inject_file_groups(
    out: &mut Vec<OsString>,
    config: &FileConfig,
    cli: &[OsString],
    environment: &impl Fn(&str) -> Option<OsString>,
) {
    inject_group(
        out,
        cli,
        environment,
        &["--source", "--source-env"],
        &["SEMANTIC_FABRIC_SOURCE", "SEMANTIC_FABRIC_SOURCE_ENV"],
        [
            ("--source", config.source.source.as_deref()),
            ("--source-env", config.source.source_env.as_deref()),
        ],
    );
    inject_group(
        out,
        cli,
        environment,
        &["--mapping", "--direct-mapping-base"],
        &[
            "SEMANTIC_FABRIC_MAPPING",
            "SEMANTIC_FABRIC_DIRECT_MAPPING_BASE",
        ],
        [
            ("--mapping", config.mappings.mapping.as_deref()),
            (
                "--direct-mapping-base",
                config.mappings.direct_mapping_base.as_deref(),
            ),
        ],
    );

    let secondary_cli = [
        "--source-2",
        "--source-env-2",
        "--mapping-2",
        "--direct-mapping-base-2",
    ];
    let secondary_env = [
        "SEMANTIC_FABRIC_SOURCE_2",
        "SEMANTIC_FABRIC_SOURCE_ENV_2",
        "SEMANTIC_FABRIC_MAPPING_2",
        "SEMANTIC_FABRIC_DIRECT_MAPPING_BASE_2",
    ];
    inject_group(
        out,
        cli,
        environment,
        &secondary_cli,
        &secondary_env,
        [
            ("--source-2", config.source.source_2.as_deref()),
            ("--source-env-2", config.source.source_env_2.as_deref()),
            ("--mapping-2", config.mappings.mapping_2.as_deref()),
            (
                "--direct-mapping-base-2",
                config.mappings.direct_mapping_base_2.as_deref(),
            ),
        ],
    );

    inject_group(
        out,
        cli,
        environment,
        &[
            "--auth-subjects-env",
            "--auth-token-env",
            "--pg-rls-context-env",
            "--allow-unauthenticated",
        ],
        &[
            "SEMANTIC_FABRIC_AUTH_SUBJECTS_ENV",
            "SEMANTIC_FABRIC_AUTH_TOKEN_ENV",
            "SEMANTIC_FABRIC_PG_RLS_CONTEXT_ENV",
            "SEMANTIC_FABRIC_ALLOW_UNAUTHENTICATED",
        ],
        [
            (
                "--auth-subjects-env",
                config.security.auth_subjects_env.as_deref(),
            ),
            (
                "--auth-token-env",
                config.security.auth_token_env.as_deref(),
            ),
            (
                "--pg-rls-context-env",
                config.security.pg_rls_context_env.as_deref(),
            ),
            (
                "--allow-unauthenticated",
                config
                    .security
                    .allow_unauthenticated
                    .map(|value| if value { "true" } else { "false" }),
            ),
        ],
    );
}

fn inject_group<'a, const N: usize>(
    out: &mut Vec<OsString>,
    cli: &[OsString],
    environment: &impl Fn(&str) -> Option<OsString>,
    cli_names: &[&str],
    env_names: &[&str],
    file_values: [(&'a str, Option<&'a str>); N],
) {
    if cli_names.iter().any(|name| has_option(cli, name))
        || env_names.iter().any(|name| environment(name).is_some())
    {
        return;
    }
    for (name, value) in file_values {
        if let Some(value) = value {
            push_str(out, name, value);
        }
    }
}

fn inject_file_scalars(out: &mut Vec<OsString>, config: &FileConfig) {
    macro_rules! value {
        ($name:literal, $value:expr) => {
            if let Some(value) = $value {
                push_str(out, $name, &value.to_string());
            }
        };
    }
    if let Some(value) = config.graphs.ontology.as_deref() {
        push_str(out, "--ontology", value);
    }
    if let Some(value) = config.observability.log_level.as_deref() {
        push_str(out, "--log-level", value);
    }
    value!("--metrics", config.observability.metrics);
    if let Some(value) = config.serve.bind.as_deref() {
        push_str(out, "--bind", value);
    }
    value!("--timeout-secs", config.governance.timeout_secs);
    value!("--max-query-len", config.governance.max_query_len);
    value!(
        "--max-concurrent-requests",
        config.governance.max_concurrent_requests
    );
    value!("--max-source-work", config.governance.max_source_work);
    value!("--max-result-items", config.governance.max_result_items);
    value!("--max-order-rows", config.governance.max_order_rows);
    value!("--max-order-bytes", config.governance.max_order_bytes);
    value!(
        "--max-serialized-bytes",
        config.governance.max_serialized_bytes
    );
    value!("--pg-pool-size", config.serve.pg_pool_size);
    value!("--pg-pool-wait-secs", config.serve.pg_pool_wait_secs);
    value!("--sqlite-pool-size", config.serve.sqlite_pool_size);
    value!(
        "--shutdown-timeout-secs",
        config.serve.shutdown_timeout_secs
    );
}

fn inject_environment(
    out: &mut Vec<OsString>,
    cli: &[OsString],
    environment: &impl Fn(&str) -> Option<OsString>,
) {
    const VALUES: &[(&str, &str)] = &[
        ("SEMANTIC_FABRIC_SOURCE", "--source"),
        ("SEMANTIC_FABRIC_SOURCE_ENV", "--source-env"),
        ("SEMANTIC_FABRIC_MAPPING", "--mapping"),
        (
            "SEMANTIC_FABRIC_DIRECT_MAPPING_BASE",
            "--direct-mapping-base",
        ),
        ("SEMANTIC_FABRIC_SOURCE_2", "--source-2"),
        ("SEMANTIC_FABRIC_SOURCE_ENV_2", "--source-env-2"),
        ("SEMANTIC_FABRIC_MAPPING_2", "--mapping-2"),
        (
            "SEMANTIC_FABRIC_DIRECT_MAPPING_BASE_2",
            "--direct-mapping-base-2",
        ),
        ("SEMANTIC_FABRIC_ONTOLOGY", "--ontology"),
        ("SEMANTIC_FABRIC_AUTH_SUBJECTS_ENV", "--auth-subjects-env"),
        ("SEMANTIC_FABRIC_AUTH_TOKEN_ENV", "--auth-token-env"),
        ("SEMANTIC_FABRIC_PG_RLS_CONTEXT_ENV", "--pg-rls-context-env"),
        (
            "SEMANTIC_FABRIC_ALLOW_UNAUTHENTICATED",
            "--allow-unauthenticated",
        ),
        ("SEMANTIC_FABRIC_BIND", "--bind"),
        ("SEMANTIC_FABRIC_LOG_LEVEL", "--log-level"),
        ("SEMANTIC_FABRIC_METRICS", "--metrics"),
        ("SEMANTIC_FABRIC_TIMEOUT_SECS", "--timeout-secs"),
        ("SEMANTIC_FABRIC_MAX_QUERY_LEN", "--max-query-len"),
        (
            "SEMANTIC_FABRIC_MAX_CONCURRENT_REQUESTS",
            "--max-concurrent-requests",
        ),
        ("SEMANTIC_FABRIC_MAX_SOURCE_WORK", "--max-source-work"),
        ("SEMANTIC_FABRIC_MAX_RESULT_ITEMS", "--max-result-items"),
        ("SEMANTIC_FABRIC_MAX_ORDER_ROWS", "--max-order-rows"),
        ("SEMANTIC_FABRIC_MAX_ORDER_BYTES", "--max-order-bytes"),
        (
            "SEMANTIC_FABRIC_MAX_SERIALIZED_BYTES",
            "--max-serialized-bytes",
        ),
        ("SEMANTIC_FABRIC_PG_POOL_SIZE", "--pg-pool-size"),
        ("SEMANTIC_FABRIC_PG_POOL_WAIT_SECS", "--pg-pool-wait-secs"),
        ("SEMANTIC_FABRIC_SQLITE_POOL_SIZE", "--sqlite-pool-size"),
        (
            "SEMANTIC_FABRIC_SHUTDOWN_TIMEOUT_SECS",
            "--shutdown-timeout-secs",
        ),
    ];
    for (env_name, cli_name) in VALUES {
        if cli_blocks_environment_group(cli, env_name) {
            continue;
        }
        if let Some(value) = environment(env_name) {
            push_os(out, cli_name, value);
        }
    }
}

fn cli_blocks_environment_group(cli: &[OsString], env_name: &str) -> bool {
    let group: &[&str] = match env_name {
        "SEMANTIC_FABRIC_SOURCE" | "SEMANTIC_FABRIC_SOURCE_ENV" => &["--source", "--source-env"],
        "SEMANTIC_FABRIC_MAPPING" | "SEMANTIC_FABRIC_DIRECT_MAPPING_BASE" => {
            &["--mapping", "--direct-mapping-base"]
        }
        "SEMANTIC_FABRIC_SOURCE_2"
        | "SEMANTIC_FABRIC_SOURCE_ENV_2"
        | "SEMANTIC_FABRIC_MAPPING_2"
        | "SEMANTIC_FABRIC_DIRECT_MAPPING_BASE_2" => &[
            "--source-2",
            "--source-env-2",
            "--mapping-2",
            "--direct-mapping-base-2",
        ],
        "SEMANTIC_FABRIC_AUTH_SUBJECTS_ENV"
        | "SEMANTIC_FABRIC_AUTH_TOKEN_ENV"
        | "SEMANTIC_FABRIC_PG_RLS_CONTEXT_ENV"
        | "SEMANTIC_FABRIC_ALLOW_UNAUTHENTICATED" => &[
            "--auth-subjects-env",
            "--auth-token-env",
            "--pg-rls-context-env",
            "--allow-unauthenticated",
        ],
        _ => return false,
    };
    group.iter().any(|name| has_option(cli, name))
}

fn option_value(args: &[OsString], name: &str) -> Result<Option<PathBuf>, ConfigError> {
    let mut found = None;
    let prefix = format!("{name}=");
    let mut index = 0;
    while index < args.len() {
        if args[index] == OsStr::new(name) {
            let value = args.get(index + 1).ok_or(ConfigError::InvalidArguments)?;
            found = Some(PathBuf::from(value));
            index += 2;
        } else if let Some(value) = args[index]
            .to_str()
            .and_then(|arg| arg.strip_prefix(&prefix))
        {
            found = Some(PathBuf::from(value));
            index += 1;
        } else {
            index += 1;
        }
    }
    Ok(found)
}

fn has_option(args: &[OsString], name: &str) -> bool {
    let prefix = format!("{name}=");
    args.iter().any(|arg| {
        arg == OsStr::new(name) || arg.to_str().is_some_and(|value| value.starts_with(&prefix))
    })
}

fn push_str(out: &mut Vec<OsString>, name: &str, value: &str) {
    push_os(out, name, OsString::from(value));
}

fn push_os(out: &mut Vec<OsString>, name: &str, value: OsString) {
    out.push(OsString::from(name));
    out.push(value);
}

#[cfg(test)]
#[path = "layered_config_tests.rs"]
mod tests;
