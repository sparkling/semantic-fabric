//! Scalar condition emission; structural traversal belongs to condition_control.
use super::*;

pub(super) fn render(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    work.charge(1).map_err(source_control::validation_error)?;
    Ok(match cond {
        SqlCond::ExpressionError => "(NULL = 1)".to_owned(),
        SqlCond::IriCmp(cmp) => {
            iri_cmp::render(cmp, dialect, catalog, actuals, params, pidx, work)?
        }
        SqlCond::LiteralCmp(cmp) => {
            literal_cmp::render_controlled(cmp, dialect, catalog, actuals, params, pidx, work)?
        }
        SqlCond::ColEq(a, b) => render_key_equality(a, b, dialect, catalog, actuals)?,
        SqlCond::NativeColEq(a, b) => format!(
            "{} = {}",
            colref(a, dialect, actuals),
            colref(b, dialect, actuals)
        ),
        SqlCond::NullSafeEq(a, b) => {
            let (la, lb) = (colref(a, dialect, actuals), colref(b, dialect, actuals));
            let equal = render_key_equality(a, b, dialect, catalog, actuals)?;
            format!("({equal} OR {la} IS NULL OR {lb} IS NULL)")
        }
        SqlCond::Cmp(a, op, val) | SqlCond::NativeCmp(a, op, val) => {
            work.parameter(params, pidx, val)
                .map_err(source_control::validation_error)?;
            format!(
                "{} {} {}",
                if matches!(cond, SqlCond::NativeCmp(..)) {
                    colref(a, dialect, actuals)
                } else {
                    path_comparison::rdf_column(a, dialect, catalog, actuals)
                },
                op.as_sql(),
                dialect.placeholder(*pidx)
            )
        }
        SqlCond::StrMatch { col, op, param } => {
            // The pattern/regex is a bound parameter (ADR-0010 R1) — never inlined.
            // The `ESCAPE '\'` char is a fixed engine constant (not query data), so
            // it is part of the trusted skeleton, like an identifier.
            work.parameter(params, pidx, param)
                .map_err(source_control::validation_error)?;
            let ph = dialect.placeholder(*pidx);
            let c = colref(col, dialect, actuals);
            match op {
                StrMatchOp::CoarseLexicalEqual => match dialect {
                    Dialect::Sqlite => {
                        format!("(typeof({c}) <> 'text' OR rtrim({c}, ' ') = rtrim({ph}, ' '))")
                    }
                    Dialect::Postgres => format!(
                        "(pg_catalog.pg_typeof({c}) NOT IN ('pg_catalog.text'::regtype, \
                         'pg_catalog.varchar'::regtype) OR CAST({c} AS TEXT) = {ph})"
                    ),
                    Dialect::MySql => format!(
                        "(CASE WHEN @@character_set_client <> 'utf8mb4' \
                         OR @@character_set_connection <> 'utf8mb4' \
                         OR @@character_set_results IS NULL \
                         OR @@character_set_results <> 'utf8mb4' \
                         OR CHARSET({c}) = 'binary' THEN TRUE \
                         WHEN JSON_VALID(CAST({c} AS CHAR)) THEN TRUE \
                         ELSE RTRIM(CAST({c} AS CHAR)) = RTRIM({ph}) END)"
                    ),
                    _ => return Err(Error::Unsupported("bounded join reducer dialect".into())),
                },
                StrMatchOp::Like => format!("{c} LIKE {ph} ESCAPE '\\'"),
                StrMatchOp::RegexMatch => format!("{c} ~ {ph}"),
                StrMatchOp::RegexMatchI => format!("{c} ~* {ph}"),
            }
        }
        SqlCond::IsNotNull(a) => format!("{} IS NOT NULL", colref(a, dialect, actuals)),
        SqlCond::DecodedIsNotNull(a) => {
            let raw = colref(a, dialect, actuals);
            let decoded = if pg_numeric::is_numeric(a, dialect, actuals) {
                pg_numeric::lexical(&raw)
            } else {
                raw
            };
            format!("{decoded} IS NOT NULL")
        }
        SqlCond::IsNull(a) => format!("{} IS NULL", colref(a, dialect, actuals)),
        SqlCond::Not(_)
        | SqlCond::And(_)
        | SqlCond::Or(_)
        | SqlCond::Exists { .. }
        | SqlCond::NotExists { .. }
        | SqlCond::PathExists { .. } => return Err(Error::Sql("non-leaf condition".into())),
        // Run 4 Wave B3 — `unify::align_templates`'s shape-mismatch fallback:
        // render each side as a SQL string concatenation and compare with
        // `=`. See `render_template_concat`'s doc comment for the
        // dialect-support boundary and the NULL-propagation soundness
        // argument (why a NULL underlying column correctly excludes the row
        // rather than needing special-casing here).
        SqlCond::TemplateEq(sx, a1, sy, a2, encode_iri) => {
            if let Some(sql) = mysql_float_value::identity::template_comparison(
                cond, dialect, catalog, actuals, params, pidx, work,
            )? {
                return Ok(sql);
            }
            let r1 = render_template_concat(
                sx,
                *encode_iri,
                dialect,
                catalog,
                |c| path_comparison::rdf_column(&ColRef::new(*a1, c), dialect, catalog, actuals),
                params,
                pidx,
                work,
            )?;
            let r2 = render_template_concat(
                sy,
                *encode_iri,
                dialect,
                catalog,
                |c| path_comparison::rdf_column(&ColRef::new(*a2, c), dialect, catalog, actuals),
                params,
                pidx,
                work,
            )?;
            format!("{r1} = {r2}")
        }
    })
}
