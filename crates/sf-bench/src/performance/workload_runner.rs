use std::fmt;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::Instant;

use rusqlite::Connection;
use sf_core::ir::TriplesMap;
use sf_sql::TableSchema;

use super::config::parse_scenarios;
use super::digest::sha256_hex;
use super::model::{BoundaryId, MetricId, ScenarioConfig, Unit, M0_SAMPLE_COUNT};
use crate::{driver, mem, workload};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadError(pub String);

impl fmt::Display for WorkloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorkloadError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkloadKind {
    ConstructFirst,
    ConstructFull,
    ConstructHeap,
    ConstructRss,
    Select(usize),
}

struct Fixture {
    path: PathBuf,
    connection: Connection,
    maps: Vec<TriplesMap>,
    schemas: Vec<TableSchema>,
    kind: WorkloadKind,
    delete_on_finish: bool,
}

pub trait WorkloadExecutor {
    type Error: fmt::Display;

    fn begin_scenario(&mut self, config: &ScenarioConfig) -> Result<(), Self::Error>;
    fn execute_once(&mut self, config: &ScenarioConfig) -> Result<u64, Self::Error>;
    fn finish_scenario(&mut self) -> Result<(), Self::Error>;
}

pub struct SqliteGtfsExecutor {
    run_directory: PathBuf,
    fixture: Option<Fixture>,
}

impl SqliteGtfsExecutor {
    pub fn new(run_directory: PathBuf) -> Result<Self, WorkloadError> {
        let metadata = std::fs::symlink_metadata(&run_directory).map_err(|error| {
            WorkloadError(format!("inspect {}: {error}", run_directory.display()))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(WorkloadError(
                "run directory must be a non-symlink directory".into(),
            ));
        }
        Ok(Self {
            run_directory,
            fixture: None,
        })
    }

    fn execute_fixture(fixture: &Fixture) -> Result<u64, WorkloadError> {
        match fixture.kind {
            WorkloadKind::ConstructFirst => {
                let (count, first, _) = driver::stream_construct_timed(
                    &fixture.maps,
                    &fixture.connection,
                    &fixture.schemas,
                    workload::DUMP_QUERY,
                )
                .map_err(workload_error)?;
                require_nonzero(count, "CONSTRUCT first-result")?;
                duration_ns(first.as_nanos())
            }
            WorkloadKind::ConstructFull => {
                let start = Instant::now();
                let count = driver::stream_construct_count(
                    &fixture.maps,
                    &fixture.connection,
                    &fixture.schemas,
                    workload::DUMP_QUERY,
                )
                .map_err(workload_error)?;
                let elapsed = start.elapsed();
                require_nonzero(count, "CONSTRUCT full-result")?;
                duration_ns(elapsed.as_nanos())
            }
            WorkloadKind::ConstructHeap => {
                let baseline = mem::reset_peak();
                let count = driver::stream_construct_count(
                    &fixture.maps,
                    &fixture.connection,
                    &fixture.schemas,
                    workload::DUMP_QUERY,
                )
                .map_err(workload_error)?;
                require_nonzero(count, "CONSTRUCT heap")?;
                u64::try_from(mem::window_peak(baseline))
                    .map_err(|_| WorkloadError("heap sample does not fit u64".into()))
            }
            WorkloadKind::ConstructRss => Err(WorkloadError(
                "RSS workload must use the typed fresh-process execution path".into(),
            )),
            WorkloadKind::Select(index) => {
                let queries = workload::queries();
                let (_, sparql) = queries
                    .get(index)
                    .ok_or_else(|| WorkloadError("invalid SELECT workload index".into()))?;
                let start = Instant::now();
                let rows = driver::run_select(
                    &fixture.maps,
                    &fixture.connection,
                    &fixture.schemas,
                    sparql,
                )
                .map_err(workload_error)?;
                let elapsed = start.elapsed();
                require_nonzero(rows as u64, "SELECT full-result")?;
                duration_ns(elapsed.as_nanos())
            }
        }
    }

    pub fn prepare_rss_source(
        run_directory: &std::path::Path,
        config: &ScenarioConfig,
    ) -> Result<PathBuf, WorkloadError> {
        if fixed_workload_kind(config)? != WorkloadKind::ConstructRss {
            return Err(WorkloadError(
                "RSS source preparation requires an RSS scenario".into(),
            ));
        }
        let path = rss_source_path(run_directory, config);
        create_scratch_file(&path)?;
        let prepared = workload::open_source_db(&path, config.scale).map_err(workload_error);
        match prepared {
            Ok((connection, _)) => {
                drop(connection);
                Ok(path)
            }
            Err(error) => {
                let _ = std::fs::remove_file(path);
                Err(error)
            }
        }
    }

    pub fn begin_rss_scenario(&mut self, config: &ScenarioConfig) -> Result<(), WorkloadError> {
        if self.fixture.is_some() || fixed_workload_kind(config)? != WorkloadKind::ConstructRss {
            return Err(WorkloadError(
                "fresh-process RSS fixture state or scenario is invalid".into(),
            ));
        }
        let path = rss_source_path(&self.run_directory, config);
        validate_existing_source(&path)?;
        let connection = Connection::open(&path).map_err(workload_error)?;
        let maps = driver::mapping().map_err(workload_error)?;
        let schemas = driver::introspect(&connection).map_err(workload_error)?;
        self.fixture = Some(Fixture {
            path,
            connection,
            maps,
            schemas,
            kind: WorkloadKind::ConstructRss,
            delete_on_finish: false,
        });
        Ok(())
    }

    pub fn execute_rss_once(&mut self, config: &ScenarioConfig) -> Result<(), WorkloadError> {
        if fixed_workload_kind(config)? != WorkloadKind::ConstructRss {
            return Err(WorkloadError(
                "fresh-process RSS execution requires an RSS scenario".into(),
            ));
        }
        let fixture = self
            .fixture
            .as_ref()
            .filter(|fixture| fixture.kind == WorkloadKind::ConstructRss)
            .ok_or_else(|| WorkloadError("RSS fixture is not active".into()))?;
        let count = driver::stream_construct_count(
            &fixture.maps,
            &fixture.connection,
            &fixture.schemas,
            workload::DUMP_QUERY,
        )
        .map_err(workload_error)?;
        require_nonzero(count, "CONSTRUCT RSS")?;
        Ok(())
    }
}

impl WorkloadExecutor for SqliteGtfsExecutor {
    type Error = WorkloadError;

    fn begin_scenario(&mut self, config: &ScenarioConfig) -> Result<(), Self::Error> {
        if self.fixture.is_some() {
            return Err(WorkloadError(
                "previous scenario fixture is still active".into(),
            ));
        }
        let kind = fixed_workload_kind(config)?;
        let path = self.run_directory.join(format!("{}.sqlite", config.id));
        create_scratch_file(&path)?;
        let setup = (|| {
            let (connection, _) =
                workload::open_source_db(&path, config.scale).map_err(workload_error)?;
            let maps = driver::mapping().map_err(workload_error)?;
            let schemas = driver::introspect(&connection).map_err(workload_error)?;
            Ok::<_, WorkloadError>(Fixture {
                path: path.clone(),
                connection,
                maps,
                schemas,
                kind,
                delete_on_finish: true,
            })
        })();
        match setup {
            Ok(fixture) => {
                self.fixture = Some(fixture);
                Ok(())
            }
            Err(error) => {
                let _ = std::fs::remove_file(path);
                Err(error)
            }
        }
    }

    fn execute_once(&mut self, config: &ScenarioConfig) -> Result<u64, Self::Error> {
        let expected = fixed_workload_kind(config)?;
        let fixture = self
            .fixture
            .as_ref()
            .ok_or_else(|| WorkloadError("scenario fixture is not active".into()))?;
        if fixture.kind != expected {
            return Err(WorkloadError(
                "active fixture does not match scenario".into(),
            ));
        }
        Self::execute_fixture(fixture)
    }

    fn finish_scenario(&mut self) -> Result<(), Self::Error> {
        let fixture = self
            .fixture
            .take()
            .ok_or_else(|| WorkloadError("scenario fixture is not active".into()))?;
        let path = fixture.path.clone();
        let delete = fixture.delete_on_finish;
        drop(fixture);
        if delete {
            std::fs::remove_file(&path)
                .map_err(|error| WorkloadError(format!("remove {}: {error}", path.display())))?;
        }
        Ok(())
    }
}

impl Drop for SqliteGtfsExecutor {
    fn drop(&mut self) {
        if let Some(fixture) = self.fixture.take() {
            let path = fixture.path.clone();
            let delete = fixture.delete_on_finish;
            drop(fixture);
            if delete {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

pub fn validate_fixed_m0_scenarios(scenarios: &[ScenarioConfig]) -> Result<(), WorkloadError> {
    if scenarios.len() != 17 {
        return Err(WorkloadError(
            "M0 workload must contain exactly 17 scenarios".into(),
        ));
    }
    for scenario in scenarios {
        fixed_workload_kind(scenario)?;
    }
    Ok(())
}

pub fn workload_sha256(manifest: &[u8]) -> Result<String, WorkloadError> {
    let scenarios = parse_scenarios(manifest).map_err(|error| WorkloadError(error.to_string()))?;
    validate_fixed_m0_scenarios(&scenarios)?;
    let mut binding = b"sf-performance-workload-binding-v1\n".to_vec();
    append_blob(&mut binding, "scenario-manifest", manifest);
    append_blob(
        &mut binding,
        "r2rml-mapping",
        workload::MAPPING_TTL.as_bytes(),
    );
    append_blob(
        &mut binding,
        "construct-query",
        workload::DUMP_QUERY.as_bytes(),
    );
    for (name, query) in workload::queries() {
        append_blob(&mut binding, name, query.as_bytes());
    }
    Ok(sha256_hex(&binding))
}

fn fixed_workload_kind(config: &ScenarioConfig) -> Result<WorkloadKind, WorkloadError> {
    let (kind, scale, metric, boundary, unit, warmups) = match config.id.as_str() {
        "gtfs.sqlite.construct.first.scale001" => first(1),
        "gtfs.sqlite.construct.first.scale010" => first(10),
        "gtfs.sqlite.construct.first.scale100" => first(100),
        "gtfs.sqlite.construct.full.scale001" => full(1),
        "gtfs.sqlite.construct.full.scale010" => full(10),
        "gtfs.sqlite.construct.full.scale100" => full(100),
        "gtfs.sqlite.construct.heap.scale001" => heap(1),
        "gtfs.sqlite.construct.heap.scale010" => heap(10),
        "gtfs.sqlite.construct.heap.scale100" => heap(100),
        "gtfs.sqlite.construct.rss.scale001" => rss(1),
        "gtfs.sqlite.construct.rss.scale010" => rss(10),
        "gtfs.sqlite.construct.rss.scale100" => rss(100),
        "gtfs.sqlite.q1.full.scale001" => select(0),
        "gtfs.sqlite.q2.full.scale001" => select(1),
        "gtfs.sqlite.q3.full.scale001" => select(2),
        "gtfs.sqlite.q4.full.scale001" => select(3),
        "gtfs.sqlite.q5.full.scale001" => select(4),
        _ => {
            return Err(WorkloadError(format!(
                "unknown fixed M0 scenario: {}",
                config.id
            )))
        }
    };
    if config.scale != scale
        || config.metric != metric
        || config.boundary != boundary
        || config.unit != unit
        || config.warmup_count != warmups
        || config.sample_count != M0_SAMPLE_COUNT
    {
        return Err(WorkloadError(format!(
            "scenario {} does not match the fixed M0 workload",
            config.id
        )));
    }
    Ok(kind)
}

type Expected = (WorkloadKind, u32, MetricId, BoundaryId, Unit, u16);

fn first(scale: u32) -> Expected {
    (
        WorkloadKind::ConstructFirst,
        scale,
        MetricId::LatencyFirstResult,
        BoundaryId::SqliteExecuteToFirstTriple,
        Unit::Nanoseconds,
        3,
    )
}

fn full(scale: u32) -> Expected {
    (
        WorkloadKind::ConstructFull,
        scale,
        MetricId::LatencyFullResult,
        BoundaryId::SqliteParseTranslateExecuteDiscard,
        Unit::Nanoseconds,
        3,
    )
}

fn heap(scale: u32) -> Expected {
    (
        WorkloadKind::ConstructHeap,
        scale,
        MetricId::HeapRustRequestedLiveDelta,
        BoundaryId::SqliteParseTranslateExecuteDiscard,
        Unit::Bytes,
        0,
    )
}

fn rss(scale: u32) -> Expected {
    (
        WorkloadKind::ConstructRss,
        scale,
        MetricId::RssLinuxProcessPeak,
        BoundaryId::LinuxFreshProcessLifetime,
        Unit::Bytes,
        0,
    )
}

fn select(index: usize) -> Expected {
    (
        WorkloadKind::Select(index),
        1,
        MetricId::LatencyFullResult,
        BoundaryId::SqliteParseTranslateExecuteCollect,
        Unit::Nanoseconds,
        3,
    )
}

fn append_blob(output: &mut Vec<u8>, label: &str, bytes: &[u8]) {
    output.extend_from_slice(format!("blob\t{label}\t{}\n", bytes.len()).as_bytes());
    output.extend_from_slice(bytes);
    output.push(b'\n');
}

fn rss_source_path(run_directory: &std::path::Path, config: &ScenarioConfig) -> PathBuf {
    run_directory.join(format!("{}.source.sqlite", config.id))
}

fn create_scratch_file(path: &std::path::Path) -> Result<(), WorkloadError> {
    let metadata = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|file| file.metadata())
        .map_err(|error| WorkloadError(format!("create {}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(WorkloadError(
            "scratch database is not a regular file".into(),
        ));
    }
    Ok(())
}

fn validate_existing_source(path: &std::path::Path) -> Result<(), WorkloadError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| WorkloadError(format!("inspect {}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkloadError(
            "prepared RSS source is not a regular non-symlink file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(WorkloadError("prepared RSS source is a hard link".into()));
        }
    }
    Ok(())
}

fn require_nonzero(value: u64, label: &str) -> Result<(), WorkloadError> {
    if value == 0 {
        Err(WorkloadError(format!("{label} produced no results")))
    } else {
        Ok(())
    }
}

fn duration_ns(value: u128) -> Result<u64, WorkloadError> {
    u64::try_from(value).map_err(|_| WorkloadError("duration does not fit u64 nanoseconds".into()))
}

fn workload_error(error: impl fmt::Display) -> WorkloadError {
    WorkloadError(error.to_string())
}
