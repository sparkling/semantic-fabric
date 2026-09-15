//! Ordered, per-arm subplan metadata merge. Child branches are built by metadata.rs.
use super::*;
use sf_core::datatype::XsdTypeCode;
use sf_sql::source_work::{SourceVec, SourceWork};
use source_control::validation_error as error;

/// Metadata records source positions, not aggregate outputs, and does not
/// validate DISTINCT eligibility. Validation remains a separate emission step.
pub(super) fn source_layout(
    branch: &Branch,
    distinct: bool,
    dialect: Dialect,
    work: SourceWork<'_>,
) -> Result<Vec<Option<ColRef>>> {
    let mut output = SourceVec::default();
    work.charge(1).map_err(error)?;
    if let Some(agg) = branch.agg.as_ref().filter(|_| branch.path.is_none()) {
        let mut add = |column: Option<&ColRef>| -> Result<()> {
            let column = if let Some(column) = column {
                let name = work.string(&column.column).map_err(error)?;
                work.charge(name.len()).map_err(error)?;
                Some(ColRef::new(column.alias, name.into_boxed_str()))
            } else {
                None
            };
            output.push(column, work).map_err(error)
        };
        for key in &agg.keys {
            work.charge(1).map_err(error)?;
            for column in &key.cols {
                work.charge(1).map_err(error)?;
                add(Some(column))?;
            }
        }
        for aggregate in &agg.aggs {
            work.charge(1).map_err(error)?;
            add(None)?;
            if dialect == Dialect::Sqlite && aggregate.kind == AggKind::Avg {
                if let Some(column) = &aggregate.arg {
                    add(Some(column))?;
                }
            }
        }
    } else {
        let mut columns = SourceVec::default();
        for def in branch.bindings.values() {
            source_control::validate_definition_columns(def, work, |alias, name| {
                aggregate_projection::add_column(&mut columns, alias, name, true, work)
            })?;
        }
        if !distinct {
            aggregate_projection::condition_columns(&branch.where_conds, &mut columns, work)?;
            for opt in &branch.opts {
                work.charge(1).map_err(error)?;
                aggregate_projection::condition_columns(&opt.on, &mut columns, work)?;
                aggregate_projection::condition_columns(&opt.extra, &mut columns, work)?;
            }
            for join in &branch.subplan_joins {
                work.charge(1).map_err(error)?;
                aggregate_projection::condition_columns(&join.on, &mut columns, work)?;
            }
        }
        for column in columns.into_vec() {
            output.push(Some(column), work).map_err(error)?;
        }
    }
    work.checkpoint().map_err(error)?;
    Ok(output.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

    #[test]
    fn controlled_subplan_metadata_finish_preserves_facts_and_stable_cost() {
        let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
            for scalar in [
                None,
                Some(NativeScalarKey::PostgresFloat4),
                Some(NativeScalarKey::Integer),
            ] {
                let mut cost = None;
                for order in [[0, 10], [10, 0]] {
                    let make = || Accumulator {
                        path: true,
                        width: 12,
                        common: Some(order.map(|i| (i, TextKey::Verbatim)).into()),
                        common_static_iris: Some(order.into()),
                        common_scalars: scalar.map(|key| order.map(|i| (i, key)).into()),
                        common_datatypes: Some(
                            order.map(|i| (i, Some(XsdTypeCode::Double))).into(),
                        ),
                        common_temporals: Some(
                            order.map(|i| (i, Some(XsdTypeCode::Double))).into(),
                        ),
                    };
                    let natural = if dialect == Dialect::Postgres
                        && scalar != Some(NativeScalarKey::PostgresFloat4)
                    {
                        None
                    } else {
                        Some(XsdTypeCode::Double)
                    };
                    let expected = AliasActuals {
                        datatype_columns: [0, 10]
                            .map(|i| (format!("c{i}"), Some(XsdTypeCode::Double)))
                            .into(),
                        natural_columns: [0, 10].map(|i| (format!("c{i}"), natural)).into(),
                        scalar_columns: scalar
                            .map(|key| [0, 10].map(|i| (format!("c{i}"), key)).into())
                            .unwrap_or_default(),
                        sqlite_columns: HashMap::new(),
                        lexical_columns: HashMap::new(),
                        lexical_comparison_columns: HashMap::new(),
                        source_kind: AliasSourceKind::Derived,
                        columns: (0..12).map(|i| format!("c{i}")).collect(),
                        path: true,
                        text_columns: [0, 10].map(|i| (format!("c{i}"), TextKey::Verbatim)).into(),
                        static_iri_columns: ["c0".into(), "c10".into()].into(),
                        iri_unreserved_columns: HashSet::new(),
                    };
                    let run = |control: &QueryBudget| {
                        make().finish(dialect, SourceWork::new(Some(control)))
                    };
                    let measured = budget(u64::MAX);
                    assert_eq!(run(&measured).unwrap(), expected);
                    let n = measured.consumed(QueryCharge::SourceWork);
                    assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
                    if let Some(previous) = cost {
                        assert_eq!(n, previous);
                    } else {
                        cost = Some(n);
                    }
                    assert_eq!(run(&budget(n)).unwrap(), expected);
                    assert!(matches!(
                        run(&budget(n - 1)),
                        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                    ));
                }
            }
        }
    }
}

fn output_name(index: usize, work: SourceWork<'_>) -> Result<String> {
    use std::fmt::Write;
    let len = index.checked_ilog10().unwrap_or(0) as usize + 2;
    work.product(2, len).map_err(error)?;
    let mut name = String::new();
    name.try_reserve_exact(len)
        .map_err(|_| Error::Sql("subplan name allocation failed".into()))?;
    write!(&mut name, "c{index}")
        .map_err(|_| Error::Sql("subplan name formatting failed".into()))?;
    Ok(name)
}

fn output_map<V>(
    source: Option<HashMap<usize, V>>,
    work: SourceWork<'_>,
) -> Result<HashMap<String, V>> {
    let source = source.unwrap_or_default();
    let mut entries = work.vector(source.len()).map_err(error)?;
    for entry in source {
        work.charge(1).map_err(error)?;
        entries.push(entry);
    }
    // Stable admission independent of randomized hash iteration. A quadratic
    // comparison/move envelope bounds the in-place unstable index sort.
    work.product(entries.len(), entries.len()).map_err(error)?;
    let moves = entries
        .len()
        .checked_mul(entries.len())
        .expect("product already admitted above");
    work.product(moves, std::mem::size_of::<(usize, V)>())
        .map_err(error)?;
    entries.sort_unstable_by_key(|(index, _)| *index);
    let mut out = HashMap::new();
    for (index, value) in entries {
        work.charge(1).map_err(error)?;
        let name = output_name(index, work)?;
        metadata_ref_atom::insert(&mut out, &name, value, work)?;
    }
    work.checkpoint().map_err(error)?;
    Ok(out)
}

#[derive(Default)]
pub(super) struct Accumulator {
    path: bool,
    width: usize,
    common: Option<HashMap<usize, TextKey>>,
    common_static_iris: Option<HashSet<usize>>,
    common_scalars: Option<HashMap<usize, NativeScalarKey>>,
    common_datatypes: Option<HashMap<usize, Option<XsdTypeCode>>>,
    common_temporals: Option<HashMap<usize, Option<XsdTypeCode>>>,
}

impl Accumulator {
    pub(super) fn add(
        &mut self,
        plan: &crate::Plan,
        branch: &Branch,
        dialect: Dialect,
        actuals: &ActualColumns,
        branch_path: bool,
        work: SourceWork<'_>,
    ) -> Result<()> {
        let Self {
            path,
            mut width,
            mut common,
            mut common_static_iris,
            mut common_scalars,
            mut common_datatypes,
            mut common_temporals,
        } = std::mem::take(self);
        let effective_distinct = if plan.branches.len() == 1 {
            plan.distinct
        } else {
            branch.distinct
        };
        let projection = source_layout(branch, effective_distinct, dialect, work)?;
        width = width.max(projection.len());
        use metadata_source::{index_insert, metadata_lookup as lookup};
        let (mut datatypes, mut temporals, mut scalars, mut text) = (
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );
        let mut static_iris = HashSet::new();
        let mut verbatim = None;
        for (index, column) in projection.iter().enumerate() {
            work.charge(1).map_err(error)?;
            let Some(column) = column else { continue };
            work.charge(actuals.len().saturating_add(1))
                .map_err(error)?;
            let Some(source) = actuals.get(&column.alias) else {
                continue;
            };
            for candidate in &source.columns {
                work.charge(3).map_err(error)?;
                work.product(2, candidate.len().min(column.column.len()))
                    .map_err(error)?;
            }
            let name = resolve_col(&column.column, Some(&source.columns));
            if let Some(code) = lookup(&source.datatype_columns, name, work)?.copied() {
                index_insert(
                    &mut datatypes,
                    index,
                    literal_datatype::after_union(code, dialect, plan.branches.len()),
                    work,
                )?;
            }
            if let Some(code) = lookup(&source.natural_columns, name, work)?.copied() {
                // MySQL's pooled DECIMAL precision cannot retain natural authority.
                let code = if dialect == Dialect::MySql
                    && plan.branches.len() > 1
                    && code == Some(XsdTypeCode::Decimal)
                {
                    None
                } else {
                    literal_datatype::after_union(code, dialect, plan.branches.len())
                };
                index_insert(&mut temporals, index, code, work)?;
            }
            if let Some(key) = lookup(&source.scalar_columns, name, work)?
                .copied()
                .filter(|key| {
                    !matches!(
                        key,
                        NativeScalarKey::MysqlDate | NativeScalarKey::MysqlDateTime
                    )
                })
                .filter(|key| {
                    plan.branches.len() == 1
                        || !matches!(
                            key,
                            NativeScalarKey::MysqlDecimal
                                | NativeScalarKey::MysqlFloat4
                                | NativeScalarKey::MysqlFloat8
                                | NativeScalarKey::MysqlBit
                                | NativeScalarKey::MysqlTimestamp
                                | NativeScalarKey::MysqlTime
                        )
                })
            {
                index_insert(&mut scalars, index, key, work)?;
            }
            if let Some(key) = lookup(&source.text_columns, name, work)?.copied() {
                let force = match verbatim {
                    Some(force) => force,
                    None => {
                        let force = branch.agg.is_none()
                            && branch.path.is_none()
                            && (plan.distinct || effective_distinct)
                            && !metadata_source::term_dedup(branch, effective_distinct, work)?;
                        verbatim = Some(force);
                        force
                    }
                };
                index_insert(
                    &mut text,
                    index,
                    if force { TextKey::Verbatim } else { key },
                    work,
                )?;
            }
            if metadata_ref_atom::contains(&source.static_iri_columns, name, work)? {
                metadata_source::pay_index_insert(
                    static_iris.len(),
                    std::mem::size_of::<usize>(),
                    work,
                )?;
                static_iris
                    .try_reserve(1)
                    .map_err(|_| Error::Sql("metadata index allocation failed".into()))?;
                static_iris.insert(index);
            }
        }
        metadata_source::merge_types(&mut common_datatypes, datatypes, work)?;
        metadata_source::merge_types(&mut common_temporals, temporals, work)?;
        metadata_source::intersect(&mut common_scalars, scalars, work)?;
        metadata_source::intersect(&mut common, text, work)?;
        work.charge(1).map_err(error)?;
        match common_static_iris.as_mut() {
            None => common_static_iris = Some(static_iris),
            Some(common) => {
                work.charge(common.capacity()).map_err(error)?;
                work.product(common.len(), static_iris.len().saturating_add(2))
                    .map_err(error)?;
                common.retain(|index| static_iris.contains(index));
            }
        }

        *self = Self {
            path: path || branch_path,
            width,
            common,
            common_static_iris,
            common_scalars,
            common_datatypes,
            common_temporals,
        };
        work.checkpoint().map_err(error)
    }
    pub(super) fn finish(self, dialect: Dialect, work: SourceWork<'_>) -> Result<AliasActuals> {
        let Self {
            path,
            width,
            common,
            common_static_iris,
            common_scalars,
            common_datatypes,
            mut common_temporals,
        } = self;
        // Equal XSD double types do not prove equal native float decoders. A
        // FLOAT4/FLOAT8 UNION widens the former and changes Rust's raw spelling.
        if dialect == Dialect::Postgres {
            for (index, code) in common_temporals.iter_mut().flat_map(|keys| keys.iter_mut()) {
                work.charge(2).map_err(error)?;
                work.charge(common_scalars.as_ref().map_or(0, HashMap::len))
                    .map_err(error)?;
                if *code == Some(sf_core::datatype::XsdTypeCode::Double)
                    && !common_scalars
                        .as_ref()
                        .and_then(|keys| keys.get(index))
                        .is_some_and(|key| pg_float::is_float(*key))
                {
                    *code = None;
                }
            }
        }
        let mut columns = work.vector(width).map_err(error)?;
        for index in 0..width {
            work.charge(1).map_err(error)?;
            columns.push(output_name(index, work)?);
        }
        let source = common_static_iris.unwrap_or_default();
        let mut indices = work.vector(source.len()).map_err(error)?;
        for index in source {
            work.charge(1).map_err(error)?;
            indices.push(index);
        }
        work.product(indices.len(), indices.len()).map_err(error)?;
        let moves = indices
            .len()
            .checked_mul(indices.len())
            .expect("product already admitted above");
        work.product(moves, std::mem::size_of::<usize>())
            .map_err(error)?;
        indices.sort_unstable();
        let mut static_iri_columns = HashSet::new();
        for index in indices {
            work.charge(1).map_err(error)?;
            let name = output_name(index, work)?;
            metadata_ref_atom::set_insert(&mut static_iri_columns, &name, work)?;
        }
        let out = AliasActuals {
            datatype_columns: output_map(common_datatypes, work)?,
            natural_columns: output_map(common_temporals, work)?,
            scalar_columns: output_map(common_scalars, work)?,
            sqlite_columns: HashMap::new(),
            lexical_columns: HashMap::new(),
            lexical_comparison_columns: HashMap::new(),
            source_kind: AliasSourceKind::Derived,
            columns,
            path,
            text_columns: output_map(common, work)?,
            static_iri_columns,
            // Conservatively discard this optimization across pooled outputs.
            iri_unreserved_columns: HashSet::new(),
        };
        work.checkpoint().map_err(error)?;
        Ok(out)
    }
}
