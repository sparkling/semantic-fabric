//! `semantic-fabric` — the single-binary CLI (ADR-0006). Subcommands:
//! `serve` · `conformance` · `bench`. `conformance` runs the real W3C RDB2RDF
//! harness (ADR-0005) and `bench` runs the GTFS-Madrid OBDA driver
//! (ADR-0005/0006); `serve` runs the live SPARQL 1.2 Protocol endpoint over the
//! OBDA virtualiser (ADR-0019 G8, ADR-0010/0011; `sf-serve`).

#[cfg(feature = "development-tools")]
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
#[cfg(feature = "development-tools")]
use std::time::Instant;

use clap::{Parser, Subcommand};
#[cfg(feature = "development-tools")]
use sf_bench::{run_obda_scenario, Scenario};
#[cfg(feature = "development-tools")]
use sf_conformance::{run_and_report, Kind};
use sf_serve::{serve_blocking, ServeOptions};
#[cfg(test)]
use sf_serve::{
    DEFAULT_MAX_CONCURRENT_REQUESTS, DEFAULT_MAX_ORDER_BYTES, DEFAULT_MAX_ORDER_ROWS,
    DEFAULT_SHUTDOWN_TIMEOUT,
};

mod layered_config;
mod metrics;
mod serve_args;
mod telemetry;

use serve_args::ServeArgs;
#[cfg(test)]
use serve_args::{
    AdditionalMappingSelector, AdditionalSourceArgs, AdditionalSourceSelector, MappingArgs,
    SourceArgs,
};
use telemetry::TelemetryLevel;

#[derive(Parser)]
#[command(
    name = "semantic-fabric",
    version,
    about = "RDBMS data fabric: SPARQL/OBDA virtualization (ADR-0001)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the live SPARQL 1.2 Protocol endpoint over an RDBMS (ADR-0019 G8).
    Serve(Box<ServeArgs>),
    /// Run the W3C RDB2RDF conformance suite (ADR-0005).
    #[cfg(feature = "development-tools")]
    Conformance,
    /// Run GTFS-Madrid OBDA benchmarks (ADR-0005).
    #[cfg(feature = "development-tools")]
    Bench,
}

#[cfg(test)]
impl Command {
    fn into_serve(self) -> Box<ServeArgs> {
        match self {
            Self::Serve(args) => args,
            #[cfg(feature = "development-tools")]
            _ => panic!("serve command"),
        }
    }
}

fn main() -> ExitCode {
    sf_sparql::dispatch_private_parser_worker_v1();
    let argv = match layered_config::expand(std::env::args_os().collect()) {
        Ok(argv) => argv,
        Err(error) => {
            eprintln!("semantic-fabric: {error}");
            return ExitCode::FAILURE;
        }
    };
    let command = Cli::parse_from(argv).command;
    if let Err(error) = initialize_telemetry_for(&command, telemetry::init) {
        eprintln!("semantic-fabric: {error}");
        return ExitCode::FAILURE;
    }
    match command {
        #[cfg(feature = "development-tools")]
        Command::Conformance => conformance(),
        Command::Serve(args) => serve(*args),
        #[cfg(feature = "development-tools")]
        Command::Bench => bench(),
    }
}

fn initialize_telemetry_for(
    command: &Command,
    initialize: impl FnOnce(TelemetryLevel) -> Result<(), telemetry::InitError>,
) -> Result<(), telemetry::InitError> {
    match command {
        Command::Serve(args) => initialize(args.log_level),
        #[cfg(feature = "development-tools")]
        Command::Conformance | Command::Bench => Ok(()),
    }
}

/// Run the SPARQL 1.2 Protocol endpoint (`sf-serve`). Returns a clear error
/// (non-zero exit, no panic) if a required input is missing or invalid.
fn serve(args: ServeArgs) -> ExitCode {
    let query_admission = if let Some(name) = args.auth_subjects_env.as_deref() {
        match sf_serve::ProvisionedBearerAdmission::from_env(name) {
            Ok(profile) => sf_serve::QueryAdmission::ProvisionedBearers(profile),
            Err(error) => {
                error.record_telemetry();
                return ExitCode::FAILURE;
            }
        }
    } else {
        match args.auth_token_env.as_deref() {
            Some(name) => {
                match sf_serve::BearerQueryAdmission::from_env(name).and_then(|profile| match args
                    .pg_rls_context_env
                    .as_deref()
                {
                    Some(name) => {
                        profile.with_postgres_rls(sf_serve::PostgresRlsClaims::from_env(name)?)
                    }
                    None => Ok(profile),
                }) {
                    Ok(profile) => sf_serve::QueryAdmission::Bearer(profile),
                    Err(error) => {
                        error.record_telemetry();
                        return ExitCode::FAILURE;
                    }
                }
            }
            None if args.allow_unauthenticated => sf_serve::QueryAdmission::UnrestrictedDevelopment,
            None => sf_serve::QueryAdmission::Deny,
        }
    };
    let metrics = match metrics::init(args.metrics) {
        Ok(metrics) => metrics,
        Err(error) => {
            eprintln!("semantic-fabric: {error}");
            return ExitCode::FAILURE;
        }
    };
    let source = args.source_input.into_source_ref();
    let source = match args.source_tls_roots_env {
        Some(name) => source.with_tls_roots_env(name),
        None => source,
    };
    let mapping = args.mapping_input.into_mapping_ref();
    let additional_source = args
        .additional_source_input
        .into_options()
        .map(|mut additional| {
            if let Some(name) = args.source_tls_roots_env_2 {
                additional.source = additional.source.with_tls_roots_env(name);
            }
            additional
        });
    let opts = ServeOptions {
        query_admission,
        source,
        mapping,
        additional_source,
        ontology_path: args.ontology,
        bind: args.bind,
        timeout: Duration::from_secs(args.timeout_secs),
        max_query_len: args.max_query_len,
        max_concurrent_requests: args.max_concurrent_requests,
        max_compiler_work: args.max_compiler_work,
        max_source_work: args.max_source_work,
        max_result_items: args.max_result_items,
        max_order_rows: args.max_order_rows,
        max_order_bytes: args.max_order_bytes,
        max_serialized_bytes: args.max_serialized_bytes,
        pg_pool_size: args.pg_pool_size,
        pg_pool_wait: Duration::from_secs(args.pg_pool_wait_secs),
        sqlite_pool_size: args.sqlite_pool_size,
        shutdown_timeout: Duration::from_secs(args.shutdown_timeout_secs),
        reload_interval: Duration::from_secs(args.reload_interval_secs),
        require_verified_generation: args.require_verified_generation,
        metrics,
    };
    match serve_blocking(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            e.record_telemetry();
            ExitCode::FAILURE
        }
    }
}

/// Run the GTFS-Madrid OBDA benchmark driver (ADR-0005/0006): generate the
/// file-backed source at a couple of scale factors and execute every
/// representative query plus the streaming CONSTRUCT through the live virtualiser
/// (no materialisation). Prints wall-clock per scale; the quantitative per-query
/// latency and the constant-memory demonstration live in the `criterion` benches
/// and the `constant_memory` test (pointers below).
#[cfg(feature = "development-tools")]
fn bench() -> ExitCode {
    println!("=== GTFS-Madrid OBDA benchmark (live SPARQL->SQL over SQLite; ADR-0005/0006) ===");
    for scale in [1u32, 4] {
        let scenario = Scenario::new(format!("gtfs-madrid-{scale}x"), scale);
        let t = Instant::now();
        if let Err(e) = run_obda_scenario(&scenario) {
            eprintln!("semantic-fabric: bench scenario {scale}x failed: {e}");
            return ExitCode::FAILURE;
        }
        println!(
            "  {scale:>3}x  all queries + streaming CONSTRUCT   {:?}",
            t.elapsed()
        );
    }
    println!(
        "\nFull numbers:\n  \
         per-query latency:           cargo bench -p sf-bench --bench obda_latency\n  \
         constant-memory (ADR-0006):  cargo test -p sf-bench --test constant_memory -- --nocapture"
    );
    ExitCode::SUCCESS
}

/// The vendored W3C RDB2RDF suite root, fixed relative to the workspace; the same
/// location the harness test drives (ADR-0005). `cases/` holds the `D###`
/// scenarios; the EARL reports are written here beside the suite.
#[cfg(feature = "development-tools")]
fn suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

/// Run the W3C RDB2RDF conformance suite through the real harness
/// (`sf_conformance::run_and_report`): execute every case via the CONSTRUCT dump,
/// write both EARL reports beside the suite, print a summary, and exit non-zero
/// only on an UNEXPECTED failure (a regression). Documented standards deviations
/// (`EXPECTED_DEVIATIONS`, e.g. R2RMLTC0002f — ADR-0015) are reported as such, not
/// as failures; skips are untested, not failures (ADR-0005 honesty contract).
#[cfg(feature = "development-tools")]
fn conformance() -> ExitCode {
    let root = suite_root();
    conformance_to(&root)
}

#[cfg(feature = "development-tools")]
fn conformance_to(out_dir: &Path) -> ExitCode {
    let root = suite_root();
    let report = match run_and_report(&root, out_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("semantic-fabric: conformance suite failed to run: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("=== W3C RDB2RDF conformance (CONSTRUCT dump, SQLite; ADR-0005) ===");
    println!("R2RML          {}", report.split(Kind::R2rml));
    println!("Direct Mapping {}", report.split(Kind::DirectMapping));
    let deviations = report.expected_deviations();
    let unexpected = report.unexpected_failures();
    println!(
        "overall passed={} adjudicated={} skipped(501/fixture)={} documented-deviations={}",
        report.passed(None),
        report.adjudicated(None),
        report.skipped(None),
        deviations.len(),
    );

    if !deviations.is_empty() {
        println!("\n--- documented standards deviations (not failures; ADR-0015) ---");
        for d in &deviations {
            println!("  DEVIATION {d}");
        }
    }
    if !unexpected.is_empty() {
        println!("\n--- UNEXPECTED failures (regressions) ---");
        for f in &unexpected {
            println!("  FAIL {f}");
        }
    }

    println!(
        "\nEARL written:\n  {}\n  {}",
        out_dir.join("earl-semantic-fabric-r2rml.ttl").display(),
        out_dir.join("earl-semantic-fabric-direct.ttl").display(),
    );

    if unexpected.is_empty() {
        println!(
            "\nPASS — {} adjudicated, 0 unexpected failures ({} documented deviation(s)).",
            report.adjudicated(None),
            deviations.len(),
        );
        ExitCode::SUCCESS
    } else {
        println!("\nFAIL — {} unexpected failure(s).", unexpected.len());
        ExitCode::FAILURE
    }
}

/// `sf-cli` had zero tests at any level. `serve`/`bench`/`conformance` mostly
/// dispatch into other crates (`sf-serve`/`sf-bench`/`sf-conformance`), which own
/// their own coverage — re-testing their internals here would duplicate, not add,
/// coverage. What genuinely belongs at THIS layer: `suite_root()`'s own
/// path-building logic, and that the dispatch functions surface a clean non-zero
/// exit (never panic) on bad input, since that's this crate's own responsibility
/// as the process entry point.
#[cfg(test)]
mod main_tests;
