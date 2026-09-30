//! Comparison-only source execution binding, not a grant or stream-completion proof.
//! v1 uses ordered, u64-big-endian length-framed fields. Source ID is local to
//! the compiler, not an externally provisioned dataset/source UUID. Limits are
//! immutable configured ceilings, never remaining time or consumed counters.

use std::fmt;

use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;
use sha2::{Digest, Sha256};

use crate::binding::RuntimeBinding;
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::generated_profile_identity::GeneratedProfileIdentity;

pub(crate) const EXECUTION_HEADER: &str = "x-semantic-fabric-execution";
const DOMAIN: &[u8] = b"semantic-fabric/generated-execution-identity/v1";
const CHUNK: usize = 4096;

/// Only successful authoritative generated compilation constructs this pair.
/// Wire claims cannot construct it. Debug reveals neither query nor identity.
pub(crate) struct GeneratedResponseIdentity {
    profile: GeneratedProfileIdentity,
    execution: [u8; 32],
}

impl GeneratedResponseIdentity {
    /// Existing profile-only regression assertions keep their original meaning.
    #[cfg(test)]
    pub(crate) fn wire(&self) -> String {
        self.profile.wire()
    }

    pub(super) fn profile(&self) -> &GeneratedProfileIdentity {
        &self.profile
    }

    pub(super) fn execution_wire(&self) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(70);
        out.push_str("sfgx1:");
        for byte in self.execution {
            write!(out, "{byte:02x}").expect("String formatting is infallible");
        }
        out
    }
}

impl fmt::Debug for GeneratedResponseIdentity {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("GeneratedResponseIdentity(<redacted>)")
    }
}

pub(super) fn mint(
    binding: &RuntimeBinding,
    cfg: &ServeConfig,
    query: &str,
    profile: GeneratedProfileIdentity,
    budget: &RequestBudget,
) -> Result<GeneratedResponseIdentity, QueryControlError> {
    let scope = binding.scope();
    let digests = binding.digests();
    let limits = cfg.query_limits;
    let reserve = limits.reservation_limits();
    let inputs = Inputs {
        query,
        source: scope.source_id().index() as u64,
        dialect: dialect(scope.dialect()),
        epoch: scope.epoch().0,
        digests: [
            *digests.mapping().as_bytes(),
            *digests.ontology().as_bytes(),
            *digests.semantic_admission().as_bytes(),
            *digests.schema().as_bytes(),
            *digests.structural_schema().as_bytes(),
            *digests.type_schema().as_bytes(),
            *digests.constraint_policy().as_bytes(),
            *digests.capability().as_bytes(),
        ],
        profile: profile.wire(),
        policy: budget
            .security_context()
            .map(|context| context.policy_lineage_digest()),
        ceilings: [
            cfg.timeout.as_secs(),
            u64::from(cfg.timeout.subsec_nanos()),
            cfg.max_query_len() as u64,
            cfg.max_order_rows() as u64,
            limits.max_compiler_work(),
            limits.max_source_work(),
            limits.max_result_items(),
            limits.max_serialized_bytes(),
            reserve.max_retained_bytes(),
            reserve.max_spill_bytes(),
            reserve.max_spill_files(),
            reserve.max_file_descriptors(),
            reserve.max_operator_tasks(),
        ],
    };
    Ok(GeneratedResponseIdentity {
        profile,
        execution: inputs.digest(budget)?,
    })
}

// Private encoding input, populated only from the authoritative binding above.
// Keeping extraction separate permits independent field-sensitivity regression
// tests without exposing digest constructors or wire-to-mint authority.
struct Inputs<'a> {
    query: &'a str,
    source: u64,
    dialect: &'static [u8],
    epoch: u64,
    digests: [[u8; 32]; 8],
    profile: String,
    policy: Option<[u8; 32]>,
    ceilings: [u64; 13],
}

impl Inputs<'_> {
    fn digest(&self, control: &dyn QueryControl) -> Result<[u8; 32], QueryControlError> {
        let mut out = Framed::new(control);
        out.field(DOMAIN)?;
        out.field(self.query.as_bytes())?;
        out.number(self.source)?;
        out.field(self.dialect)?;
        out.number(self.epoch)?;
        for digest in &self.digests {
            out.field(digest)?;
        }
        out.field(self.profile.as_bytes())?;
        match self.policy {
            Some(policy) => {
                out.field(b"policy-present")?;
                out.field(&policy)?;
            }
            None => out.field(b"policy-absent")?,
        }
        for number in self.ceilings {
            out.number(number)?;
        }
        control.checkpoint()?;
        Ok(out.hash.finalize().into())
    }
}

struct Framed<'a> {
    hash: Sha256,
    control: &'a dyn QueryControl,
}

impl<'a> Framed<'a> {
    fn new(control: &'a dyn QueryControl) -> Self {
        Self {
            hash: Sha256::new(),
            control,
        }
    }

    fn number(&mut self, number: u64) -> Result<(), QueryControlError> {
        self.field(&number.to_be_bytes())
    }

    fn field(&mut self, bytes: &[u8]) -> Result<(), QueryControlError> {
        let len = u64::try_from(bytes.len()).map_err(|_| {
            self.control
                .terminate(QueryControlError::AccountingOverflow)
        })?;
        self.control.checkpoint()?;
        self.control.consume(QueryCharge::CompilerWork, 8)?;
        self.hash.update(len.to_be_bytes());
        for chunk in bytes.chunks(CHUNK) {
            self.control.checkpoint()?;
            self.control
                .consume(QueryCharge::CompilerWork, chunk.len() as u64)?;
            self.hash.update(chunk);
        }
        self.control.checkpoint()
    }
}

fn dialect(value: Dialect) -> &'static [u8] {
    match value {
        Dialect::Postgres => b"postgres",
        Dialect::Sqlite => b"sqlite",
        Dialect::MySql => b"mysql",
        Dialect::Redshift => b"redshift",
        Dialect::DuckDb => b"duckdb",
        Dialect::SqlServer => b"sqlserver",
        Dialect::Oracle => b"oracle",
        Dialect::SapHana => b"sap-hana",
        Dialect::MonetDb => b"monetdb",
        Dialect::Snowflake => b"snowflake",
        Dialect::BigQuery => b"bigquery",
        Dialect::Athena => b"athena",
        Dialect::Databricks => b"databricks",
        Dialect::Trino => b"trino",
        Dialect::PrestoDB => b"presto-db",
        Dialect::Db2 => b"db2",
        Dialect::H2 => b"h2",
        Dialect::Spark => b"spark",
        Dialect::Dremio => b"dremio",
        Dialect::Denodo => b"denodo",
        Dialect::Teiid => b"teiid",
    }
}

#[cfg(test)]
#[path = "generated_execution_identity_tests.rs"]
mod tests;
