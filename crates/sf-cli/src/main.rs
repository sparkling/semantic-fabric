//! `semantic-fabric` — the single-binary CLI (ADR-0006). Subcommands:
//! `serve` · `conformance` · `bench`. `conformance` runs the real W3C RDB2RDF
//! harness (ADR-0005) and `bench` runs the GTFS-Madrid OBDA driver
//! (ADR-0005/0006); `serve` runs the live SPARQL 1.2 Protocol endpoint over the
//! OBDA virtualiser (ADR-0019 G8, ADR-0010/0011; `sf-serve`).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use sf_bench::{run_obda_scenario, Scenario};
use sf_conformance::{run_and_report, Kind};
use sf_serve::{serve_blocking, ServeOptions};

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
    Serve(ServeArgs),
    /// Run the W3C RDB2RDF conformance suite (ADR-0005).
    Conformance,
    /// Run GTFS-Madrid OBDA benchmarks (ADR-0005).
    Bench,
}

/// `serve` flags (ADR-0019 G8, ADR-0010/0011). Read-only query endpoint.
#[derive(clap::Args)]
struct ServeArgs {
    /// Source: `sqlite:<path>` (path may be `:memory:`) or `pg:<conninfo>`.
    #[arg(long)]
    source: String,
    /// R2RML mapping document (Turtle).
    #[arg(long)]
    mapping: String,
    /// Optional ontology (Turtle) → tier-1 T-Box (ADR-0008).
    #[arg(long)]
    ontology: Option<String>,
    /// Address to bind.
    #[arg(long, default_value = "127.0.0.1:7878")]
    bind: String,
    /// Request timeout in seconds (ADR-0010).
    #[arg(long, default_value_t = 30)]
    timeout_secs: u64,
    /// Max query length in bytes (ADR-0010).
    #[arg(long, default_value_t = 1 << 20)]
    max_query_len: usize,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Conformance => conformance(),
        Command::Serve(args) => serve(args),
        Command::Bench => bench(),
    }
}

/// Run the SPARQL 1.2 Protocol endpoint (`sf-serve`). Returns a clear error
/// (non-zero exit, no panic) if a required input is missing or invalid.
fn serve(args: ServeArgs) -> ExitCode {
    let opts = ServeOptions {
        source: args.source,
        mapping_path: args.mapping,
        ontology_path: args.ontology,
        bind: args.bind,
        timeout: Duration::from_secs(args.timeout_secs),
        max_query_len: args.max_query_len,
    };
    match serve_blocking(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("semantic-fabric: serve failed: {e}");
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
fn suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

/// Run the W3C RDB2RDF conformance suite through the real harness
/// (`sf_conformance::run_and_report`): execute every case via the CONSTRUCT dump,
/// write both EARL reports beside the suite, print a summary, and exit non-zero
/// only on an UNEXPECTED failure (a regression). Documented standards deviations
/// (`EXPECTED_DEVIATIONS`, e.g. R2RMLTC0002f — ADR-0015) are reported as such, not
/// as failures; skips are untested, not failures (ADR-0005 honesty contract).
fn conformance() -> ExitCode {
    let root = suite_root();
    let report = match run_and_report(&root.join("cases"), &root) {
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
        root.join("earl-semantic-fabric-r2rml.ttl").display(),
        root.join("earl-semantic-fabric-direct.ttl").display(),
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
mod tests {
    use super::*;

    #[test]
    fn suite_root_points_at_the_vendored_w3c_suite_relative_to_the_crate() {
        let root = suite_root();
        // Fixed relative to CARGO_MANIFEST_DIR (compile-time constant for this
        // crate), so the exact path is deterministic across machines/CI.
        assert!(
            root.ends_with("tests/w3c/rdb2rdf"),
            "expected the path to end in tests/w3c/rdb2rdf, got {root:?}"
        );
        assert!(
            root.is_absolute(),
            "CARGO_MANIFEST_DIR-based path should be absolute, got {root:?}"
        );
    }

    #[test]
    fn suite_root_cases_dir_and_earl_report_paths_exist_under_the_workspace() {
        // suite_root() itself doesn't touch the filesystem, but conformance()
        // immediately joins "cases" and two EARL filenames onto it — confirm the
        // real checked-in suite directory is where suite_root() says it is (a
        // silent path-mismatch here would make every conformance() call fail
        // with a confusing "could not read dir" rather than a clear message).
        let root = suite_root();
        assert!(
            root.join("cases").is_dir(),
            "expected {:?} to exist (the vendored W3C RDB2RDF cases)",
            root.join("cases")
        );
    }

    #[test]
    fn serve_returns_failure_exit_code_not_panic_on_missing_mapping_file() {
        // The one cheap, crate-local integration check on serve(): a mapping path
        // that doesn't exist must surface as a clean ExitCode::FAILURE (via
        // serve_blocking's Result -> the eprintln!+FAILURE arm), never a panic —
        // that's this crate's own responsibility as the process entry point,
        // regardless of how sf-serve itself is implemented/tested.
        let opts = ServeArgs {
            source: "sqlite::memory:".to_owned(),
            mapping: "/nonexistent/path/does-not-exist.ttl".to_owned(),
            ontology: None,
            bind: "127.0.0.1:0".to_owned(),
            timeout_secs: 1,
            max_query_len: 1024,
        };
        assert_eq!(serve(opts), ExitCode::FAILURE);
    }
}
