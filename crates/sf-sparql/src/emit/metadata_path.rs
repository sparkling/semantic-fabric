//! Branch-local path flags; completed subplan flags are propagated by the builder.
use super::*;
use crate::iq::ScanSource;
use sf_sql::source_work::{SourceVec, SourceWork};
use source_control::validation_error as error;

fn text_endpoint(
    source: &LogicalSource,
    column: &str,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<bool> {
    use metadata_source::metadata_lookup;
    let name = match source {
        LogicalSource::Table(name) | LogicalSource::Query(name) => name,
    };
    work.charge(name.len()).map_err(error)?;
    work.charge(2).map_err(error)?;
    let key = source_key(source);
    let columns = metadata_lookup(&catalog.by_source, &key, work)?.map(Vec::as_slice);
    for name in columns.unwrap_or_default() {
        // Admission inventory plus the exact and unique-folded lookup passes.
        work.charge(3).map_err(error)?;
        work.product(2, name.len().min(column.len()))
            .map_err(error)?;
    }
    let column = resolve_col(column, columns);
    let Some(text) = metadata_lookup(&catalog.text_by_source, &key, work)? else {
        return Ok(false);
    };
    Ok(metadata_lookup(text, column, work)?.is_some())
}

/// An endpoint proof is the conjunction of its selected leaves. Inverse swaps
/// the selection; sequence takes only its outer endpoint; alternatives require
/// every arm. Empty alternatives have no endpoint proof.
pub(super) fn hop_text(
    hop: &HopExpr,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<(bool, bool)> {
    enum Step<'a> {
        Visit(&'a HopExpr, bool),
        Many(&'a [HopExpr], bool),
    }
    let mut endpoints = [true; 2];
    for (index, endpoint) in endpoints.iter_mut().enumerate() {
        let mut pending = SourceVec::default();
        pending
            .push(Step::Visit(hop, index == 0), work)
            .map_err(error)?;
        while !pending.as_slice().is_empty() {
            work.charge(1).map_err(error)?;
            match pending.pop().expect("nonempty endpoint traversal") {
                Step::Visit(hop, subject) => match hop {
                    HopExpr::Pred(rel) => {
                        *endpoint &= text_endpoint(
                            &rel.source,
                            if subject { &rel.subj_col } else { &rel.obj_col },
                            catalog,
                            work,
                        )?;
                    }
                    HopExpr::Inverse(inner) => pending
                        .push(Step::Visit(inner, !subject), work)
                        .map_err(error)?,
                    HopExpr::Seq(left, right) => pending
                        .push(
                            Step::Visit(if subject { left } else { right }, subject),
                            work,
                        )
                        .map_err(error)?,
                    HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                        if parts.is_empty() {
                            *endpoint = false;
                        }
                        pending
                            .push(Step::Many(parts, subject), work)
                            .map_err(error)?;
                    }
                },
                Step::Many(parts, subject) => {
                    if let Some((part, rest)) = parts.split_first() {
                        if !rest.is_empty() {
                            pending
                                .push(Step::Many(rest, subject), work)
                                .map_err(error)?;
                        }
                        pending
                            .push(Step::Visit(part, subject), work)
                            .map_err(error)?;
                    }
                }
            }
        }
    }
    work.checkpoint().map_err(error)?;
    Ok((endpoints[0], endpoints[1]))
}

pub(super) fn actuals(
    path: &PathClosure,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<AliasActuals> {
    let (subject, object) = hop_text(&path.hop, catalog, work)?;
    let mut columns = work.vector(2).map_err(error)?;
    let mut text_columns = HashMap::new();
    work.charge(2).map_err(error)?;
    work.product(2, std::mem::size_of::<(String, TextKey)>())
        .map_err(error)?;
    text_columns
        .try_reserve(2)
        .map_err(|_| Error::Sql("path metadata allocation failed".into()))?;
    for (name, proven) in [("sf_s", subject), ("sf_o", object)] {
        columns.push(work.string(name).map_err(error)?);
        if proven {
            work.charge(1 + name.len()).map_err(error)?;
            work.product(text_columns.len(), 1 + name.len())
                .map_err(error)?;
            text_columns.insert(work.string(name).map_err(error)?, TextKey::Verbatim);
        }
    }
    work.checkpoint().map_err(error)?;
    Ok(AliasActuals {
        datatype_columns: HashMap::new(),
        natural_columns: HashMap::new(),
        scalar_columns: HashMap::new(),
        sqlite_columns: HashMap::new(),
        lexical_columns: HashMap::new(),
        lexical_comparison_columns: HashMap::new(),
        source_kind: AliasSourceKind::Derived,
        columns,
        path: true,
        text_columns,
        static_iri_columns: HashSet::new(),
        iri_unreserved_columns: HashSet::new(),
    })
}

pub(super) fn local_flag(branch: &Branch, work: SourceWork<'_>) -> Result<bool> {
    work.charge(1).map_err(error)?;
    let mut found = branch.path.is_some();
    for scan in &branch.core {
        work.charge(1).map_err(error)?;
        found |= matches!(scan.source, ScanSource::Path { .. });
    }
    let mut pending = SourceVec::default();
    pending
        .push(branch.where_conds.as_slice(), work)
        .map_err(error)?;
    for join in &branch.opts {
        work.charge(1).map_err(error)?;
        found |= matches!(join.scan.source, ScanSource::Path { .. });
        pending.push(join.on.as_slice(), work).map_err(error)?;
        pending.push(join.extra.as_slice(), work).map_err(error)?;
    }
    for join in &branch.subplan_joins {
        work.charge(1).map_err(error)?;
        pending.push(join.on.as_slice(), work).map_err(error)?;
    }
    while !pending.as_slice().is_empty() {
        work.charge(1).map_err(error)?;
        let conditions = pending.pop().expect("nonempty path flag stack");
        let Some((condition, rest)) = conditions.split_first() else {
            continue;
        };
        if !rest.is_empty() {
            pending.push(rest, work).map_err(error)?;
        }
        match condition {
            SqlCond::PathExists { .. } => found = true,
            SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                for scan in scans {
                    work.charge(1).map_err(error)?;
                    found |= matches!(scan.source, ScanSource::Path { .. });
                }
                pending.push(conds.as_slice(), work).map_err(error)?;
            }
            SqlCond::And(conditions) | SqlCond::Or(conditions) => {
                pending.push(conditions.as_slice(), work).map_err(error)?
            }
            SqlCond::Not(inner) => pending
                .push(std::slice::from_ref(inner.as_ref()), work)
                .map_err(error)?,
            _ => {}
        }
    }
    work.checkpoint().map_err(error)?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::HopRelation;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    fn budget(n: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX))
    }
    fn leaf(subject: &str, object: &str) -> HopExpr {
        HopExpr::Pred(HopRelation {
            source: LogicalSource::Table("items".into()),
            subj_col: subject.into(),
            obj_col: object.into(),
        })
    }
    fn catalog() -> ColumnCatalog {
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &LogicalSource::Table("items".into()),
                ["a", "b", "C", "FOO", "foo"]
                    .into_iter()
                    .map(|name| sf_sql::backend::ResultColumn {
                        name: name.into(),
                        natural_datatype: None,
                        native_scalar: None,
                        sqlite_decode: None,
                        text_key: ["a", "C", "FOO"]
                            .contains(&name)
                            .then_some(TextKey::Verbatim),
                    })
                    .collect(),
            )
            .unwrap();
        catalog
    }
    #[test]
    fn controlled_path_metadata_matches_recursive_endpoint_oracle() {
        let catalog = catalog();
        let names = ["a", "b", "c", "fOo"];
        for i in 0..4 {
            for j in 0..4 {
                let left = leaf(names[i], names[3 - i]);
                let right = leaf(names[j], names[3 - j]);
                for hop in [
                    left.clone(),
                    HopExpr::Inverse(Box::new(left.clone())),
                    HopExpr::Seq(Box::new(left.clone()), Box::new(right.clone())),
                    HopExpr::Alt(vec![left.clone(), right.clone()]),
                    HopExpr::Nps(vec![left.clone(), right.clone()]),
                    HopExpr::Alt(vec![]),
                    HopExpr::Nps(vec![]),
                    HopExpr::Inverse(Box::new(HopExpr::Seq(Box::new(left), Box::new(right)))),
                ] {
                    let expected = path_comparison::hop_text(&hop, &catalog);
                    let path = PathClosure {
                        alias: 0,
                        kind: PathKind::One,
                        hop,
                    };
                    let run = |control: &QueryBudget| {
                        actuals(&path, &catalog, SourceWork::new(Some(control)))
                    };
                    let measured = budget(u64::MAX);
                    let result = run(&measured).unwrap();
                    assert_eq!(result.columns, ["sf_s", "sf_o"]);
                    assert_eq!(
                        (
                            result.text_columns.contains_key("sf_s"),
                            result.text_columns.contains_key("sf_o")
                        ),
                        expected
                    );
                    assert!(result.path && result.source_kind == AliasSourceKind::Derived);
                    let n = measured.consumed(QueryCharge::SourceWork);
                    assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
                    assert_eq!(run(&budget(n)).unwrap().text_columns, result.text_columns);
                    assert!(matches!(
                        run(&budget(n - 1)),
                        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                    ));
                }
            }
        }
    }
    #[test]
    fn deep_path_metadata_uses_bounded_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let catalog = catalog();
                let mut hop = leaf("a", "b");
                for _ in 0..4097 {
                    hop = HopExpr::Inverse(Box::new(hop));
                }
                let measured = budget(u64::MAX);
                assert_eq!(
                    hop_text(&hop, &catalog, SourceWork::new(Some(&measured))).unwrap(),
                    (false, true)
                );
                let n = measured.consumed(QueryCharge::SourceWork);
                assert_eq!(
                    hop_text(&hop, &catalog, SourceWork::new(Some(&budget(n)))).unwrap(),
                    (false, true)
                );
                assert!(matches!(
                    hop_text(&hop, &catalog, SourceWork::new(Some(&budget(n - 1)))),
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                while let HopExpr::Inverse(inner) = hop {
                    hop = *inner;
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
