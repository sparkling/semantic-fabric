---
status: accepted
date: 2026-07-16
ratified: 2026-07-16
tags: [testing, coverage, quality-engineering, ci, agentic-qe, sql-server, correctness]
supersedes: []
depends-on:
  - ADR-0005
  - ADR-0012
  - ADR-0024
implements: ADR-0012
---

# Adopting the `agentic-qe` fleet for coverage-gap analysis, and what it found

## Context and Problem Statement

ADR-0012 sets this project's test-strategy charter but has no standing tool for
*finding* what to test next beyond manual audits (see the prior
`test-coverage-audit-2026-07-07` line of work). A real coverage-gap analysis pass
was wanted, backed by actual `cargo llvm-cov` data rather than manual grep-driven
review, plus a path to close whatever it found.

Note: the `ADR-0025`/`ADR-0026` numbers were previously used for a since-abandoned
Claude-only GEPA harness-evolution track (`tools/gepa-loop/`, branch
`feat/adr-0026-gepa-loop`) that was marked obsolete on 2026-07-07 and never merged
to `main`. Neither numbered ADR file for that track exists in this tree; `0025` was
independently reused for the ontop-parity-residue-closure ADR, and `0026` is reused
here. This is noted so a reader of old handover docs referencing those numbers
isn't misled into thinking this ADR relates to harness-evolution — it does not.

## Decision Drivers

* Need real, `cargo llvm-cov`-backed coverage-gap data, risk-ranked, not a manual
  line-count guess.
* Whatever tool is chosen must not duplicate `ADR-0005`'s existing charter: OBDA
  query-latency / constant-memory comparison against real Ontop (`sf-bench`,
  `scripts/run_ontop_bench.sh`, `scripts/compare/`) is **out of scope** here and
  stays fully owned by `ADR-0005`. This ADR is about *finding untested code*, not
  performance benchmarking.
* Should generalize beyond this one pass — a standing MCP-wired tool the project
  can reach for again, not a one-shot script.

## Considered Options

* Hand-rolled coverage-gap scripts over `cargo llvm-cov` output — rejected::
  reinvents risk-ranking and gap-chunking that a maintained tool already does.
* `agentic-qe` (`github.com/proffesor-for-testing/agentic-qe`) — chosen: a real,
  installable QE fleet (`aqe` CLI / `aqe-mcp` MCP server) with a coverage-gap tool
  (`qe_coverage_gaps`) that consumes real `lcov` data and risk-ranks by
  criticality/complexity.
* `@metaharness/darwin` (Darwin Mode harness-evolution) — rejected for this
  purpose: it evolves the *agent's own policy* (planner/reviewer/context-builder),
  never writes a test or touches product code; a distinct, already-explored-and-
  parked idea (see the obsoleted `ADR-0025`/`0026` harness-evolution track above).

## Decision Outcome

Installed `agentic-qe@3.12.2` globally and wired its MCP server into this repo's
`.mcp.json` (`aqe init`'s own `--with-mcp` default did not actually write the
file — wired by hand after verifying the gap). `.agentic-qe/` (local memory/
pattern DB) added to `.gitignore`, same treatment as `.swarm/`.

### What the coverage-gap pass found and closed

Starting baseline: 85 real gap-chunks (`qe_coverage_gaps`, `dataSource: "real"`).
Closed, in order, each verified against fresh `cargo llvm-cov` output (not
estimated):

1. **`sqlserver.rs::parse_conn_str`** (ADO.NET connection-string parsing) — 8 new
   unit tests, no live server needed.
2. **Live-DB CI wiring** (`.github/workflows/ci.yml`): Postgres/MySQL/SQL Server
   service containers, closing the graceful-skip integration suites
   (`differential_pg_sqlite`, `mysql_e2e`, `differential_mssql`, `mysql_release`,
   `constant_memory`'s PG/MySQL variants) that previously no-op'd in CI. Verifying
   this live (real Docker containers, not just YAML review) surfaced and fixed a
   real test-isolation bug: `constant_memory.rs`'s three tests share one
   process-wide global-allocator peak-tracker with no serialization, so the
   PG/MySQL tests doing real work (instead of instant skips) could pollute the
   SQLite test's concurrent measurement. Fixed with a poison-tolerant `Mutex`.
3. **`rest.rs`'s four cloud REST backends** (Snowflake/Athena/Databricks/Trino) —
   added `wiremock` as a `sf-sql` dev-dependency; mocked-server tests exercise the
   real request/response path (including presto's multi-page `nextUri`
   pagination and its query-error branch) without live cloud credentials.
   `BigQueryBackend` hardcodes `bigquery.googleapis.com` and is not covered this
   way; closing it needs a small production-code change (accept an endpoint
   override) — not done here, left as a documented gap.
4. **A real, previously-undetected production bug**, found only because closing
   the coverage gap meant testing code paths nobody had exercised: SQL Server's
   `date_from_proleptic()` implements Howard Hinnant's `civil_from_days`, which
   expects "days since 1970-01-01," but callers fed it "days since 0001-01-01"
   (Rata Die) unadjusted, and a second, independent constant
   (`date_from_days_1900`'s `693961`) was wrong by exactly one year on top of
   that. Every live `DATE`/`DATETIME`/`DATETIME2`/`SMALLDATETIME` value the SQL
   Server backend ever marshaled was wrong (e.g. `DATE '0001-01-01'` →
   `"1970-01-01"`). Fixed; verified two ways — independent ground-truth
   arithmetic across 6 dates (leap year + SQL Server's max-date boundary), and a
   new live round-trip test (`differential_mssql.rs`) reading real values back
   from an actual SQL Server container.
5. **`monetdb.rs`'s MAPI/TCP framing** (`read_mapi_message`/`send_mapi_message`)
   — closed via a hand-rolled local TCP mock MAPI server (plain `std::net`, no
   new dependency; the protocol is simple enough to drive directly), including a
   direct test of the multi-block message reassembly loop.
6. **`sf-cli/main.rs`'s `conformance()`** — nothing called it, so its own
   dispatch/exit-code logic (this crate's stated responsibility, per its own
   module doc) was untested. One test runs the real vendored W3C RDB2RDF suite
   (no live infra) and asserts `ExitCode::SUCCESS`.

**Final verified state**: under a full real-`lcov` pass with Postgres/MySQL/SQL
Server all live and reachable, `qe_coverage_gaps` returns **zero gap-chunks**.
Without live services (a plain local `cargo test`), the only gaps remaining are
ones that structurally require a live DB connection to exercise at all — already
proven to close the moment CI's live-DB services are up.

## Consequences

### What gets easier

* A standing, MCP-wired tool (`qe_coverage_gaps`, `test_generate_enhanced`,
  `security_scan_comprehensive`, and more) the project can reach for again
  without re-installing or re-deriving risk-ranking logic.
* CI now genuinely exercises the Postgres/MySQL/SQL Server integration paths that
  were previously silent no-ops — a real correctness gate, not an illusory one.

### What gets harder / costs incurred

* CI wall-clock and minutes: three service containers now start on every
  `build` job run.
* One new dev-dependency (`wiremock`, `sf-sql`) — test-only, no production
  runtime impact.
* `agentic-qe`'s own MCP wiring has a real, confirmed bug (`aqe init`'s
  `--with-mcp` default doesn't write `.mcp.json`) — worth re-checking on any
  future `agentic-qe` upgrade rather than assuming it's fixed upstream.

### Explicitly out of scope (do not conflate with this ADR)

* Load/performance testing and Ontop comparison — fully owned by `ADR-0005`
  (`sf-bench`, `scripts/run_ontop_bench.sh`, `scripts/compare/`). `agentic-qe`'s
  `qe_tests_load` tool was considered and initially set aside here as a
  duplicate of that harness.

  > **Correction (2026-07-16, same day).** That reasoning was too broad on
  > closer reading: `ADR-0005`'s scripts are all sequential, single-client
  > (`race.sh`/`footprint.sh` fire one `curl` at a time, median-of-N) — they
  > never test *concurrent* multi-client load, which is a genuinely different
  > axis from what they measure and is exactly what `ADR-0010`'s
  > admission-control claims are about. `qe_tests_load` is not redundant for
  > that axis. See `ADR-0027`, which also found that one of `ADR-0010`'s
  > concurrency-governance clauses (the stream-lane pool / `503` shedding) is
  > accepted in name but not actually implemented in `sf-serve`'s source.
* Security scanning (`security_scan_comprehensive`) — not yet run; a reasonable
  next step given this codebase's dynamic per-dialect SQL emission, but tracked
  as future work, not part of this ADR's decision.
* `BigQueryBackend`'s hardcoded endpoint — a real, small, deliberately-deferred
  gap (see item 3 above).

## More Information

* Upstream: `github.com/proffesor-for-testing/agentic-qe`, MCP server `aqe-mcp`.
* Commits: `e59c9f9`, `d0adcc4`, `09e5737`, `f491ee9`, `0bcd30f`, `a8bdafc`,
  `85d0c4f` (this session, in order).
* Related, explicitly not superseded: `ADR-0005` (benchmark/conformance
  charter), `ADR-0012` (test strategy), `ADR-0024` (executor backend
  abstraction — the SQL Server backend this pass found a real bug in).
