//! REST/HTTP `SqlBackend` adapters (ADR-0024 M8).
//!
//! Covers databases that expose a SQL-over-REST API: Snowflake (SQL API v2),
//! Google BigQuery (Jobs API), AWS Athena (the real AWS JSON 1.1 API over
//! SigV4), Databricks (Statement Execution API), and Trino/PrestoDB (native
//! REST protocol).
//!
//! Snowflake, BigQuery, Databricks, and Trino/PrestoDB require the aggregate
//! `rest-backends` feature. Athena has its own `athena-backend` feature so its
//! AWS signing dependency closure is not enabled for unrelated providers.
//! Disabled providers compile to stubs that return `Error::Unsupported`.
//!
//! Authentication: ****** read from environment variables:
//! - Snowflake:   `SF_SNOWFLAKE_TOKEN`
//! - BigQuery:    `SF_BIGQUERY_TOKEN`
//! - Databricks:  `SF_DATABRICKS_TOKEN`
//! - Athena:      `SF_ATHENA_ACCESS_KEY_ID` / `SF_ATHENA_SECRET_ACCESS_KEY` /
//!   `SF_ATHENA_SESSION_TOKEN` (see [`athena`]).
//! - Trino/Presto: none required for test clusters.
//!
//! Verification tier: compile + unit (JSON parsing tests). Live-parity requires
//! a real provider account and provider-specific configuration.
//!
//! Per ADR-0036, the implementation is split into per-provider modules (each
//! under 500 lines) instead of one large `rest.rs`: `shared` holds the
//! Presto/Trino wire protocol and JSON-parsing helpers common to more than
//! one provider, and `snowflake`, `bigquery`, `databricks`, `athena`, and
//! `presto` each own a single provider. `real` and `trino_real` remain as
//! compatibility re-export modules so pre-split import paths keep resolving.

#[cfg(feature = "athena-backend")]
pub mod athena;
#[cfg(feature = "rest-backends")]
mod bigquery;
#[cfg(feature = "rest-backends")]
mod databricks;
#[cfg(feature = "rest-backends")]
mod presto;
#[cfg(feature = "rest-backends")]
mod shared;
#[cfg(feature = "rest-backends")]
mod snowflake;

#[cfg(any(not(feature = "rest-backends"), not(feature = "athena-backend")))]
mod stub;

/// Compatibility re-export module preserving the pre-split `rest::real` API
/// surface for the feature-gated (`rest-backends`) implementations.
#[cfg(any(feature = "rest-backends", feature = "athena-backend"))]
pub mod real {
    #[cfg(feature = "athena-backend")]
    pub use super::athena::{AthenaBackend, AthenaConfig, AthenaCredentials, AthenaStream};
    #[cfg(feature = "rest-backends")]
    pub use super::bigquery::{parse_bigquery_response, BigQueryBackend, BigQueryStream};
    #[cfg(feature = "rest-backends")]
    pub use super::databricks::{parse_databricks_response, DatabricksBackend, DatabricksStream};
    #[cfg(feature = "rest-backends")]
    pub use super::shared::{inline_params, json_value_to_string, presto_execute};
    #[cfg(feature = "rest-backends")]
    pub use super::snowflake::{parse_snowflake_response, SnowflakeBackend, SnowflakeStream};
}

/// Compatibility re-export module preserving the pre-split `rest::trino_real`
/// API surface for the Trino/PrestoDB REST backend.
#[cfg(feature = "rest-backends")]
pub mod trino_real {
    pub use super::presto::{PrestoDbBackend, TrinoBackend, TrinoStream};
}

#[cfg(feature = "athena-backend")]
pub use real::{AthenaBackend, AthenaConfig, AthenaCredentials, AthenaStream};
#[cfg(feature = "rest-backends")]
pub use real::{BigQueryBackend, DatabricksBackend, SnowflakeBackend};
#[cfg(feature = "rest-backends")]
pub use trino_real::{PrestoDbBackend, TrinoBackend};

#[cfg(not(feature = "athena-backend"))]
pub use stub::AthenaBackend;
#[cfg(not(feature = "rest-backends"))]
pub use stub::{BigQueryBackend, DatabricksBackend, SnowflakeBackend};
#[cfg(not(feature = "rest-backends"))]
pub use stub::{PrestoDbBackend, TrinoBackend};

// SAP HANA and MonetDB get their own files; re-export the stub types here so
// the type aliases in the old API surface still resolve.
pub use super::hana::HanaBackend as SapHanaBackend;
pub use super::monetdb::MonetDbBackend;
