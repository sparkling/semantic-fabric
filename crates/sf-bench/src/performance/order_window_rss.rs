//! ADR-0054 fresh-process RSS workload for a fixed root ORDER window.

use std::fmt;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use sf_core::Term;
use sf_sparql::{exec, parse_and_translate_with, Tbox};
use sf_sql::Dialect;

use super::compare::MEMORY_SCALING_LIMIT_PERCENT;
use super::model::{
    BoundaryId, MetricId, ScenarioConfig, ScenarioObservation, Unit, M0_SAMPLE_COUNT,
};

pub const ORDER_WINDOW_SCALES: [u32; 3] = [1, 10, 100];
pub const ORDER_WINDOW_BASE_ROWS: u32 = 1_000;
pub const ORDER_WINDOW_RESULT_ROWS: usize = 80;
pub const ORDER_WINDOW_SQLITE_CACHE_KIB: i64 = 64;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ; rr:objectMap [ rr:column "value" ]
  ] .
"#;

const QUERY: &str = "SELECT ?value WHERE { ?item <http://example.test/value> ?value } \
                     ORDER BY ?value OFFSET 20 LIMIT 80";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderWindowRssError(pub String);

impl fmt::Display for OrderWindowRssError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OrderWindowRssError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RssGrowthQualification {
    pub median_10x_x2: u128,
    pub median_100x_x2: u128,
    pub p95_10x: u64,
    pub p95_100x: u64,
}

pub fn scenarios() -> Result<Vec<ScenarioConfig>, OrderWindowRssError> {
    ORDER_WINDOW_SCALES
        .into_iter()
        .map(scenario_for_scale)
        .collect()
}

pub fn scenario_by_id(id: &str) -> Result<ScenarioConfig, OrderWindowRssError> {
    scenarios()?
        .into_iter()
        .find(|scenario| scenario.id == id)
        .ok_or_else(|| OrderWindowRssError(format!("unknown ORDER window RSS scenario: {id}")))
}

pub fn source_rows(config: &ScenarioConfig) -> Result<u32, OrderWindowRssError> {
    validate_scenario(config)?;
    ORDER_WINDOW_BASE_ROWS
        .checked_mul(config.scale)
        .ok_or_else(|| OrderWindowRssError("ORDER window source row count overflow".into()))
}

pub fn prepare_source(
    run_directory: &Path,
    config: &ScenarioConfig,
) -> Result<PathBuf, OrderWindowRssError> {
    validate_run_directory(run_directory)?;
    let rows = source_rows(config)?;
    let path = source_path(run_directory, config);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|file| file.metadata())
        .map_err(|error| {
            OrderWindowRssError(format!("create ORDER source {}: {error}", path.display()))
        })?;

    let prepared = (|| {
        let connection = Connection::open(&path).map_err(workload_error)?;
        connection
            .execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT NOT NULL);")
            .map_err(workload_error)?;
        let transaction = connection.unchecked_transaction().map_err(workload_error)?;
        {
            let mut insert = transaction
                .prepare("INSERT INTO items (id, value) VALUES (?1, ?2)")
                .map_err(workload_error)?;
            for index in 0..rows {
                insert
                    .execute(rusqlite::params![index, format!("value-{index:09}")])
                    .map_err(workload_error)?;
            }
        }
        transaction.commit().map_err(workload_error)?;
        let actual_rows: u32 = connection
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .map_err(workload_error)?;
        if actual_rows != rows {
            return Err(OrderWindowRssError(format!(
                "prepared ORDER source has {actual_rows} rows; expected {rows}"
            )));
        }
        Ok::<(), OrderWindowRssError>(())
    })();
    match prepared {
        Ok(()) => Ok(path),
        Err(error) => {
            let _ = std::fs::remove_file(path);
            Err(error)
        }
    }
}

pub fn execute_once(
    run_directory: &Path,
    config: &ScenarioConfig,
) -> Result<(), OrderWindowRssError> {
    validate_run_directory(run_directory)?;
    validate_scenario(config)?;
    let path = source_path(run_directory, config);
    validate_source_file(&path)?;
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(workload_error)?;
    // Whole-process RSS necessarily includes SQLite's in-process source cache.
    // Fix it for this measurement only so the gate observes ORDER retention and
    // allocator behavior, not a source-size-dependent cache warm-up. This is not
    // a production connection-tuning claim.
    connection
        .pragma_update(None, "cache_size", -ORDER_WINDOW_SQLITE_CACHE_KIB)
        .map_err(workload_error)?;
    let cache_size: i64 = connection
        .pragma_query_value(None, "cache_size", |row| row.get(0))
        .map_err(workload_error)?;
    if cache_size != -ORDER_WINDOW_SQLITE_CACHE_KIB {
        return Err(OrderWindowRssError(format!(
            "SQLite cache control is {cache_size}; expected -{ORDER_WINDOW_SQLITE_CACHE_KIB} KiB"
        )));
    }
    let mapping = sf_mapping::parse_r2rml(MAPPING).map_err(workload_error)?;
    let schema =
        sf_sql::introspect::introspect_sqlite(&connection, "items").map_err(workload_error)?;
    let plan = parse_and_translate_with(
        QUERY,
        &mapping,
        Dialect::Sqlite,
        &Tbox::default(),
        &[schema],
    )
    .map_err(workload_error)?;
    let results = exec::select(&plan, &connection).map_err(workload_error)?;
    verify_results(&results)?;
    Ok(())
}

pub fn remove_source(
    run_directory: &Path,
    config: &ScenarioConfig,
) -> Result<(), OrderWindowRssError> {
    validate_run_directory(run_directory)?;
    validate_scenario(config)?;
    let path = source_path(run_directory, config);
    validate_source_file(&path)?;
    std::fs::remove_file(&path).map_err(|error| {
        OrderWindowRssError(format!("remove ORDER source {}: {error}", path.display()))
    })
}

pub fn qualify_growth(
    observations: &[ScenarioObservation],
) -> Result<RssGrowthQualification, OrderWindowRssError> {
    let expected = scenarios()?;
    if observations.len() != expected.len()
        || observations
            .iter()
            .zip(&expected)
            .any(|(observation, scenario)| observation.config != *scenario)
    {
        return Err(OrderWindowRssError(
            "ORDER window RSS observations do not match exact 1x/10x/100x scenarios".into(),
        ));
    }
    let ten_x = &observations[1].summary;
    let hundred_x = &observations[2].summary;
    let qualification = RssGrowthQualification {
        median_10x_x2: ten_x.median.doubled_ns(),
        median_100x_x2: hundred_x.median.doubled_ns(),
        p95_10x: ten_x.p95,
        p95_100x: hundred_x.p95,
    };
    let median_passes = checked_within_percent(
        qualification.median_100x_x2,
        qualification.median_10x_x2,
        MEMORY_SCALING_LIMIT_PERCENT,
    )?;
    let p95_passes = checked_within_percent(
        u128::from(qualification.p95_100x),
        u128::from(qualification.p95_10x),
        MEMORY_SCALING_LIMIT_PERCENT,
    )?;
    if !median_passes || !p95_passes {
        return Err(OrderWindowRssError(format!(
            "ORDER window RSS 10x-to-100x growth exceeds {MEMORY_SCALING_LIMIT_PERCENT}%: \
             median_x2={}->{}, p95={}->{}",
            qualification.median_10x_x2,
            qualification.median_100x_x2,
            qualification.p95_10x,
            qualification.p95_100x
        )));
    }
    Ok(qualification)
}

fn scenario_for_scale(scale: u32) -> Result<ScenarioConfig, OrderWindowRssError> {
    let id = match scale {
        1 => "order.sqlite.window.rss.scale001",
        10 => "order.sqlite.window.rss.scale010",
        100 => "order.sqlite.window.rss.scale100",
        _ => {
            return Err(OrderWindowRssError(format!(
                "unsupported ORDER window RSS scale: {scale}"
            )))
        }
    };
    ScenarioConfig::new(
        id,
        scale,
        MetricId::RssLinuxProcessPeak,
        BoundaryId::LinuxFreshProcessLifetime,
        Unit::Bytes,
        0,
        M0_SAMPLE_COUNT,
    )
    .map_err(|error| OrderWindowRssError(error.to_string()))
}

fn validate_scenario(config: &ScenarioConfig) -> Result<(), OrderWindowRssError> {
    if scenario_for_scale(config.scale)? != *config {
        return Err(OrderWindowRssError(format!(
            "scenario {} does not match the fixed ORDER window RSS workload",
            config.id
        )));
    }
    Ok(())
}

fn source_path(run_directory: &Path, config: &ScenarioConfig) -> PathBuf {
    run_directory.join(format!("{}.source.sqlite", config.id))
}

fn validate_run_directory(path: &Path) -> Result<(), OrderWindowRssError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        OrderWindowRssError(format!(
            "inspect ORDER run directory {}: {error}",
            path.display()
        ))
    })?;
    if !path.is_absolute()
        || metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || path.canonicalize().ok().as_deref() != Some(path)
    {
        return Err(OrderWindowRssError(
            "ORDER run directory must be absolute, canonical, and non-symlink".into(),
        ));
    }
    Ok(())
}

fn validate_source_file(path: &Path) -> Result<(), OrderWindowRssError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        OrderWindowRssError(format!("inspect ORDER source {}: {error}", path.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OrderWindowRssError(
            "prepared ORDER source is not a regular non-symlink file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(OrderWindowRssError(
                "prepared ORDER source is a hard link".into(),
            ));
        }
    }
    Ok(())
}

fn verify_results(results: &exec::Solutions) -> Result<(), OrderWindowRssError> {
    if results.vars != ["value"] || results.rows.len() != ORDER_WINDOW_RESULT_ROWS {
        return Err(OrderWindowRssError(format!(
            "fixed ORDER window returned vars={:?}, rows={}; expected ?value and {} rows",
            results.vars,
            results.rows.len(),
            ORDER_WINDOW_RESULT_ROWS
        )));
    }
    for (position, row) in results.rows.iter().enumerate() {
        let expected_index = position
            .checked_add(20)
            .ok_or_else(|| OrderWindowRssError("expected result index overflow".into()))?;
        let expected = format!("value-{expected_index:09}");
        match row.as_slice() {
            [Some(Term::Literal(literal))] if literal.value() == expected => {}
            _ => {
                return Err(OrderWindowRssError(format!(
                    "fixed ORDER window row {position} was {row:?}; expected {expected:?}"
                )))
            }
        }
    }
    Ok(())
}

fn checked_within_percent(
    candidate: u128,
    baseline: u128,
    limit_percent: u8,
) -> Result<bool, OrderWindowRssError> {
    let candidate_scaled = candidate
        .checked_mul(100)
        .ok_or_else(|| OrderWindowRssError("RSS candidate percentage overflow".into()))?;
    let permitted_percent = 100_u128
        .checked_add(u128::from(limit_percent))
        .ok_or_else(|| OrderWindowRssError("RSS permitted percentage overflow".into()))?;
    let permitted = baseline
        .checked_mul(permitted_percent)
        .ok_or_else(|| OrderWindowRssError("RSS baseline percentage overflow".into()))?;
    Ok(candidate_scaled <= permitted)
}

fn workload_error(error: impl fmt::Display) -> OrderWindowRssError {
    OrderWindowRssError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_scenarios_are_exact_1x_10x_100x_sources() {
        let scenarios = scenarios().unwrap();
        assert_eq!(
            scenarios
                .iter()
                .map(source_rows)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            [1_000, 10_000, 100_000]
        );
        assert!(scenarios.iter().all(|scenario| {
            scenario.metric == MetricId::RssLinuxProcessPeak
                && scenario.boundary == BoundaryId::LinuxFreshProcessLifetime
                && scenario.warmup_count == 0
                && scenario.sample_count == M0_SAMPLE_COUNT
        }));
    }

    #[test]
    fn percentage_gate_is_inclusive_and_overflow_checked() {
        assert!(checked_within_percent(110, 100, 10).unwrap());
        assert!(!checked_within_percent(111, 100, 10).unwrap());
        assert!(checked_within_percent(u64::MAX.into(), u64::MAX.into(), 10).unwrap());
        assert!(checked_within_percent(u128::MAX, u128::MAX, 10).is_err());
    }
}
