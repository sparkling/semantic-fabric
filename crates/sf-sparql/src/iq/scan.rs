//! Typed source ownership for a relational scan. A generated path stays a path
//! until live emission; it is never an authored SQL query or table authority.
use sf_core::ir::LogicalSource;

use super::PathClosure;

#[derive(Debug, Clone)]
pub enum ScanSource {
    Logical(LogicalSource),
    Path {
        closure: Box<PathClosure>,
        cte_alias: usize,
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
            Self::Path { .. } => None,
        }
    }
}

/// One FROM relation, with an outer alias independent of any inner path CTE.
#[derive(Debug, Clone)]
pub struct Scan {
    pub alias: usize,
    pub source: ScanSource,
}
