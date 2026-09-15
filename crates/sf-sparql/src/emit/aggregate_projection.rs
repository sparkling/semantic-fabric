//! Positional layouts for SQL and derived-column metadata.
use super::source_control::validation_error as controlled_error;
use super::*;
use sf_sql::source_work::{SourceVec, SourceWork};

#[cfg(test)]
mod controlled_tests {
    use super::*;
    use crate::iq::{OptJoin, Scan};
    use sf_core::ir::{LogicalSource, Template, TermMap, TermSpec};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    fn budget(units: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
    }
    fn scan(alias: usize) -> Scan {
        Scan {
            alias,
            source: LogicalSource::Table("t".into()).into(),
        }
    }
    #[test]
    fn merged_projection_and_distinct_match_explicit_overlay() {
        for contributors in 0..=2 {
            for spec in [TermSpec::iri(), TermSpec::plain_literal()] {
                for template in ["{a}/{b}", "{a}{b}"] {
                    let mut b = Branch::empty();
                    b.core = (0..contributors).map(scan).collect();
                    b.bindings.insert(
                        "middle".into(),
                        TermDef::Const(sf_core::Literal::new_simple_literal("constant").into()),
                    );
                    let overlay = [(
                        "hidden".into(),
                        TermDef::Derived {
                            alias: 0,
                            term_map: TermMap::Template(
                                Template::parse(template).unwrap(),
                                spec.clone(),
                            ),
                        },
                    )]
                    .into();
                    let work = SourceWork::new(None);
                    let view = BindingView::merged(&b.bindings, Some(&overlay), work).unwrap();
                    let mut oracle = b.clone();
                    oracle.bindings.extend(overlay.clone());
                    for distinct in [false, true] {
                        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
                            let expected = super::super::projection_layout::validate_distinct_with(
                                &oracle, distinct,
                            )
                            .map(|dedup| (oracle.projection_with_distinct(distinct), dedup));
                            let actual =
                                projection_from_bindings(&b, &view, dialect, distinct, work);
                            assert_eq!(
                                actual.map_err(|e| e.to_string()),
                                expected.map_err(|e| e.to_string())
                            );
                        }
                    }
                    assert_eq!(b.bindings.len(), 1);
                }
            }
        }
    }
    #[test]
    fn controlled_projection_matches_raw_order_distinct_and_errors() {
        for distinct in [false, true] {
            for contributors in [1, 2] {
                for spec in [
                    TermSpec::iri(),
                    TermSpec::plain_literal(),
                    TermSpec::blank_node(),
                ] {
                    for template in ["{a}", "{a}/{b}", "{a}{b}", "literal"] {
                        let mut b = Branch::single(scan(0));
                        if contributors == 2 {
                            b.core.push(scan(1));
                        }
                        b.distinct = distinct;
                        b.bindings.insert(
                            "v".into(),
                            TermDef::Derived {
                                alias: 0,
                                term_map: TermMap::Template(
                                    Template::parse(template).unwrap(),
                                    spec.clone(),
                                ),
                            },
                        );
                        b.where_conds = vec![SqlCond::Not(Box::new(SqlCond::Or(vec![
                            SqlCond::ColEq(ColRef::new(0, "a"), ColRef::new(1, "extra")),
                            SqlCond::IsNull(ColRef::new(1, "extra")),
                        ])))];
                        if contributors == 2 {
                            b.opts.push(OptJoin {
                                scan: scan(2),
                                on: vec![SqlCond::IsNull(ColRef::new(2, "on"))],
                                extra: vec![SqlCond::IsNotNull(ColRef::new(2, "extra"))],
                            });
                        }
                        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
                            let raw = super::super::projection_layout(&b, dialect)
                                .map_err(|e| e.to_string());
                            let measured = budget(u64::MAX);
                            let run = |control: &QueryBudget| {
                                projection_controlled(
                                    &b,
                                    dialect,
                                    distinct,
                                    SourceWork::new(Some(control)),
                                )
                            };
                            assert_eq!(run(&measured).map_err(|e| e.to_string()), raw);
                            let total = measured.consumed(QueryCharge::SourceWork);
                            assert_eq!(run(&budget(total)).map_err(|e| e.to_string()), raw);
                            assert!(matches!(
                                run(&budget(total - 1)),
                                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                            ));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn controlled_projection_handles_deep_conditions_without_stack_recursion() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut c = SqlCond::IsNull(ColRef::new(0, "key"));
                for _ in 0..4096 {
                    c = SqlCond::Not(Box::new(c));
                }
                let mut b = Branch::single(scan(0));
                b.where_conds.push(c);
                let measured = budget(u64::MAX);
                assert_eq!(
                    projection_controlled(
                        &b,
                        Dialect::Sqlite,
                        false,
                        SourceWork::new(Some(&measured))
                    )
                    .unwrap(),
                    vec![ColRef::new(0, "key")]
                );
                let units = measured.consumed(QueryCharge::SourceWork);
                assert!(projection_controlled(
                    &b,
                    Dialect::Sqlite,
                    false,
                    SourceWork::new(Some(&budget(units)))
                )
                .is_ok());
                assert!(projection_controlled(
                    &b,
                    Dialect::Sqlite,
                    false,
                    SourceWork::new(Some(&budget(units - 1)))
                )
                .is_err());
                let mut c = b.where_conds.pop().unwrap();
                while let SqlCond::Not(next) = c {
                    c = *next;
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn controlled_projection_preserves_aggregate_duplicates_and_sqlite_avg_operand() {
        let mut b = Branch::single(scan(0));
        let col = ColRef::new(0, "key");
        b.agg = Some(Aggregation {
            keys: vec![crate::iq::GroupKey {
                var: "v".into(),
                cols: vec![col.clone(), col.clone()],
            }],
            aggs: vec![AggCol {
                var: "avg".into(),
                kind: AggKind::Avg,
                arg: Some(col.clone()),
                distinct: false,
                out: col,
                fixed_type: None,
            }],
        });
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
            let expected = super::super::projection_layout(&b, dialect).unwrap();
            assert_eq!(
                expected.len(),
                if dialect == Dialect::Sqlite { 4 } else { 3 }
            );
            let measured = budget(u64::MAX);
            let run = |control: &QueryBudget| {
                projection_controlled(&b, dialect, true, SourceWork::new(Some(control)))
            };
            assert_eq!(run(&measured).unwrap(), expected);
            let total = measured.consumed(QueryCharge::SourceWork);
            assert_eq!(run(&budget(total)).unwrap(), expected);
            assert!(matches!(
                run(&budget(total - 1)),
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
        }
    }
}

pub(super) fn add_column(
    out: &mut SourceVec<ColRef>,
    alias: usize,
    name: &str,
    unique: bool,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(1).map_err(controlled_error)?;
    if unique {
        for existing in out.as_slice() {
            work.charge(1).map_err(controlled_error)?;
            if existing.alias == alias {
                work.charge(existing.column.len().min(name.len()))
                    .map_err(controlled_error)?;
                if existing.column.as_ref() == name {
                    return Ok(());
                }
            }
        }
    }
    let name = work.string(name).map_err(controlled_error)?;
    work.charge(name.len()).map_err(controlled_error)?;
    out.push(ColRef::new(alias, name.into_boxed_str()), work)
        .map_err(controlled_error)
}

/// Same first-occurrence layout and DISTINCT law as the raw metadata path,
/// without recursive column collection or unaccounted vector/string copies.
pub(crate) fn projection_controlled(
    b: &Branch,
    dialect: Dialect,
    distinct: bool,
    work: SourceWork<'_>,
) -> Result<Vec<ColRef>> {
    projection_from_bindings(
        b,
        &BindingView::Direct(&b.bindings),
        dialect,
        distinct,
        work,
    )
    .map(|(columns, _)| columns)
}

/// Keep projection order and the SQL-vs-term DISTINCT decision on one view.
pub(super) fn projection_from_bindings(
    b: &Branch,
    bindings: &BindingView<'_>,
    dialect: Dialect,
    distinct: bool,
    work: SourceWork<'_>,
) -> Result<(Vec<ColRef>, bool)> {
    work.charge(1).map_err(controlled_error)?;
    let mut out = SourceVec::default();
    let mut term_dedup = false;
    if b.path.is_none() {
        if let Some(agg) = &b.agg {
            for key in &agg.keys {
                work.charge(1).map_err(controlled_error)?;
                for col in &key.cols {
                    add_column(&mut out, col.alias, &col.column, false, work)?;
                }
            }
            for aggregate in &agg.aggs {
                work.charge(1).map_err(controlled_error)?;
                add_column(
                    &mut out,
                    aggregate.out.alias,
                    &aggregate.out.column,
                    false,
                    work,
                )?;
                if dialect == Dialect::Sqlite && aggregate.kind == AggKind::Avg {
                    if let Some(col) = &aggregate.arg {
                        add_column(&mut out, col.alias, &col.column, false, work)?;
                    }
                }
            }
            return Ok((out.into_vec(), false));
        }
        term_dedup = validate_distinct_controlled(b, bindings, distinct, work)?;
    }
    for (_, def) in bindings.iter() {
        source_control::validate_definition_columns(def, work, |alias, name| {
            add_column(&mut out, alias, name, true, work)
        })?;
    }
    if !distinct {
        condition_columns(&b.where_conds, &mut out, work)?;
        for opt in &b.opts {
            work.charge(1).map_err(controlled_error)?;
            condition_columns(&opt.on, &mut out, work)?;
            condition_columns(&opt.extra, &mut out, work)?;
        }
        for join in &b.subplan_joins {
            work.charge(1).map_err(controlled_error)?;
            condition_columns(&join.on, &mut out, work)?;
        }
    }
    work.checkpoint().map_err(controlled_error)?;
    Ok((out.into_vec(), term_dedup))
}

fn validate_distinct_controlled(
    b: &Branch,
    bindings: &BindingView<'_>,
    distinct: bool,
    work: SourceWork<'_>,
) -> Result<bool> {
    if !distinct {
        return Ok(false);
    }
    let mut noninjective = false;
    let mut safe = true;
    for (_, def) in bindings.iter() {
        work.charge(1).map_err(controlled_error)?;
        let injective = match def {
            TermDef::Derived { term_map, .. } => ref_atom::injective_map(term_map, work)?,
            TermDef::R2rmlBlank {
                term_map, graph, ..
            } => {
                ref_atom::injective_map(term_map, work)?
                    && match graph {
                        R2rmlGraphScope::Default => true,
                        R2rmlGraphScope::Mapped { term_map, .. } => {
                            ref_atom::injective_map(term_map, work)?
                        }
                    }
            }
            _ => true,
        };
        noninjective |= !injective;
        safe &= crate::cascade::binding_is_term_dedup_safe_with_injectivity(def, injective);
    }
    let one_source = b
        .core
        .len()
        .saturating_add(b.opts.len())
        .saturating_add(b.subplan_joins.len())
        <= 1;
    if noninjective && !(one_source && safe) {
        let message = "SELECT DISTINCT over a non-injective term cannot be pushed to raw SQL DISTINCT soundly -> 501 (ADR-0025 C.3)";
        return Err(Error::Unsupported(
            work.string(message).map_err(controlled_error)?,
        ));
    }
    Ok(noninjective && one_source && safe)
}

pub(super) fn condition_columns(
    conditions: &[SqlCond],
    out: &mut SourceVec<ColRef>,
    work: SourceWork<'_>,
) -> Result<()> {
    use crate::iq::iri_cmp::{IriOperand, IriPart};
    use sf_core::ir::Segment;
    enum Item<'a> {
        One(&'a SqlCond),
        Many(&'a [SqlCond]),
    }
    let mut stack = SourceVec::default();
    stack
        .push(Item::Many(conditions), work)
        .map_err(controlled_error)?;
    while let Some(item) = stack.pop() {
        work.charge(1).map_err(controlled_error)?;
        let cond = match item {
            Item::One(c) => c,
            Item::Many(cs) => {
                if let Some((first, rest)) = cs.split_first() {
                    stack
                        .push(Item::Many(rest), work)
                        .map_err(controlled_error)?;
                    stack
                        .push(Item::One(first), work)
                        .map_err(controlled_error)?;
                }
                continue;
            }
        };
        let mut add = |c: &ColRef| add_column(out, c.alias, &c.column, true, work);
        match cond {
            SqlCond::ExpressionError
            | SqlCond::Exists { .. }
            | SqlCond::NotExists { .. }
            | SqlCond::PathExists { .. } => {}
            SqlCond::LiteralCmp(cmp) => {
                work.charge(2).map_err(controlled_error)?;
                for c in cmp.columns() {
                    add(c)?;
                }
            }
            SqlCond::IriCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    work.charge(1).map_err(controlled_error)?;
                    match operand {
                        IriOperand::Constant(_) => {}
                        IriOperand::Column { column, .. } => add(column)?,
                        IriOperand::Template { parts, .. } => {
                            for part in parts {
                                work.charge(1).map_err(controlled_error)?;
                                if let IriPart::Column(c) = part {
                                    add(c)?;
                                }
                            }
                        }
                    }
                }
            }
            SqlCond::ColEq(a, b) | SqlCond::NativeColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
                add(a)?;
                add(b)?;
            }
            SqlCond::Cmp(c, ..)
            | SqlCond::NativeCmp(c, ..)
            | SqlCond::IsNotNull(c)
            | SqlCond::DecodedIsNotNull(c)
            | SqlCond::IsNull(c) => add(c)?,
            SqlCond::StrMatch { col, .. } => add(col)?,
            SqlCond::Not(c) => stack.push(Item::One(c), work).map_err(controlled_error)?,
            SqlCond::And(cs) | SqlCond::Or(cs) => {
                stack.push(Item::Many(cs), work).map_err(controlled_error)?
            }
            SqlCond::TemplateEq(a, x, b, y, _) => {
                for (segments, alias) in [(a, x), (b, y)] {
                    for segment in segments {
                        work.charge(1).map_err(controlled_error)?;
                        if let Segment::Column(name) = segment {
                            add_column(out, *alias, name, true, work)?;
                        }
                    }
                }
            }
        }
    }
    work.checkpoint().map_err(controlled_error)
}

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
