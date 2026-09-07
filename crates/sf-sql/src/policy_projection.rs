//! Fail-closed AST handling for compiler-generated single-table source views.

use sqlparser::ast::{
    Expr, GroupByExpr, Ident, SelectFlavor, SelectItem, SetExpr, Statement, TableFactor,
};

use crate::{Dialect, Error, Result};

/// Return the exact base-table name of a compiler-generated single-table view.
/// Complex/authored SQL shapes are deliberately rejected for portable policy.
pub fn single_table_view_source(sql: &str, dialect: Dialect) -> Result<String> {
    let mut statements = dialect.parse(sql)?;
    let select = generated_select(&mut statements)?;
    let table = select.from.first().expect("shape checked");
    let TableFactor::Table { name, .. } = &table.relation else {
        return Err(unsupported());
    };
    Ok(name
        .0
        .iter()
        .map(|part| part.as_ident().map(|ident| ident.value.as_str()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(unsupported)?
        .join("."))
}

/// Expose trusted policy columns from a compiler-generated view so the outer
/// query can compare them with ordinary bound parameters.
pub fn expose_single_table_view_columns(
    sql: &str,
    dialect: Dialect,
    columns: &[&str],
) -> Result<String> {
    if columns.is_empty() {
        return Err(unsupported());
    }
    let mut statements = dialect.parse(sql)?;
    let select = generated_select(&mut statements)?;
    let table = select.from.first().expect("shape checked");
    let TableFactor::Table { alias, .. } = &table.relation else {
        return Err(unsupported());
    };
    let qualifier = alias.as_ref().map(|alias| alias.name.clone());
    for item in &select.projection {
        let SelectItem::ExprWithAlias { expr, alias } = item else {
            return Err(unsupported());
        };
        let Expr::CompoundIdentifier(parts) = expr else {
            return Err(unsupported());
        };
        if parts.len() != 2
            || qualifier.as_ref() != parts.first()
            || parts
                .last()
                .is_none_or(|column| column.value != alias.value)
        {
            return Err(unsupported());
        }
    }
    for column in columns {
        if select.projection.iter().any(|item| {
            matches!(item, SelectItem::ExprWithAlias { alias, .. } if alias.value == *column)
        }) {
            continue;
        }
        let Some(qualifier) = qualifier.clone() else {
            return Err(unsupported());
        };
        let column = Ident::with_quote(dialect.quote_char(), *column);
        select.projection.push(SelectItem::ExprWithAlias {
            expr: Expr::CompoundIdentifier(vec![qualifier, column.clone()]),
            alias: column,
        });
    }
    Ok(statements
        .into_iter()
        .map(|statement| statement.to_string())
        .collect::<Vec<_>>()
        .join("; "))
}

fn generated_select(statements: &mut [Statement]) -> Result<&mut sqlparser::ast::Select> {
    let [Statement::Query(query)] = statements else {
        return Err(unsupported());
    };
    if query.with.is_some()
        || query.order_by.is_some()
        || query.limit_clause.is_some()
        || query.fetch.is_some()
        || !query.locks.is_empty()
        || query.for_clause.is_some()
        || query.settings.is_some()
        || query.format_clause.is_some()
        || !query.pipe_operators.is_empty()
    {
        return Err(unsupported());
    }
    let SetExpr::Select(select) = query.body.as_mut() else {
        return Err(unsupported());
    };
    if select.from.len() != 1
        || !select.from[0].joins.is_empty()
        || select.projection.is_empty()
        || !select.optimizer_hints.is_empty()
        || select.select_modifiers.is_some()
        || select.top.is_some()
        || select.exclude.is_some()
        || select.selection.is_some()
        || select.prewhere.is_some()
        || select.having.is_some()
        || select.qualify.is_some()
        || select.into.is_some()
        || !select.lateral_views.is_empty()
        || !select.connect_by.is_empty()
        || !matches!(&select.group_by, GroupByExpr::Expressions(values, _) if values.is_empty())
        || !select.cluster_by.is_empty()
        || !select.distribute_by.is_empty()
        || !select.sort_by.is_empty()
        || !select.named_window.is_empty()
        || select.value_table_mode.is_some()
        || select.flavor != SelectFlavor::Standard
    {
        return Err(unsupported());
    }
    let TableFactor::Table {
        alias,
        args,
        with_hints,
        version,
        with_ordinality,
        partitions,
        json_path,
        sample,
        index_hints,
        ..
    } = &select.from[0].relation
    else {
        return Err(unsupported());
    };
    if alias.as_ref().is_none_or(|alias| !alias.columns.is_empty())
        || args.is_some()
        || !with_hints.is_empty()
        || version.is_some()
        || *with_ordinality
        || !partitions.is_empty()
        || json_path.is_some()
        || sample.is_some()
        || !index_hints.is_empty()
    {
        return Err(unsupported());
    }
    Ok(select)
}

fn unsupported() -> Error {
    Error::Emit("source view is not eligible for portable row authorization".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_a_bound_outer_policy_column_without_adding_a_value() {
        for (dialect, sql, expected) in [
            (
                Dialect::Sqlite,
                "SELECT DISTINCT sfs0.\"name\" AS \"name\" FROM \"people\" sfs0",
                "sfs0.\"tenant\" AS \"tenant\"",
            ),
            (
                Dialect::Postgres,
                "SELECT DISTINCT sfs0.\"name\" AS \"name\" FROM \"people\" sfs0",
                "sfs0.\"tenant\" AS \"tenant\"",
            ),
            (
                Dialect::MySql,
                "SELECT DISTINCT sfs0.`name` AS `name` FROM `people` sfs0",
                "sfs0.`tenant` AS `tenant`",
            ),
        ] {
            assert_eq!(single_table_view_source(sql, dialect).unwrap(), "people");
            let out = expose_single_table_view_columns(sql, dialect, &["tenant"]).unwrap();
            assert!(out.contains(expected), "{out}");
            assert!(!out.contains('?'));
        }
    }

    #[test]
    fn expression_alias_and_complex_sources_fail_closed() {
        for sql in [
            "SELECT 'tenant' AS tenant FROM people p",
            "SELECT p.name AS name FROM people p JOIN teams t ON p.id=t.id",
            "SELECT p.name AS name FROM people p WHERE p.active = 1",
            "SELECT p.name AS name FROM people p GROUP BY p.name",
        ] {
            assert!(expose_single_table_view_columns(sql, Dialect::Sqlite, &["tenant"]).is_err());
        }
    }
}
