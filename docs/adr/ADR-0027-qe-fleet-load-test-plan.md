---
status: accepted
date: 2026-07-16
ratified: 2026-07-16
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

### Correction (2026-07-16, same day) — `qe_tests_load` was not actually used

On trying to invoke it, `qe_tests_load`'s own description and source
(`agentic-qe/src/domains/chaos-resilience/services/load-tester.ts`) turned out
to test **`agentic-qe`'s own agent-fleet scalability** (spawning N QE agents,
gated behind `fleet_init`), not an arbitrary HTTP target. `qe_workflows_browser-load`
is browser-automation only (login/OAuth flow templates). Neither fits "fire
concurrent requests at a plain SPARQL HTTP endpoint." No RuvNet tool covers
generic HTTP load generation. Disclosing plainly rather than forcing a
mismatched tool or silently hand-rolling: installed **`oha`** (Homebrew,
`brew install oha`, a maintained Rust HTTP load generator) plus plain `curl`
for finer per-request control, and ran the three scenarios below directly
against a locally-launched `sf-serve`.

## Measured Results (2026-07-16)

Setup: PostgreSQL 16 in Docker (port 15432, avoiding collision with a
locally-installed native Postgres), GTFS fixture at scale 3000
(`scripts/load_gtfs_postgres.sh 3000` — 2.4M `stop_times` rows), `sf-serve`
built in `--release`. Baseline: one unthrottled full `CONSTRUCT {?s ?p ?o}`
dump (15,560,014 triples) takes **12.4s** solo.

**1. Timeout under concurrency — holds, once measured correctly.** First
attempt (3 concurrent dumps, `--timeout-secs 2`, output discarded via
`-o /dev/null`) appeared to show the timeout not firing — all three eventually
returned `http=200` after 25-30s. That reading was wrong: discarding the
response body hides mid-stream truncation, since the `200` status line is
sent before the body streams and can't retroactively change. Re-run capturing
the actual body: all three requests truncated at **~2.0-2.07s** (`curl exit
18`, "partial file") — the deadline check in `stream.rs` does fire close to
the configured bound. Lesson banked: a load test must check response
completeness, not just the status code.

A genuine, unexplained-but-not-alarming sub-finding from that same run: the
three "identical" concurrent requests got very unequal throughput in the same
~2s window (two streamed ~34MB, one streamed ~370MB) — the shared
single-connection model does not guarantee fairness across concurrent
requests. Not a correctness bug (no data corruption — see scenario 3), but
worth knowing before assuming equal service under load.

**2. `max_query_len` under concurrency — holds cleanly.** 10 concurrent
oversized-query (`--max-query-len 1024`) requests: all 10 correctly rejected
with `413` and the ADR-0010 cap message. No degradation under concurrency.

**3. Overload (5-way, then 20-way concurrent full-dump CONSTRUCT, 30s
timeout) — no crash, no hang, no data corruption; degrades via proportional
slowdown + the existing timeout, not via any admission control.** All 25
requests (5 + 20) returned clean `200`s with well-formed (if
deadline-truncated) N-Triples output — line counts ranged from 85K to 12.6M
triples per request depending on how much of its own 30s window each got
before contention slowed it down. Verified this is truncation, not
corruption: initial byte-count comparison looked alarming (some outputs
appeared larger than a rough mental estimate of the full dump size), but
checking actual line counts confirmed every output was a **prefix** of the
full 15.56M-line dump, never more, never duplicated. Confirms
`ADR-0026`/`ADR-0010`'s drift finding in practice: there is no stream-lane
pool and no `503`+`Retry-After` shedding — concurrent load simply shares the
one PG connection's throughput unevenly and lets each request's own timeout
be the only backstop. That backstop **works** (nothing hangs indefinitely,
nothing corrupts), but a well-behaved client under real overload gets a slow,
incomplete stream after waiting out its full timeout, not a fast, honest
`503` — worse UX than what `ADR-0010` describes, though not unsafe.

### Recommendation

`ADR-0010`'s stream-lane-pool/`503`-shedding clause should have its status
corrected from implicitly-accepted-and-built to **accepted, not implemented**
(a documentation fix, done by this ADR's cross-reference) — and separately,
whether to actually build it is a real prioritization call: current behavior
is *safe* (bounded by existing timeouts, no crashes) but not *graceful*
(slow truncated responses instead of fast, honest rejection) under
concurrent overload. Not decided here — flagged for the user.

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
