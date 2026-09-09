//! Strict full lexical validation, bounded nearest-IEEE conversion and tags.
use super::*;

pub(super) fn operand(raw: &str, kind: &str, double: bool) -> String {
    let grammar = if kind == "decimal" {
        "^[+-]?([0-9]+([.][0-9]*)?|[.][0-9]+)$"
    } else if matches!(kind, "float" | "double") {
        "^[+-]?([0-9]+([.][0-9]*)?|[.][0-9]+)([eE][+-]?[0-9]+)?$"
    } else {
        "^[+-]?[0-9]+$"
    };
    let specials = if matches!(kind, "float" | "double") {
        "WHEN v='INF' THEN 1 WHEN v='-INF' THEN -1 WHEN v='NaN' THEN 2"
    } else {
        ""
    };
    let facets = mysql_decimal_value::facets(kind);
    let overflow = round::overflow(double);
    let order = overflow.len() - 1;
    let stages = if double { "" } else { round::stages() };
    let source = if double {
        "__sf_float_number"
    } else {
        "__sf_float_result"
    };
    let result = if double { "x" } else { "result" };
    // A provider field is <2^32 bytes; a >10-significant-digit exponent
    // cannot be cancelled by its mantissa. Length arithmetic is signed.
    // Every binary64 midpoint has <=1092 coefficient digits. 1100 digits plus
    // a nonzero sticky digit preserve its side. Use an INTEGER coefficient:
    // MySQL dtoa truncates long *fractional* scanning after ~30 digits.
    format!(
        r#"(WITH
      __sf_float_input AS (SELECT (REGEXP_REPLACE(CONVERT(({raw}) USING utf8mb4),CONCAT('^[',CONVERT(CHAR(32,9,13,10) USING utf8mb4),']+|[',CONVERT(CHAR(32,9,13,10) USING utf8mb4),']+$'),'',1,0,'c')) COLLATE utf8mb4_bin AS v LIMIT 18446744073709551615),
      __sf_float_parts AS (SELECT v,REGEXP_LIKE(v,'{grammar}','c') AND NOT REGEXP_LIKE(v,'[^0-9eE+.-]','c') AS valid,
        LEFT(v,1)='-' AS neg, SUBSTRING_INDEX(LOWER(CASE WHEN LEFT(v,1) IN ('+','-') THEN SUBSTRING(v,2) ELSE v END),'e',1) AS m,
        CASE WHEN LOCATE('e',LOWER(v))>0 THEN SUBSTRING_INDEX(LOWER(v),'e',-1) ELSE '' END AS ex FROM __sf_float_input LIMIT 18446744073709551615),
      __sf_float_digits AS (SELECT v,valid,neg,TRIM(LEADING '0' FROM REPLACE(m,'.','')) AS d,
        CASE WHEN LOCATE('.',m)>0 THEN CAST(LENGTH(SUBSTRING_INDEX(m,'.',-1)) AS SIGNED) ELSE 0 END AS fractional,
        TRIM(LEADING '0' FROM CASE WHEN LEFT(ex,1) IN ('+','-') THEN SUBSTRING(ex,2) ELSE ex END) AS ed,LEFT(ex,1)='-' AS eneg FROM __sf_float_parts LIMIT 18446744073709551615),
      __sf_float_order AS (SELECT v,valid,neg,d,
        (CASE WHEN eneg THEN -1 ELSE 1 END)*(CASE WHEN NOT valid OR ed='' THEN 0 WHEN LENGTH(ed)>10 THEN 10000000000 ELSE CAST(CASE WHEN valid THEN ed END AS SIGNED) END)-fractional+CAST(LENGTH(d) AS SIGNED)-1 AS ord,
        CONCAT(LEFT(d,1100),CASE WHEN REGEXP_LIKE(SUBSTRING(d,1101),'[1-9]','c') THEN '1' ELSE '' END) AS prefix FROM __sf_float_digits LIMIT 18446744073709551615),
      __sf_float_tag AS (SELECT *,CASE {specials} WHEN valid IS NULL OR NOT valid OR NOT ({facets}) THEN 3
        WHEN d<>'' AND (ord,CAST(TRIM(TRAILING '0' FROM d) AS BINARY)) >= ({order},CAST(TRIM(TRAILING '0' FROM '{overflow}') AS BINARY)) THEN CASE WHEN neg THEN -1 ELSE 1 END ELSE 0 END AS tag FROM __sf_float_order LIMIT 18446744073709551615),
      __sf_float_number AS (SELECT *,CAST(CASE WHEN tag=0 AND d<>'' AND ord>=-500 THEN CONCAT(prefix,'e',CAST(ord+1-CAST(LENGTH(prefix) AS SIGNED) AS CHAR)) ELSE '0' END AS DOUBLE) AS x FROM __sf_float_tag LIMIT 18446744073709551615)
      {stages}
      SELECT JSON_ARRAY(tag,CAST(CASE WHEN tag=0 THEN (CASE WHEN neg THEN -1 ELSE 1 END)*{result} ELSE 0 END AS DOUBLE)) FROM {source})"#
    )
}
