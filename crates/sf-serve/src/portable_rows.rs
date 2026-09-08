//! Portable, parameterised row authorization applied to compiled relational plans.

use std::collections::BTreeSet;
use std::sync::Arc;

use sf_core::{ir::LogicalSource, SourceId};
use sf_sparql::iq::{Branch, CmpOp, ColRef, Scan, SqlCond};
use sf_sparql::{Error, Plan};
use sf_sql::Dialect;
use sha2::{Digest, Sha256};

use crate::problem::StartupCause;
use crate::ServeError;

const MAX_RULES: usize = 1024;
const MAX_TABLE_BYTES: usize = 256;
const MAX_COLUMN_BYTES: usize = 128;
const MAX_VALUE_BYTES: usize = 16 * 1024;

/// One exact equality predicate for one snapshot-local source table.
///
/// The value is sensitive configuration and is never exposed by `Debug`; SQL
/// execution receives it only as a bound parameter.
#[derive(Clone, Eq, PartialEq)]
pub struct PortableRowRule {
    source: SourceId,
    table: Box<str>,
    column: Box<str>,
    value: Box<str>,
}

impl std::fmt::Debug for PortableRowRule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PortableRowRule([REDACTED])")
    }
}

impl PortableRowRule {
    /// Construct a bounded rule. Names are matched exactly to admitted mapping
    /// identifiers; control characters and empty names are rejected.
    pub fn new(
        source_index: usize,
        table: impl Into<Box<str>>,
        column: impl Into<Box<str>>,
        value: impl Into<Box<str>>,
    ) -> Result<Self, ServeError> {
        let source = SourceId::new(source_index).map_err(|_| invalid_policy())?;
        let table = table.into();
        let column = column.into();
        let value = value.into();
        if !valid_name(&table, MAX_TABLE_BYTES)
            || !valid_name(&column, MAX_COLUMN_BYTES)
            || value.len() > MAX_VALUE_BYTES
            || value.bytes().any(|byte| byte == 0)
        {
            return Err(invalid_policy());
        }
        Ok(Self {
            source,
            table,
            column,
            value,
        })
    }
}

/// Immutable, bounded allowlist of row predicates. Every base table reached by
/// a protected plan must have at least one rule for that plan's source.
#[derive(Clone)]
pub struct PortableRowPolicy {
    rules: Arc<[PortableRowRule]>,
    identity: [u8; 32],
}

impl std::fmt::Debug for PortableRowPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PortableRowPolicy([REDACTED])")
    }
}

impl PartialEq for PortableRowPolicy {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for PortableRowPolicy {}

impl PortableRowPolicy {
    /// Canonicalize a non-empty rule set and reject duplicate predicates.
    pub fn new(mut rules: Vec<PortableRowRule>) -> Result<Self, ServeError> {
        if rules.is_empty() || rules.len() > MAX_RULES {
            return Err(invalid_policy());
        }
        rules.sort_unstable_by(|left, right| {
            (left.source, &left.table, &left.column).cmp(&(
                right.source,
                &right.table,
                &right.column,
            ))
        });
        let mut keys = BTreeSet::new();
        if rules
            .iter()
            .any(|rule| !keys.insert((rule.source, rule.table.clone(), rule.column.clone())))
        {
            return Err(invalid_policy());
        }
        let mut hash = Sha256::new();
        hash.update(b"sf-portable-row-policy-v1\0");
        hash.update((rules.len() as u64).to_be_bytes());
        for rule in &rules {
            hash.update((rule.source.index() as u64).to_be_bytes());
            hash_framed(&mut hash, &rule.table);
            hash_framed(&mut hash, &rule.column);
            hash_framed(&mut hash, &rule.value);
        }
        Ok(Self {
            rules: rules.into(),
            identity: hash.finalize().into(),
        })
    }

    pub(crate) const fn identity(&self) -> &[u8; 32] {
        &self.identity
    }

    pub(crate) fn authorize(&self, source: SourceId, plan: &mut Plan) -> sf_sparql::Result<()> {
        let dialect = plan.dialect;
        for branch in &mut plan.branches {
            authorize_branch(self, source, dialect, branch)?;
        }
        Ok(())
    }

    fn conditions(
        &self,
        source: SourceId,
        dialect: Dialect,
        scan: &mut Scan,
    ) -> sf_sparql::Result<Vec<SqlCond>> {
        let sf_sparql::iq::ScanSource::Logical(logical) = &mut scan.source else {
            return Err(denied());
        };
        let table = match &*logical {
            LogicalSource::Table(table) => table.clone(),
            LogicalSource::Query(sql) => {
                sf_sql::policy_projection::single_table_view_source(sql, dialect)
                    .map_err(|_| denied())?
            }
        };
        let matching: Vec<_> = self
            .rules
            .iter()
            .filter(|rule| rule.source == source && rule.table.as_ref() == table.as_str())
            .collect();
        if matching.is_empty() {
            return Err(denied());
        }
        if let LogicalSource::Query(sql) = logical {
            let columns: Vec<_> = matching.iter().map(|rule| rule.column.as_ref()).collect();
            *sql =
                sf_sql::policy_projection::expose_single_table_view_columns(sql, dialect, &columns)
                    .map_err(|_| denied())?;
        }
        Ok(matching
            .into_iter()
            .map(|rule| {
                SqlCond::Cmp(
                    ColRef::new(scan.alias, rule.column.clone()),
                    CmpOp::Eq,
                    rule.value.to_string(),
                )
            })
            .collect())
    }
}

fn authorize_branch(
    policy: &PortableRowPolicy,
    source: SourceId,
    dialect: Dialect,
    branch: &mut Branch,
) -> sf_sparql::Result<()> {
    // Path CTEs carry their own nested source grammar. Until policy predicates
    // are represented inside every hop arm, rejecting is the only exact result.
    if branch.path.is_some() {
        return Err(denied());
    }
    for scan in &mut branch.core {
        branch
            .where_conds
            .extend(policy.conditions(source, dialect, scan)?);
    }
    for optional in &mut branch.opts {
        authorize_conditions(policy, source, dialect, &mut optional.on)?;
        authorize_conditions(policy, source, dialect, &mut optional.extra)?;
        optional
            .extra
            .extend(policy.conditions(source, dialect, &mut optional.scan)?);
    }
    authorize_conditions(policy, source, dialect, &mut branch.where_conds)?;
    for nested in &mut branch.subplan_joins {
        authorize_conditions(policy, source, dialect, &mut nested.on)?;
        let nested_dialect = nested.plan.dialect;
        for child in &mut nested.plan.branches {
            authorize_branch(policy, source, nested_dialect, child)?;
        }
    }
    Ok(())
}

fn authorize_conditions(
    policy: &PortableRowPolicy,
    source: SourceId,
    dialect: Dialect,
    conditions: &mut [SqlCond],
) -> sf_sparql::Result<()> {
    for condition in conditions.iter_mut() {
        authorize_condition(policy, source, dialect, condition)?;
    }
    Ok(())
}

fn authorize_condition(
    policy: &PortableRowPolicy,
    source: SourceId,
    dialect: Dialect,
    condition: &mut SqlCond,
) -> sf_sparql::Result<()> {
    match condition {
        SqlCond::Not(inner) => authorize_condition(policy, source, dialect, inner),
        SqlCond::And(inner) | SqlCond::Or(inner) => {
            authorize_conditions(policy, source, dialect, inner)
        }
        SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
            authorize_conditions(policy, source, dialect, conds)?;
            for scan in scans.iter_mut() {
                conds.extend(policy.conditions(source, dialect, scan)?);
            }
            Ok(())
        }
        SqlCond::PathExists { .. } => Err(denied()),
        _ => Ok(()),
    }
}

fn valid_name(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn hash_framed(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value.as_bytes());
}

fn invalid_policy() -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: "portable row policy is missing or invalid".into(),
    })
}

fn denied() -> Error {
    Error::Mapping("portable row authorization denied".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#people> a rr:TriplesMap; rr:logicalTable [rr:tableName "people"];
 rr:subjectMap [rr:template "http://ex/{id}"];
 rr:predicateObjectMap [rr:predicate <http://ex/name>; rr:objectMap [rr:column "name"]].
"#;

    fn policy(table: &str) -> PortableRowPolicy {
        PortableRowPolicy::new(vec![PortableRowRule::new(0, table, "tenant", "a").unwrap()])
            .unwrap()
    }

    #[test]
    fn compiled_base_table_gets_a_bound_predicate() {
        let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();
        let query = spargebra::SparqlParser::new()
            .parse_query("SELECT ?name WHERE { ?s <http://ex/name> ?name }")
            .unwrap();
        for (dialect, placeholder) in [
            (sf_sql::Dialect::Sqlite, "?"),
            (sf_sql::Dialect::Postgres, "$1"),
            (sf_sql::Dialect::MySql, "?"),
        ] {
            let mut plan = sf_sparql::translate(&query, &maps, dialect).unwrap();
            if let Err(error) = policy("people").authorize(SourceId::new(0).unwrap(), &mut plan) {
                panic!("{error:?}; plan={plan:#?}");
            }
            let emitted = plan.emitted().unwrap();
            assert!(emitted[0].sql.contains("tenant"), "{}", emitted[0].sql);
            assert!(emitted[0].sql.contains(placeholder), "{}", emitted[0].sql);
            assert_eq!(emitted[0].params, ["a"]);
        }
    }

    #[test]
    fn uncovered_table_and_sensitive_debug_fail_closed() {
        let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();
        let query = spargebra::SparqlParser::new()
            .parse_query("SELECT ?name WHERE { ?s <http://ex/name> ?name }")
            .unwrap();
        let mut plan = sf_sparql::translate(&query, &maps, sf_sql::Dialect::Sqlite).unwrap();
        let policy = PortableRowPolicy::new(vec![PortableRowRule::new(
            0,
            "other",
            "tenant",
            "tenant-secret-value",
        )
        .unwrap()])
        .unwrap();
        assert!(policy
            .authorize(SourceId::new(0).unwrap(), &mut plan)
            .is_err());
        assert!(!format!("{policy:?}").contains("tenant-secret-value"));
    }
}
