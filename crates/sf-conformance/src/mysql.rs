//! Required-live MySQL execution for the sealed W3C RDB2RDF suite.
//!
//! Inventory validation and byte capture happen before configuration or provider
//! access. Each run owns one randomly named scratch database; every case starts
//! from an empty schema and is adjudicated in canonical inventory order. This is
//! execution evidence only and grants no production or admission status.

use std::collections::BTreeSet;
use std::env::VarError;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder};

use crate::sealed_suite::{ClassifiedReport, SealedSuite};
use crate::Report;

mod case;
mod cleanup;
mod outcome;
mod query;
mod sql_split;
#[cfg(test)]
mod tests;

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);
const SQL_MODE: &str = "ANSI_QUOTES,PIPES_AS_CONCAT,PAD_CHAR_TO_FULL_LENGTH,STRICT_TRANS_TABLES,ERROR_FOR_DIVISION_BY_ZERO,NO_ENGINE_SUBSTITUTION";
const SQL_MODES: [&str; 6] = [
    "ANSI_QUOTES",
    "ERROR_FOR_DIVISION_BY_ZERO",
    "NO_ENGINE_SUBSTITUTION",
    "PAD_CHAR_TO_FULL_LENGTH",
    "PIPES_AS_CONCAT",
    "STRICT_TRANS_TABLES",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveMode {
    LocalOptional,
    CiRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UntestedReason {
    ProviderUnavailable { detail: String },
}

#[derive(Debug)]
pub enum LiveRun<T> {
    Tested(T),
    Untested(UntestedReason),
}

pub type SuiteRun = LiveRun<Report>;
pub type ClassifiedSuiteRun = LiveRun<ClassifiedReport>;

/// Validate and seal the complete suite before reading connection configuration.
pub fn run(suite_root: &Path, mode: LiveMode) -> Result<SuiteRun, String> {
    let sealed = SealedSuite::load(suite_root)?;
    match run_sealed_suite(&sealed, mode)? {
        LiveRun::Tested(report) => Ok(LiveRun::Tested(outcome::plain_report(report))),
        LiveRun::Untested(reason) => Ok(LiveRun::Untested(reason)),
    }
}

/// Execute an already sealed suite against one live MySQL provider.
pub fn run_sealed_suite(
    sealed: &SealedSuite,
    mode: LiveMode,
) -> Result<ClassifiedSuiteRun, String> {
    let Some(opts) = base_opts()? else {
        return unavailable(mode, "MySQL provider is not configured");
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "create MySQL conformance runtime failed".to_owned())?;
    rt.block_on(run_async(sealed, opts, mode))
}

/// Receipt boundary: provider absence can never become passing or untested data.
pub fn run_sealed_suite_required(sealed: &SealedSuite) -> Result<ClassifiedReport, String> {
    match run_sealed_suite(sealed, LiveMode::CiRequired)? {
        LiveRun::Tested(report) => Ok(report),
        LiveRun::Untested(_) => Err("required MySQL run returned untested evidence".to_owned()),
    }
}

async fn run_async(
    sealed: &SealedSuite,
    opts: Opts,
    mode: LiveMode,
) -> Result<ClassifiedSuiteRun, String> {
    let mut admin = match Conn::new(opts.clone()).await {
        Ok(conn) => conn,
        Err(_) => return unavailable(mode, "MySQL connection failed"),
    };
    let database = scratch_database_name()?;
    admin
        .query_drop(format!(
            "CREATE DATABASE `{database}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin"
        ))
        .await
        .map_err(|_| "create MySQL scratch database failed".to_owned())?;
    let scratch = cleanup::ScratchDatabase::new(admin, opts.clone(), database.clone());

    let work_opts: Opts = OptsBuilder::from_opts(opts)
        .db_name(Some(database.as_str()))
        .into();
    let report = match Conn::new(work_opts).await {
        Ok(mut work) => {
            let result = match configure_session(&mut work).await {
                Ok(()) => async_run_cases(sealed, &mut work).await,
                Err(error) => Err(error),
            };
            let closed = work
                .disconnect()
                .await
                .map_err(|_| "close MySQL work connection failed".to_owned());
            prefer_cleanup_error(result, closed)
        }
        Err(_) => Err("connect to MySQL scratch database failed".to_owned()),
    };

    let cleanup = scratch.cleanup().await;
    prefer_cleanup_error(report, cleanup).map(LiveRun::Tested)
}

fn prefer_cleanup_error<T>(
    operation: Result<T, String>,
    cleanup: Result<(), String>,
) -> Result<T, String> {
    match cleanup {
        Err(error) => Err(error),
        Ok(()) => operation,
    }
}

async fn async_run_cases(
    sealed: &SealedSuite,
    conn: &mut Conn,
) -> Result<ClassifiedReport, String> {
    case::run_cases(sealed, conn).await
}

async fn configure_session(conn: &mut Conn) -> Result<(), String> {
    conn.exec_drop("SET SESSION sql_mode = ?", (SQL_MODE,))
        .await
        .map_err(|_| "configure MySQL conformance SQL mode failed".to_owned())?;
    conn.query_drop("SET NAMES utf8mb4 COLLATE utf8mb4_bin")
        .await
        .map_err(|_| "configure MySQL conformance character set failed".to_owned())?;
    conn.query_drop("SET SESSION time_zone = '+00:00'")
        .await
        .map_err(|_| "configure MySQL conformance time zone failed".to_owned())?;
    let observed: Option<(String, String, String, String)> = conn
        .query_first(
            "SELECT @@SESSION.sql_mode, @@character_set_connection, \
                    @@collation_connection, @@SESSION.time_zone",
        )
        .await
        .map_err(|_| "verify MySQL conformance session failed".to_owned())?;
    let Some((modes, charset, collation, time_zone)) = observed else {
        return Err("verify MySQL conformance session returned no row".to_owned());
    };
    let actual: BTreeSet<_> = modes.split(',').collect();
    let expected: BTreeSet<_> = SQL_MODES.into_iter().collect();
    if actual != expected
        || charset != "utf8mb4"
        || collation != "utf8mb4_bin"
        || time_zone != "+00:00"
    {
        return Err("MySQL conformance session differs from the sealed profile".to_owned());
    }
    Ok(())
}

fn base_opts() -> Result<Option<Opts>, String> {
    let socket = environment("SF_MYSQL_SOCKET")?;
    let url = environment("SF_MYSQL_URL")?;
    let user = if socket.is_some() {
        environment("SF_MYSQL_USER")?
    } else {
        None
    };
    let password = if socket.is_some() {
        environment("SF_MYSQL_PASSWORD")?
    } else {
        None
    };
    connection_opts(socket.as_deref(), url.as_deref(), user, password)
}

fn connection_opts(
    socket: Option<&str>,
    url: Option<&str>,
    user: Option<String>,
    password: Option<String>,
) -> Result<Option<Opts>, String> {
    if socket.is_some() && url.is_some() {
        return Err("configure exactly one of SF_MYSQL_SOCKET or SF_MYSQL_URL".to_owned());
    }
    let opts = match (socket, url) {
        (Some(socket), None) => socket_opts(socket, user, password)?,
        (None, Some(url)) => url_opts(url)?,
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => unreachable!("mutual exclusion checked above"),
    };
    Ok(Some(
        OptsBuilder::from_opts(opts)
            .db_name(Some("mysql"))
            .stmt_cache_size(Some(0))
            .into(),
    ))
}

fn environment(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err("invalid MySQL connection configuration".to_owned()),
    }
}

fn url_opts(value: &str) -> Result<Opts, String> {
    if value.len() > 4_096 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("invalid MySQL connection configuration".to_owned());
    }
    let opts =
        Opts::from_url(value).map_err(|_| "invalid MySQL connection configuration".to_owned())?;
    Ok(OptsBuilder::from_opts(opts)
        .prefer_socket(Some(false))
        .into())
}

fn socket_opts(
    socket: &str,
    user: Option<String>,
    password: Option<String>,
) -> Result<Opts, String> {
    if socket.is_empty()
        || socket.len() > 100
        || !Path::new(socket).is_absolute()
        || socket.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err("invalid MySQL socket configuration".to_owned());
    }
    let user = user.unwrap_or_else(|| "root".to_owned());
    if user.is_empty()
        || user.len() > 128
        || user.bytes().any(|byte| byte.is_ascii_control())
        || password
            .as_deref()
            .is_some_and(|value| value.len() > 1_024 || value.contains('\0'))
    {
        return Err("invalid MySQL connection configuration".to_owned());
    }
    Ok(OptsBuilder::default()
        .user(Some(user))
        .pass(password)
        .db_name(Some("mysql"))
        .socket(Some(socket))
        .prefer_socket(Some(true))
        .stmt_cache_size(Some(0))
        .into())
}

fn scratch_database_name() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_owned())?
        .as_nanos();
    let serial = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "sf_conf_{:x}_{:016x}_{serial:x}",
        std::process::id(),
        nanos as u64
    ))
}

fn unavailable<T>(mode: LiveMode, detail: &'static str) -> Result<LiveRun<T>, String> {
    match mode {
        LiveMode::LocalOptional => Ok(LiveRun::Untested(UntestedReason::ProviderUnavailable {
            detail: detail.to_owned(),
        })),
        LiveMode::CiRequired => Err(format!("required MySQL provider is unavailable: {detail}")),
    }
}
