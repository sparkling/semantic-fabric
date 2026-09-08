//! Native Ref witnesses are joined/authorized before decoded RDF tuple dedup.
use super::*;

pub(super) fn actuals(
    input: &Branch,
    columns: &[ColRef],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> AliasActuals {
    let sources = branch_actuals(input, dialect, catalog);
    AliasActuals {
        source_kind: AliasSourceKind::Derived,
        columns: (0..columns.len()).map(|i| format!("c{i}")).collect(),
        path: false,
        text_columns: columns
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                path_comparison::column_text(c, &sources).map(|key| (format!("c{i}"), key))
            })
            .collect(),
    }
}

pub(super) fn validate_output(width: usize, name: &str) -> Result<()> {
    if (0..width).any(|i| name == format!("c{i}")) {
        Ok(())
    } else {
        Err(Error::Sql("missing reference atom output".into()))
    }
}

pub(super) fn sql(
    input: &Branch,
    columns: &[ColRef],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    crate::iq::scan::ref_atom::validate_shape(input, columns)?;
    let actuals = branch_actuals(input, dialect, catalog);
    // SQL numeric equality is not decoded RDF identity (notably +0/-0).
    // Only the live-proven text families authorize this new atom-level key.
    // Unknown families retain the previous per-source D1/native-join behavior;
    // neither a raw atom DISTINCT nor an undecuplicated join is that fallback.
    let window = columns
        .iter()
        .all(|c| path_comparison::column_text(c, &actuals).is_some());
    let legacy = if window {
        None
    } else {
        let mut legacy = input.clone();
        crate::cascade::force_distinct_for_dup_safety(
            std::slice::from_mut(&mut legacy),
            &[],
            dialect,
        );
        Some(legacy)
    };
    let input = legacy.as_ref().unwrap_or(input);
    let actuals = if legacy.is_some() {
        branch_actuals(input, dialect, catalog)
    } else {
        actuals
    };
    let from = render_from(input, dialect, catalog, &actuals, params, pidx)?;
    let filter = render_where(&input.where_conds, dialect, catalog, &actuals, params, pidx)?
        .map(|sql| format!(" WHERE {sql}"))
        .unwrap_or_default();
    let mut items = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &actuals)))
        .collect::<Vec<_>>();
    if !window {
        return Ok(format!("SELECT {} FROM {from}{filter}", items.join(", ")));
    }
    let keys = columns
        .iter()
        .map(|c| path_comparison::rdf_column(c, dialect, catalog, &actuals))
        .collect::<Vec<_>>();
    let keys = if keys.is_empty() {
        "1".to_owned()
    } else {
        keys.join(", ")
    };
    items.push(format!(
        "ROW_NUMBER() OVER (PARTITION BY {keys}) AS __sf_rank"
    ));
    let projected = if columns.is_empty() {
        "1 AS c0".to_owned()
    } else {
        (0..columns.len())
            .map(|i| format!("__sf_atom.c{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(format!("SELECT {projected} FROM (SELECT {} FROM {from}{filter}) __sf_atom WHERE __sf_atom.__sf_rank = 1", items.join(", ")))
}
