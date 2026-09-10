//! Retained binary64 -> Rust Display through qualified MySQL shortest digits.
//! JSON's J_DOUBLE writer uses dtoa mode 4 with an untruncated field width.
//! Its shortest/nearest rule agrees with Rust except decimal midpoint ties:
//! MySQL chooses even, while Rust chooses the larger magnitude. Never use a
//! column's CAST AS CHAR (fixed scale/ZEROFILL can change that representation).
use super::identity;

pub(super) fn lexical(raw: &str) -> String {
    static POWERS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let powers = POWERS.get_or_init(|| {
        (0_u32..=24)
            .map(|e| format!("WHEN {e} THEN {}", 5_u128.pow(e)))
            .collect::<Vec<_>>()
            .join(" ")
    });
    let negative_zero = identity::negative_zero("v");
    // For c with <=17 digits the upper decimal midpoint is
    // (2*c+1)*5^q*2^(q-1). Its odd coefficient must fit 53 bits. For q<0,
    // 5^-q must first divide 2*c+1 exactly. Thus only -24<=q<=22 can be a
    // binary64 midpoint; all arithmetic needed for this proof fits DECIMAL(65).
    // A rounded float equality alone is NOT evidence that it is an exact tie.
    format!(
        r#"(WITH
      __sf_double_input AS (SELECT {raw} AS v LIMIT 18446744073709551615),
      __sf_double_json AS (SELECT v,ABS(COALESCE(v,0)) AS x,
        JSON_UNQUOTE(JSON_EXTRACT(JSON_ARRAY(CAST(ABS(COALESCE(v,0)) AS DOUBLE)),'$[0]')) AS j
        FROM __sf_double_input LIMIT 18446744073709551615),
      __sf_double_parts AS (SELECT *,SUBSTRING_INDEX(j,'e',1) AS m,
        CASE WHEN LOCATE('e',j)>0 THEN CAST(SUBSTRING_INDEX(j,'e',-1) AS SIGNED) ELSE 0 END AS e
        FROM __sf_double_json LIMIT 18446744073709551615),
      __sf_double_digits AS (SELECT *,COALESCE(NULLIF(TRIM(LEADING '0' FROM REPLACE(m,'.','')),''),'0') AS d0,
        CASE WHEN LOCATE('.',m)>0 THEN CAST(LENGTH(SUBSTRING_INDEX(m,'.',-1)) AS SIGNED) ELSE 0 END AS fraction_digits
        FROM __sf_double_parts LIMIT 18446744073709551615),
      __sf_double_trim AS (SELECT *,COALESCE(NULLIF(TRIM(TRAILING '0' FROM d0),''),'0') AS d
        FROM __sf_double_digits LIMIT 18446744073709551615),
      __sf_double_coefficient AS (SELECT *,CAST(d AS DECIMAL(18,0)) AS c,
        e-fraction_digits+CAST(LENGTH(d0) AS SIGNED)-CAST(LENGTH(d) AS SIGNED) AS q
        FROM __sf_double_trim LIMIT 18446744073709551615),
      __sf_double_factor AS (SELECT *,2*c+1 AS n,
        CAST(CASE ABS(q) {powers} ELSE 1 END AS DECIMAL(18,0)) AS five
        FROM __sf_double_coefficient LIMIT 18446744073709551615),
      __sf_double_exact AS (SELECT *,CASE WHEN q BETWEEN -24 AND -1 THEN MOD(n,five)=0 AND n DIV five<=9007199254740991
        WHEN q BETWEEN 0 AND 22 THEN n*five<=9007199254740991 ELSE FALSE END AS exact_midpoint
        FROM __sf_double_factor LIMIT 18446744073709551615),
      __sf_double_corrected AS (SELECT *,c+CASE WHEN exact_midpoint THEN
        CASE WHEN CAST(CONCAT(CAST(n AS CHAR),'e',q) AS DOUBLE)/2=x
          AND CAST(CONCAT(CAST(c+1 AS CHAR),'e',q) AS DOUBLE)=x THEN 1 ELSE 0 END
        ELSE 0 END AS chosen FROM __sf_double_exact LIMIT 18446744073709551615),
      __sf_double_display AS (SELECT *,CAST(chosen AS CHAR) AS chosen_digits,
        CAST(LENGTH(CAST(chosen AS CHAR)) AS SIGNED)+q AS point
        FROM __sf_double_corrected LIMIT 18446744073709551615)
      SELECT CASE WHEN v IS NULL THEN NULL WHEN v=0 THEN CASE WHEN {negative_zero} THEN '-0' ELSE '0' END
        WHEN LENGTH(d)>17 OR q NOT BETWEEN -324 AND 308 THEN JSON_UNQUOTE(JSON_EXTRACT('semantic-fabric-invalid-double-lexical','$'))
        ELSE CONCAT(CASE WHEN v<0 THEN '-' ELSE '' END,
          CASE WHEN point<=0 THEN CONCAT('0.',REPEAT('0',-point),chosen_digits)
            WHEN point>=LENGTH(chosen_digits) THEN RPAD(chosen_digits,point,'0')
            ELSE CONCAT(SUBSTRING(chosen_digits,1,point),'.',SUBSTRING(chosen_digits,point+1)) END)
        END FROM __sf_double_display)"#
    )
}
