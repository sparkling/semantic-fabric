//! `semantic-fabric` — the single-binary CLI (ADR-0006). Subcommands:
//! `serve` · `conformance` · `bench`. `conformance` runs the real W3C RDB2RDF
//! harness (ADR-0005) and `bench` runs the GTFS-Madrid OBDA driver
//! (ADR-0005/0006); `serve` is a later-wave scaffold (the SPARQL 1.2 Protocol
//! endpoint, ADR-0019 G8) that reports its not-yet-implemented status.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{Parser, Subcommand};
use sf_bench::{run_obda_scenario, Scenario};
use sf_conformance::{run_and_report, Kind};

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
    /// Serve a live SPARQL endpoint over an RDBMS (OBDA virtualiser; ADR-0003).
    Serve,
    /// Run the W3C RDB2RDF conformance suite (ADR-0005).
    Conformance,
    /// Run GTFS-Madrid OBDA benchmarks (ADR-0005).
    Bench,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Conformance => conformance(),
        Command::Serve => not_implemented("serve / OBDA virtualiser", "ADR-0003/0007"),
        Command::Bench => bench(),
    }
}

fn not_implemented(what: &str, adr: &str) -> ExitCode {
    eprintln!("semantic-fabric: {what} is not yet implemented (scaffold; {adr}).");
    ExitCode::from(2)
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
