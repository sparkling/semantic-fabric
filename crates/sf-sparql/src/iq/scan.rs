//! Typed source ownership for a relational scan. A generated path stays a path
//! until live emission; it is never an authored SQL query or table authority.
use sf_core::ir::{LogicalSource, TermMap};

use super::{Branch, ColRef, PathClosure, SqlCond};

pub(crate) mod ref_atom;

#[derive(Debug, Clone)]
/// Every original consumer of this column is an IRI template.
pub struct LexicalKey {
    pub column: Box<str>,
}

#[derive(Debug, Clone)]
pub enum ScanSource {
    Logical(LogicalSource),
    /// One reference-object atom: native join/filter first, RDF tuple dedup second.
    /// The original leaves remain metadata/policy inputs, never outer table authority.
    RefAtom {
        input: Box<Branch>,
        columns: Vec<ColRef>,
    },
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
        /// Original RDF consumers are all IRI templates, not literals, blank
        /// nodes or base-resolved column IRIs. Captured
        /// before projection narrowing; synthetic `columns` specs are no proof.
        lexical_keys: Vec<LexicalKey>,
    },
}

impl From<LogicalSource> for ScanSource {
    fn from(source: LogicalSource) -> Self {
        Self::Logical(source)
    }
}

impl ScanSource {
    /// Column-level origin for native type compatibility only, not constraints.
    pub(crate) fn raw_column_origin<'a>(
        &'a self,
        name: &'a str,
    ) -> Option<(&'a LogicalSource, &'a str)> {
        match self {
            Self::Logical(source) => Some((source, name)),
            Self::RefAtom { input, columns } => {
                let column = columns
                    .iter()
                    .enumerate()
                    .find(|(i, _)| name == format!("c{i}"))?
                    .1;
                let scan = input.core.iter().find(|s| s.alias == column.alias)?;
                scan.source.raw_column_origin(&column.column)
            }
            // Other wrappers keep their existing separately captured authority.
            Self::Path { .. } | Self::Projection { .. } => None,
        }
    }
    /// Only authored tables/queries confer ordinary source/constraint authority.
    pub fn logical(&self) -> Option<&LogicalSource> {
        match self {
            Self::Logical(source) => Some(source),
            Self::Path { .. } | Self::Projection { .. } | Self::RefAtom { .. } => None,
        }
    }

    pub(crate) fn is_logical_projection(&self) -> bool {
        match self {
            Self::Logical(_) => true,
            Self::Projection { input, .. } => input.source.is_logical_projection(),
            Self::Path { .. } | Self::RefAtom { .. } => false,
        }
    }

    /// The sealed same-named raw-column D1 shape, not arbitrary projections.
    /// This proof permits policy-guard insertion / guard-free bounded-join restore;
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
