//! Exact PostgreSQL IEEE wire values → Rust shortest-decimal lexical keys.
//!
//! PostgreSQL's float text formatter has different boundary/tie choices from
//! Rust's decoder. Derive the decoder's interval from the binary value instead;
//! at most 9/17 decimal precisions and two candidates per precision are needed.
//! All arithmetic is exact NUMERIC, bounded by the IEEE format (max 1076 scale).
use super::*;

pub(super) fn is_float(key: NativeScalarKey) -> bool {
    matches!(
        key,
        NativeScalarKey::PostgresFloat4 | NativeScalarKey::PostgresFloat8
    )
}

/// `canonical` selects the natural xsd:double consumer; otherwise keep the raw
/// Rust Display spelling used by IRI templates and actual datatype overrides.
pub(super) fn lexical(raw: &str, key: NativeScalarKey, canonical: bool) -> String {
    let (bytes, fraction, bias, max_exponent, precision) = match key {
        NativeScalarKey::PostgresFloat4 => (4, 23, 127, 255, 9),
        NativeScalarKey::PostgresFloat8 => (8, 52, 1023, 2047, 17),
        _ => unreachable!("PostgreSQL floating decoder proof"),
    };
    let hidden = 1_u64 << fraction;
    let magnitude = (0..bytes)
        .map(|i| {
            let byte = format!("CAST(pg_catalog.get_byte(b, {i}) AS BIGINT)");
            let byte = if i == 0 {
                format!("({byte} & 127)")
            } else {
                byte
            };
            format!("({byte} << {})", 8 * (bytes - i - 1))
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let (zero, infinity) = if canonical {
        ("0.0E0", "INF")
    } else {
        ("0", "inf")
    };
    let result = if canonical {
        let order = decimal_order("s");
        format!("(WITH digits AS (SELECT pg_catalog.rtrim(pg_catalog.ltrim(pg_catalog.replace(s, '.', ''), '0'), '0') AS d, {order} - 1 AS e) SELECT SUBSTRING(d FROM 1 FOR 1) || '.' || CASE WHEN length(d) = 1 THEN '0' ELSE SUBSTRING(d FROM 2) END || 'E' || CAST(e AS TEXT) FROM digits)")
    } else {
        "s".into()
    };
    let order = decimal_order("s");
    // Mirror Rust flt2dec::decoder: integer_decode doubles subnormal mantissas;
    // normal powers of two have asymmetric intervals, including min-normal.
    // Even decoded mantissas admit endpoints. Decimal midpoint ties round up.
    format!(
        r#"(WITH wire AS MATERIALIZED (
        SELECT pg_catalog.float{bytes}send({raw}) AS b
    ), bits AS MATERIALIZED (
        SELECT b, {magnitude} AS mag,
            CASE WHEN pg_catalog.get_byte(b, 0) >= 128 THEN '-' ELSE '' END AS sign FROM wire
    ), fields AS MATERIALIZED (
        SELECT *, (mag >> {fraction}) AS e, (mag & {mask}) AS f FROM bits
    ) SELECT CASE WHEN b IS NULL THEN NULL
        WHEN e = {max_exponent} THEN CASE WHEN f <> 0 THEN 'NaN' ELSE sign || '{infinity}' END
        WHEN mag = 0 THEN sign || '{zero}'
        ELSE sign || COALESCE((WITH decoded AS MATERIALIZED (
            SELECT CASE WHEN e = 0 THEN 2 * f ELSE {hidden} + f END AS m,
                e - {bias} - {fraction} AS t FROM fields
        ), interval_bits AS MATERIALIZED (
            SELECT CASE WHEN e = 0 THEN m WHEN f = 0 THEN 4 * m ELSE 2 * m END AS m,
                CASE WHEN e = 0 THEN t WHEN f = 0 THEN t - 2 ELSE t - 1 END AS t,
                CASE WHEN e <> 0 AND f = 0 THEN 2 ELSE 1 END AS plus,
                (m % 2 = 0) AS inclusive FROM decoded
        ), unit AS MATERIALIZED (
            SELECT *, CASE WHEN t >= 0 THEN pg_catalog.power(CAST(2 AS NUMERIC), t)
                ELSE CAST(CAST(pg_catalog.trim_scale(pg_catalog.power(CAST(5 AS NUMERIC), -t)) AS TEXT) || 'e' || CAST(t AS TEXT) AS NUMERIC) END AS u FROM interval_bits
        ), bounds AS MATERIALIZED (
            SELECT m * u AS x, (m - 1) * u AS lo, (m + plus) * u AS hi, inclusive FROM unit
        ), spelled AS MATERIALIZED (
            SELECT *, CAST(pg_catalog.trim_scale(x) AS TEXT) AS s FROM bounds
        ), ordered AS MATERIALIZED (
            SELECT *, {order} AS k FROM spelled
        ), grid AS MATERIALIZED (
            SELECT *, pg_catalog.trunc(x * CAST('1e' || CAST(p - k AS TEXT) AS NUMERIC)) AS q,
                CAST('1e' || CAST(k - p AS TEXT) AS NUMERIC) AS step
            FROM ordered CROSS JOIN pg_catalog.generate_series(1, {precision}) AS precisions(p)
        ), candidates AS MATERIALIZED (
            SELECT p, x, lo, hi, inclusive, (q + delta) * step AS c
            FROM grid CROSS JOIN (VALUES (0), (1)) AS offsets(delta)
        ), selected AS (
            SELECT CAST(pg_catalog.trim_scale(c) AS TEXT) AS s FROM candidates
            WHERE (c > lo AND c < hi) OR (inclusive AND (c = lo OR c = hi))
            ORDER BY p, pg_catalog.abs(c - x), c DESC LIMIT 1
        ) SELECT {result} FROM selected),
            CAST(CAST('semantic-fabric-invalid-float-' || CAST(mag AS TEXT) AS BIGINT) AS TEXT))
        END FROM fields)"#,
        mask = hidden - 1
    )
}

fn decimal_order(text: &str) -> String {
    format!("CASE WHEN pg_catalog.split_part({text}, '.', 1) <> '0' THEN length(pg_catalog.split_part({text}, '.', 1)) ELSE -(length(pg_catalog.split_part({text}, '.', 2)) - length(pg_catalog.ltrim(pg_catalog.split_part({text}, '.', 2), '0'))) END")
}

/// Within one unchanged decoder, bits are an injective identity key except NaN
/// payloads/signs, which Rust always spells `NaN`. Retain signed-zero bits.
pub(super) fn identity(raw: &str, key: NativeScalarKey) -> String {
    let bytes = if key == NativeScalarKey::PostgresFloat4 {
        4
    } else {
        8
    };
    format!("CASE WHEN {raw} = 'NaN' THEN pg_catalog.decode('7f', 'hex') ELSE pg_catalog.float{bytes}send({raw}) END")
}

/// SQL pooling may widen FLOAT4 to FLOAT8, altering both natural and raw RDF
/// output. Independent top-level UNION arms do not cross this coercion boundary.
pub(super) fn validate_union(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    distinct: bool,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    if dialect != Dialect::Postgres || branches.len() < 2 {
        return Ok(());
    }
    let mut keys: Vec<Vec<_>> = Vec::new();
    for branch in branches {
        let actuals = branch_actuals_controlled(branch, dialect, catalog, work)?;
        let consumed: HashSet<_> = branch
            .bindings
            .values()
            .flat_map(TermDef::columns)
            .collect();
        keys.push(
            source_projection(branch, distinct || branch.distinct, dialect)
                .iter()
                .map(|column| {
                    column
                        .as_ref()
                        .filter(|column| consumed.contains(column))
                        .and_then(|column| iri_cmp::scalar_column(column, &actuals))
                })
                .collect(),
        );
    }
    for arm in &keys {
        for (index, key) in arm.iter().enumerate() {
            if key.is_some_and(is_float) && keys.iter().any(|other| other.get(index) != Some(key)) {
                return Err(Error::Unsupported(
                    "native floating UNION requires the same exact decoder in every pooled arm"
                        .into(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_floating_guards_do_not_block_integer_subject_union() {
        let maps = sf_mapping::parse_r2rml(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
          <#m> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "http://ex/{id}"];
          rr:predicateObjectMap [rr:predicate <http://ex/n>; rr:objectMap [rr:column "narrow"]];
          rr:predicateObjectMap [rr:predicate <http://ex/w>; rr:objectMap [rr:column "wide"]]."#,
        )
        .unwrap();
        let mut catalog = ColumnCatalog::default();
        catalog
            .insert_live_result(
                &LogicalSource::Table("items".into()),
                [
                    (
                        "id",
                        sf_core::datatype::XsdTypeCode::Integer,
                        NativeScalarKey::Integer,
                    ),
                    (
                        "narrow",
                        sf_core::datatype::XsdTypeCode::Double,
                        NativeScalarKey::PostgresFloat4,
                    ),
                    (
                        "wide",
                        sf_core::datatype::XsdTypeCode::Double,
                        NativeScalarKey::PostgresFloat8,
                    ),
                ]
                .into_iter()
                .map(|(name, code, key)| sf_sql::backend::ResultColumn {
                    name: name.into(),
                    natural_datatype: Some(code),
                    native_scalar: Some(key),
                    text_key: None,
                    sqlite_decode: None,
                })
                .collect(),
            )
            .unwrap();
        let plan = crate::parse_and_translate("SELECT ?s WHERE { { SELECT ?s WHERE { { ?s <http://ex/n> ?o } UNION { ?s <http://ex/w> ?o } } } }", &maps, Dialect::Postgres).unwrap();
        let sql = emit_subplan_sql(&plan, Dialect::Postgres, &catalog)
            .unwrap()
            .0;
        assert!(sql.contains(" UNION ALL "));
        assert!(
            sql.contains("float4send") && sql.contains("float8send"),
            "each arm retains its original dedup key"
        );
    }

    #[test]
    fn sql_pooled_float_arms_require_unchanged_decoders() {
        let maps = sf_mapping::parse_r2rml(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
          <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
          rr:predicateObjectMap [rr:predicate <http://ex/n>; rr:objectMap [rr:column "narrow"]];
          rr:predicateObjectMap [rr:predicate <http://ex/w>; rr:objectMap [rr:column "wide"]]."#,
        )
        .unwrap();
        let source = LogicalSource::Table("items".into());
        for wide in [
            Some(NativeScalarKey::PostgresFloat4),
            Some(NativeScalarKey::PostgresFloat8),
            None,
        ] {
            let mut catalog = ColumnCatalog::default();
            catalog
                .insert_live_result(
                    &source,
                    [
                        ("narrow", Some(NativeScalarKey::PostgresFloat4)),
                        ("wide", wide),
                    ]
                    .into_iter()
                    .map(|(name, scalar)| sf_sql::backend::ResultColumn {
                        name: name.into(),
                        natural_datatype: Some(sf_core::datatype::XsdTypeCode::Double),
                        native_scalar: scalar,
                        text_key: None,
                        sqlite_decode: None,
                    })
                    .collect(),
                )
                .unwrap();
            for distinct in ["", "DISTINCT "] {
                let plan = crate::parse_and_translate(&format!("SELECT {distinct}?o WHERE {{ {{ ?s <http://ex/n> ?o }} UNION {{ ?s <http://ex/w> ?o }} }}"), &maps, Dialect::Postgres).unwrap();
                let result = emit_subplan_sql(&plan, Dialect::Postgres, &catalog);
                let actuals = subplan_actuals(&plan, Dialect::Postgres, &catalog);
                if wide == Some(NativeScalarKey::PostgresFloat4) {
                    let sql = result.unwrap().0;
                    assert!(sql.contains(" UNION ALL "));
                    assert!(sql.contains("float4send"));
                    assert_eq!(
                        actuals.scalar_columns["c0"],
                        NativeScalarKey::PostgresFloat4
                    );
                    assert_eq!(
                        actuals.natural_columns["c0"],
                        Some(sf_core::datatype::XsdTypeCode::Double)
                    );
                } else {
                    assert!(result
                        .unwrap_err()
                        .to_string()
                        .contains("same exact decoder"));
                    assert_eq!(actuals.natural_columns["c0"], None);
                }
            }
        }
    }

    #[test]
    fn formatter_is_bounded_and_ast_round_trips_without_float_text_casts() {
        for key in [
            NativeScalarKey::PostgresFloat4,
            NativeScalarKey::PostgresFloat8,
        ] {
            for canonical in [false, true] {
                let sql = lexical("t0.src", key, canonical);
                assert!(!sql.contains("CAST(t0.src AS"));
                assert!(sql.contains("generate_series(1, "));
                assert!(sql.contains("c DESC LIMIT 1"));
                assert!(sql.contains("semantic-fabric-invalid-float-"));
                Dialect::Postgres
                    .emit_via_ast(&format!("SELECT {sql} FROM items t0"))
                    .unwrap_or_else(|error| panic!("{error}: {sql}"));
            }
        }
    }
}
