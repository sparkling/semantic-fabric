---
status: accepted
date: 2026-06-27
updated: 2026-09-11
tags: [security, resource-governance, injection-safety, dos, recursive-cte, result-streaming, query-limits, production]
supersedes: []
depends-on:
  - ADR-0006
  - ADR-0007
  - ADR-0008
implements:
  - ADR-0001
---

# Security & resource governance for the SPARQL→SQL path

## Context and Problem Statement

The virtualiser (ADR-0007) is a security boundary: untrusted SPARQL is translated into SQL and executed against a live source database. Three concerns are intrinsic to the rewriter/executor and cannot be retrofitted at a gateway (which never sees the generated SQL): **injection**, **denial of service**, and **result streaming** (a SPARQL `SELECT` may return millions of rows). This ADR fixes the controls the **engine** owns. Authorization (authN/Z, row-level security, multi-tenancy, sensitivity) is **ADR-0018**; deployment-edge operations (TLS, secrets store, rate-limiting, audit transport) are **ADR-0014**.

## Considered Options

* **Engine-owned controls (chosen)** — build injection-safety, DoS governance, and result streaming into the rewriter/executor itself, since these concerns are intrinsic to the SPARQL→SQL path.
* **Retrofit at a deployment gateway/edge** — rejected: a gateway never sees the generated SQL, so injection, denial of service, and result streaming (intrinsic to the rewriter/executor) cannot be addressed there.

## Decision Outcome

### A. Injection-safety by construction
* Values originating from the SPARQL (FILTER constants, VALUES, bound terms) become **bound SQL parameters**, never string-concatenated; SQL is built as a `sqlparser` **AST**, not assembled from strings.
* **The mapping is the reachability allow-list:** generated SQL can reference only the tables/columns the R2RML mapping IR exposes; identifiers come from the *trusted mapping*, never user input — so neither table/column injection nor access to un-mapped data is expressible. *This bounds what is reachable; it is not authorization (ADR-0018).*

### B. Resource governance (DoS controls)
* **Exact governed recursion:** every supported `P+`/`P*` recursive CTE collapses cycles on semantic node-pair identity. A work/deadline limit aborts semantic completion; accumulated rows are never labelled or receipted as complete. Once HTTP `200` begins, transport bytes may remain observable, so atomic no-prefix delivery is a separate response-layer gate (ADR-0049).
* **Statement timeout + result-size cap + pre-execution cost check + admission control** on every generated query must bound engine-originated load. These controls reduce overload risk; they cannot guarantee source-database availability.

### C. Result streaming (bounded memory + backpressure)
* Results stream via `tokio-postgres` `query_raw()` → `RowStream` (never `query()`, which buffers a `Vec<Row>`); `RowStream` already bounds client memory **and** propagates TCP backpressure to the backend. Serialise per-solution with `sparesults`, coalesce ~32 KiB chunks, into an `axum` streaming body (the Oxigraph `ReadForWrite` pattern). `prepare()` the SQL before the `200` (clean `4xx`); on stream drop, **cancel the query and discard the connection** (never recycle a possibly-undrained one).
* **Stream lifetime is bounded at the DB:** `statement_timeout` is per-`FETCH`, not per-cursor, so a slow client would otherwise pin a connection indefinitely — bound total lifetime with PostgreSQL 17 `transaction_timeout` (pre-17: `idle_in_transaction_session_timeout` + an app wall-clock watchdog; the watchdog is mandatory for DuckDB/SQLite sources). Run streams in a small, hard-capped **stream-lane connection pool** distinct from the point-query pool; shed overflow as HTTP `503` + `Retry-After` rather than queue; **never** `WITH HOLD` cursors (they materialise the full result at COMMIT).

### D. Delegated
* **Authorization / RLS / tenancy / sensitivity → ADR-0018.** **TLS, secrets store, rate-limiting, audit transport, deployment packaging → ADR-0014.** The engine consumes DB credentials via secret injection only (never logged; ADR-0011) and emits governance + access-decision events to observability (ADR-0011).

### Consequences
* Good, because neither table/column injection nor access to un-mapped data is expressible (user values are bound parameters; identifiers derive only from the trusted mapping IR).
* Good, because statement timeout, result-size cap, cost pre-check and admission control bound engine-originated load when implemented; they are not a source-database availability guarantee.
* Good, because pair-fixed recursion terminates on finite sources without authorizing or receipting a depth-truncated answer; total work must abort semantic completion under ADR-0049.
* Good, because client memory is bounded and TCP backpressure propagates to the backend (`RowStream`), and slow/abandoned clients cannot pin a connection indefinitely (DB-bounded stream lifetime, stream-lane pool, cancel-on-drop).
* Neutral, because authorization / RLS / tenancy / sensitivity is delegated to ADR-0018 and TLS / secrets store / rate-limiting / audit transport / deployment packaging to ADR-0014.

### Confirmation
* Fuzzing the rewriter (ADR-0012) surfaces no injection (always parameterised; identifiers always from the mapping).
* A `P+` query over a cyclic fixture reaches the exact pair fixed point; a pathological query must never report or receipt partial semantic success, while post-`200` transport atomicity remains a separate gate.
* A million-row `SELECT` streams with bounded memory; a slow/abandoned client is bounded by `transaction_timeout` and does not exhaust the stream-lane pool.

> **Status correction (2026-07-16, measured, `ADR-0027`).** The "stream-lane
> connection pool" / "shed overflow as `503` + `Retry-After`" clause above
> describes design intent this ADR presented as decided, but it was never
> built: `grep` for `stream_lane`/`Retry-After`/`503` across `sf-serve`'s
> source and tests returns zero matches, and PostgreSQL is served over a
> single `tokio_postgres::Client`, not a pool. Live load testing (`ADR-0027`)
> confirmed the practical consequence: under concurrent overload, requests do
> **not** crash, hang, or corrupt data (the existing per-request `timeout` and
> `max_query_len` both hold correctly under concurrency, verified directly)
> — but there is no fast, honest overload signal either. Concurrent clients
> simply share the one connection's throughput unevenly and each waits out
> its own full timeout before getting a truncated response, worse UX than
> this clause describes, though not unsafe. Treat this clause as **accepted,
> not implemented** until the pool/shedding is actually built or is formally
> descoped — do not read the rest of this ADR's "accepted" status as implying
> this specific piece shipped.

> **Status correction, part 2 (2026-07-18, built + measured).** The PG half of
> the clause above IS now implemented (M4 wave-2): `Backend::Pg` is a
> `deadpool_postgres::Pool` (`max_size` 16, `wait_timeout` 5s,
> `Runtime::Tokio1` — the runtime must be set explicitly or the timeout is
> silently never enforced), and pool exhaustion sheds `503` + `Retry-After: 1`
> instead of queueing (`acquire_pg`, test-locked incl. the exhaustion path).
> Measured under 16 concurrent SELECTs: ~2.3× wall-clock improvement over the
> single-client behavior (4.10s→1.75s / 3.94s→1.68s, all responses complete
> and correct). SQLite remains a single `Mutex<Connection>` by choice (an
> embedded-source serialization question, out of this clause's scope);
> `Retry-After` is a fixed `1`, not pressure-derived — both recorded as open
> refinements, not gaps in the clause.

> **Status correction, part 3 (2026-08-25, issues #6 and #7).** Four adapter
> families currently substitute values into SQL text with quote doubling
> rather than binding them: REST, MonetDB, HANA, and ODBC. None conforms to R1.
> The REST family additionally collects complete result pages before returning
> a `BranchStream`, so it does not conform to R5; a one-row stream interface
> alone proves neither bounded first-result latency nor bounded memory. The
> supported `sf-serve` surface remains SQLite, PostgreSQL, and MySQL. Any other
> adapter may join that surface only after provider-native parameter transport,
> bounded streaming, lifecycle/error handling, cancellation, and direct
> conformance evidence exist. Issue #6's optional fallible/early-exit quad sink
> is an API refinement and does not change this ADR's accepted status.

> **Status correction, part 4 (2026-09-01, completion audit).** ADR-0049 removes
> the successful hard-coded 256-hop prefix: evidenced compiler targets now use an
> exact finite-pair fixed point and unproved dialects reject before emission.
> R4 remains incomplete: compilation is outside the request timeout and there is
> no common result-row/result-byte cap, cost pre-check, source-native statement
> timeout, or cancellation contract across all three backend paths. No production-
> admission claim follows from pair exactness or the current timeout/pool controls.

> **Status correction, part 5 (2026-09-01, built + measured).** Commit `6cd85eb`
> mints one absolute deadline before request-body extraction and carries that same
> instant through a fixed-capacity compiler admission wait, the blocking compile
> waiter, PostgreSQL acquisition, ASK execution, the complete stream driver,
> serializer finish and body send. A timed-out or abandoned compile keeps its one
> of four permits until the blocking closure really returns, and the executor
> yields once per bounded raw batch even when OFFSET/dedup discards every row
> before the sink. Deterministic tests cover every phase and prove that no phase
> refreshes the clock. This is a narrow elapsed-time boundary, not completed R4/R5:
> compiler CPU is not cooperatively cancellable; queued HTTP waiters, rows, bytes,
> source work and recursion lack one total budget; sources lack a common
> source-native statement-cancellation contract; SQLite cannot interrupt an in-flight blocking
> bridge; and work inside one raw batch is not pre-empted. After HTTP `200`, a
> SELECT/CONSTRUCT timeout terminates the body and may expose a usable prefix, so
> no atomic/no-success-prefix response claim or backend admission follows.

> **Status correction, part 6 (2026-09-02, accounting foundation).** The
> runtime-neutral `sf-core` port now owns typed inclusive limits for observable
> source work, semantic result items and serialized bytes, plus checked atomic
> counters and one sticky terminal reason for deadline, cancellation, limit or
> arithmetic failure. Exact-limit, overflow, concurrency and first-cause tests
> pass. This foundation alone is not request-wide enforcement: until serving,
> execution and serialization all carry the same identity, the part-5
> cancellation, recursion, response-atomicity and production-admission
> nonclaims remain unchanged.

> **Status correction, part 7 (2026-09-02, executor enforcement).** The shared
> SQLite/PostgreSQL/MySQL executor now accepts the runtime-neutral control and
> charges source work before every catalog probe, branch open and row-pull
> attempt (including the final EOF pull). SELECT charges each semantic row,
> CONSTRUCT atomically charges the number of emitted triples, and ASK charges
> exactly one boolean; every charge occurs before the caller's sink. Raw and
> conformance entry points remain explicitly uncontrolled, while dedicated
> controlled siblings exist for the serving lane. Boundary tests prove
> zero-budget rejection before source I/O, exact accounting for OFFSET-discarded
> rows, no over-limit sink call, and all-or-nothing multi-triple charging. This
> still does not complete R4: `sf-serve` does not yet mint and carry this budget
> through compilation and serialization, recursive work inside source SQL is not
> observable, and no common source-native cancellation contract or atomic
> pre-`200` response exists.

> **Status correction, part 8 (2026-09-02, governed serving lane).** `sf-serve`
> now mints one finite request budget before body extraction and carries that same
> identity through the absolute deadline, compiler admission/wait, pool acquisition,
> controlled SQLite/PostgreSQL/MySQL execution, semantic result charging and every
> serializer write. Defaults and CLI flags expose inclusive ceilings for source
> work, result items and serialized bytes. An impossible zero-result ASK budget
> rejects before source I/O; every ASK breach is known before response and uses a
> redacted `429 query-budget-exceeded`. SELECT/CONSTRUCT breaches after
> the status handoff always terminate with the stable `result stream failed` body
> error. Streamed SELECT/CONSTRUCT serializers pass at their exact byte ceiling and
> fail before an over-limit append; bounded ASK serialization charges before the
> response. Dropping a sub-chunk body cancels and drops a stalled Rust driver.
> R4/R5 are still incomplete: source work means only catalog
> probes, branch opens and pull attempts—not database rows scanned, recursive CTE
> iterations, compiler CPU or source cost. MySQL acquisition is deadline-bounded
> but lacks PostgreSQL's fast `503`/`Retry-After` pool-shedding contract.
> Raw/conformance APIs remain explicitly uncontrolled, and no common source-native
> statement-cancellation contract, SQLite in-flight interruption, bounded recursive work,
> or atomic/no-prefix streamed response is claimed.

> **Status correction, part 9 (2026-09-02, adversarial race closure).** Commit
> `087e7e2` makes charge/termination transitions linearizable: an in-flight
> charge completes before a later terminal transition, while a rejected charge
> changes no counter; barrier and concurrency tests lock both cases. Commit
> `f1f4747` rechecks the absolute clock before accepting handler output, makes a
> body-drop cancellation deadline-aware, and rejects an impossible ASK result
> capacity before backend selection or acquisition. Commit `136f21b` pulls ASK
> one row at a time and stops after the first final solution without truncating a
> Rust-group inner collection or losing ordered OFFSET/LIMIT semantics. Commit
> `481f870` aligns serving admission: top-level ASK ordering cannot change
> existence and uses no global sort buffer, while genuinely source-sized states
> and ordered nested subplans remain rejected. These
> repairs do not change the part-8 database-work, source-native statement-cancellation,
> raw/conformance, recursive-work, or post-`200` atomicity nonclaims.

> **Status correction, part 10 (2026-09-02, deterministic deadline handoff).**
> Commit `d391927` separates the public handoff classification from sticky
> internal accounting. At an expired representable deadline, handler handoff is
> always `DeadlineExceeded`/HTTP `504`, even when an earlier resource terminal
> remains the accounting identity's first cause. Before that instant a prepared
> response is handed off and later non-deadline stream failures remain post-`200`.
> An unrepresentable deadline remains `AccountingOverflow`, not a timeout.

> **Status correction, part 11 (2026-09-02, outer deadline and strict request
> admission).** Commit `1e85249` moves deadline creation to Tower
> `Service::call`: Hyper has parsed the HTTP/request target, but Axum route and
> method dispatch have not begun. Commit `36488d5` admits exactly one query for
> GET or form POST, or one raw POST query; optional version, dataset, duplicate,
> and extra parameters reject instead of being ignored. Unsupported media
> rejects without polling the application body. The raw per-request body cap is
> `n`; the checked form wire cap is `3n+16` with a decoded-query cap of `n`, and
> unrepresentable configuration rejects before source/file/network I/O. These
> are request-admission limits, not aggregate service quotas. Full Protocol,
> compiler/source/recursive work, native cancellation and atomic post-`200`
> delivery remain open, so R4/R5 and production admission remain incomplete.

> **Status correction, part 12 (2026-09-03, narrow SQLite active-VM
> cancellation).** The owned serving SQLite backend installs a query-local
> `rusqlite` progress handler after acquiring the connection mutex and before
> metadata/query work. It shares the request's exact control identity, preserves
> the recorded cancellation/deadline cause, removes the handler on every
> exit/unwind, and leaves unrelated `SQLITE_INTERRUPT` failures as driver errors.
> This active-VM profile does not make raw mutex wait or already-submitted/running
> `spawn_blocking` work cancellable; busy timeout, blocking UDF/VFS/I/O, compiler
> CPU, raw/conformance callers, PostgreSQL/MySQL, database
> rows or recursive source work, total M2, source admission, or atomic post-`200`
> delivery. The common source-native statement-cancellation contract remains open.

> **Status correction, part 13 (2026-09-03, SQLite connection admission).**
> Every physical `SqlitePool` member now has one permanent cap-one async
> admission identity. Serving SELECT, ASK, and CONSTRUCT acquire the selected
> member under the request's existing `RequestBudget` before submitting any
> SQLite blocking worker. A deadline or cancellation drops that acquisition
> waiter; an admission timeout therefore returns a redacted pre-`200` HTTP `504`
> without entering SQLite. The public lease is non-`Clone` and consumed once;
> its private state is retained and cloned into both `column_names` and
> `open_branch` workers through worker exit, so cancelling the async caller
> cannot return capacity while submitted work still owns it.
> Deterministic unit and HTTP tests lock the cap-one identity, both worker
> lifetimes, pre-response timeout, no-UDF-entry, and recovery behavior.
> This closes the serving admission wait, not total queue governance: the
> semaphore's waiter count is bounded only by external request admission and
> each waiter's deadline/cancellation. Raw `SqlitePool::pick` and foreign mutex
> holders bypass the lease; the standard mutex and already-submitted/running
> `spawn_blocking` work remain non-cancellable. Busy waits and blocking
> UDF/VFS/I/O are still non-preemptible, selection remains round-robin rather
> than availability-aware, and no PostgreSQL/MySQL, compiler, raw/conformance,
> database-row, recursive-work, atomic post-`200`, total-M2, or production-
> admission claim follows.

> **Status correction, part 14 (2026-09-03, aggregate serving admission and
> terminal delivery).** Commits `25196b4`, `04dc983`, and `20f3df2` add one
> shared fail-fast gate for admitted application work. The
> `--max-concurrent-requests` option has a conservative finite default of 64—not a
> measured throughput target—and startup accepts only
> `1..=Semaphore::MAX_PERMITS` before source resolution, file reads, runtime
> construction, or network I/O. Tower `poll_ready` does not queue for this gate:
> `Service::call` checks the request control, uses `try_acquire_owned`, and does
> not poll the Router or request body when saturated. Saturation is the distinct
> redacted `503 service-overloaded` response with fixed `Retry-After: 1`; an
> expired representable deadline takes precedence as `504`, while a closed gate
> is an internal `500`. This is zero-queue shedding, not fairness or bounded
> waiting, and the flag does not count sockets, completed responses, or rate-limit
> clients.
>
> The owned permit is retained by the request budget through active handler,
> producer, detached compiler, and backend-worker clones; caller cancellation
> cannot return capacity while one of those internal tasks still owns it. The
> permit is not retained merely while a client drains bytes from an already-
> completed bounded producer. Streaming terminal state is therefore carried
> out-of-band from the bounded data channel: even a full unpolled channel can
> finish at its deadline, after which the body yields its buffered prefix, exactly one stable
> `result stream failed` error, then fused EOF, without a `Content-Length`.
> This does not prove compiler or database cooperative cancellation, database-row
> or recursive-work accounting, raw/conformance governance, SQLite raw-mutex or
> busy/UDF/VFS/I/O pre-emption, PostgreSQL/MySQL native cancellation, response
> atomicity after `200`, per-request fairness, or production backend admission.

### Native serving cancellation status (2026-09-08)

Ordinary PostgreSQL and MySQL serving SELECT/ASK/CONSTRUCT and source fragments
now own a dirty connection before query setup. Success requires an acknowledged
drain/reset barrier before reuse; error, timeout and drop instead retain request
capacity through a bounded native stop attempt, then discard the connection.
PostgreSQL uses the pool's immutable TLS policy for CancelRequest (one-second
allowance); transaction-local RLS/generation cleanup remains protected. MySQL
uses a separate same-credential/TLS control connection (two seconds), not the
possibly exhausted data pool. Unpredictable named locks pin target/control
sessions; a missing, SQL-NULL or mismatched target witness refuses KILL. Only
the driver's numeric ID is used and KILL is never retried. These are
session-affine endpoint controls, not arbitrary multiplexing/failover-proxy
qualification or an atomic check-and-KILL guarantee against external termination
and ID reuse. Stop acknowledgements are not termination acknowledgements.

The public MySQL pool excludes constructor queries/callbacks and supplies client
settings before connection creation, avoiding an unowned settings-result drain
on cancellation. Defaults are a 4 MiB packet limit and 30-second client idle TTL;
explicit client settings are preserved. Neither is a server execution deadline.
The control constructor has no settings queries or caller setup either. Dirty
disconnect is polled before drop so the pinned driver's recycler discards rather
than drains it, including an aborted/unpolled cleanup task. Raw caller-created
embedding pools require separate constructor/recycler qualification.

Native statement limits supplement the original absolute application deadline;
MySQL retains a stricter existing SELECT execution limit and its native timeout
code remains a typed pre-response 504. An unrelated SQL cancellation is not
mislabelled deadline expiry. Required owned PostgreSQL 16.15/MySQL 8.4.11 TLS CLI
tests observe server work stop after ASK timeout and SELECT/CONSTRUCT disconnect,
then prove cap-one pool recovery; a stricter MySQL source timeout is also tested.
Peer/unit tests cover retained capacity, nullable witnesses and setup exclusions.
Protected PostgreSQL generation and public RLS isolation/cleanup tests still pass.
The same live fixture observes native stop and clean process exit after forced
SIGTERM during ASK/SELECT/CONSTRUCT on each backend. HTTP completion alone cannot
end the runtime while native cleanup retains request capacity. Normal drain uses
the original signal deadline; Forced cancellation then permits three seconds for
owned cleanup, returning an error if capacity remains held (ADR-0011).
Required mixed PostgreSQL/MySQL CLI evidence now covers UNION and both join
pattern orders under deadline, pre-header join disconnect and forced SIGTERM.
Each provider is deliberately blocked; native session IDs and granted-lock
witnesses prove target work stops while its lock remains held. An independent
public CLI process with the same database credentials retains its distinct
blocked query and completes exactly after release. Both cap-one source pools
recover the full federated bag after timeout/disconnect. Join deadlines return
504 before success; a streamed UNION failure cannot complete its chunked 200.
This is the pinned session-affine TLS profile, not raw-pool/proxy, every backend
combination, source-generation or exact-release admission. SQLite busy/UDF/VFS/
I/O, total compiler/database/recursive work and post-200 atomicity remain open.

### Public compiler-input admission (2026-09-08)

The serving boundary now charges one compiler-work unit per decoded UTF-8 query
byte before each public compilation entry: lineage preparation, structural
preflight and authoritative compilation, including cache hits. Authentication
and policy validation retain precedence. All entries share the request's sticky
counter; an insufficient first-pass allowance returns redacted 429 before compiler
queue/source admission. Verified-generation preflight and authoritative compilation
charge cumulatively: a later shortage can occur after lease acquisition and still
requires normal owned cleanup. Existing lineage-specific work charges remain.

Required `query_budget` HTTP tests prove zero allowance, exact/one-below decoded
UTF-8 limits, GET/form/raw transport, authenticated cold/warm caches and exact
successful results. `request_compile::tests` prove cumulative two-pass accounting,
unchanged counters on rejection, permit recovery and no source admission. This
closes the ignored compiler-input allowance, not total parser/optimizer CPU,
catalog/product growth, recursive destruction or the broader governance gate.
The raw compiler and dormant governed pipeline are not promoted by this change.

**Owned clone-work update (2026-09-10):** ordinary and security-scoped serving
misses, uncached preflight and bounded federation tree compilation now carry
the same control through existing normalization/lowering/nested-cascade clone
operations. Each performed clone is measured and reserved before copying; its
operation-local limits are not a whole-plan admission profile. Cache hits retain
their existing identity and share `Arc<Plan>` without charging for avoided clones.
Clone failures or cancellation observed before insertion cannot populate caches;
checkpoints bracket key lookup, hits and insertion. A cancellation racing after
insertion may retain the completed valid plan, but cannot return it to that caller.
Tests prove exact/N-1 charging, cumulative uncached passes, cancellation,
security partitioning and public `EXISTS` rejection before source admission with
exact sufficient-budget results. Tree inner joins also prospectively reserve every
candidate pair and exact scalar left-branch clone, including later-pruned pairs.
Before direct right-field copies, one whole-branch measurement conservatively
reserves that same borrowed source without a shadow clone. Checkpoints surround
the operation; the existing path guard still rejects/prunes before right-copy work.
Exact/N-1 tests cover bindings, scans, conditions, OPTIONAL and subplan payloads;
authenticated right-heavy VALUES tests prove pre-source rejection, permit recovery,
exact successful bags and completed-cache reuse. This covers direct copies only:
unifier-produced conditions, nullable-alias sets and extra left-internal copies
remain open, as do upstream temporary allocation, other resolve work, products/copies
and destruction. `l-query-budget` stays open; direct multi-origin unfolding
retains its separate eligibility/recipe charges.

**Canonical-key work update (2026-09-10):** ordinary and security-scoped cold/warm
cache paths now pay for every UTF-8 output fragment, each requested geometric
capacity target plus existing-payload relocation, and the one canonical hash
before that work. Logical growth is independent of allocator overgrant; finite
cumulative request work bounds requested capacity, not exact physical heap usage.
The existing AST envelope is checked before recursive formatting. Its iterative
walk now prepays nodes, collection slots/payload and logical stack target/relocation
before use, with checkpoints and fallible stack allocation. The measured AST
also conservatively prepays upstream projection/Extend dependency searches,
including nested EXISTS and repeated variable comparisons, plus logical projection
payload before Display. Checked work uses actual Extend/project/variable counts;
large unrelated strings do not create a fictitious quadratic variable-scan charge.
Security keys
reuse the canonical string/hash without a second render/hash; exact identity,
policy-first errors, raw APIs and shared `Arc` reuse in both directions remain intact.
Exact/N-1, expanded UTF-8, allocation/cancellation and held-source tests prove
failure precedes cache/source access and compiler permits recover. Warm hits pay
key work but not avoided compilation. Independent walker/preparation tests cover
exact/N-1, no unpaid stack allocation, unbound VALUES cells, duplicate projections,
root reentry, overflow and sticky cancellation/deadline causes. Later-phase HTTP
tests calibrate prerequisite key work through a raw-populated identical warm hit.
The upstream projection Vec remains infallible; physical allocator overgrant and
cancellation inside its prepaid recursive scans are not governed by this change.
**Cache-lock update (2026-09-10):** controlled ordinary/security compilation now
attempts each lookup/insertion lock once without waiting. Contention is an
optimization miss/skipped write: authoritative compilation retains the same
request control, exact key/security scope and separately owned result. Required
real-shard-lock tests complete before release, preserve the old resident `Arc`,
recover shared hits and retain budget/cancellation/deadline and policy-first errors.
Raw APIs remain blocking and interoperable. Hashing/probing/collision equality,
work inside an acquired lock, insertion allocation/eviction/destruction and other
compiler phases remain open. `GovernedV1` stays dormant; `l-query-budget` blocks.

**Structural BUILD update (2026-09-10):** the live AST-to-IQ stage carries the
request control through all supported arms, including GRAPH and nested EXISTS.
Visits, fixed collection slots, logical vector growth/relocation, stable scope
comparisons and source-bound scalar/recursive copies are prepaid; a depth envelope
also protects uncached entry; small recursive dispatcher/arm frames retain that
boundary on ordinary test/worker stacks. Named paths avoid a redundant temporary copy.
Required tests cover raw-tree equivalence, exact/N-1 work, every charge's sticky
cancellation/deadline, UTF-8/duplicate scopes and allocator-independent growth.
Public ordinary/security tests retain paid-key hits without rebuilding, reject
unpaid BUILD before held-source admission and recover compiler capacity; later-phase
tests separately pay BUILD rather than masking their original operation boundary.
Fallible vector allocation is local: derived Clone/Box/string/map allocations,
physical allocator overgrant and destruction
remain unqualified, alongside other compiler/source/release work. No governed
profile or whole-application completion is claimed.

**Clone-measurement update (2026-09-11):** every existing controlled copy now
uses the same request identity inside its iterative V1 measurement walk. Entry,
pushes, metric records, collection iteration and logical stack target/relocation
are prepaid before use; allocation is fallible and allocator slack grants no free
logical growth. Optional/UNDEF slots observe cancellation even without a payload.
Measurement reads payload lengths without copying bytes; its work is separate
from the unchanged exact clone metric and subsequent copy reservation. A refused
copy retains completed measurement work. Raw metrics and depth/pending envelopes
remain unchanged. Hand-counted exact/N-1, no-unpaid-allocation, every-step
cancellation/deadline and default-stack tests cover the walker; caller schedules
retain later-phase targets and public pre-source rejection/cache recovery checks.
Derived Clone/Box/string/map allocation, physical heap/destruction, other compiler
phases, source/backend and exact-release guarantees remain required and open.

**Constant-row NORMALIZE update (2026-09-11):** VALUES UNION folding/reordering,
DISTINCT comparison/first-occurrence removal and SLICE planning/row movement now
share request control. Visits, name/constant comparisons, fixed slots, logical
growth/relocation and constant-expression probes/materialization are prepaid.
RDF comparisons pay measurement and payload scanning separately; constant BIND
probes match the empty-environment unifier without speculative term/error copies.
Cancellation/resource refusal is terminal, never an optimization decline. Required
tests preserve bags, UNDEF, projection guards, contiguous runs and residual offsets;
malformed VALUES headers/rows decline folding safely. Exact/N-1, hand-counted work,
per-charge cancellation/deadline and default-stack depth checks cover this slice.
Ordinary/security HTTP tests prove pre-source rejection, capacity recovery, exact
results and paid-key warm reuse. The following update extends this boundary.

**Structural NORMALIZE update (2026-09-11):** node/condition traversal (including EXISTS), stable scopes, construction composition, INNER/FILTER/left-only LEFT distribution and substitution merging now reuse the same control. Visits, logical vectors/boxes/map entries, key comparisons and movement are paid before use; depth and cancellation cover both node and condition recursion. Shared resolved bindings preserve ordered Sat/Empty/Unsupported behavior after paid input measurement and a checked conservative logical unification allowance (512 + 64 times the two V1 deep-clone measures), audited against the current linear helper call graph. This is not physical byte-allocation proof or in-call unifier preemption. Raw-equivalence/exact/N-1 tests retain outer substitution precedence and right-Union non-distribution; hand-counted downstream prerequisites preserve the original clone/product failure targets. Default-stack depth and every-charge cancellation/deadline checks cover structural paths. Required ordinary/authenticated HTTP checks demonstrate rejection before held source admission, capacity recovery, exact-result recovery and paid-key warm reuse. The mapped-table witness records the existing NORMALIZE span counters, independently checks its work, and proves both cold and paid-key-only warm requests wait at the held source before release and exact duplicate-bag recovery; source-free fixtures alone are not that positive control. Uncontrolled logical growth in other compiler/source/backend paths and release qualification remain blockers. Exact allocator-byte/overgrant or bounded derived-drop proof is a non-claim, not a separate v1 gate under ADR-0055; finite admitted growth and terminal cleanup remain required. GovernedV1 stays dormant. OPTIONAL expansion now prepays fast L candidates or decomposed 2*L*R match/anti-match candidates plus L owned tails, logical result-vector growth and owned condition/join appends (2026-09-11). Same-source whole-branch reservations precede direct copied fields; SubPlan nullable-left scalar copies are separately paid. The original left branch moves into each no-match tail without a shadow clone. Required exact/N-1, prospective overflow, independent dispatcher-cost, raw-equivalence and every-charge cancellation/deadline tests cover fast, ordinary/forced decomposition, nested-right and DISTINCT-SubPlan paths. Mapped ordinary/authenticated HTTP checks observe the LOWER span and independently measure its cost, reject before held source admission, then prove exact duplicate/UNBOUND bags and compiler/source recovery on funded cold and input/key-only warm requests. Existing sliced-SubPlan rejection is unchanged. This closes candidate/result-vector/direct-copy admission only: generated ON conditions, nullable sets, FILTER/unifier temporaries, other structural LOWER/compiler/source work, backend guarantees and exact-release checks remain required. It does not activate GovernedV1, prove physical heap/drop behavior or establish application completion. LOWER entry/scope update (2026-09-11): the actual root and nested SubPlan alias walk now pays IQ/condition/SQL visits, including literal-only IRI-template parts, and checks combined recursive depth and the initial alias successor. Spine dispatch, stable output-scope construction, explicit projection copies and final ordered result-variable materialization share request control; implicit fallback remains lexically sorted/unique with paid comparisons and logical growth. Required exact/N-1, raw-equivalence, UTF-8/project-order/duplicate/slack, every-charge cancellation/deadline, overflow and default-stack depth tests cover this boundary. Existing clone/product tests independently pay the preparation prefix and keep their original failure points; successful budgets alone add final materialization. Three mapped ordinary/authenticated HTTP shapes prove unpaid LOWER scope never enters the held source, funded cold/key-only warm requests do enter, and exact bags, renamed fields, head order and permit recovery survive. Internal alias reservations now use checked prospective ranges on LOWER SubPlan/path/SQL-group and shared Unfolder/flat-pool paths (2026-09-11). One work unit per identifier precedes use; overflow and terminal refusal preserve counters, path batches and two-alias SQL pool arms. Resource errors propagate rather than falling back to Rust grouping. Exact/raw, N-1, overflow and per-charge cancellation/deadline checks cover this boundary; mapped ordinary/authenticated single/UNION GROUP BY requests prove pre-source refusal, exact funded cold/key-only warm results and capacity recovery. This is not whole-helper traversal accounting. The COUNT/constant-DESCRIBE identity defect is now repaired (2026-09-11): a prospectively paid cache-only whole-query copy normalizes unique non-exposed aggregate binders and isolated constant-IRI DESCRIBE targets. Fresh names, preserved authored bindings/outputs and exact canonical equality prevent capture and cross-query/scope/profile/security reuse; ambiguous roles retain safe misses. Raw/controlled/security independent-parse shared hits, exact/N-1/no-publication and every-charge interruption checks pass. Mapped ordinary/authenticated single/UNION COUNT HTTP checks prove paid-key refusal before source, cold/warm source admission, exact counts and capacity recovery; observed parse-only warm stages prove reuse without assuming random-name work totals are identical. This does not mutate the submitted AST/plan, activate GovernedV1 or establish general alpha-equivalence (ADR-0007). Other operator helpers, generated conditions/unification, source/backend controls and exact release remain open. No GovernedV1, whole-compiler, physical-heap/drop or application-completion claim. OPTIONAL helper update (2026-09-11): scan, match/anti decomposition and pure-SubPlan OPTIONAL now prepay nullable/derived-alias inventories, shared-name comparisons, depth-checked borrowed term/graph/segment scans, NULL-safe condition vectors/column copies and generated-condition/deferred-binding appends. They no longer clone column inventories or shift NULL-safe disjunctions. Required exact/N-1 and every-charge stop tests retain raw semantics, graph-scoped blank-node absence, aggregate nullability and sound-501 boundaries. Two mapped ordinary/authenticated chained OPTIONAL paths, including DISTINCT SubPlan, reject at the observed nullable-alias helper before held source admission, then recover exact duplicate/UNBOUND-coalesced results, right-only bindings and compiler/source capacity on funded cold/key-only warm requests. OPTIONAL unification update (2026-09-11): all four fast/match/anti-match/pure-SubPlan callers now reuse `CompileContext::unify_terms`: separately paid exact-input measurement precedes the existing checked 512 + 64-times-input logical allowance. Funded Sat/Empty/Unsupported order and messages, early guards and raw behavior are unchanged. Exact/N-1 and every-charge cancellation/deadline tests cover all verdicts; mapped ordinary/authenticated fast, UNION decomposition and DISTINCT-SubPlan HTTP tests observe every actual call and refuse at the final unifier before held source admission, then recover exact bags/UNBOUND and compiler/source capacity on funded cold and key-only warm requests. OPTIONAL FILTER update (2026-09-11): fast and match/anti-match paths measure the expression, prospectively pay all key comparisons per variable occurrence and measure only referenced TermDefs before checked 1024 + 128*(expression+1)*(lookup-footprint+1) construction admission. Source validation now pays actual recursive source/projection visits, comparisons and logical output growth, preserving derived-plan fanout, DISTINCT order, aggregate/SQLite AVG positions, first-match/error order and the existing path-decoder refusal. FILTER output appends and anti-FILTER entry searches are paid without moving R5 conditions. Exact/N-1, arithmetic, UTF-8/layout, depth and every-charge cancellation/deadline tests preserve raw conditions/refusals; mapped ordinary/bearer fast and UNION HTTP tests observe both actual phases, reject each before held source admission, then recover exact duplicate/UNBOUND bags and capacity on cold/key-only warm requests. OPTIONAL shape/R2 update (2026-09-11): initial opts and constant-binding scans now pay visits and UTF-8 comparisons, retaining first refusal, Const-variant decisions, RefAtom decomposition and pure-SubPlan priority. R2 right-only insertions and nullable COALESCE construction pay current-map comparisons, logical entry carriers and boxes; matched WHERE/core/SubPlan vectors pay growth and relocation independently of allocator slack. Fast/matched left payloads move only after FILTER lowering; pure-SubPlan scalar copies retain exact-source admission. Hand-counted schedules, exact/N-1 and every-charge cancellation/deadline checks preserve decisions, copy boundaries and left-first values. Mapped ordinary/bearer shape and chained scan/DISTINCT-SubPlan tests observe the actual phases, reject before held source admission and recover exact duplicate/UNBOUND-coalesced bags and capacity on funded cold/key-only warm requests. Ordinary IQ FILTER update (2026-09-11): iterative group peeling, per-branch/group visits, boolean condition-tree depth, vector/box construction, exact borrowed SQL copies and visible WHERE growth now share request control. Expr construction reuses the measured-input allowance and then the actual source validator; Sql remains pass-through, earlier branches borrow and the final branch moves. EXISTS/NOT EXISTS/MINUS keep their existing delegation and independent clone-refusal boundaries; their inner work and combined recursion are not newly qualified. Exact hand-counted ownership/allocation schedules and every-charge cancellation/deadline tests preserve raw dialect decisions. Mapped ordinary/bearer plain, UNION and OPTIONAL-plus-FILTER HTTP paths refuse at actual construction, validation and append phases before held source admission, then recover exact cold/key-only warm bags and permits. The unchanged default-admission typed-literal OPTIONAL/outer-FILTER regression also passes: irrelevant binding payload is excluded, not actual lookup/copy work, and neither production limits nor the conservative multiplier is relaxed. Remaining compiler/source/backend/release guarantees stay open; no in-call raw-helper preemption, physical-heap/drop, GovernedV1 or completion claim. BIND/substitution update (2026-09-11): ordered dependency passes and pending reference vectors, resolved/existing definition copies, binding searches/edits and nullable/unification/WHERE output now pay request work. Supported BIND constants, variables and recursive CONCAT use internal depth, copy, lookup and output checks; unsupported Debug errors use measured pre-admission without changing text or retry order. Borrowed left-SubPlan scans retain actual-column semantics without allocating column inventories. Post-modifier folds now discard proven-disjoint branches; the regression covers DISTINCT, Slice, OrderBy and SQL aggregation. Exact/N-1, every-charge cancellation/deadline and default-admitted ordinary/bearer plain/UNION/OPTIONAL BIND requests prove pre-source refusal, funded cold/key-only warm exact bags and permit recovery. Construction projection/aggregate helpers and other compiler/source/backend/release controls remain separate; no whole-compiler, physical allocator/drop or completion claim.

**Mapping-expansion update (2026-09-08):** public resolution and direct lineage
unfolding now retain that same work mode through nested contexts. Map/POM visits,
graph-union visits/comparison candidates, named-graph enumeration, fixed graph
filter candidates, class/POM products and parent lookup candidates are reserved
before their corresponding work, including later-pruned atoms. Actual child/parent
logical-source copies use exact scalar measurement; parent maps are borrowed.
No graph-attempt vector is allocated. Required tests prove absent-predicate rejection
before held source admission, exact six-triple results, inclusive bounds, cancellation,
graph/default/class semantics and raw equivalence.

**Path-search update (2026-09-08):** the shared predicate-hop resolver now reserves
map/POM/predicate/object search candidates and graph union/filter work before
execution, with exact scalar term-map/source-copy reservations. Negated paths also
charge exclusion/complement comparisons and leaf visits, borrow predicate IRIs and
move first-hop endpoints without collapsing the `Nps` bag marker. Required public
tests reject exhausted work before held source admission and preserve six exact
duplicate pairs; focused checks cover cancellation, cache exclusion, graph fallback,
duplicate declarations and ambiguity. Raw semantics and supported shapes remain.
Graph-variable inventory and reflexive eligibility now also reserve mapping visits,
prospective graph comparisons, scalar graph copies and enumeration slots, with
request cancellation and graph-scope restoration on failure. Required HTTP tests
prove that an empty named-graph answer still pays for mapping inspection, rejects
before source admission when exhausted, and preserves exact named-graph `+`, `*`
and `?` results when paid. Shape/rewrite construction, TBox/unifier internals and
other phases still lack complete controls; this is not
total compiler CPU, source-recursion governance or release admission.

**Typed path transport (2026-09-08):** joined paths retain their source recipes
until live preflight/emission. Existing clone measurement traverses those retained
trees; resource admission still sees them as source-backed, and portable policies
reject them without partial source authorization. This preserves the controls
while fixing folded-column execution, not total path work or RDF-key equality.

### Public parser lifetime (2026-09-08)

The accepted ADR-0055 bounded-execution contract now reuses the Rust process
boundary for every public parse, rather than leaving a recursive upstream parser
inside an uninterruptible server thread. The CLI prepares its held dispatcher
before readiness; embeddings install an explicit `ParserRuntime`. Missing setup
never falls back to raw parsing. Request control interrupts parent pipe/pidfd
waits; owned process cleanup/reap completes before the compiler releases its permit.
Parent QueryV1 decode validates finite bytes/records/edges/scalars and depth 256
before recursive AST construction. It never reparses the original text.
Syntax is redacted 400, structural envelope 429, resource allocation 503, deadline
504 and infrastructure/protocol/abnormal worker exit 500; an abnormal exit is not
mislabelled query fuel exhaustion. Cap-one CLI survival/recovery, exact ordinary
bindings, normal/lineage inputs, scope restoration and owned pipe cancellation
tests cover this slice. Wider compiler/source work and release qualification stay open.
**Qualification correction (2026-09-10):** I/O tests separate short deadline expiry
from partial-write/EOF fixtures that must first make progress. They retain exact
byte/error/reaping assertions and the immutable spawn deadline; production limits are unchanged.

## More Information

* **Rewriter / `P+`:** ADR-0007. **Exact closure:** ADR-0049. **Exec / pooling:** ADR-0006. **Reasoning:** ADR-0008. **Authorization:** ADR-0018. **Observability / secrets:** ADR-0011. **Fuzzing:** ADR-0012. **Edge ops:** ADR-0014.
* **Research:** `docs/research/` — `virtualization-streaming`, `obda-resource-governance`.

## Rules
* **R1** — user values are bound parameters, never concatenated or textually interpolated, even with escaping.
* **R2** — SQL identifiers derive only from the mapping IR (the reachability floor; authorization is ADR-0018).
* **R3** — every supported recursive CTE collapses semantic cycles; any work or deadline ceiling aborts semantic completion and never labels or receipts a depth prefix as complete. Post-`200` bytes may remain observable until a response-layer atomicity gate is implemented (ADR-0049).
* **R4** — every generated query is governed (statement timeout, result cap, cost pre-check, admission control).
* **R5** — `open_branch` returns without first collecting the complete result, and results stream with bounded memory **and** DB-bounded lifetime (`transaction_timeout`, stream-lane pool, cancel-on-drop).
* **R6** — an adapter that fails R1 or R5 is excluded from `sf-serve`; an admission test locks the supported serving set until the adapter passes those rules.
