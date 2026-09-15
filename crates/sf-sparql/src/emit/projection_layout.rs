//! Projection layout and pre-source validation; neither requires executable SQL.
use super::*;

#[cfg(test)]
mod controlled_tests {
    use super::*;
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };
    use sf_sql::source_work::SourceWork;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Stop {
        budget: QueryBudget,
        calls: AtomicUsize,
        at: usize,
        cause: QueryControlError,
    }
    impl QueryControl for Stop {
        fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
            self.budget.checkpoint()
        }
        fn consume(
            &self,
            kind: QueryCharge,
            units: u64,
        ) -> std::result::Result<(), QueryControlError> {
            self.budget.consume(kind, units)?;
            if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
                self.budget.terminate(self.cause);
            }
            Ok(())
        }
        fn terminate(&self, cause: QueryControlError) -> QueryControlError {
            self.budget.terminate(cause)
        }
    }
    fn budget(units: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
    }
    #[test]
    fn reference_input_is_qualified_once_even_for_zero_outputs() {
        use crate::iq::{Scan, ScanSource};
        use sf_core::ir::TermSpec;
        let source = LogicalSource::Table("t".into());
        let mut catalog = ColumnCatalog::default();
        catalog.insert(&source, vec!["key".into()]);
        let input = |extra| {
            let mut branch = Branch::empty();
            branch.core = (0..2)
                .map(|alias| Scan {
                    alias,
                    source: source.clone().into(),
                })
                .collect();
            let key = ColRef::new(0, "key");
            branch
                .where_conds
                .push(SqlCond::NativeColEq(key.clone(), ColRef::new(1, "key")));
            branch
                .where_conds
                .extend((0..extra).map(|_| SqlCond::ExpressionError));
            branch.bindings.insert(
                "x".into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: TermMap::Column("key".into(), TermSpec::iri()),
                },
            );
            Scan {
                alias: 2,
                source: ScanSource::RefAtom {
                    input: Box::new(branch),
                    columns: vec![key],
                },
            }
        };
        let columns = [
            (
                Box::<str>::from("a"),
                TermMap::Column("c0".into(), TermSpec::iri()),
            ),
            (
                Box::<str>::from("b"),
                TermMap::Column("c0".into(), TermSpec::iri()),
            ),
        ];
        let cost = |extra, width| {
            let control = budget(u64::MAX);
            validate_projection_controlled(
                &input(extra),
                &columns[..width],
                &[],
                Dialect::Sqlite,
                &catalog,
                SourceWork::new(Some(&control)),
            )
            .unwrap();
            control.consumed(QueryCharge::SourceWork)
        };
        let extra_input_work = cost(128, 0) - cost(0, 0);
        assert!(extra_input_work > 0);
        for width in [1, 2] {
            assert_eq!(cost(128, width) - cost(0, width), extra_input_work);
        }
        let relation = input(1);
        let run = |control: &dyn QueryControl| {
            validate_projection_controlled(
                &relation,
                &columns,
                &[],
                Dialect::Sqlite,
                &catalog,
                SourceWork::new(Some(control)),
            )
        };
        let counted = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        run(&counted).unwrap();
        let total = counted.budget.consumed(QueryCharge::SourceWork);
        run(&budget(total)).unwrap();
        assert!(matches!(
            run(&budget(total - 1)),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=counted.calls.load(Ordering::Relaxed) {
                let stopped = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(
                    matches!(run(&stopped), Err(Error::QueryControl(actual)) if actual == cause)
                );
            }
        }
        let malformed = Scan {
            alias: 2,
            source: ScanSource::RefAtom {
                input: Box::new(Branch::empty()),
                columns: vec![],
            },
        };
        assert!(
            matches!(validate_projection_controlled(&malformed, &[], &[],
            Dialect::Sqlite, &catalog, SourceWork::new(Some(&budget(u64::MAX)))),
            Err(Error::Unsupported(message)) if message == "invalid reference atom relation")
        );
    }
    #[test]
    fn projection_guard_preflight_is_paid_and_shape_precedes_column_errors() {
        let source = LogicalSource::Table("t".into());
        let input = crate::iq::Scan {
            alias: 0,
            source: source.clone().into(),
        };
        let mut catalog = ColumnCatalog::default();
        catalog.insert(&source, vec!["x".into()]);
        let mut condition = SqlCond::IsNotNull(ColRef::new(0, Box::<str>::from("x")));
        for _ in 0..64 {
            condition = SqlCond::Not(Box::new(condition));
        }
        let guards = [condition];
        let run = |control: &dyn QueryControl| {
            validate_projection_controlled(
                &input,
                &[],
                &guards,
                Dialect::Sqlite,
                &catalog,
                SourceWork::new(Some(control)),
            )
        };
        let counted = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        run(&counted).unwrap();
        let total = counted.budget.consumed(QueryCharge::SourceWork);
        run(&budget(total)).unwrap();
        assert!(matches!(
            run(&budget(total - 1)),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=counted.calls.load(Ordering::Relaxed) {
                let stopped = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(
                    matches!(run(&stopped), Err(Error::QueryControl(actual)) if actual == cause)
                );
            }
        }
        let invalid = [SqlCond::And(vec![
            SqlCond::IsNotNull(ColRef::new(0, Box::<str>::from("missing"))),
            SqlCond::ExpressionError,
        ])];
        assert!(
            matches!(validate_projection_controlled(&input, &[], &invalid, Dialect::Sqlite, &catalog,
            SourceWork::new(Some(&budget(u64::MAX)))), Err(Error::Unsupported(message)) if message == "projection guard shape")
        );
    }
}

#[cfg(test)]
fn validate_projection_controlled(
    input: &crate::iq::Scan,
    columns: &[(Box<str>, TermMap)],
    guards: &[SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    validate_source_root(
        ValidationRoot::Projection(input, columns, guards),
        dialect,
        catalog,
        work,
    )
}

pub(super) fn validate_projection_level(
    input: &crate::iq::Scan,
    columns: &[(Box<str>, TermMap)],
    guards: &[SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    let mut outputs = source_control::SourceSet::default();
    for (name, term) in columns {
        work.charge(1).map_err(source_control::validation_error)?;
        if !outputs
            .insert_key((0, name.as_ref()), work)
            .map_err(source_control::validation_error)?
        {
            return Err(Error::Sql("duplicate projection output".into()));
        }
        match term {
            TermMap::Column(column, _) => {
                validate_input_column(input, column, dialect, catalog, work)?
            }
            TermMap::Template(template, _) => {
                for segment in template.segments() {
                    work.charge(1).map_err(source_control::validation_error)?;
                    if let Segment::Column(column) = segment {
                        validate_input_column(input, column, dialect, catalog, work)?;
                    }
                }
            }
            TermMap::Constant(_) => {
                return Err(Error::Unsupported("constant projection recipe".into()))
            }
        }
    }
    for guard in guards {
        work.charge(1).map_err(source_control::validation_error)?;
        match guard {
            SqlCond::IsNull(c)
            | SqlCond::IsNotNull(c)
            | SqlCond::NativeCmp(c, crate::iq::CmpOp::Eq, _)
                if c.alias == input.alias =>
            {
                validate_input_column(input, &c.column, dialect, catalog, work)?
            }
            _ => {
                // Prove the entire guard shape before resolving any of its
                // columns, preserving the previous error/admission order.
                for column in guard_columns(guard, input.alias, work)? {
                    validate_input_column(input, &column.column, dialect, catalog, work)?;
                }
            }
        }
    }

    work.checkpoint().map_err(source_control::validation_error)
}

fn guard_columns<'a>(
    guard: &'a SqlCond,
    alias: usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Vec<&'a ColRef>> {
    use crate::iq::iri_cmp::{IriOperand, IriPart};
    let invalid = || Error::Unsupported("projection guard shape".into());
    let mut pending = sf_sql::source_work::SourceVec::default();
    let mut columns = sf_sql::source_work::SourceVec::default();
    pending
        .push(guard, work)
        .map_err(source_control::validation_error)?;
    let mut add = |column: &'a ColRef| -> Result<()> {
        work.charge(1).map_err(source_control::validation_error)?;
        if column.alias != alias {
            return Err(invalid());
        }
        columns
            .push(column, work)
            .map_err(source_control::validation_error)
    };
    while let Some(condition) = pending.pop() {
        work.charge(1).map_err(source_control::validation_error)?;
        match condition {
            SqlCond::IsNull(c) | SqlCond::IsNotNull(c) | SqlCond::DecodedIsNotNull(c) => add(c)?,
            SqlCond::IriCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    work.charge(1).map_err(source_control::validation_error)?;
                    match operand {
                        IriOperand::Column { column, .. } => add(column)?,
                        IriOperand::Template { parts, .. } => {
                            for part in parts {
                                work.charge(1).map_err(source_control::validation_error)?;
                                if let IriPart::Column(column) = part {
                                    add(column)?;
                                }
                            }
                        }
                        IriOperand::Constant(_) => {}
                    }
                }
            }
            SqlCond::Not(inner) => pending
                .push(inner.as_ref(), work)
                .map_err(source_control::validation_error)?,
            SqlCond::And(parts) | SqlCond::Or(parts) => {
                for part in parts.iter().rev() {
                    pending
                        .push(part, work)
                        .map_err(source_control::validation_error)?;
                }
            }
            _ => return Err(invalid()),
        }
    }
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    Ok(columns.into_vec())
}

fn validate_input_column(
    input: &crate::iq::Scan,
    name: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    use crate::iq::ScanSource;
    work.charge(1).map_err(source_control::validation_error)?;
    match &input.source {
        ScanSource::RefAtom { columns, .. } => {
            ref_atom::validate_output_controlled(columns.len(), name, work)
        }
        ScanSource::Logical(source) => {
            catalog.validate_live_column_controlled(source, name, dialect, work)
        }
        ScanSource::Projection { columns, .. } => validate_output_controlled(columns, name, work),
        ScanSource::Path { .. } => Err(Error::Unsupported("projection over path".into())),
    }
}

pub(super) fn validate_output_controlled(
    columns: &[(Box<str>, TermMap)],
    name: &str,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    for (output, _) in columns {
        work.charge(1).map_err(source_control::validation_error)?;
        work.charge(output.len().min(name.len()))
            .map_err(source_control::validation_error)?;
        if output.as_ref() == name {
            return Ok(());
        }
    }
    let mut folded = 0;
    for (output, _) in columns {
        work.charge(1).map_err(source_control::validation_error)?;
        work.charge(output.len().min(name.len()))
            .map_err(source_control::validation_error)?;
        if output.eq_ignore_ascii_case(name) {
            folded += 1;
        }
    }
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    if folded == 1 {
        Ok(())
    } else {
        Err(Error::Sql("missing or ambiguous projection output".into()))
    }
}

/// Original source columns by emitted position; aggregate values have no raw
/// decoder authority. Shared by metadata and pre-source identity checks.
pub(crate) fn source_projection(
    branch: &Branch,
    distinct: bool,
    dialect: Dialect,
) -> Vec<Option<ColRef>> {
    match &branch.agg {
        Some(agg) if branch.path.is_none() => aggregate_projection(agg, dialect)
            .iter()
            .map(|item| item.source_column().cloned())
            .collect(),
        _ => branch
            .projection_with_distinct(distinct)
            .into_iter()
            .map(Some)
            .collect(),
    }
}

pub(crate) fn projection_layout(b: &Branch, dialect: Dialect) -> Result<Vec<ColRef>> {
    projection_layout_with_distinct(b, dialect, b.distinct)
}

pub(crate) fn projection_layout_with_distinct(
    b: &Branch,
    dialect: Dialect,
    distinct: bool,
) -> Result<Vec<ColRef>> {
    // Keep the same path-before-aggregate precedence as actual emission.
    if b.path.is_some() {
        return Ok(b.projection_with_distinct(distinct));
    }
    if let Some(agg) = &b.agg {
        return Ok(aggregate_projection(agg, dialect)
            .iter()
            .map(|item| item.column().clone())
            .collect());
    }
    validate_distinct_with(b, distinct)?;
    Ok(b.projection_with_distinct(distinct))
}

pub(super) fn validate_distinct_with(b: &Branch, distinct: bool) -> Result<bool> {
    let term_dedup = crate::cascade::eligible_for_term_dedup_with_distinct(b, distinct);
    if distinct
        && !term_dedup
        && b.bindings
            .values()
            .any(|def| !crate::cascade::binding_is_injective(def))
    {
        return Err(Error::Unsupported(
            "SELECT DISTINCT over a non-injective term cannot be pushed to raw SQL DISTINCT soundly -> 501 (ADR-0025 C.3)".into()
        ));
    }
    Ok(term_dedup)
}
