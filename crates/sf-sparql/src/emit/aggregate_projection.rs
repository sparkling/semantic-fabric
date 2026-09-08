//! One positional layout for aggregate SQL and derived-column metadata.
use super::*;

pub(super) enum AggregateProjection<'a> {
    Key(&'a ColRef),
    Aggregate(&'a AggCol),
    AvgOperand(&'a ColRef),
}

impl AggregateProjection<'_> {
    pub(super) fn column(&self) -> &ColRef {
        match self {
            Self::Key(column) | Self::AvgOperand(column) => column,
            Self::Aggregate(aggregate) => &aggregate.out,
        }
    }

    pub(super) fn source_column(&self) -> Option<&ColRef> {
        match self {
            Self::Key(column) | Self::AvgOperand(column) => Some(column),
            Self::Aggregate(_) => None,
        }
    }
}

pub(super) fn aggregate_projection(
    agg: &Aggregation,
    dialect: Dialect,
) -> Vec<AggregateProjection<'_>> {
    let mut projection = Vec::new();
    for key in &agg.keys {
        projection.extend(key.cols.iter().map(AggregateProjection::Key));
    }
    for aggregate in &agg.aggs {
        projection.push(AggregateProjection::Aggregate(aggregate));
        // SQLite AVG erases the operand datatype. Preserve the existing extra
        // bare operand, used only for reconstruction metadata, in this layout.
        if dialect == Dialect::Sqlite && aggregate.kind == AggKind::Avg {
            if let Some(operand) = &aggregate.arg {
                projection.push(AggregateProjection::AvgOperand(operand));
            }
        }
    }
    projection
}
