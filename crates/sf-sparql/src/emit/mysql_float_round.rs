//! Correct Double-then-Float casts only at binary32 midpoints.
//! Every midpoint is binary64-representable: double rounding can cross its
//! side only when the nearest Double equals that midpoint.

pub(super) fn power_digits(base: u8, exponent: usize) -> String {
    let mut digits = vec![1_u8];
    for _ in 0..exponent {
        let mut carry = 0_u16;
        for digit in &mut digits {
            let product = u16::from(*digit) * u16::from(base) + carry;
            *digit = (product % 10) as u8;
            carry = product / 10;
        }
        while carry != 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    digits
        .into_iter()
        .rev()
        .map(|d| char::from(b'0' + d))
        .collect()
}

pub(super) fn overflow(double: bool) -> &'static str {
    static FLOAT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static DOUBLE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    (if double { &DOUBLE } else { &FLOAT }).get_or_init(|| build_overflow(double))
}

fn build_overflow(double: bool) -> String {
    let (p, q) = if double { (1024, 970) } else { (128, 103) };
    let mut upper = power_digits(2, p).into_bytes();
    let lower = power_digits(2, q);
    let mut borrow = 0_i16;
    for (i, digit) in upper.iter_mut().rev().enumerate() {
        let subtract = lower
            .as_bytes()
            .iter()
            .rev()
            .nth(i)
            .map_or(0, |d| i16::from(d - b'0'));
        let result = i16::from(*digit - b'0') - subtract - borrow;
        borrow = i16::from(result < 0);
        *digit = b'0' + result.rem_euclid(10) as u8;
    }
    String::from_utf8(upper).unwrap()
}

fn power_two(exponent: i32) -> f64 {
    f64::from_bits(((exponent + 1023) as u64) << 52)
}

/// Append stages to a CTE containing finite nonnegative Double x, full
/// significant digits d and scientific order ord. Invalid/special inputs
/// have x=0 and are dispatched by the caller's tag, not by this recipe.
pub(super) fn stages() -> &'static str {
    static STAGES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    STAGES.get_or_init(build_stages)
}

fn build_stages() -> String {
    let max = f64::from(f32::MAX);
    let factor = (-150..=103)
        .map(|e: i32| {
            format!(
                "WHEN {e} THEN '{}'",
                power_digits(if e < 0 { 5 } else { 2 }, e.unsigned_abs() as usize)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let base = "1000000000000000000000000000000"; // 10^30, not a BIGINT
    let mut sql = format!(
        r#",
      __sf_float_candidate AS (SELECT *,CAST(CAST(LEAST(x,CAST('{max}' AS DOUBLE)) AS FLOAT) AS DOUBLE) AS r FROM __sf_float_number LIMIT 18446744073709551615),
      __sf_float_scale0 AS (SELECT *,r/CAST('{}' AS DOUBLE) AS y0,-126 AS e0,CAST('{}' AS DOUBLE) AS s0 FROM __sf_float_candidate LIMIT 18446744073709551615)"#,
        power_two(-126),
        power_two(-149)
    );
    // Eight exact binary-scaling steps replace 253 linear table branches.
    // y starts at r*2^126 (<2^254). Scaling by powers of two is exact in
    // binary64 throughout; subnormals keep the minimum binary32 spacing.
    for (i, shift) in [128, 64, 32, 16, 8, 4, 2, 1].into_iter().enumerate() {
        let next = i + 1;
        let power = power_two(shift);
        sql.push_str(&format!(",__sf_float_scale{next} AS (SELECT *,CASE WHEN y{i}>=CAST('{power}' AS DOUBLE) THEN y{i}/CAST('{power}' AS DOUBLE) ELSE y{i} END AS y{next},e{i}+CASE WHEN y{i}>=CAST('{power}' AS DOUBLE) THEN {shift} ELSE 0 END AS e{next},CASE WHEN y{i}>=CAST('{power}' AS DOUBLE) THEN s{i}*CAST('{power}' AS DOUBLE) ELSE s{i} END AS s{next} FROM __sf_float_scale{i} LIMIT 18446744073709551615)"));
    }
    sql.push_str(&format!(r#",
      __sf_float_spacing AS (SELECT *,e8 AS e,s8 AS s FROM __sf_float_scale8 LIMIT 18446744073709551615),
      __sf_float_lower AS (SELECT *,CASE WHEN e>-126 AND r=s*8388608 THEN s/2 ELSE s END AS ls FROM __sf_float_spacing LIMIT 18446744073709551615),
      __sf_float_midpoint AS (SELECT *,CASE WHEN r>0 AND x=r-ls/2 THEN -1 WHEN x=r+s/2 THEN 1 ELSE 0 END AS side FROM __sf_float_lower LIMIT 18446744073709551615),
      __sf_float_coefficient AS (SELECT *,CAST(CASE WHEN side<0 THEN 2*r/ls-1 ELSE 2*r/s+1 END AS DECIMAL(10,0)) AS n,
        e-24-CASE WHEN side<0 AND ls<s THEN 1 ELSE 0 END AS me FROM __sf_float_midpoint LIMIT 18446744073709551615),
      __sf_float_factor AS (SELECT *,LPAD(CASE me {factor} END,120,'0') AS factor FROM __sf_float_coefficient LIMIT 18446744073709551615)"#));
    // Four 30-digit limbs suffice: 5^150 * N has <=113 digits, N<2^25.
    // Each limb product plus carry has <38 digits. Carry uses the exact
    // integer remainder before division, preventing fractional carry rounding.
    for i in 0..4 {
        let previous = if i == 0 {
            "__sf_float_factor".into()
        } else {
            format!("__sf_float_limb{}", i - 1)
        };
        let carry = if i == 0 {
            "0".into()
        } else {
            format!("(p{}-MOD(p{},{base}))/{base}", i - 1, i - 1)
        };
        sql.push_str(&format!(", __sf_float_limb{i} AS (SELECT *,CAST(n*CAST(SUBSTRING(factor,{},30) AS DECIMAL(30,0))+({carry}) AS DECIMAL(65,0)) AS p{i} FROM {previous} LIMIT 18446744073709551615)", 91-i*30));
    }
    let chunks = (0..4)
        .rev()
        .map(|i| format!("LPAD(CAST(MOD(p{i},{base}) AS CHAR),30,'0')"))
        .collect::<Vec<_>>()
        .join(",");
    sql.push_str(&format!(r#",
      __sf_float_boundary AS (SELECT *,TRIM(LEADING '0' FROM CONCAT({chunks})) AS md FROM __sf_float_limb3 LIMIT 18446744073709551615),
      __sf_float_side AS (SELECT *,
        CASE WHEN (ord,CAST(TRIM(TRAILING '0' FROM d) AS BINARY)) < (CAST(LENGTH(md) AS SIGNED)-1+LEAST(me,0),CAST(TRIM(TRAILING '0' FROM md) AS BINARY)) THEN -1
          WHEN (ord,CAST(TRIM(TRAILING '0' FROM d) AS BINARY)) > (CAST(LENGTH(md) AS SIGNED)-1+LEAST(me,0),CAST(TRIM(TRAILING '0' FROM md) AS BINARY)) THEN 1 ELSE 0 END AS relation FROM __sf_float_boundary LIMIT 18446744073709551615),
      __sf_float_result AS (SELECT *,CASE WHEN side<0 AND relation<0 THEN r-ls WHEN side>0 AND relation>0 THEN r+s ELSE r END AS result FROM __sf_float_side LIMIT 18446744073709551615)"#));
    sql
}
