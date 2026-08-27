# semantic-fabric

**A uniform query layer over systems of record, at OLTP speed—the live data
foundation for agents, applications, analytics, and compliance.**

[![CI](https://github.com/sparkling/semantic-fabric/actions/workflows/ci.yml/badge.svg)](https://github.com/sparkling/semantic-fabric/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#contributing-and-license)
[![W3C RDB2RDF](https://img.shields.io/badge/W3C%20RDB2RDF-81%2F82%20SQLite%20%C2%B7%2080%2F81%20PostgreSQL-success.svg)](#correctness-and-verification)
[![Rust 1.96](https://img.shields.io/badge/rust-1.96.0-orange.svg)](rust-toolchain.toml)

semantic-fabric is a Rust-native, virtualisation-only OBDA engine. It answers
SPARQL 1.2 by rewriting queries to SQL that runs directly against live
relational databases through R2RML mappings. It has no JVM and keeps no copy of
the instance data: only the ontology `T` and mappings `M` live in the engine;
source rows are streamed on demand and discarded.

The public serving path currently admits **SQLite, PostgreSQL, and MySQL**.
Cloud/REST adapter prototypes are library-only and deliberately not admitted to
`serve` yet. See [Current status](#current-status-and-open-work).

## Why this exists

Operational data is split across systems whose table names and schemas do not
share meaning. Warehouses and ETL provide a common view by copying data, trading
freshness, storage, and operational simplicity for convenience.

semantic-fabric leaves the data in place and exposes it as one virtual,
ontology-shaped RDF graph. Consumers query stable domain concepts rather than
per-system schemas; the source database still performs the set work.

| | Warehouse / ETL | semantic-fabric |
|---|---|---|
| Setup | Pipeline plus duplicate store | Database plus an R2RML mapping |
| Freshness | Last completed load | Live at query time |
| Instance storage | Full second copy | None |
| Query surface | SQL over the copy | SPARQL over the live source |

The architecture is governed by
[ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md),
[ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md), and
[ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md).

## How it works

```text
SPARQL 1.2
    │ parse with Oxigraph/spargebra
    ▼
algebra ── unfold against mappings M + tier-1 T-box saturation
    │ ISWC-2018 base translation and operator-tree normalization
    ▼
relational plan ── dialect SQL + parameters
    │ SQLite / PostgreSQL / MySQL
    ▼
live source ── set work and native spilling
    │ bounded RowStream batches
    ▼
RDF terms or SPARQL results; the A-box is never retained
```

Key properties:

- **Virtualisation only:** no persistent triple store or ETL mode
  ([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md)).
- **One shared executor core:** dialect SQL plus thin native backend adapters
  ([ADR-0024](docs/adr/ADR-0024-executor-backend-abstraction.md)).
- **Bounded memory:** `O(|T| + |M| + batch)` while the database performs and
  spills the set work
  ([ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md)).
- **Correctness before coverage:** unsupported shapes return an explicit
  `501`/`Error::Unsupported`, never a guessed answer
  ([ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)).
- **RDF 1.2 / RDF-star:** native triple terms and reification over live SQL,
  with the basic encoding confined below the visible query surface
  ([ADR-0029](docs/adr/ADR-0029-rdf-star-mapping-extension-rml-star-vocabulary-basic-encoding.md)–[ADR-0032](docs/adr/ADR-0032-rdf-12-soundness-completeness-native-reification.md)).
- **Named graphs:** `GRAPH <g>` and `GRAPH ?g`, including normalized
  subject-map/POM graph unions and exclusion of `rr:defaultGraph` from named
  graph bindings
  ([ADR-0035](docs/adr/ADR-0035-variable-graph-querying.md)).

## Quick start

Prerequisite: the pinned Rust toolchain in `rust-toolchain.toml`.

```bash
cargo build --workspace
cargo run -p sf-cli -- --help
```

The binary has three commands:

| Command | Purpose |
|---|---|
| `serve` | Governed SPARQL 1.2 Protocol endpoint over a live relational source |
| `conformance` | W3C RDB2RDF suite over SQLite with EARL reporting |
| `bench` | GTFS-Madrid OBDA workload over SQLite |

Start the endpoint with an R2RML mapping:

```bash
# SQLite
cargo run -p sf-cli -- serve \
  --source sqlite:/path/to/app.db \
  --mapping /path/to/mapping.ttl

# PostgreSQL
cargo run -p sf-cli -- serve \
  --source 'pg:host=localhost dbname=app' \
  --mapping /path/to/mapping.ttl

# MySQL; prefer an environment-expanded secret rather than a literal in history
cargo run -p sf-cli -- serve \
  --source "mysql://user:${MYSQL_PASSWORD}@127.0.0.1/app" \
  --mapping /path/to/mapping.ttl
```

Optional flags include `--ontology`, `--bind`, `--timeout-secs`,
`--max-query-len`, `--pg-pool-size`, `--pg-pool-wait-secs`, and
`--sqlite-pool-size`. The default endpoint is
`http://127.0.0.1:7878/sparql`.

```bash
curl -s 'http://127.0.0.1:7878/sparql' \
  -H 'Accept: application/sparql-results+json' \
  --data-urlencode 'query=PREFIX gtfs: <http://vocab.gtfs.org/terms#>
    SELECT ?route ?agency WHERE { ?route a gtfs:Route ; gtfs:agency ?agency . }'
```

`GET` and `POST /sparql` are read-only and content-negotiated. SELECT/ASK can
return SPARQL Results JSON, XML, CSV, or TSV. CONSTRUCT/DESCRIBE can return
Turtle, N-Triples, or JSON-LD. Requests are bounded by timeout, query-size,
pool, and cancellation controls from
[ADR-0010](docs/adr/ADR-0010-security-and-resource-governance.md).

## What works today

- SPARQL 1.2 BGPs, joins, OPTIONAL, UNION, MINUS, FILTER, EXISTS/NOT EXISTS,
  BIND, VALUES, subqueries, aggregation, ordering, slicing, and all four query
  forms through the supported plan shapes.
- Property paths: inverse, sequence, alternative, optional, negated property
  sets, and composite `+`/`*` through source-dialect recursive CTEs.
- R2RML plus Direct Mapping, datatype/dialect canonicalisation, templates,
  graph maps, and streamed term reconstruction.
- RDF-star quoted triples in native RDF 1.2 reification form, including path
  joins and named-graph composition.
- SQLite, PostgreSQL, and MySQL execution through the shared `SqlBackend`
  contract. The published W3C figures currently cover SQLite and PostgreSQL;
  MySQL has adapter, endpoint, and live differential coverage.
- A governed HTTP endpoint with streaming, content negotiation, bounded pools,
  request cancellation, and overload shedding.

## Correctness and verification

The load-bearing authority is direct repository evidence, not a model or
harness score:

- W3C RDB2RDF: **81/82 SQLite**, **80/81 PostgreSQL**. The one shared deviation
  is `R2RMLTC0002f`; the per-dialect fixture denominators are documented in
  [ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md) and
  [ADR-0015](docs/adr/ADR-0015-datatype-dialect-correctness.md).
- Differential suites compare flat and operator-tree planners with native
  materialized RDF and spareval across ordinary queries, paths, graphs, and
  RDF-star.
- The 2026-08-26 closeout passed format, clippy with warnings denied, all-target
  build, issue-#8 tests 4/4, differential oracle 7/7, differential tree 178/178,
  workspace tests 1,088 passed with 3 ignored, and conformance with zero
  unexpected failures.
- The versioned engineering harness passes 332 tests across 48 files; one
  environment-specific test is skipped by this provider-free run.

Reproduce the primary gates:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --all-targets
cargo test --workspace
cargo run -p sf-cli -- conformance
```

## Application-completion programme

The issue-independent [SOTA completion programme](docs/plans/sota-application-completion-programme.md),
governed by accepted
[ADR-0038](docs/adr/ADR-0038-sota-application-completion-programme.md), derives
the remaining work from the charter, source, accepted ADRs, tests, CI, standards,
and measured benchmarks. Its verdict is evolutionary, not a compiler rewrite:
make every advertised global operator bounded, then add source identity and a
federated physical plan so the accepted cross-RDBMS charter becomes real.

The first release blockers are silent 256-hop property-path truncation,
source-sized Rust state in some global sort/group/dedup paths, incomplete total
request governance, and a non-reproducible/broad production dependency closure.
A hardened single-source build is an interim release profile; it is not the
charter-complete application while cross-RDBMS federation remains in scope.

## Open-issue remediation closeout

[ADR-0036](docs/adr/ADR-0036-correctness-first-open-issue-remediation.md) and
the [execution plan](docs/plans/open-issues-ruflo-metaharness-implementation-plan.md)
record the complete decisions and evidence.

| Issue | Disposition | Evidence |
|---|---|---|
| [#8](https://github.com/sparkling/semantic-fabric/issues/8) incompatible binding pruning | Closed: every incompatible subject/predicate/object/class/graph bind prunes its branch | `10dedd4`; flat/tree/materialized-oracle regressions; green CI `8b66428` |
| [#9](https://github.com/sparkling/semantic-fabric/issues/9) graph-union wrong results | Closed: normalized subject/POM graph union and default-graph handling across BGP, paths, and RDF-star | `5218874`; W3C and differential regressions; green CI `8b66428` |
| [#10](https://github.com/sparkling/semantic-fabric/issues/10) `rusqlite` link conflict | Closed: one workspace `rusqlite 0.40.2`, preserving `bundled` and `column_decltype`; the MySQL dependency chain is also upgraded | `5b8415c`; `mysql_async 0.37.0`; one `libsqlite3-sys` link target; green CI `8b66428` |
| [#7](https://github.com/sparkling/semantic-fabric/issues/7) cloud backends | Open and deliberately deferred. Only SQLite, PostgreSQL, and MySQL are admitted to `serve` | `9d709dd`; provider-specific protocol/security gates remain |
| [#6](https://github.com/sparkling/semantic-fabric/issues/6) Nova collaboration | Closed: federation/materialization pilots work without exposing raw plans; the optional fallible early-exit sink remains consumer-driven | No speculative public API added; green CI and Pages `8b66428` |

The `mysql_async 0.37.0` upgrade resolves the `lru 0.16.4` unsoundness warning,
and a fresh resolution selects fixed `h2`. `cargo audit` is a blocking CI gate
and currently passes its configured policy. That policy has six documented
exceptions: two `quick-xml` denial-of-service advisories in the pinned RDF/XML
stack, three `rustls-webpki` advisories in the SQL Server-only `tiberius` path,
and `RUSTSEC-2026-0235` in an unused optional `rust_decimal` archive feature.
They are accepted exposure, not closure, and ADR-0038 requires owner/expiry/
reachability evidence and removal when upstream constraints permit. Cloud
adapters are not relabelled production-ready merely because mocked happy paths
exist.

## Engineering MetaHarness status

`coding-harness/` is a private, development-only Ruflo MetaHarness control
plane governed by
[ADR-0037](docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md). It
uses native ChatGPT/Codex and Claude Code subscriptions, isolated candidate and
evaluator worktrees, frozen inputs, exact-origin egress, bounded repair,
independent review, provider-free QE/SAST, and digest-chained receipts. Native
subscription calls have no artificial dollar ceiling, but retain task, turn,
time, output, concurrency, rate-limit, and receipt bounds. The harness has no
commit, push, publication, deployment, or promotion authority and never uses
provider API keys or OpenRouter.

The frozen issue-#8 path remains a schema-v2 `exact-reference` transaction. The
reusable foundation now also parses schema-v3 `verifier-only` tasks and derives
their admitted paths, generated outputs, commands and QE evidence from protected
task data through a versioned Rust runtime binding. Commit `b40dbc6` adds the
strict, externally anchored schema-v5 replay-policy foundation while preserving
the frozen v4 parser and delegating unambiguous schema-v4 bytes to it unchanged.
Schema v5 is still disabled: its evaluator, envelope and trusted-launcher
emitter must land in H0b/H0c before another manifest task can produce accepted
programme evidence.

The first sealed issue-#8 programme transaction was **honestly rejected**. Run
`issue8_dual_native_20260826_29` admitted the model patch only to `unfold.rs`,
then exhausted one post-admission verifier-directed repair: 40/100 against the
required 98, hard gates failed, no fitness eligibility. The independently
implemented product fix and direct tests remain valid; they are not substituted
for a passing sealed model transaction. OIA/MCP and persisted ADR-graph
visibility remain `INCONCLUSIVE` where the active surfaces could not be read.

Darwin/GEPA remains disabled until at least five discriminating training tasks
and five sealed holdouts exist. No diagnostic score can override a failed
product oracle.

The separate Ruflo retrieval-policy flywheel is also **off by default**. A
48-task, ADR-derived candidate relevance benchmark with balanced deterministic
halves and a canonical SHA-256 pin is tracked under `.claude/eval/` and protected
by the harness. It still requires maintainer label review and calibration against
a live retrieval baseline before activation. The 2026-08-27 operational check
found no flywheel opt-in variables or `harness` worker in the live daemon; that
runtime fact must be rechecked after every restart. The explicit evaluation path
validates the anchor but currently reports `store too small to harvest a corpus`:
Ruflo's flywheel-visible `neural_patterns` store is empty even though older
learning counters and ReasoningBank files contain history. Those stores are not
silently conflated or seeded from benchmark labels.

Eight owner-visible records with four harvestable are only enough to begin an
evaluation; they do not establish production readiness. Once the store,
non-fallback embedding provider, immutable snapshot, and reviewed benchmark are
ready, an operator may run a model-call-free, local evaluation-only trial. It
cannot apply a policy; a separate confirmed promotion requires an amended
project decision, a trusted Ed25519 key, frozen gates, sequential evidence,
stale-head protection, durable receipt retention, and ledger compare-and-swap.
Background generation remains
prohibited because Ruflo 3.38.20 does not route that daemon path through the same
transaction. A replayable receipt verifies signatures, lineage, the frozen gate,
and its decision over sealed scores—it does **not** re-run the benchmark.

GitHub-hosted CI runs the portable harness contract on its pinned Node 20
baseline. Native integration remains fail-closed and is run only by the manually dispatched
`harness-native` workflow on a labelled `self-hosted`, `bwrap`, `systemd-user`
runner; hosted runners cannot create the network namespace the production
isolation contract requires.

## Benchmarks

The reproducible methodology and full caveats live in
[BENCHMARKS.md](BENCHMARKS.md) and [COMPARISON.md](COMPARISON.md).

The strongest measured invariant is constant engine heap during a streamed
CONSTRUCT dump over a file-backed SQLite source:

| Scale | Triples | Peak engine heap | Bytes/triple |
|---:|---:|---:|---:|
| 1× | 5,200 | **129,358 B** | 24.88 |
| 10× | 51,880 | **129,358 B** | 2.49 |
| 100× | 518,680 | **129,358 B** | 0.249 |

The peak is byte-identical across 100× source/result growth. The published
Ontop 5.5.0 comparison uses the same PostgreSQL source and warm HTTP endpoints;
semantic-fabric wins the measured Q1–Q7 cells except one tie within noise, while
the report preserves the pre-optimization Q5 loss and avoids a blanket speed
claim. It is a small localhost workload, not a production sizing result.

## Current status and open work

| Area | Honest status |
|---|---|
| Serving | Working read-only SPARQL 1.2 Protocol endpoint over SQLite, PostgreSQL, and MySQL |
| Cloud/REST adapters | Prototype/library-only; Databricks, AWS Athena, Snowflake, BigQuery, Trino/Presto and other adapters are not admitted to `serve` |
| Property paths | Broad support; explicit `501` residuals remain for bound-endpoint, nested-closure, shape-mismatched, and some reflexive composite forms |
| Named graphs | `GRAPH <g>` and `GRAPH ?g` work; a path under `GRAPH ?g` remains unsupported when mappings contain dynamic graph maps |
| Federation | Cross-RDBMS planning is in scope but not implemented: the current runtime owns one source and the semi-join planner has no production caller. External SPARQL `SERVICE` remains excluded |
| Materialization | Not a product mode. A one-off streamed dump uses the query/execution path; Nova owns its downstream bulk-load adapter |
| Exactness and boundedness | Recursive closures silently stop at 256 hops; some global ORDER/GROUP/DISTINCT/CONSTRUCT paths retain source-sized Rust collections. Both are release blockers in ADR-0038 |
| Production hardening | Reliability, security, operability, lifecycle and packaging have graduated from proposed ADR-0014 into the sequenced ADR-0038 programme |
| Accepted designs not wired | Observability/configuration (ADR-0011), property/fuzz/snapshot testing (ADR-0012), query-time provenance (ADR-0017), and the security edge (ADR-0018) |
| Dependency security | `cargo audit` passes its configured gate; six documented advisory exceptions and three unmaintained-crate warnings remain release debt. The application `Cargo.lock` is currently ignored, so clean resolutions are not yet reproducible |

Unsupported shapes are designed to fail explicitly. The current 256-hop path
truncation violates that invariant and is release-blocking until fixed.

## Workspace

| Crate | Role |
|---|---|
| `sf-core` | Shared mapping IR, RDF terms, graph-map semantics, datatypes |
| `sf-sql` | Dialects, source adapters, typed binding, schema introspection |
| `sf-mapping` | R2RML and Direct-Mapping parsing into the core IR |
| `sf-sparql` | SPARQL algebra unfolding, normalization, SQL emission, execution |
| `sf-conformance` | W3C, differential, mutation, and EARL evidence |
| `sf-bench` | GTFS-Madrid and constant-memory benchmarks |
| `sf-serve` | Governed SPARQL 1.2 Protocol HTTP endpoint |
| `sf-cli` | `serve`, `conformance`, and `bench` binary |

## Architecture decisions

The canonical [ADR corpus](docs/adr/) contains 35 records: 33 accepted, one
proposed ([ADR-0014](docs/adr/ADR-0014-production-hardening-backlog.md)), and one
superseded ([ADR-0030](docs/adr/ADR-0030-metaharness-darwin-mode-dev-process-adoption.md),
replaced by ADR-0037). ADRs are living plans and must be updated with the code.
`accepted` means the decision is adopted; the dated implementation-status note
and direct evidence say whether it has shipped.

| Area | Records |
|---|---|
| Charter, substrate, conformance, execution, rewriting, reasoning | ADR-0001–0008 |
| Governance, tests, datatype correctness, provenance, security, readiness | ADR-0010–0019 |
| Optimisation, Ontop parity, operator-tree IR, backend abstraction, QE | ADR-0020–0028 |
| RDF-star mapping/query, path joins, set/graph semantics | ADR-0029, ADR-0031–0035 |
| Remediation, engineering control plane, application completion | [ADR-0036](docs/adr/ADR-0036-correctness-first-open-issue-remediation.md), [ADR-0037](docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md), [ADR-0038](docs/adr/ADR-0038-sota-application-completion-programme.md) |

Research grounding and prior-art reviews are under
[`docs/research/`](docs/research/). RDF-star has a normative
[specification](docs/rdf-star/specification.html) and practical
[guide](docs/rdf-star/guide.html).

## Contributing and license

Before opening a pull request, run the primary gates from
[Correctness and verification](#correctness-and-verification). Architectural
changes must update or add an ADR in the same commit.

semantic-fabric is dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option.
