//! Bounded Rust-shortest decimal selection, independent of MySQL formatting.
//! Four base-10^30 limbs retain full dyadic digits. Decimal ties go UP in
//! Rust's shortest mode, unlike its fixed-precision formatter's ties-to-even.
use super::round;

fn power_two(exponent: i32) -> f64 {
    f64::from_bits(((exponent + 1023) as u64) << 52)
}

/// Scale a finite nonnegative binary32 r exactly; subnormals retain min spacing.
/// Input CTE is __sf_native_candidate; output is __sf_native_scale8.
pub(super) fn scaling() -> &'static str {
    static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SQL.get_or_init(|| {
        let mut sql = format!(",__sf_native_scale0 AS (SELECT *,r/CAST('{}' AS DOUBLE) AS y0,-126 AS e0,CAST('{}' AS DOUBLE) AS s0 FROM __sf_native_candidate LIMIT 18446744073709551615)",power_two(-126),power_two(-149));
        for (i, shift) in [128,64,32,16,8,4,2,1].into_iter().enumerate() {
            let next=i+1;
            let power=power_two(shift);
            sql.push_str(&format!(",__sf_native_scale{next} AS (SELECT *,CASE WHEN y{i}>=CAST('{power}' AS DOUBLE) THEN y{i}/CAST('{power}' AS DOUBLE) ELSE y{i} END AS y{next},e{i}+CASE WHEN y{i}>=CAST('{power}' AS DOUBLE) THEN {shift} ELSE 0 END AS e{next},CASE WHEN y{i}>=CAST('{power}' AS DOUBLE) THEN s{i}*CAST('{power}' AS DOUBLE) ELSE s{i} END AS s{next} FROM __sf_native_scale{i} LIMIT 18446744073709551615)"));
        }
        sql
    })
}

/// n*2^me -> exact significant digits d, with decimal exponent min(me,0).
/// n < 2^26 and -151 <= me <= 103. Input __sf_short_coeff; output __sf_short_digits.
pub(super) fn digits() -> &'static str {
    static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SQL.get_or_init(|| {
        let factor=(-151_i32..=103).map(|e|format!("WHEN {e} THEN '{}'",round::power_digits(if e<0 {5} else {2},e.unsigned_abs() as usize))).collect::<Vec<_>>().join(" ");
        let base="1000000000000000000000000000000";
        let mut sql=format!(",__sf_short_factor AS (SELECT *,LPAD(CASE me {factor} END,120,'0') AS factor FROM __sf_short_coeff LIMIT 18446744073709551615)");
        for i in 0..4 {
            let previous=if i==0 {"__sf_short_factor".into()} else {format!("__sf_short_limb{}",i-1)};
            let carry=if i==0 {"0".into()} else {format!("(p{}-MOD(p{},{base}))/{base}",i-1,i-1)};
            sql.push_str(&format!(",__sf_short_limb{i} AS (SELECT *,CAST(n*CAST(SUBSTRING(factor,{},30) AS DECIMAL(30,0))+({carry}) AS DECIMAL(65,0)) AS p{i} FROM {previous} LIMIT 18446744073709551615)",91-i*30));
        }
        let chunks=(0..4).rev().map(|i|format!("LPAD(CAST(MOD(p{i},{base}) AS CHAR),30,'0')")).collect::<Vec<_>>().join(",");
        sql.push_str(&format!(",__sf_short_digits AS (SELECT *,TRIM(LEADING '0' FROM CONCAT({chunks})) AS d FROM __sf_short_limb3 LIMIT 18446744073709551615)"));
        sql
    })
}

/// Candidate floor/ceil at each precision, preserving exact nearest/ties-up order.
/// Input __sf_short_bounds contains d,k and any other caller-specific proof.
pub(super) fn candidates(precision: usize) -> String {
    let series = (1..=precision)
        .map(|p| format!("SELECT {p} AS precision_digits"))
        .collect::<Vec<_>>()
        .join(" UNION ALL ");
    format!(
        r#",
      __sf_short_grid AS (SELECT *,CAST(RPAD(LEFT(d,precision_digits),precision_digits,'0') AS DECIMAL(18,0)) AS q FROM __sf_short_bounds CROSS JOIN ({series}) AS precisions LIMIT 18446744073709551615),
      __sf_short_candidates AS (SELECT *,q+delta AS c,
        CASE WHEN delta=CASE WHEN SUBSTRING(d,precision_digits+1,1)>='5' THEN 1 ELSE 0 END THEN 0 ELSE 1 END AS preference
        FROM __sf_short_grid CROSS JOIN (SELECT 0 AS delta UNION ALL SELECT 1) AS offsets LIMIT 18446744073709551615),
      __sf_short_lexicals AS (SELECT *,CONCAT(CAST(c AS CHAR),'e',CAST(k-precision_digits AS CHAR)) AS lexical,
        CAST(LENGTH(CAST(c AS CHAR)) AS SIGNED)-1+k-precision_digits AS candidate_order,
        CAST(TRIM(TRAILING '0' FROM CAST(c AS CHAR)) AS BINARY) AS candidate_digits FROM __sf_short_candidates LIMIT 18446744073709551615)"#
    )
}

pub(super) fn float_value(raw: &str) -> String {
    let min = power_two(-126);
    let scaling = scaling();
    let digits = digits();
    let candidates = candidates(9);
    // Match Rust flt2dec::decoder, including doubled subnormal mantissas
    // and the asymmetric minimum-normal interval. Keep this authority local
    // to numeric value conversion, not generic lexical/template identity.
    format!(
        r#"(WITH
      __sf_native_input AS (SELECT {raw} AS v LIMIT 18446744073709551615),
      __sf_native_candidate AS (SELECT v,CASE WHEN v IS NULL OR v=0 THEN CAST(1 AS DOUBLE) ELSE ABS(CAST(v AS DOUBLE)) END AS r FROM __sf_native_input LIMIT 18446744073709551615)
      {scaling},
      __sf_short_mantissa AS (SELECT *,CAST(r/s8 AS DECIMAL(10,0)) AS mantissa FROM __sf_native_scale8 LIMIT 18446744073709551615),
      __sf_short_interval AS (SELECT v,r,CASE WHEN r<CAST('{min}' AS DOUBLE) THEN 2*mantissa WHEN mantissa=8388608 THEN 4*mantissa ELSE 2*mantissa END AS m,
        CASE WHEN r<CAST('{min}' AS DOUBLE) THEN -150 WHEN mantissa=8388608 THEN e8-25 ELSE e8-24 END AS me,
        CASE WHEN r>=CAST('{min}' AS DOUBLE) AND mantissa=8388608 THEN 2 ELSE 1 END AS plus,
        (r<CAST('{min}' AS DOUBLE) OR MOD(mantissa,2)=0) AS inclusive FROM __sf_short_mantissa LIMIT 18446744073709551615),
      __sf_short_coeff AS (SELECT *,CAST(m+CASE which_bound WHEN 0 THEN -1 WHEN 1 THEN 0 ELSE plus END AS DECIMAL(10,0)) AS n
        FROM __sf_short_interval CROSS JOIN (SELECT 0 AS which_bound UNION ALL SELECT 1 UNION ALL SELECT 2) AS bounds LIMIT 18446744073709551615)
      {digits},
      __sf_short_pivot AS (SELECT v,me,inclusive,MAX(CASE WHEN which_bound=0 THEN d END) AS lo,MAX(CASE WHEN which_bound=1 THEN d END) AS d,MAX(CASE WHEN which_bound=2 THEN d END) AS hi FROM __sf_short_digits GROUP BY v,me,inclusive),
      __sf_short_bounds AS (SELECT *,CAST(LENGTH(d) AS SIGNED)+LEAST(me,0) AS k,
        CAST(LENGTH(lo) AS SIGNED)-1+LEAST(me,0) AS low_order,CAST(TRIM(TRAILING '0' FROM lo) AS BINARY) AS low_digits,
        CAST(LENGTH(hi) AS SIGNED)-1+LEAST(me,0) AS high_order,CAST(TRIM(TRAILING '0' FROM hi) AS BINARY) AS high_digits FROM __sf_short_pivot LIMIT 18446744073709551615)
      {candidates},
      __sf_short_selected AS (SELECT lexical FROM __sf_short_lexicals WHERE
        ((candidate_order,candidate_digits)>(low_order,low_digits) AND (candidate_order,candidate_digits)<(high_order,high_digits))
        OR (inclusive AND ((candidate_order,candidate_digits)=(low_order,low_digits) OR (candidate_order,candidate_digits)=(high_order,high_digits)))
        ORDER BY precision_digits,preference LIMIT 1)
      SELECT JSON_ARRAY(CASE WHEN v IS NULL THEN 3 ELSE 0 END,CAST(CASE WHEN v IS NULL OR v=0 THEN 0 ELSE SIGN(v)*COALESCE((SELECT CAST(lexical AS DOUBLE) FROM __sf_short_selected),CAST(JSON_EXTRACT('semantic-fabric-invalid-native-float','$') AS DOUBLE)) END AS DOUBLE)) FROM __sf_native_input)"#
    )
}

/// Parse the wire f64's shortest decimal as Float. Away from an exact f32
/// midpoint, every round-tripping decimal narrows to the same f32. At a midpoint
/// only its shortest decimal's side matters; x has <=25 significant binary bits
/// and exponent >=-150, so no general full-range f64 decimal expansion is needed.
pub(super) fn double_as_float(raw: &str) -> String {
    let max = f64::from(f32::MAX);
    let overflow = round::overflow(false);
    let scaling = scaling();
    let digits = digits();
    let candidates = candidates(17);
    format!(
        r#"(WITH
      __sf_native_input AS (SELECT {raw} AS v LIMIT 18446744073709551615),
      __sf_native_number AS (SELECT v,LEAST(ABS(COALESCE(v,0)),CAST('{overflow}' AS DOUBLE)) AS x FROM __sf_native_input LIMIT 18446744073709551615),
      __sf_native_candidate AS (SELECT *,CAST(CAST(LEAST(x,CAST('{max}' AS DOUBLE)) AS FLOAT) AS DOUBLE) AS r FROM __sf_native_number LIMIT 18446744073709551615)
      {scaling},
      __sf_native_lower AS (SELECT *,CASE WHEN e8>-126 AND r=s8*8388608 THEN s8/2 ELSE s8 END AS ls FROM __sf_native_scale8 LIMIT 18446744073709551615),
      __sf_native_side AS (SELECT *,CASE WHEN r>0 AND x=r-ls/2 THEN -1 WHEN x=r+s8/2 THEN 1 ELSE 0 END AS side FROM __sf_native_lower LIMIT 18446744073709551615),
      __sf_short_coeff AS (SELECT *,CAST(CASE WHEN side<0 THEN 2*r/ls-1 ELSE 2*r/s8+1 END AS DECIMAL(10,0)) AS n,
        e8-24-CASE WHEN side<0 AND ls<s8 THEN 1 ELSE 0 END AS me FROM __sf_native_side LIMIT 18446744073709551615)
      {digits},
      __sf_short_bounds AS (SELECT *,CAST(LENGTH(d) AS SIGNED)+LEAST(me,0) AS k FROM __sf_short_digits LIMIT 18446744073709551615)
      {candidates},
      __sf_short_selected AS (SELECT CASE WHEN delta=1 THEN 1 WHEN REPLACE(SUBSTRING(d,precision_digits+1),'0','')='' THEN 0 ELSE -1 END AS relation
        FROM __sf_short_lexicals WHERE CAST(lexical AS DOUBLE)=x ORDER BY precision_digits,preference LIMIT 1),
      __sf_native_result AS (SELECT *,CASE WHEN side=0 THEN 0 ELSE COALESCE((SELECT relation FROM __sf_short_selected),CAST(JSON_EXTRACT('semantic-fabric-invalid-native-double','$') AS SIGNED)) END AS relation FROM __sf_native_side LIMIT 18446744073709551615),
      __sf_native_dispatch AS (SELECT *,CASE WHEN v IS NULL THEN 3 WHEN ABS(v)>CAST('{overflow}' AS DOUBLE) OR (x=CAST('{overflow}' AS DOUBLE) AND relation>=0) THEN CASE WHEN v<0 THEN -1 ELSE 1 END ELSE 0 END AS tag FROM __sf_native_result LIMIT 18446744073709551615)
      SELECT JSON_ARRAY(tag,CAST(CASE WHEN tag=0 THEN SIGN(v)*(CASE WHEN side<0 AND relation<0 THEN r-ls WHEN side>0 AND relation>0 THEN r+s8 ELSE r END) ELSE 0 END AS DOUBLE)) FROM __sf_native_dispatch)"#
    )
}
