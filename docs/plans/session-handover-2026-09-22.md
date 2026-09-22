# Claude programme handover - 2026-09-22

## Start here

User requested this handover to continue in Claude. Programme is **not complete**.
No application executor is running after this handover. Start continuation
explicitly in the receiving Claude session; do not infer a background build.

Repository: `/home/claude/src/hm/semantic-fabric`, canonical `main`.
Application HEAD at handover: `f71a29b913c2f445e9bfd4520e33cf7ad592647e`.
Working tree was clean before adding this document. A subsequent documentation
commit contains this handover; the application evidence remains bound to f71a29b9.

Read, in order:

1. `AGENTS.md` and `CLAUDE.md` (current Claude-only amendment governs).
2. `coding-harness/README.md` (mandatory delivery lifecycle).
3. `docs/plans/sota-application-completion-programme.md`, authoritative G1-G6 rows.
4. ADR-0055, ADR-0050, and this document's outstanding-work sections.

User authorized completing the programme, resolving blockers and committing all
existing scoped work. Latest request transfers execution to Claude. No push,
publication, deployment, history rewrite or unrelated database changes authorized.
Do not resume work in this old coordinating host after handing over.

## Executor, transport and ownership

- Claude only for implementation and independent review; Codex remains paused.
- Coordinator/normal implementation: Claude Code Sonnet high. Use Opus high for
  named unresolved correctness questions, such as G3 authority proof.
- Existing Claude CLI: `/home/claude/.local/bin/claude`.
- User explicitly authorized configured local 9router Claude subscription
  transport. Existing Claude settings use loopback port 20128 and `cc/claude-*`
  aliases. Preserve configured transport; do not copy, print or commit tokens.
- Never inspect credential values. No provider API keys, OpenRouter fallback,
  usage budgets or provider-quota routing.
- One integration writer on `main`; no new branches/worktrees. Independent
  read-only research may run concurrently. Keep source stable during formal
  checks/review; serialize heavy builds, fixtures and Git operations separately.
- Existing native conversation can continue as sole writer. A tracked Ruflo
  record or delivery request does not launch an agent.

## Verified commits

| Commit | Outcome | Delivery state |
| --- | --- | --- |
| `791725fb1bc821f4e8a06cbf8abc82ca2177a5cb` | Claude-only harness/instructions restored from historical behavior | Original task superseded by completed PG successor; original finish refusal retained |
| `58796ff22de1201c099c8a0bf9fc41cbeb7b709e` | PostgreSQL application-owned row decoding governed by SourceWork | `g1-pg-row-decoding-claude-20260922`, complete/pass |
| `3bd2b2a501edc54335ae702ec5c3de944e9e7444` | Helper metadata 3.42.5 | `helper-metadata-bump-claude-20260922`, complete/pass |
| `f71a29b913c2f445e9bfd4520e33cf7ad592647e` | SQLite application-owned row decoding governed by SourceWork | `g1-sqlite-row-decoding-claude-20260922b`, complete/pass |

SQL parser isolation was completed earlier at
`3bb59ca19f55134b493f4a313f2f28bb3a5bd81f`; do not reopen it.

Historical restoration evidence: `02736d56` supported Claude effort during
earlier Claude-only operation; `1ad11c8f` restored cross-provider review on
September 19. Current restoration reused preceding workflow/test behavior.
Generic Codex support remains dormant, not deleted from historical adapters.

### PostgreSQL evidence

Actual `PgRowStream::next_row_controlled` threads caller SourceWork through row
vectors, cells, text, scalar/temporal/hex formatting and borrowed NUMERIC decode.
NUMERIC digit checkpoints are bounded; FLOAT8 allowance is 327.

Eight checks passed: SQL, executor, serving build, CLI build, PG live, capability,
format and diff. Independent Claude review also ran full workspace build and live
numeric group. Two actual control-disconnection mutations failed and were restored
byte-identical. Logs remain `/tmp/mutation1_live.log`, `/tmp/mutation2_live.log`,
`/tmp/mutation1_restore.log`, `/tmp/mutation2_restore.log`.

Live decoder unit test is intentionally ignored in ordinary unit runs; owned
fixture explicitly invokes it with `--ignored --exact`. Diagnostic libpq string
strips application `pg:` prefix. Separate `sf_decoder_probe` NoTls diagnostic
role is not TLS transport evidence; application `sf_tls` remains TLS-only.

### SQLite evidence and limits

`sqlite/decode.rs` and `decode_tests.rs` now hold decoding and focused tests.
Borrowing `SqliteBranch::next_row_controlled` overrides checkpoint-only default;
owned worker also passes its real control. Text/lexical helper Real bounds changed
from 32 to 327. Existing uncontrolled entry behavior remains available.

Eight harness checks passed at source digest
`5cd34b017bfeb62bb1c79410e60c56a9bc5f935e326197c1d1ed6297ee4c6ccf`:
SQL (298 passed, one ignored), executor, public query-budget suite, serving build,
CLI build, capability, format and diff. Independent reviewer
`claude-sqlite-review-20260922` reported 89 SQLite tests passing and reproduced
both borrowing-dispatch and owned-worker mutation witnesses. Review response is
in the completed task's `workflow.results`.

Do not inflate this to full G1 closure: commit added no new serving integration
test file. Existing public query-budget suite passed, but dedicated decoder-bound
cold/warm SELECT/ASK/CONSTRUCT refusal/recovery coverage must be checked against
remaining G1d obligations. Native-driver allocations and arbitrary CPU preemption
are not claimed. Catalogue `asOf` was reported stale, a nonblocking prior issue.

## Harness state and process corrections

Records: ignored `.metaharness/delivery/<task-id>.json`; logs alongside them.
Read exact task status/commit/verdict, not progress prose alone.

The first SQLite task edited before begin, then changed
`sqlite/lexical_key/tests.rs` outside its declared scope. Coordinator interrupted
the writer, preserved all five changed files and created successor `...20260922b`
with explicit adoption and expanded scope. Original binding incorrectly said
default effort; successor records actual Sonnet high and session identity.
No original evidence was relabeled as passing.

At handover, original SQLite task is terminally **superseded** by completed b.
Also superseded: PG tasks `g1-pg-row-decoding-20260920`, `...20260920b`,
`...20260920c`, `...20260920d`, and `claude-only-restoration-20260922`.
Original failures remain in their records.

Use lifecycle commands from `coding-harness/` consistently:

```bash
npm run delivery -- /home/claude/src/hm/semantic-fabric begin /absolute/task.json
npm run delivery -- /home/claude/src/hm/semantic-fabric bind TASK OWNER /absolute/native.json
npm run delivery -- /home/claude/src/hm/semantic-fabric advance TASK OWNER
npm run delivery -- /home/claude/src/hm/semantic-fabric submit TASK OWNER /absolute/response.json
npm run delivery -- /home/claude/src/hm/semantic-fabric verify TASK OWNER
# Commit only verified scope, then:
npm run delivery -- /home/claude/src/hm/semantic-fabric finish TASK OWNER FULL_COMMIT_SHA
```

Begin BEFORE edits, including prerequisite test files in scope. Bind actual model,
effort and executor ID. `advance` returns work, not an executor. Review needs a
distinct Claude executor and stable source. Obtain source digest through exported
`sourceSnapshot(repo).digest`, not a Git tree hash. Mixing direct `node` lifecycle
commands with `npm run delivery` previously changed environment digest and staled
checks. Keep invocation consistent.

Use foreground review. A prior background native reviewer never delivered because
its parent CLI exited. Poll actual process handle; when terminal, read output and
dispatch next ready work. Never describe a finished process as still running.

## Remaining programme and next allocation

| Gate | State | Next work / dependency |
| --- | --- | --- |
| G1 | Open; PG/MySQL/SQLite decoding slices complete | Shared runtime RDF reconstruction controls, remaining declared adapter/rendering coverage, unchanged-default combined cold/warm/two-source qualification |
| G2 | Closed | Preserve six named acceptance outcomes; rerun relevant candidate matrix later |
| G3 | Open | Ordinary-mode generation coherence and native PostgreSQL RLS authority/protected integration |
| G4 | Implemented protected federation evidence | Final qualification depends on G1/G3; 25 ordered source pairs plus six lifecycle directions already exist |
| G5 | Pending candidate | Required fail-closed transport/live matrix after G1-G4 |
| G6 | Pending candidate | Exact local release artifact, workspace checks, smoke, licence/advisory/SBOM/signature/provenance and independent Claude review |

Initial continuation: sole writer scopes G1 runtime RDF reconstruction. In
parallel, independent read-only Opus finishes G3 authority review; another
researcher may prepare ordinary-mode generation integration. Allocate reviewers
when source is stable. Refill from dependency-ready work, not arbitrary agent caps.
Do not create a new scheduler, expand Darwin/GEPA/AVO or invent filler tasks.

### G1 concrete runtime reconstruction gap

Corrected read-only audit traced:
`sf-sparql/src/exec_core/driver.rs` -> `batch.rs::reconstruct_batch` ->
`row.rs::reconstruct/build_term/derived_term` ->
`sf-core/src/term.rs::generate_into` -> `ir.rs::Template::expand` and
`ir/encoding.rs::percent_encode_iri`.

Driver charges source pulls and checkpoints before batch reconstruction, but
`reconstruct_batch` receives no QueryControl. Term construction, CONCAT loops,
blank-node hex labels, template expansion and percent encoding have no threaded
runtime work control in this path. Preexisting controlled column-index/binding
interning does not cover it. Rayon batch-size bounds do not establish work bounds.

Next slice must audit caller envelopes, then govern actual runtime loops/copies,
including sequential/parallel equivalence, sticky cancellation/deadline, exact/N-1,
control-disconnection witness and public recovery. Avoid double charging shared
helpers. Explicitly scope files before editing. Preserve output ordering/NULL/
identity/errors and unchanged defaults. Audit downstream DISTINCT/order clones
and recursive terms as part of admitted runtime obligations.

The audit's first report incorrectly identified `emit.rs` SQL generation as
post-row reconstruction and proposed mere structural splitting. It was rejected.
Do not reuse that conclusion. Corrected report is in local artifacts below.

### G3 native RLS candidate: NOT accepted proof

Existing `crates/sf-serve/src/pg_rls.rs` holds data-table ACCESS SHARE locks.
They fence policy/table DDL, not role or membership changes. Repeatable-read
`pg_roles` scans may stay stale; `row_security_active()` consults live privilege
machinery. A final catalogue check cannot undo rows already streamed.

Opus investigated a narrow administrator-installed, zero-argument,
superuser-owned SECURITY DEFINER function, fixed `search_path=pg_catalog`,
PUBLIC execute revoked and reader-only execute granted. Function takes SHARE
locks on `pg_authid`, `pg_auth_members`, `pg_database`, `pg_db_role_setting`, held
through request transaction. Call precedes authority checks and data execution;
existing data-table locks and `row_security_active` checks remain required.

Investigator reports 16.9/16.15 disposable-fixture evidence that ordinary
ALTER ROLE/membership changes block; two-catalog design failed database-owner
membership drift, motivating `pg_database`. Function requires superuser owner;
reader itself gains no general catalogue writes. Bounded lock waits and fail-closed
absence/permission failure are mandatory. Locks block cluster-wide role/database
administration during requests; reads and concurrent shared leases can continue.

This is a candidate, NOT implementation or independently accepted closure.
Independent Opus review was interrupted for this handover without verdict.
Resolve SELECT snapshot creation before function locking, pre-pin membership
changes, live syscache versus MVCC identity, policy/dependency protection, function
ownership/replacement, full protected-generation binding and cleanup semantics.
Do not infer that `row_security_active=true` proves every authority identity.

Author's suggested namespace filter combined with `--exact` would select zero
tests. Use full exact test names or omit `--exact` for namespace filtering.
Existing actual test command:

```bash
cargo test --locked -p sf-cli --no-default-features --test source_tls_live \
  support::pg_rls_authority::policy_locks_and_role_changes_have_distinct_authority \
  -- --ignored --exact --nocapture
```

No `source-tls-live` Cargo feature exists. Use owned fixtures, not unrelated DBs.
Investigator exceeded initial read-only/no-DB instruction by creating disposable
`sfg3`/`sfg3b` containers and reports cleanup; do not treat this as authority for
mutating existing databases. Verify any remaining resources before touching them.

### G3 ordinary modes

Read-only trace: ordinary startup produces `SourceGeneration::Unverified`;
`pg_generation.rs::requirement()` returns `Ok(None)`; per-request
`snapshot.rs::generation_requirements()` therefore obtains no generation lease.
Protected authored SQLite/PostgreSQL/MySQL and Direct PostgreSQL paths already
exist and must be reused where applicable.

Review ADR-0050/0055 promises and admitted profiles, then implement actual
same-connection transaction/DDL protection or equivalent guarantee. An off-path
observation-equality check alone cannot close TOCTOU. Do not silently narrow
promised semantics or mark a documentation disclaimer as implementation closure.
Require ordinary-mode DDL-race rejection or consistent old-view completion tests.

### G5/G6 local release

Existing `scripts/release/build-serving-image.sh` builds committed source only,
pins local Docker socket, exports exact image archive/ID/checksums and metadata.
It explicitly does not sign or admit release. `cargo-audit` and Docker were found;
SBOM/signing tools were not found on PATH during preparation. Inspect existing
policy/tooling before choosing smallest necessary additions.

Required selectors live in `tests/capabilities/catalog-v1.json`: verified-source
TLS library/CLI/live and native-query-profile live, plus admitted backend and
federation matrix. Candidate must be immutable with exact evidence, no unexpected
skips. Full locked workspace, strict Clippy, serving-only closure, clean smoke,
licence/advisory disposition, SBOM, checksums, signature/provenance remain required.
No publishing implied. Some ADR-0055 section 3/M7/G6 text still says Codex/Claude;
current Claude-only override governs. Correct stale wording in scoped docs slice.

## Local continuation artifacts and sessions

These ignored/private files are useful but not portable or durable Git evidence.
This document preserves essential conclusions if `/tmp` is cleared.

| Artifact/session | Meaning |
| --- | --- |
| `/tmp/sf-programme-progress.txt` | Writer checkpoint; stale about parallel G3 proof and predecessor supersession; this handover corrects it |
| `/tmp/sf-programme-output.json` | Last writer response |
| `/tmp/sf-coordinator-handoff.txt` | Detailed coordinator corrections and queued findings |
| `/tmp/sf-rdf-governance-prep-output.json` | Corrected runtime reconstruction audit |
| `/tmp/sf-g3-ordinary-prep-output.json` | Ordinary generation audit |
| `/tmp/sf-g3-authority-proof-output.json` | Opus candidate proof, requires independent review |
| `/tmp/sf-g3-proof-review-output.json` | Interrupted review, `terminal_reason=aborted_streaming`, no verdict |
| `/tmp/pgsrc` | Investigator's PostgreSQL source excerpts, verify provenance before treating as proof |
| `b6af6730-6000-4d0d-8499-ddc092e8ff3e` | Claude Sonnet implementation session; terminal after SQLite completion |
| `bb2b5f5c-16eb-4e43-90e7-4949b12e4ae2` | Completed Opus authority investigator |
| `72f56a59-c5fc-43fd-be15-312f14cacc5f` | Interrupted Opus independent authority review; can resume read-only |
| `a5e11e66-fce2-41a2-99eb-9709a1e72fbe` | Completed Sonnet RDF/ordinary-mode researcher |

Old tool handles 83852, 5261, 70980, 94157 and earlier handles are terminal.
Do not poll them as running work. No next application task was dispatched after
SQLite completion. Receiving Claude owns next dispatch.

For native CLI continuation, preserve configured subscription settings; disable
helper hooks for scoped runs with `--settings '{"disableAllHooks":true}'`.
Use `--model sonnet --effort high` for writer, explicit Opus high for proof review.
Do not run two processes against same resume UUID. Launch environment should omit
variables ending `API_KEY` and `OPENAI_AUTH_TOKEN` without reading their values;
preserve authorized gateway authentication. Never copy full transcripts into Git:
an earlier native worker exposed an ambient provider credential in a private
transcript; user was informed. No credentials belong in this handover or memory.

## Memory, checkpoint and completion discipline

Use structured Ruflo MCP only. Project memory and `ruflo_user` are separate stores.
Recall `user-patterns/metaharness-phase-gating-proportionality`, superseded by
`metaharness-full-operational-harness-v1`; user/repository current instructions
override older pattern transport/model advice. A missed project lookup was
repaired: `semantic-fabric/g1-pg-row-decoding-complete-20260922` stored and read back.
Recheck SQLite memory against exact local receipt; do not assume writer's reported
remote synchronization succeeded merely because local delivery passed.

`.ruvnet-brain/checkpoint.json` is local/ignored; historical unrelated RDF-star
DONE checkpoint preserved as `checkpoint-rdf-star-iteration28.json`.
Current programme done criterion names `node scripts/verify-programme-completion.mjs`,
but that verifier and `scripts/loop-checkpoint.mjs` are absent. They are not passing
gates. Supply only necessary fail-closed completion verification through existing
harness before final closure; do not create another scheduler or delay application
work for invented process machinery.

Programme completion requires actual current G1-G6 application and release
evidence, coherent commits and independent reviews. Green workspace, local
receipts, model confidence, task counts and old RDF-star DONE state are insufficient.
Report active/ready/blocked/review queues and exact failures. User specifically
objects to silent stopped execution and misleading running claims.
