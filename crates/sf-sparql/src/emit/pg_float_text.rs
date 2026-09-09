//! Exact numeric text promotion without source-sized arbitrary-precision casts.
use super::*;

pub(super) fn operand(
    raw: &str,
    key: TextKey,
    datatype: &str,
    promotion: Promotion,
) -> Option<String> {
    let kind = datatype.strip_prefix("http://www.w3.org/2001/XMLSchema#")?;
    let integer = sf_core::numeric_compare::is_integer_datatype(datatype);
    if !integer && !matches!(kind, "decimal" | "float" | "double") {
        return None;
    }
    if kind == "double" && !matches!(promotion, Promotion::Double) {
        return None;
    }
    let raw = match key {
        TextKey::Verbatim => raw.to_owned(),
        TextKey::PostgresCharacter => {
            format!("pg_catalog.convert_from(pg_catalog.bpcharsend({raw}), 'UTF8')")
        }
        _ => return None,
    };
    let parsed = if kind == "float" {
        Promotion::Float
    } else {
        promotion
    };
    let sql_type = parsed.sql_type();
    let grammar = if integer {
        "^[+-]?[0-9]+$"
    } else if kind == "decimal" {
        "^[+-]?([0-9]+([.][0-9]*)?|[.][0-9]+)$"
    } else {
        "^[+-]?([0-9]+([.][0-9]*)?|[.][0-9]+)([eE][+-]?[0-9]+)?$"
    };
    let special = if matches!(kind, "float" | "double") {
        format!("WHEN v = 'INF' THEN CAST('Infinity' AS {sql_type}) WHEN v = '-INF' THEN CAST('-Infinity' AS {sql_type}) WHEN v = 'NaN' THEN CAST('NaN' AS {sql_type})")
    } else {
        String::new()
    };
    let facets = facets(kind);
    let number = numeric("__sf_text_number.n", parsed);
    // Every positive binary64 rounding boundary is N*2^e, N<2^54 and
    // -1075<=e<=970. Its exact decimal coefficient has <=1092 digits (the
    // conservative bound17+1075). Keeping1100 digits plus a nonzero sticky
    // digit preserves <,=,> against every boundary, without rounding a prefix.
    // Scientific order, not the source exponent, determines range. PostgreSQL
    // text is <2^31 bytes: an exponent with >10 significant digits cannot be
    // cancelled by its mantissa. All length/order arithmetic stays BIGINT.
    // Quote substr below so sqlparser keeps a qualified function call instead
    // of dispatching the bare SUBSTR keyword while backtracking a CASE.
    let sql = format!(
        r#"(WITH
      __sf_text_input AS MATERIALIZED (SELECT (pg_catalog.btrim(CAST({raw} AS TEXT), pg_catalog.chr(32)||pg_catalog.chr(9)||pg_catalog.chr(13)||pg_catalog.chr(10))) COLLATE "C" AS v),
      __sf_text_parts AS MATERIALIZED (SELECT v, v ~ '{grammar}' AS valid, pg_catalog.left(v,1) = '-' AS neg,
        pg_catalog.split_part(pg_catalog.lower(pg_catalog.ltrim(v,'+-')),'e',1) AS m,
        pg_catalog.split_part(pg_catalog.lower(v),'e',2) AS ex FROM __sf_text_input),
      __sf_text_digits AS MATERIALIZED (SELECT v, valid, neg,
        pg_catalog.ltrim(pg_catalog.replace(m,'.',''),'0') AS d,
        CAST(pg_catalog.length(pg_catalog.split_part(m,'.',2)) AS BIGINT) AS fractional,
        pg_catalog.ltrim(pg_catalog.ltrim(ex,'+-'),'0') AS ed, pg_catalog.left(ex,1) = '-' AS eneg FROM __sf_text_parts),
      __sf_text_order AS MATERIALIZED (SELECT v, valid, neg, d,
        (CASE WHEN eneg THEN -1 ELSE 1 END) *
        (CASE WHEN NOT valid OR ed = '' THEN CAST(0 AS BIGINT) WHEN pg_catalog.length(ed)>10 THEN CAST(10000000000 AS BIGINT) ELSE CAST(CASE WHEN valid THEN ed END AS BIGINT) END)
        - fractional + CAST(pg_catalog.length(d) AS BIGINT) - 1 AS ord,
        pg_catalog.left(d,1100) || (CASE WHEN (pg_catalog."substr"(d,1101)) ~ '[1-9]' THEN '1' ELSE '' END) AS prefix FROM __sf_text_digits),
      __sf_text_number AS MATERIALIZED (SELECT v, valid, neg, d, ord,
        CAST(CASE WHEN valid AND d<>'' AND ord BETWEEN -500 AND 400 THEN prefix || 'e' || CAST(ord+1-pg_catalog.length(prefix) AS TEXT) END AS NUMERIC) AS n FROM __sf_text_order)
      SELECT CASE {special} WHEN NOT valid OR NOT ({facets}) THEN CAST(NULL AS {sql_type})
        ELSE (CASE WHEN neg THEN CAST(-1 AS {sql_type}) ELSE CAST(1 AS {sql_type}) END) *
          (CASE WHEN d='' OR ord < -500 THEN CAST(0 AS {sql_type}) WHEN ord > 400 THEN CAST('Infinity' AS {sql_type}) ELSE {number} END)
      END FROM __sf_text_number)"#
    );
    Some(
        if kind == "float" && matches!(promotion, Promotion::Double) {
            format!("CAST({sql} AS DOUBLE PRECISION)")
        } else {
            sql
        },
    )
}

fn facets(kind: &str) -> String {
    let negative = "(neg AND d<>'')";
    let bounds = match kind {
        "nonPositiveInteger" => return format!("({negative} OR d='')"),
        "negativeInteger" => return negative.into(),
        "nonNegativeInteger" => return format!("NOT {negative}"),
        "positiveInteger" => return format!("(NOT {negative} AND d<>'')"),
        "long" => ("9223372036854775807", "9223372036854775808"),
        "int" => ("2147483647", "2147483648"),
        "short" => ("32767", "32768"),
        "byte" => ("127", "128"),
        "unsignedLong" => ("18446744073709551615", ""),
        "unsignedInt" => ("4294967295", ""),
        "unsignedShort" => ("65535", ""),
        "unsignedByte" => ("255", ""),
        _ => return "TRUE".into(),
    };
    let bound = format!(
        "(CASE WHEN {negative} THEN '{}' ELSE '{}' END)",
        bounds.1, bounds.0
    );
    format!("({bound}<>'' AND (pg_catalog.length(d)<pg_catalog.length({bound}) OR (pg_catalog.length(d)=pg_catalog.length({bound}) AND d<={bound})))")
}
