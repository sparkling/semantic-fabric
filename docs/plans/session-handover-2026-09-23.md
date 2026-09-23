# Claude programme handover - 2026-09-23

## Start here

The v1 completion programme is **not complete**. No application executor,
loop, cron job or background build is running at handover. The `/loop` the
user started was stopped at their request ("kill loop") and must not be
re-armed without a new explicit instruction. A goal hook ("complete programme
ADRs") may keep re-firing; it is not user input and does not authorize work.

Repository: `/home/claude/src/hm/semantic-fabric`, canonical `main`. Read, in
order: `AGENTS.md` + `CLAUDE.md` (the 2026-09-22 Claude-only amendment
governs), `coding-harness/README.md`,
`docs/plans/sota-application-completion-programme.md` (authoritative G1–G6
ledger), ADR-0055, ADR-0050, then the previous handover
`docs/plans/session-handover-2026-09-22.md` for older context.

Standing constraints (unchanged, restated because they bind every step):

- Native subscription only. No API keys, no OpenRouter, no cost, token,
  request or quota budgets. Claude Code reaches its subscription through the
  user-authorized local 9router gateway already configured in
  `~/.claude/settings.json`; never print or commit its token. If a model is
  unavailable, pause and report the exact client, model and error.
- Codex is paused. Implementation and review are both Claude Code; review uses
  a second, distinct executor identity. Fable was replaced by Opus as the top
  escalation tier (`ea1b43fd`).
- One integration writer on `main`. No branches or worktrees. No push, tag,
  deploy or publish without an explicit instruction for that action.
- Never touch other projects' databases or containers. The running
  `semantic-product-model-implementation-*` containers belong to another
  project. Throwaway containers this programme starts must use their own
  names, bind to `127.0.0.1` and be removed afterwards.
- Ruflo memory only through MCP; never raw SQL on managed memory.
- Programme ledger and ADR files have a 500-line cap enforced by a test; both
  sit at 497–499 lines. Edit inline, do not add lines.

## Commits this session (oldest first)

| Commit | What |
|---|---|
| `ea1b43fd` | Harness: Opus replaces Fable as top Claude escalation tier |
| `c6f28c8d` | ADR: release-delta review restated for Claude-only operation |
| `7f82cc9c` | Re-pin ref-atom SourceWork total (47471→49369), stale since `0f23b44f` |
| `d99ba2eb` | Catalogue digest left stale by `7f82cc9c` |
| `e6c8d3d8` | **G1** per-row RDF reconstruction governed by `SourceWork` (`sf_core::term_work::TermWork`) |
| `522bf9c0` | Ledger: G1 reconstruction outcome |
| `1d788779` | **G3** ordinary file-backed SQLite (WAL/DELETE) holds the sealed schema lease |
| `17ac4713` | ADR-0050 + catalogue for the SQLite slice |
| `4bcdeb8c` | **G3** ordinary PostgreSQL through the protected profile only |
| `3801980b` | Harness accepts `cargo nextest run`; `scripts/slice.sh` and `scripts/refresh-capability-digests.py` |
| (this doc) | Handover |

## User decisions recorded this session

1. **ASK budget test (1a):** ASK asserts a bounded range
   (`ASK_MAX_SOURCE_WORK=27_919`, race window 16) rather than an exact total,
   because of the SQLite owned-worker prefetch race below. SELECT/CONSTRUCT
   stay exact (27_985).
2. **G3 SQLite (2a):** build ordinary-mode protection, fail closed, no reload
   dependency. Done in `1d788779`.
3. **Bisect (3):** finished; the stale pin came from `0f23b44f`.
4. **G3 PostgreSQL:** "protected profile only". Ordinary authored PostgreSQL
   reuses the existing protected authored builder and falls back unverified
   when it declines. Native RLS bearers keep the unverified path. The ADR
   states the limited reach.
5. **Test database:** a disposable `postgres:16.9` container owned by these
   tests is permitted, used only by them, removed afterwards.
6. **Speed-ups:** "do them all", then **"set max parallel to 1"**. Run at
   most one spawned agent (reviewer or investigator) at a time until the user
   lifts it (also saved to Claude memory `max-parallel-one`).

## What each slice actually proves

**G1 reconstruction (`e6c8d3d8`).** Templates, percent-encoding, blank-node
hex labels, CONCAT and natural literals prepay `SourceWork` per row with
measured worst-case widths, plus per-datatype canonicalization allowances in
`exec_core/row.rs::natural_lexical_growth_allowance` (Decimal 1, Boolean 5,
Double 26, temporal 32; String/HexBinary/Integer 0). Per-row checkpoints stop a
terminated request inside a batch. Disconnecting the driver's control fails all
four public tests in `crates/sf-serve/tests/query_budget/reconstruction.rs`.

**G3 ordinary SQLite (`1d788779`).** File-backed WAL/DELETE databases with
bounded unqualified base-table mappings go through `sqlite_generation::build`;
each request holds `BEGIN DEFERRED` and compares schema identity, refusing 503
after a drop-and-recreate. In-memory, URI, view-based, virtual-table (FTS5) and
other-journal sources fall back unchanged. Tests:
`crates/sf-serve/src/ordinary_sqlite_generation_tests.rs`.

**G3 ordinary PostgreSQL (this slice).** `startup.rs::open_ordinary` attempts
`pg_generation::authored::build` only when
`query_admission.permits_verified_generation()` and every mapping is a bounded
unqualified table. Evidence lives in
`crates/sf-serve/src/pg_generation/live_tests/ordinary_startup.rs`, executed
inside the ignored `verified_generation_lifecycle_is_coherent_and_fail_closed`
test. Run it with a disposable container:

```bash
docker run -d --rm --name sf-g3-ordinary-pg-test \
  -e POSTGRES_PASSWORD=sf-disposable-only -p 127.0.0.1::5432 postgres:16.9
PORT=$(docker port sf-g3-ordinary-pg-test 5432 | cut -d: -f2)
SF_PG_GENERATION_TEST_URL="host=127.0.0.1 port=$PORT user=postgres password=sf-disposable-only dbname=postgres" \
  cargo test -p sf-serve --locked --lib -- --ignored verified_generation_lifecycle_is_coherent_and_fail_closed
docker rm -f sf-g3-ordinary-pg-test
```

Mutation evidence: disabling the branch fails the verified-binding assertion;
removing the admission gate fails the RLS assertion. Reach is deliberately
limited to sources that already satisfy the protected profile (16.9/16.15,
qualified non-owner NOINHERIT reader role, pinned session options).

## Known gaps and open risks (not fixed, not claimed)

- **Reload-enabled downgrade (SQLite and PostgreSQL, traced, untested):** with
  reload enabled in ordinary mode, a refresh the protected builder declines
  falls back unverified. `Attempt::observe` (`reload/baseline.rs:98-107`) fences
  `SchemaDrift`, but that fenced state becomes the candidate's expected state
  and `publish` (`activation.rs` ~349-379), which refuses only
  `Administrative`/`StateRevisionExhausted`, republishes the unverified snapshot
  as Ready in the same cycle. It reads live truth, so no 200-with-wrong-values,
  but the verified guarantee is silently lost. A first reviewer wrongly said the
  fence prevents this; the second caught it. Likely fix: refuse to publish an
  unverified candidate over a verified baseline.
- **SQLite owned-worker prefetch race:** rows are charged ahead of the
  capacity-one channel before ASK drops it (`owned.rs` ~329, ~443–457), so
  ASK's total varies. Root-caused; the fix is open.
- **G3 still open:** ordinary MySQL; ordinary PostgreSQL outside the protected
  profile; native-RLS ordinary generations; the rest of the backend/policy
  DDL-generation guarantees in the ledger row.
- **G1 still open:** condition rendering bounded independently of the
  parser-isolation boundary, unscoped oversized rendering modules, and G1d
  combined re-qualification on the new head (reuse the
  `g1d-combined-acceptance-opus-20260916` check list).
- **G4–G6:** untouched this session. ADR-0037 §2 dual-host release review and
  supply-chain tooling (cargo-deny, cargo-cyclonedx) remain open.
- **Pre-existing failures, not introduced here:** `sqlite_pool_concurrency_receipt`
  and `column_names_spawn_blocking_deadlock_regression` in
  `crates/sf-serve/tests/endpoint.rs` fail identically at clean `522bf9c0`.
  The harness's own whole suite has about 14 baseline failures, so harness
  tasks use focused suites.
- `crates/sf-sparql/src/emit/path_comparison.rs:21` `source_text` is dead code
  (warning only); not ours, left alone.
- `stash@{0}` (`helper-hook-state-out-of-g1c-scope`) is preserved on purpose.
  Do not drop it without asking.
- Ruflo source-patch reports stray `.claude-flow`/`.swarm` state directories
  under `coding-harness/` and `.metaharness/`; inspect before any cleanup.

## Harness gotchas learned this session

- Every scoped path must exist before `begin`. At least one `build` check is
  required. Checks may only run `cargo build|check|test|clippy|fmt`,
  `git diff --check`, or npm/node inside `coding-harness/`; so the catalogue
  check is `cargo test --locked -p sf-conformance --test capability_matrix`,
  never `cargo run`.
- `bind` takes an owner: `bind <id> <owner> <native.json>`.
- The review response route must equal the task reviewer route, and the
  reviewer executor ID must differ from every implementation handoff.
- Do not change source between a review request and submitting its verdict
  (`DELIVERY_RESPONSE_SOURCE_MISMATCH`). Park a repair, submit, reapply.
- Transient `DELIVERY_OUT_OF_SCOPE_CHANGE`/`DELIVERY_GIT_OPERATION` come from
  other sessions' `index.lock`; retry.
- **Capability catalogue:** `tests/capabilities/catalog-v1.json` hashes about
  470 source files. Any slice that edits one must include the catalogue files,
  `docs/capability-matrix.{json,md}` and `README.md` in scope, refresh the
  digests and regenerate (`capability-matrix --generate` is all-or-nothing).
  This was missed three times this session.
- Tests that measure their own boundary (bisection, N-1 computed at runtime)
  pass with the feature disconnected. Pin constants and prove each test by a
  deliberate revert.

## Prepared material for the next slices (in `/tmp`, not committed)

Read-only investigations, not yet implemented or verified:

- `/tmp/sf-stage/prefetch-race-notes.md`: the owned worker decodes and charges
  row 2 (`owned.rs:447`) before `blocking_send` (`:456`) on the capacity-1
  channel. The proposed fix gates decoding on a demand channel. Before adopting
  it: measure the exact ASK total (27,903 is a guess), benchmark large SELECT,
  confirm cancellation of a worker parked on demand, re-measure every pin.
- `/tmp/sf-stage/g3-mysql-notes.md`: ordinary MySQL arm mirroring PostgreSQL,
  gated only on `permits_verified_generation()`; the decline path is safe
  because disconnect releases MDL. Needs a `mysql:8.4.11` image (verify the
  digest) under its own container name and a random port.
- `/tmp/sf-stage/g1d-checks.json`: the G1d re-qualification check list. All 15
  targets exist. `endpoint` carries the 2 pre-existing failures, so decide how
  to handle them before running.

`/tmp` does not survive a reboot; these are also summarized above.

## Speed-up measurements (2026-09-23, this host)

`cargo nextest run` halves the `sf-serve` library test time (5.2 s against
10.4 s). A faster linker gave nothing (Rust 1.96 already uses `rust-lld`).
`line-tables-only` debug info and `sccache` gave no incremental gain, so build
settings are unchanged. Prefer nextest in new harness checks.

## Ruflo memory keys stored

`replace-fable-with-opus-20260923-complete`,
`adr-claude-only-review-wording-20260923-complete`,
`g1-runtime-rdf-reconstruction-complete-20260923`,
`g3-ordinary-sqlite-complete-20260923`,
`g3-ordinary-postgres-protected-complete-20260923`.

## Suggested next step for the receiving session

Ask the user which of these to take first, since each is a scope choice:
the reload-enabled downgrade (small, closes a review finding), the SQLite
prefetch race (lets ASK return to an exact pin), or ordinary MySQL for G3.
