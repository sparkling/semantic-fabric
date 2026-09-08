//! Typed source ownership for a relational scan. A generated path stays a path
//! until live emission; it is never an authored SQL query or table authority.
use sf_core::ir::{LogicalSource, TermMap};

use super::{Branch, PathClosure, SqlCond};

#[derive(Debug, Clone)]
pub enum ScanSource {
    Logical(LogicalSource),
    Path {
        closure: Box<PathClosure>,
        cte_alias: usize,
    },
    /// Compiler-owned projection, never an authored metadata/constraint source.
    /// Column recipes preserve raw values; templates retain their lexical recipe.
    Projection {
        input: Box<Scan>,
        columns: Vec<(Box<str>, TermMap)>,
        guards: Vec<SqlCond>,
        distinct: bool,
        /// Native comparison keys; bool also requires RDF-key equivalence.
        native_keys: Vec<(Box<str>, bool)>,
    },
}

impl From<LogicalSource> for ScanSource {
    fn from(source: LogicalSource) -> Self {
        Self::Logical(source)
    }
}

impl ScanSource {
    /// Only authored tables/queries confer ordinary source/constraint authority.
    pub fn logical(&self) -> Option<&LogicalSource> {
        match self {
            Self::Logical(source) => Some(source),
            Self::Path { .. } | Self::Projection { .. } => None,
        }
    }

    pub(crate) fn is_logical_projection(&self) -> bool {
        match self {
            Self::Logical(_) => true,
            Self::Projection { input, .. } => input.source.is_logical_projection(),
            Self::Path { .. } => false,
        }
    }

    /// The sealed same-named raw-column D1 shape, not arbitrary projections.
    /// This proof permits existing policy-column exposure / bounded-join restore;
    /// it does not confer general optimizer or source metadata authority.
    pub fn distinct_table(&self) -> Option<&str> {
        match self {
            Self::Projection {
                input,
                columns,
                guards,
                distinct: true,
                ..
            } if guards.is_empty()
                && !columns.is_empty()
                && columns.iter().all(
                    |(name, term)| matches!(term, TermMap::Column(column, _) if column == name),
                ) =>
            {
                match input.source.logical() {
                    Some(LogicalSource::Table(table)) => Some(table),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

impl Branch {
    /// Outer relation aliases, independent of authored source authority.
    /// Nested SubPlans are separate namespaces and traversed by their consumers.
    pub(crate) fn relation_scans(&self) -> Vec<&Scan> {
        fn conditions<'a>(cond: &'a SqlCond, scans: &mut Vec<&'a Scan>) {
            match cond {
                SqlCond::Exists {
                    scans: nested,
                    conds,
                }
                | SqlCond::NotExists {
                    scans: nested,
                    conds,
                } => {
                    scans.extend(nested);
                    for cond in conds {
                        conditions(cond, scans);
                    }
                }
                SqlCond::And(conds) | SqlCond::Or(conds) => {
                    for cond in conds {
                        conditions(cond, scans);
                    }
                }
                SqlCond::Not(cond) => conditions(cond, scans),
                _ => {}
            }
        }
        let mut scans: Vec<_> = self
            .core
            .iter()
            .chain(self.opts.iter().map(|opt| &opt.scan))
            .collect();
        for cond in self.where_conds.iter().chain(
            self.opts
                .iter()
                .flat_map(|opt| opt.on.iter().chain(&opt.extra)),
        ) {
            conditions(cond, &mut scans);
        }
        scans
    }
}

/// One FROM relation, with an outer alias independent of any inner path CTE.
#[derive(Debug, Clone)]
pub struct Scan {
    pub alias: usize,
    pub source: ScanSource,
}
