# Claude programme handover - 2026-09-25 (pre-harness-refactor)

## Start here

The v1 completion programme is **not complete**. Nothing is running: no loop,
cron job, background build, container or spawned agent. The user paused work
("pause") and asked for this handover before a harness refactor.

This handover continues `docs/plans/session-handover-2026-09-23.md`; read that
first for the standing constraints, which are unchanged. In short: native
subscription only (no API keys, no OpenRouter, no budgets); Claude only, Codex
paused; one writer on `main`, no branches or worktrees; no push, tag, deploy or
publish without an explicit instruction; never touch other projects' containers;
ADR and ledger files stay under 500 lines.

**Harness refactor.** While work was paused, four ADR commits (`3d958545`,
`84304bbd`, `ce06c267`, `6b4a7c0b`, 2026-09-24/25) added
`docs/adr/ADR-0057-repair-the-native-development-harness.md`. It is
**Proposed**, not accepted, and states that it authorizes no implementation and
no scheduler restart. It plans an F0-F4 repair of `coding-harness/`: frozen
Claude-only admission tests (F0), contracts/runtime/workflow changes (F1),
workspace safety fixes (F2), an integrated reviewed commit (F3), then one
product outcome (F4). Do not start it until the user accepts or directs it.
Note that it names `__tests__/delivery-runtime.test.ts` and
`src/delivery-contracts.ts`, both changed by `3801980b` below (the `cargo
nextest run` allowlist); the refactor must keep or deliberately replace that.

## Commits since the 2026-09-23 handover (none pushed)

| Commit | What |
|---|---|
| `6cf56bb6` | Reload never republishes a verified source unverified (G3) |
| `02d77f13` | Ordinary authored MySQL uses the protected generation (G3); `startup.rs` split |
| `0f2902fc` | Clearer assertion in the ordinary MySQL live test |
| `3d958545`..`6b4a7c0b` | ADR-0057 harness repair proposal (not from this session) |

Earlier in the same session, and already in the previous handover:
`4bcdeb8c` ordinary PostgreSQL, `3801980b` nextest and slice scripts,
`a38900ed` the 2026-09-23 handover.

## User decisions this session

1. **Reload downgrade: "Refuse."** Once a source is admitted verified, a
   refresh whose protected builder declines is refused with `CapabilityDrift`;
   readiness stays 503 until a verified candidate or restart.
2. **Speed-ups: "do them all", then "set max parallel to 1".** At most one
   spawned agent at a time (Claude memory `max-parallel-one`).
3. **SQLite prefetch fix: "Only when stopping early."** Keep the worker's
   one-row read-ahead for queries that drain every row (SELECT, CONSTRUCT);
   remove it only for queries that stop early (ASK). **Not yet implemented.**
4. **"pause"**, then this handover.

## What the committed G3 slices prove

- **Reload no-downgrade (`6cf56bb6`).** `reload/baseline.rs::Attempt::observe`
  refuses a verified-to-unverified observation. Test
  `reload::tests::verified_source_never_reloads_unverified` adds an FTS5 table
  after a verified SQLite start and asserts refusal across two refreshes;
  disabling the guard fails it.
- **Ordinary MySQL (`02d77f13`).** `startup/ordinary.rs::try_verified` now holds
  all three backend attempts (SQLite, PostgreSQL, MySQL) behind one fallback.
  MySQL is gated only on `permits_verified_generation()` (it has no native RLS).
  Live test `ordinary_authored_mysql_refuses_replaced_table` runs the actual TLS
  binary on the pinned `mysql:8.4.11` image with reload disabled and refuses a
  same-name table replacement with 503. Disabling the MySQL arm fails it:
  it then answers from the replaced table.
  Run:
  `cargo test --locked -p sf-cli --no-default-features --test source_tls_live authored_generation::mysql::ordinary::ordinary_authored_mysql_refuses_replaced_table -- --ignored --exact`
  (the fixture owns and removes its container).

G3 remains open for sources outside each protected profile, native-RLS ordinary
generations and the other backend/policy DDL guarantees in the ledger row.

## Parked work: SQLite prefetch race (G1)

Harness task `g1-sqlite-demand-driven-rows-20260923` is **paused** (reason
recorded). Its uncommitted work is in the named stash
`g1-prefetch-demand-wip-20260923` (also `/tmp/sf-stage/prefetch-wip.patch`,
which does not survive a reboot). It contains the **always-exact** version the
user did not choose:

- `crates/sf-sql/src/backend/sqlite/owned.rs`: a demand channel; the worker
  calls `demanded.blocking_recv()` before `rows.next()`, and `next_row` sends
  a demand before `recv`.
- `crates/sf-serve/tests/query_budget/reconstruction.rs`: ASK pinned exactly at
  `ASK_EXACT_SOURCE_WORK = 27_840` with an exact/N-1 boundary.
- `crates/sf-sql/src/backend/sqlite/owned_demand_tests.rs`: a throwaway 200k-row
  throughput probe (`#[ignore]`); do not commit it as is.

Measured on this host: ASK 27,840 in 10/10 runs; with reconstruction's control
disconnected ASK falls to 27,776, so the pin remains a valid witness. Reverting
the demand gate made the exact pin fail 5/8 runs. Throughput for 200k rows went
from about 1.5 s to 2.0 s, which is why the user chose "only when stopping early".

Remaining step: pass an "early stop" signal from the ASK entry points
(`crates/sf-sparql/src/exec.rs` `ask_sqlite_owned*`, around line 257) through
`SqlBackend::open_branch_with_*` (`crates/sf-sql/src/backend.rs` ~178-209) to
the owned worker, gate on demand only in that case, keep read-ahead otherwise,
re-measure throughput (full reads must match baseline), re-pin ASK, and re-check
`sqlite_control_identity.rs` and other pinned totals. Restore with
`git stash apply stash@{0}` after confirming the stash name, or re-implement.
The task scope already lists these files plus the catalogue.

## Other open items

- **G1:** condition rendering bounded independently of parser isolation;
  oversized rendering modules; G1d re-qualification. The check list is in
  `/tmp/sf-stage/g1d-checks.json` (all 15 targets exist). `endpoint` carries 2
  pre-existing failures (`sqlite_pool_concurrency_receipt`,
  `column_names_spawn_blocking_deadlock_regression`); decide how to treat them.
- **G4-G6:** not started this session.
- **Working tree at handover:** three hook-managed `.claude/helpers/*` files are
  modified by the Ruflo version monitor, not by this session. They are left
  alone, like the older `helper-hook-state-out-of-g1c-scope` stash (now
  `stash@{1}`).

## Lessons from this session

- A first reviewer approved an ADR sentence claiming a fence prevents a reload
  downgrade; tracing `publish` showed it does not. Verify a reviewer's reasoning
  against its verdict when they disagree.
- Build-setting speed-ups were measured, not assumed: `cargo nextest` halved
  `sf-serve` library test time; a faster linker, lighter debug info and
  sccache gave nothing here.
- `scripts/slice.sh` drives a slice end to end; it submits responses with the
  request digest, so run it only when the tree matches that digest (submit by
  hand with the live digest after local edits).
- Keep split files under 500 lines; `startup.rs` had reached 663.
