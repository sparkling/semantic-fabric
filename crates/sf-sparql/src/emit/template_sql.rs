//! Template concatenation and immediate-source column rendering.
use super::*;

/// Render one template's segments as a dialect-appropriate SQL string
/// concatenation — [`SqlCond::TemplateEq`]'s per-side rendering (Run 4 Wave
/// B3). Every `Segment::Literal` is a bound parameter (ADR-0010 R1): the
/// text is mapping-trusted, not query-supplied, but this still follows the
/// blanket "values are bound parameters only" rule rather than hand-rolling
/// SQL string-literal escaping. Every `Segment::Column` renders through the
/// SAME [`colref`] every other `SqlCond` arm uses, additionally wrapped in
/// [`percent_encode_col`] when `encode_iri` (below).
///
/// **Percent-encoding soundness (Run 4 B-repair FIX 2).** `sf_core::ir::
/// Template::expand`'s `encode_iri` flag percent-encodes EVERY column value
/// — never a template's own literal segments — when (and only when) the
/// template constructs an IRI (R2RML §7.3 / RFC 3987); a plain-literal
/// template's expansion is raw, unencoded column text. Before this fix,
/// EVERY `Segment::Column` rendered here as a bare `colref`, regardless of
/// kind — sound for two templates whose column values happen to carry no
/// IRI-encodable character, but wrong in general: a single-column template
/// `http://ex.org/v/{va}` with `va = "X/Y"` expands (RDF) to
/// `http://ex.org/v/X%2FY` (the `/` encoded), while a two-column template
/// `http://ex.org/v/{vb1}/{vb2}` with `vb1="X", vb2="Y"` expands to
/// `http://ex.org/v/X/Y` (the `/` a literal template character) — DIFFERENT
/// IRIs, yet the old raw-concat rendering compared them EQUAL (false
/// positive), and conversely could compare two RDF-equal IRIs unequal.
/// `encode_iri` is one flag for BOTH sides ([`SqlCond::TemplateEq`]'s own
/// doc comment: `align_templates`'s caller only ever builds this variant
/// when both sides share one `TermType`), so each `Segment::Column` here is
/// consistently encoded, or consistently left raw, matching whichever
/// `Template::expand` itself would do for that same template.
///
/// **NULL-propagation soundness.** On every dialect rendered below, `||`
/// (Postgres/SQLite) and `CONCAT(...)` (MySQL) return SQL `NULL` if ANY
/// operand is `NULL` — so `render(t1) = render(t2)` evaluates to UNKNOWN
/// (never `TRUE`) whenever a referenced column is `NULL`, and a WHERE/JOIN
/// condition that is UNKNOWN excludes the row, same as `FALSE`. This is NOT
/// an approximation: `sf_core::term::generate_into`'s `TermMap::Template`
/// arm (`Template::expand`, per R2RML §11) ALREADY treats "any referenced
/// column is NULL" as "this variable is UNBOUND" for that row when
/// RECONSTRUCTING the SAME template in Rust — `sf-core/term.rs`'s
/// `null_in_template_yields_no_term` test locks this in. So a NULL-collapsed
/// concatenation here excludes EXACTLY the rows whose SPARQL-level operand
/// would have been unbound anyway (comparing against an unbound variable is
/// a type error ⇒ the row is excluded from a FILTER, and an unbound shared
/// variable cannot correlate a join either) — the SQL and RDF answers agree
/// by construction, not by coincidence. [`percent_encode_col`]'s own three
/// per-dialect implementations preserve this EXPLICITLY (a `CASE WHEN col IS
/// NULL THEN NULL ELSE …` wrapper, not incidental `NULL`-propagation through
/// some other operator) — see its own doc comment for why an explicit
/// wrapper is required rather than assumed.
///
/// **Dialect support.** Only the three PRODUCTION-WIRED dialects
/// (`sf_sql::Dialect`'s own grouping) are implemented: PostgreSQL/SQLite via
/// ANSI `||`, MySQL via `CONCAT(...)` (`||` is boolean OR there by default,
/// per MySQL's non-default `PIPES_AS_CONCAT` sql_mode). Every OTHER dialect
/// returns `Unsupported` rather than guessing — e.g. SQL Server's own
/// `CONCAT()` function treats `NULL` as an EMPTY STRING (breaking the
/// soundness argument above outright), and `+`, SQL Server's NULL-safe
/// concat operator, is unverified against this rendering; Oracle/DuckDB/etc.
/// are ANSI-`||`-following by reputation but likewise unverified here —
/// "sound over complete", the same bar `str_match`'s PostgreSQL-only `LIKE`
/// pushdown already sets for an analogous dialect-behavior gap.
pub(super) fn render_template_concat(
    segs: &[sf_core::ir::Segment],
    encode_iri: bool,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    column: impl Fn(&str) -> String,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    use sf_core::ir::Segment;
    let mut parts = work
        .vector(segs.len())
        .map_err(source_control::validation_error)?;
    for seg in segs {
        work.charge(1).map_err(source_control::validation_error)?;
        parts.push(match seg {
            Segment::Literal(text) => {
                work.parameter(params, pidx, text)
                    .map_err(source_control::validation_error)?;
                dialect.placeholder(*pidx)
            }
            Segment::Column(c) => {
                let col = column(c);
                if encode_iri {
                    percent_encode_col_controlled(&col, dialect, catalog, work)?
                } else {
                    col
                }
            }
        });
    }
    // Joined bytes are copied once into the join buffer, then into its wrapper.
    for part in &parts {
        work.product(part.len(), 2)
            .map_err(source_control::validation_error)?;
    }
    work.product(parts.len(), 8)
        .map_err(source_control::validation_error)?;
    work.charge(8).map_err(source_control::validation_error)?;
    match dialect {
        Dialect::Postgres | Dialect::Sqlite => Ok(format!("({})", parts.join(" || "))),
        Dialect::MySql => Ok(format!("CONCAT({})", parts.join(", "))),
        other => Err(Error::Unsupported(format!(
            "template-shape-mismatch equality (SQL CONCAT fallback) is not implemented for \
             {other:?} → 501 (never a silently wrong NULL/concat-operator guess)"
        ))),
    }
}

/// Render mapping-trusted template literals inline, with source-aware columns.
/// No query/policy values enter this parameter-free projection recipe.
pub(crate) fn render_template_inline(
    segs: &[sf_core::ir::Segment],
    encode_iri: bool,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    column: impl Fn(&str) -> String,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    use sf_core::ir::Segment;
    let mut parts = work
        .vector(segs.len())
        .map_err(source_control::validation_error)?;
    for segment in segs {
        let part = match segment {
            Segment::Literal(text) => {
                // Quoting at most doubles the literal.
                work.product(text.len(), 2)
                    .map_err(source_control::validation_error)?;
                sql_string_literal(text)
            }
            Segment::Column(name) if encode_iri => {
                percent_encode_col_controlled(&column(name), dialect, catalog, work)?
            }
            Segment::Column(name) => column(name),
        };
        // The part is copied once more into the joined expression.
        work.charge(part.len() + 4)
            .map_err(source_control::validation_error)?;
        parts.push(part);
    }
    match dialect {
        Dialect::Postgres | Dialect::Sqlite => Ok(format!("({})", parts.join(" || "))),
        Dialect::MySql => Ok(format!("CONCAT({})", parts.join(", "))),
        _ => Err(Error::Unsupported("rendered projection dialect".into())),
    }
}

/// Render `column` against one immediate scan source at translate time.
///
/// `inner_sql == None` denotes a base [`LogicalSource::Table`]; `Some` denotes a
/// derived [`LogicalSource::Query`]. That distinction is load-bearing for Direct
/// Mapping's synthetic no-primary-key `rowid`: PostgreSQL reads base-table
/// `ctid`, while an authored or compiler-derived query output named `rowid`
/// remains an ordinary column. Query aliases also retain the existing bounded
/// bare-`AS` case-folding heuristic. Live emission uses the typed equivalent in
/// [`colref`].
pub(crate) fn render_immediate_source_column(
    alias: &str,
    column: &str,
    inner_sql: Option<&str>,
    dialect: Dialect,
) -> String {
    if dialect == Dialect::Postgres && column == "rowid" && inner_sql.is_none() {
        return format!("({alias}.ctid)::text");
    }
    if dialect == Dialect::Postgres
        && inner_sql.is_some_and(|sql| crate::cascade::col_is_unquoted_alias(sql, column))
    {
        format!("{alias}.{column}")
    } else {
        format!("{alias}.{}", dialect.quote_ident(column))
    }
}

/// A SQL single-quoted string literal for mapping-trusted text `text` — ANSI
/// `''`-doubling, the one escaping rule SQLite/PostgreSQL/MySQL all share for a
/// single-quoted string (unlike percent-encoding, no per-dialect split applies
/// here). Used only for mapping-owned literals in template projections.
pub(super) fn sql_string_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
