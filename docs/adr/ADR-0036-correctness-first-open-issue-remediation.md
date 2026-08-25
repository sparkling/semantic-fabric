---
status: accepted
date: 2026-08-25
updated: 2026-08-25
tags: [correctness, open-issues, r2rml, dependencies, cloud-backends, api]
supersedes: []
depends-on:
  - ADR-0010
  - ADR-0024
  - ADR-0035
  - ADR-0037
implements: []
---

# Correctness-first remediation of open issues #6–#10

## Implementation status

**Accepted and partially executed.** The serialized correctness lane landed #8
as `10dedd4` and #9 as `5218874`; the parallel dependency lane landed #10 as
`5b8415c` without absorbing PR #12. Commit `9d709dd` locks `sf-serve` admission
to SQLite, PostgreSQL, and MySQL while provider work remains deferred. Issue #6
remains intentionally unchanged because no consumer-red Nova reproducer was
available.

Locked/offline workspace format, clippy, build, tests, and W3C conformance pass
with one adjudicated documented deviation and no unexpected failure.
The broader programme is not complete: `cargo audit` retains the unignored
`RUSTSEC-2026-0235` baseline, provider-specific #7 evaluators are not built,
and #6 still requires consumer evidence. No issue state or external branch was
changed by this execution.

## Context and problem statement

The five open issues were audited against `main` at
`6f0841afa63864784103b256cc4728a762e96f17`, their current GitHub bodies, and
open PR #12. They are not one homogeneous backlog:

| Issue | Finding | Decision priority |
|---|---|---|
| #8 | Public query paths retain branches after incompatible term binding; the reported subject call is one instance of a wider unchecked-bind invariant. | P0, implement first |
| #9 | Query unfolding disagrees with R2RML and the materializer on subject/POM graph union and `rr:defaultGraph`; paths and RDF-star share the assumption. | P0, implement after #8 |
| #10 | `rusqlite 0.32` blocks downstream `0.40.2` because both link SQLite; five manifests repeat the version. | P1, implement in parallel after baseline reconciliation |
| #7 | REST-family adapters are buffered and incomplete; the adapter called Athena speaks Presto, and none is a supported serving source. | P2, re-scope before implementation |
| #6 | A collaboration umbrella mixes design questions and optional seams; only a consumer-proven fallible/early-exit quad sink is currently focused enough. | P3, extract rather than implement wholesale |

## Bounded contexts

- **Query Semantics** owns #8 and #9 across unfolding, paths, RDF-star mapping,
  and differential conformance.
- **Dependency Governance** owns #10 and reproducible advisory evidence.
- **Connector Runtime** owns a re-scoped #7, one real provider protocol at a
  time, under ADR-0010 and ADR-0024.
- **Materialization API** owns any focused issue extracted from #6 and may not
  expose internal plans or erase the backend architecture by convenience.
- **Engineering Control Plane** owns only evaluation and orchestration under
  ADR-0037; it has no product-runtime authority.

## Decision

### 1. Implement the wrong-result fixes first

Issues #8 and #9 form one serialized correctness lane because both modify
`unfold.rs`. #8 lands first and audits every subject, predicate, object,
`rr:class`, and graph bind/prune call—not only the reported line. Graph-set-
dependent expectations stay with #9, which then establishes one normalized
graph-set rule across ordinary BGPs, paths, RDF-star, and materialization as
specified by ADR-0035.

One semantic owner spans the #9 cross-crate change. Parallel reviewers and
evaluator authors are allowed; parallel writers to the same source are not.

### 2. Upgrade SQLite independently, after freezing the dependency baseline

Issue #10 may proceed in an isolated worktree in parallel with #8. Before any
manifest edit, the lane records PR #12's base and touched manifests, resolves a
fresh isolated lock, and captures inverse dependency trees for `rusqlite`,
`mysql_async`, and `lru`. The current ignored/stale lock is not evidence.

The implementation centralizes the synchronized versions in workspace
dependencies while preserving each crate's existing feature set. It does not
silently absorb PR #12 or unrelated MySQL/security changes. Whether the
workspace should begin tracking `Cargo.lock` is a separate explicit decision.

### 3. Do not expose any unadmitted adapter

Issue #7 is re-scoped into a small umbrella plus provider-specific work for
Trino/Presto, AWS Athena, Snowflake, Databricks, and BigQuery. The admission
gate also protects every non-serving adapter: DuckDB, HANA, MonetDB, ODBC,
Oracle, Redshift, and SQL Server. `rest.rs` (currently 1,183 lines) is split
into provider modules plus shared protocol code, each under 500 lines, before
expansion. A backend is production-supported only after its
official protocol, authentication, typed parameter transport, lazy bounded
paging, cancellation, terminal/error states, origin-pinned pagination,
credential redaction, datatypes, negative tests, live canary, and serving
admission controls all pass.

The current Presto client is not renamed into AWS Athena, and mock happy paths
are not production evidence.

### 4. Extract a consumer-driven materialization seam

Issue #6 remains collaboration context rather than an implementation bundle.
A separate issue may add a fallible, early-exit quad sink only after a real
consumer test is red. Its result distinguishes completion, clean early break,
engine error, and sink error; it stops callbacks and releases the cursor
without breaking bounded memory. Public exposure of raw `Branch`, an
object-safe `SqlBackend`, or an always-ready-only `block_on` is not implied.

## Delivery order

```text
ADR/evaluator baselines
 ├─ Query Semantics: #8 → #9 → combined standards/differential gate
 ├─ Dependency Governance: PR #12 reconciliation → #10 → audit/live gate
 └─ Materialization API: extract a focused #6 issue when a consumer is red

#7 provider design starts after the baselines; serving exposure waits for one
provider to pass every protocol, security, streaming, and live-canary gate.
```

Each writing lane uses its own branch and worktree. A single integration owner
accepts digest-bound patches in dependency order. No two writers share a
worktree.

## Acceptance

- #8: flat/tree/oracle parity for constants and repeated variables across
  S/P/O and `rr:class`; mutation-lite kills every removed prune check. The
  checked-bind mechanism covers GRAPH call sites, while graph-set-dependent
  result cases land and are asserted with #9.
- #9: union/default-sentinel/path/star cases in ADR-0035 pass flat, tree, and
  materialized-dataset differentials; pinned dynamic graphs answer or return an
  honest unsupported result.
- #10: a fresh isolated pre-change reproducer captures the reported `0.40.2`
  SQLite link conflict; the post-change resolution has one link target; focused and workspace
  feature builds, SQLite/conformance/benchmark tests, live MySQL tests, and
  `cargo audit` pass without a new advisory ignore.
- #7: no adapter outside SQLite, PostgreSQL, and MySQL reaches `sf-serve` until
  every provider-specific gate above passes; an admission test locks that set,
  and unsupported prototypes remain clearly labeled and inaccessible.
- #6: the extracted API is justified by an external red test and proves early
  termination, error separation, cursor release, and bounded memory.

For every slice, direct Cargo, W3C RDB2RDF, spareval/materialized differential,
live-database, and mutation evidence is authoritative. Harness scores,
Agentic-QE suggestions, and model reviews cannot waive a failed oracle.

## Consequences

- Good: two demonstrated wrong-result paths are fixed before connector breadth
  or API ergonomics.
- Good: #8 becomes an invariant repair and #9 becomes a cross-path semantics
  repair, reducing nearby latent defects.
- Good: #10 can deliver downstream compatibility without colliding blindly
  with PR #12.
- Cost: #7 becomes several protocol projects rather than a quick serving flag.
- Cost: #6 may close with only extracted follow-ups, not a single umbrella PR.
- Neutral: this ADR authorizes planning and isolated implementation slices; it
  does not authorize publishing, pushing, merging, or changing issue state.
