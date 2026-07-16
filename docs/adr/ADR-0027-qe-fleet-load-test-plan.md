---
status: proposed
date: 2026-07-16
ratified:
tags: [testing, load-testing, concurrency, resource-governance, sf-serve, agentic-qe, adr-drift]
supersedes: []
depends-on:
  - ADR-0005
  - ADR-0010
  - ADR-0026
implements: ADR-0012
---

# QE-fleet load testing for `sf-serve` — a concurrency axis ADR-0005 doesn't cover

## Context and Problem Statement

`ADR-0026` considered `agentic-qe`'s `qe_tests_load` tool for load testing and
explicitly declined it, reasoning it would duplicate `ADR-0005`'s existing
Ontop-comparison harness. On investigation (this ADR), that reasoning was too
broad: `ADR-0005`'s benchmark scripts (`scripts/compare/race.sh`,
`footprint.sh`, `materialise.sh`) measure exactly one client at a time —
sequential `curl` calls, median-of-N, one query in flight — and say so in their
own comments (`race.sh`: "the FAIR head-to-head... curl %{time_total}
median-of-N"). **None of them open concurrent connections.** That leaves a real,
currently-untested axis: what `sf-serve` actually does under *concurrent*
multi-client load, which is precisely what `ADR-0010`'s admission-control and
resource-governance claims are about.

### What investigating the existing code found (real drift, not assumed)

Reading `crates/sf-serve/src/lib.rs`/`run.rs` directly against `ADR-0010`'s
claims:

* **Implemented and real:** per-request `timeout` (`DEFAULT_TIMEOUT` = 30s,
  enforced via `tokio::time::timeout` → `504 GATEWAY_TIMEOUT`) and
  `max_query_len` (`DEFAULT_MAX_QUERY_LEN` = 1 MiB, enforced, rejects oversized
  queries before execution).
* **Not implemented, despite `ADR-0010` describing it as decided:** the
  "stream-lane connection pool" and "shed overflow as HTTP `503` +
  `Retry-After` rather than queue" design. `grep` for `stream_lane`,
  `Retry-After`, `503`, `SERVICE_UNAVAILABLE` across `sf-serve`'s source and
  tests returns **zero matches**. PostgreSQL is served over a single `Client`
  (not a pool) per prior performance-analysis findings; only the MySQL path
  has a real `mysql_async::Pool`. `ADR-0010` is marked `accepted`, but this
  specific clause was never built — the plan and the code disagree.

This is exactly the kind of gap a load test would make visible in practice
(concurrent requests beyond what the single PG connection can serve have no
documented, tested fallback), rather than leaving it as an unverified paper
claim.

## Decision Drivers

* Don't duplicate `ADR-0005`'s sequential single-client latency/footprint
  charter — this is additive, a different axis (concurrency), not a
  replacement.
* Test what's actually implemented (`timeout`, `max_query_len`) under
  concurrent pressure, and make the stream-lane-pool/503 gap's real status
  (undecided in practice, not just "not yet load-tested") visible rather than
  silently assumed to work.
* Reuse the already-installed `agentic-qe` fleet (`ADR-0026`) rather than
  hand-rolling a load-test harness.

## Considered Options

* **Do nothing / assume `ADR-0010`'s concurrency claims hold** — rejected: the
  code-reading above shows they don't fully hold today; shipping on an
  unverified assumption here is exactly the risk `ADR-0010` exists to prevent.
* **Extend `scripts/compare/race.sh` to fire concurrent `curl`s** — rejected:
  loses `qe_tests_load`'s bottleneck reporting and pass/fail criteria for free;
  would hand-roll what the already-installed tool already does.
* **`agentic-qe`'s `qe_tests_load` against a live local `sf-serve` instance
  (chosen)** — real fleet agents simulate concurrent clients (light/medium/
  heavy workload profiles), reports bottlenecks and pass/fail against
  configurable criteria; `mockMode=false` drives it against the real running
  binary, not synthetic mock agents.

## Decision Outcome (proposed — not yet executed)

Run `qe_tests_load` against a locally-launched `sf-serve` (SQLite or PostgreSQL
source, GTFS fixture matching `ADR-0005`'s existing dataset so results are
comparable in kind, not just novel) with three target scenarios:

1. **Timeout holds under concurrency**: N concurrent slow/long-running queries
   (e.g. a deliberately expensive join) each still resolve via `504` at
   `cfg.timeout`, not later, and don't extend each other's deadlines.
2. **`max_query_len` holds under concurrency**: N concurrent oversized-query
   requests are all rejected pre-execution, not just the first.
3. **Overload behavior above what the backend can serve** (the real open
   question): drive concurrency past what a single PG `Client`/SQLite
   connection can serve and observe what actually happens today — queuing,
   an unbounded wait, a panic, or a real (if undocumented) rejection. Report
   the finding plainly; this is the test whose *outcome* decides whether
   `ADR-0010`'s stream-lane-pool clause needs to move from "accepted" to
   "accepted, not yet implemented" (a status correction) or gets scheduled as
   real follow-up work.

Status left `proposed`, not `accepted`: this ADR plans the test; it does not
yet report a result. Update this ADR's status to `accepted` with the measured
findings once run, per this project's own "ADRs are living plans" discipline —
do not let it sit here describing an unexecuted intention.

## Consequences

### What gets easier
* A real, verified answer to whether `sf-serve` degrades gracefully under
  concurrent load, instead of an untested assumption riding on `ADR-0010`'s
  accepted status.
* Reuses `ADR-0026`'s already-installed fleet; no new tooling to stand up.

### What gets harder / risk
* If scenario 3 finds a real gap (likely, given no stream-lane pool exists),
  that's new, unscheduled work: either implement the pool/503-shedding
  `ADR-0010` already describes, or formally descope it with a status update
  on `ADR-0010` — a product/priority decision, not this ADR's to make alone.
* Load testing a live local server needs the same kind of real-infra
  verification discipline `ADR-0026` used for the DB containers (run it for
  real, don't trust a dry-run).

## More Information

* Related: `ADR-0005` (the sequential single-client benchmark this is
  additive to, never duplicative of), `ADR-0010` (the governance claims under
  test — one clause found not implemented), `ADR-0026` (the `agentic-qe`
  install this reuses).
* Source read for this ADR: `crates/sf-serve/src/lib.rs`, `run.rs`,
  `scripts/compare/race.sh`, `footprint.sh`, `materialise.sh`, `BENCHMARKS.md`.
