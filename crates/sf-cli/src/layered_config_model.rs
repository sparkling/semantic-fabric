//! Strict TOML field types; conversion retains option values as data.

use super::input::Layer;
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileConfig {
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
    source_tls_roots_env: Option<String>,
    source_tls_roots_env_2: Option<String>,
    source: Option<String>,
    source_env: Option<String>,
    source_2: Option<String>,
    source_env_2: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MappingConfig {
    mapping: Option<String>,
    mapping_base: Option<String>,
    direct_mapping_base: Option<String>,
    mapping_2: Option<String>,
    mapping_base_2: Option<String>,
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
    reload_interval_secs: Option<u64>,
    require_verified_generation: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SecurityConfig {
    auth_subjects_env: Option<String>,
    auth_token_env: Option<String>,
    pg_rls_context_env: Option<String>,
    allow_unauthenticated: Option<bool>,
}

impl FileConfig {
    pub(super) fn into_layer(self) -> Layer {
        let mut layer = Layer::new();
        macro_rules! section {
            ($section:ident, $($field:ident),+ $(,)?) => { $(
                if let Some(value) = self.$section.$field {
                    layer.insert(stringify!($field).replace('_', "-"), value.to_string().into());
                }
            )+ };
        }
        section!(
            source,
            source,
            source_env,
            source_2,
            source_env_2,
            source_tls_roots_env,
            source_tls_roots_env_2
        );
        section!(
            mappings,
            mapping,
            mapping_base,
            direct_mapping_base,
            mapping_2,
            mapping_base_2,
            direct_mapping_base_2
        );
        section!(graphs, ontology);
        section!(
            governance,
            timeout_secs,
            max_query_len,
            max_concurrent_requests,
            max_source_work,
            max_result_items,
            max_order_rows,
            max_order_bytes,
            max_serialized_bytes
        );
        section!(observability, log_level, metrics);
        section!(
            serve,
            bind,
            pg_pool_size,
            pg_pool_wait_secs,
            sqlite_pool_size,
            shutdown_timeout_secs,
            reload_interval_secs,
            require_verified_generation
        );
        section!(
            security,
            auth_subjects_env,
            auth_token_env,
            pg_rls_context_env,
            allow_unauthenticated
        );
        layer
    }
}
