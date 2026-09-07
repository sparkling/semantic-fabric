//! Typed `serve` command-line boundary.

use sf_serve::{
    AdditionalSourceOptions, MappingRef, SourceRef, DEFAULT_MAX_CONCURRENT_REQUESTS,
    DEFAULT_MAX_ORDER_BYTES, DEFAULT_MAX_ORDER_ROWS, DEFAULT_QUERY_LIMITS,
    DEFAULT_SHUTDOWN_TIMEOUT,
};

use crate::telemetry::TelemetryLevel;

#[derive(clap::Args)]
pub(super) struct ServeArgs {
    /// Environment variable with a versioned registry of opaque subjects and
    /// credential and PostgreSQL-RLS or portable-row-policy environment references.
    #[arg(long, conflicts_with_all = ["auth_token_env", "pg_rls_context_env", "allow_unauthenticated"])]
    pub(super) auth_subjects_env: Option<String>,
    /// Environment variable containing a random query bearer token (32–1024 bytes).
    /// Its holder may read all mapped data unless RLS is configured; use TLS at the edge.
    #[arg(long, conflicts_with = "allow_unauthenticated")]
    pub(super) auth_token_env: Option<String>,
    /// Environment variable with trusted PostgreSQL RLS custom settings as a JSON object.
    /// Requires authored public base-table mappings and an RLS-enforced reader role.
    #[arg(long, requires = "auth_token_env")]
    pub(super) pg_rls_context_env: Option<String>,
    /// Explicitly allow anyone to query all mapped data (development only).
    /// Without this, --auth-token-env, or --auth-subjects-env, all queries are denied.
    #[arg(long, default_value_t = false)]
    pub(super) allow_unauthenticated: bool,
    #[command(flatten)]
    pub(super) source_input: SourceArgs,
    #[command(flatten)]
    pub(super) mapping_input: MappingArgs,
    #[command(flatten)]
    pub(super) additional_source_input: AdditionalSourceArgs,
    /// Required ontology (Turtle) for semantic admission and the tier-1 T-Box (ADR-0008).
    #[arg(long)]
    pub(super) ontology: String,
    /// Address to bind.
    #[arg(long, default_value = "127.0.0.1:7878")]
    pub(super) bind: String,
    /// Structured product telemetry ceiling; applies only to `serve`.
    #[arg(long, value_enum, default_value_t = TelemetryLevel::Info)]
    pub(super) log_level: TelemetryLevel,
    /// Expose bounded-cardinality Prometheus metrics at `/metrics`.
    #[arg(long, default_value_t = false)]
    pub(super) metrics: bool,
    /// Request timeout in seconds (ADR-0010).
    #[arg(long, default_value_t = 30)]
    pub(super) timeout_secs: u64,
    /// Max query length in bytes (ADR-0010).
    #[arg(long, default_value_t = 1 << 20)]
    pub(super) max_query_len: usize,
    /// Server-wide ceiling for requests admitted into application work.
    #[arg(long, default_value_t = DEFAULT_MAX_CONCURRENT_REQUESTS)]
    pub(super) max_concurrent_requests: usize,
    /// Max metadata probes, branch opens, and row-pull attempts per request.
    #[arg(long, default_value_t = DEFAULT_QUERY_LIMITS.max_source_work())]
    pub(super) max_source_work: u64,
    /// Max semantic result items per request (rows, triples, or ASK boolean).
    #[arg(long, default_value_t = DEFAULT_QUERY_LIMITS.max_result_items())]
    pub(super) max_result_items: u64,
    /// Max exact in-process ORDER BY window (`OFFSET + LIMIT`).
    #[arg(long, default_value_t = DEFAULT_MAX_ORDER_ROWS)]
    pub(super) max_order_rows: usize,
    /// Max textual binding payload retained by ORDER BY.
    #[arg(long, default_value_t = DEFAULT_MAX_ORDER_BYTES)]
    pub(super) max_order_bytes: u64,
    /// Max serialized response bytes per request.
    #[arg(long, default_value_t = DEFAULT_QUERY_LIMITS.max_serialized_bytes())]
    pub(super) max_serialized_bytes: u64,
    /// Max PostgreSQL pool connections.
    #[arg(long, default_value_t = 16)]
    pub(super) pg_pool_size: usize,
    /// Max seconds to wait for a pooled PostgreSQL connection before shedding.
    #[arg(long, default_value_t = 5)]
    pub(super) pg_pool_wait_secs: u64,
    /// Read-only connection pool size for a file-backed SQLite source.
    #[arg(long, default_value_t = 4)]
    pub(super) sqlite_pool_size: usize,
    /// Max seconds to drain active requests after SIGTERM or Ctrl-C.
    #[arg(long, default_value_t = DEFAULT_SHUTDOWN_TIMEOUT.as_secs())]
    pub(super) shutdown_timeout_secs: u64,
}

/// Exactly one primary mapping input: authored R2RML or live Direct Mapping.
#[derive(clap::Args)]
#[group(skip)]
pub(super) struct MappingArgs {
    #[arg(
        long,
        required_unless_present = "direct_mapping_base",
        conflicts_with = "direct_mapping_base"
    )]
    pub(super) mapping: Option<String>,
    #[arg(long, required_unless_present = "mapping", conflicts_with = "mapping")]
    pub(super) direct_mapping_base: Option<String>,
}

impl MappingArgs {
    pub(super) fn into_mapping_ref(self) -> MappingRef {
        match (self.mapping, self.direct_mapping_base) {
            (Some(path), None) => MappingRef::r2rml_file(path),
            (None, Some(base_iri)) => MappingRef::direct(base_iri),
            _ => unreachable!("clap requires exactly one mapping input"),
        }
    }
}

#[derive(clap::Args)]
pub(super) struct AdditionalSourceArgs {
    #[command(flatten)]
    pub(super) source_input: AdditionalSourceSelector,
    #[command(flatten)]
    pub(super) mapping_input: AdditionalMappingSelector,
}

#[derive(clap::Args)]
#[group(id = "additional_source_selector", required = false, multiple = false)]
pub(super) struct AdditionalSourceSelector {
    #[arg(
        id = "source_2",
        long = "source-2",
        requires = "additional_mapping_selector"
    )]
    pub(super) source: Option<String>,
    #[arg(
        id = "source_env_2",
        long = "source-env-2",
        requires = "additional_mapping_selector"
    )]
    pub(super) source_env: Option<String>,
}

#[derive(clap::Args)]
#[group(id = "additional_mapping_selector", required = false, multiple = false)]
pub(super) struct AdditionalMappingSelector {
    #[arg(
        id = "mapping_2",
        long = "mapping-2",
        requires = "additional_source_selector"
    )]
    pub(super) mapping: Option<String>,
    #[arg(
        id = "direct_mapping_base_2",
        long = "direct-mapping-base-2",
        requires = "additional_source_selector"
    )]
    pub(super) direct_mapping_base: Option<String>,
}

impl AdditionalSourceArgs {
    pub(super) fn into_options(self) -> Option<AdditionalSourceOptions> {
        let mapping = match (
            self.mapping_input.mapping,
            self.mapping_input.direct_mapping_base,
        ) {
            (Some(path), None) => Some(MappingRef::r2rml_file(path)),
            (None, Some(base_iri)) => Some(MappingRef::direct(base_iri)),
            (None, None) => None,
            _ => unreachable!("clap allows at most one secondary mapping input"),
        };
        match (
            self.source_input.source,
            self.source_input.source_env,
            mapping,
        ) {
            (Some(source), None, Some(mapping)) => Some(AdditionalSourceOptions {
                source: SourceRef::inline(source),
                mapping,
            }),
            (None, Some(variable), Some(mapping)) => Some(AdditionalSourceOptions {
                source: SourceRef::environment(variable),
                mapping,
            }),
            (None, None, None) => None,
            _ => unreachable!("clap requires a complete second source and mapping pair"),
        }
    }
}

#[derive(clap::Args)]
#[group(required = true, multiple = false)]
pub(super) struct SourceArgs {
    /// Credential-free `sqlite:`, `pg:`, or `mysql://` source.
    #[arg(long)]
    pub(super) source: Option<String>,
    /// Environment variable containing the complete source (credentials allowed).
    #[arg(long)]
    pub(super) source_env: Option<String>,
}

impl SourceArgs {
    pub(super) fn into_source_ref(self) -> SourceRef {
        match (self.source, self.source_env) {
            (Some(value), None) => SourceRef::inline(value),
            (None, Some(variable)) => SourceRef::environment(variable),
            _ => unreachable!("clap requires exactly one source argument"),
        }
    }
}
