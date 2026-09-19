# Semantic Fabric — session handover

Prepared 2026-09-19 at the user's request. The user is moving from Claude Code
back to Codex. This document is local repository material, not release
evidence or publication authorization.

## Read first

1. Read the current [AGENTS.md](../../AGENTS.md), including applicable
   personal instructions supplied by the user.
2. Read accepted [ADR-0055](../adr/ADR-0055-v1-product-completion-and-release-profile.md)
   and the G1–G6 ledger in the
   [completion programme](sota-application-completion-programme.md).
3. Read [coding-harness/README.md](../../coding-harness/README.md), especially
   ownership, pause/resume, adoption, native responses and exact-commit finish.
4. Read the prior handover,
   [session-handover-2026-09-15.md](session-handover-2026-09-15.md), for
   context on the G1c source-preparation work it describes — that work is now
   folded into G1, which is closed (see below). That document's own "resume at
   the actual gaps" section is superseded by this one.

The objective remains **complete the agreed application, plan, programme and
ADRs**. There is no v1/v2/v3 release roadmap anywhere in this repository —
every "v2" string you may find is an unrelated data-schema version tag, and
ADR-0055 itself says explicitly that moving an item to "post-1.0" does not
make it complete, supported, or scheduled; it is a priority label, not a
promise. G1–G6 is the whole finite ledger. The user rejected arbitrary
deadlines and percentages: do not invent either.

## Authoritative checkpoint

- Checkout: `/home/claude/src/hm/semantic-fabric`, canonical `main`.
- HEAD at handover: `37e925b27b6952ef58bc2e62657e9f2a9a2be8b6`
  (`docs(adr): record decision not to add post-200 error signaling`).
- Working tree is **clean** with respect to application/programme/ADR work.
  Two pre-existing dirty files are unrelated infra, not part of any task this
  session: `.claude/helpers/.helpers-version`, `.claude/helpers/helpers.manifest.json`
  (auto-updated by the Claude Code helper/version-drift monitor; do not stage,
  revert or absorb them into a task commit).
- One untracked file exists (besides this handover document, before it is
  committed): `docs/adr/ADR-0056-generated-query-admission-profile-and-issued-response-identity.md`,
  status `proposed`, dated 2026-09-19. **Its origin is unclear and unverified
  — do not assume it is safe to ignore.** Filesystem birth time is
  `2026-09-19 11:14:30`, which falls *inside* this session's own active
  window (between this session's commits `62646689` at `10:28:48` and
  `4b5cc41d` at `11:45:31`), so it was created *during* this session, not
  before it, despite an earlier in-session review comment asserting the
  opposite. No task this session ran created it, and its actual origin (a
  background/automated process? a stray write?) was not investigated before
  handover. Read it and find out what created it before assuming it is
  unrelated noise — it may be evidence of something writing to this repo
  outside the harness.
- **No harness task is active or paused.** Every task from this session
  finished with `status: complete` (verified by inspecting
  `.metaharness/delivery/*.json` — the most recently modified records are
  `adr0010-post200-note-opus-20260919`, `g2-close-doc-opus-20260919`,
  `g2-query-profile-timeout-fix-opus-20260919`, all `complete`). There is
  nothing to resume, adopt, or reconcile before starting new work.
- `git stash list` shows 5 pre-existing stashes, none created this session,
  including one on `main` (`stash@{0}: On main: helper-hook-state-out-of-g1c-scope`)
  touching the same `.claude/helpers/*` files mentioned above. Not acted on;
  flagging so they aren't mistaken for something new.
- `.metaharness/delivery/53f0c53f-bb66-48c7-a5ee-f520ab41f76f.recovered-lock.json`
  is a stale, already-reclaimed lock artifact from 2026-09-15 (the
  `.recovered-lock.json` naming means the harness already recovered it).
  Benign, not acted on.

## What closed this session (commits, in order)

All five commits below were made in this session. They build on the prior
handover's G1c/G1d work, which had already closed by 2026-09-16, in an
earlier session — no commits from that closure are relisted here:

- `8532606b` — `feat(serve): expose --max-compiler-work, fixing a silent no-op
  limit`. Root cause: `sf-serve/src/startup.rs`'s `configure()` read
  `config.query_limits.max_compiler_work()` back into itself — a no-op
  self-assignment that silently discarded any attempt to change it.
- `80fd022e` — `fix(sparql): bind MySQL percent-encoder's BINARY cast once,
  not ~76 times`. MySQL's `percent_encode_col_mysql` (`crates/sf-sparql/src/emit.rs`)
  re-embedded a column expression ~76 times per encode call; measured
  `repeats`: MySQL 148 → 1, vs PostgreSQL 2, SQLite 1.
- `62646689` — `test(query-profile): raise --timeout-secs 2->15, confirmed via
  instrumentation`. **The actual root cause of G2's remaining intermittency**
  (4 pass/1 fail across 5 runs of `cmd-native-query-profile-live`): `crates/sf-cli/tests/source_tls_live/query_profile.rs::start_command`'s
  `--timeout-secs 2` is a single absolute per-request wall-clock deadline
  (`crates/sf-serve/src/budget.rs`'s `RequestBudget`) that can fire mid-stream,
  after HTTP `200` headers are already committed, truncating the response
  with no typed error. This is architecturally distinct from the two fixes
  above (which are governed-resource-budget/`SourceWork`/`CompilerWork`
  issues) — confirmed by direct instrumentation, not inference: an env-gated
  per-row streaming delay was run against the real, unmodified `--timeout-secs
  2`; it reproduced the exact truncated-response symptom, and the server
  itself logged `reason=DeadlineExceeded` at the moment of truncation. Fixed
  by raising to 15s with a comment citing the measured ~661-766ms worst case
  (the suite's heaviest streamed query).
- `4b5cc41d` — `docs(programme): close G2`. Records the above, plus post-fix
  verification: 2 full `native_describe_and_recursive_paths_are_exact`
  reruns, both passing (680.64s, 700.56s — these two numbers are in the
  commit/programme-doc text itself, verifiable by grep). A third independent
  reviewer, while reviewing the G2-closure doc itself, reported its own fresh
  rerun also passed (~691s) — that number is **not** captured in any repo
  artifact (it only exists in that review's conversation output), so treat it
  as reported-but-unverifiable rather than citable evidence.
- `37e925b2` — `docs(adr): record decision not to add post-200 error
  signaling`. A reviewer flagged, correctly, that the *general* mechanism
  behind the `--timeout-secs` bug (any post-`200` mid-stream failure
  truncates with no typed reason) is not test-specific — it is also how
  production would behave. Investigated what a real fix (HTTP/1.1 trailers)
  would take: ~10 streaming call sites, ~15+ existing tests that key off exact
  truncation bytes as their failure oracle, and a change from "abrupt reset"
  to "clean chunked terminator" for the failure path itself. **Decision:
  leave it alone** — this is deliberate architecture, independently
  documented in **ADR-0010, 0011 and 0049** (ADR-0049 verbatim: "post-`200`
  transport atomicity remains an explicit nonclaim"; ADR-0049 also already
  considered and rejected "send a partial result with a warning"), and the
  value (self-service diagnosis for a remote caller without server-log
  access) didn't justify the cost absent a demonstrated need. Recorded as a
  dated status-correction paragraph in ADR-0010, matching its existing log
  format. **This is closed — do not re-investigate it without a new,
  concrete reason** (e.g. an actual external consumer asking for it).

## Ledger status

- **G1 — CLOSED** (all sub-phases G1a-d; closed before this session). Note:
  the programme doc's G1 row has no single explicit "G1 is closed" string —
  the basis for this is the detailed G1a-d completion narrative in that row
  plus its closing sentence "G2–G6 remain open" (G1 is conspicuously absent
  from that list). Read the G1 row yourself rather than trusting this label
  alone if anything about G1 becomes load-bearing for new work.
- **G2 — CLOSED** (this session; all six named acceptance commands pass:
  `cmd-property-path-exact`, `cmd-property-path-key-equality`,
  `cmd-postgresql-rowid-boundary`, `cmd-describe-compile`,
  `cmd-describe-sqlite`, `cmd-native-query-profile-live`).
- **G3 — OPEN, unscoped.** Coherent generations/semantic admission. Existing:
  immutable reload leases, protected PostgreSQL Direct/authored generations
  are public; invalid generations fence readiness. Missing: complete
  backend/policy generation and DDL-race guarantees, binding `M ⋈ T`/source
  validation and cache identity to the same lease. Commands:
  `cmd-runtime-snapshot-state`, `cmd-runtime-snapshot-http`,
  `cmd-runtime-snapshot-body`, `cmd-public-authored-generation-live`,
  `cmd-public-direct-lifecycle-live`, `cmd-semantic-admission-runtime`,
  `cmd-semantic-admission-validation`, `cmd-semantic-admission-mapping`.
- **G4 — OPEN, unscoped, depends on G1-G3.** Declared cross-source execution.
  Existing: two-source UNION and fixed-cap two-pattern join, actual lineage,
  native stop/sibling isolation/recovery implemented. Missing: qualify
  source consistency, protected generations and every admitted backend
  combination. Commands: `cmd-federated-union-sparql`,
  `cmd-federated-union-serve`, `cmd-federated-union-cli`,
  `cmd-federated-join-sparql` plus required owned mixed-source TLS cases.
- **G5 — OPEN, unscoped, after G1-G4.** Required native transport/live
  matrix. Existing: verified remote TLS and native auth/exactness suites have
  owned-fixture evidence. Missing: run the required fail-closed matrix
  against one immutable candidate; record exact artifact/profile/log
  digests. Commands: `cmd-verified-source-tls`, `cmd-verified-source-tls-cli`,
  `cmd-verified-source-tls-live`, `cmd-native-query-profile-live` (this last
  one is shared with G2 — G2's closure doesn't automatically close this G5
  usage of it; check whether G5's matrix has its own separate requirements
  around it).
- **G6 — OPEN, after G1-G5.** Exact candidate/admission verdict. This is an
  aggregate release gate, not new feature work: one immutable candidate
  passes the full locked workspace + serving-only checks, backend/profile
  matrix, clean-machine smoke, licence/advisory review, SBOM, checksums,
  signature/provenance, independent native Codex+Claude exact-delta review.
  Publication requires separate, explicit current approval — this gate does
  not grant it.

**Recommended next step: scope G3.** Per this project's own process rule
("finish integration and public request behavior before starting another
foundation... group compatible work by complete phase, not a
next-unmetered-helper queue"), audit G3's prerequisite tests and current
actual state before starting implementation, the same way G2 was scoped on
2026-09-16 before any fix was attempted.

## Process notes carried forward (not written anywhere else in this repo)

These were learned the hard way this session and are not captured in
AGENTS.md, the ADRs, or the programme doc — worth reading before the next
harness task, regardless of which native host runs it:

- **Harness task-spec schema, exact fields**: `schemaVersion`, `id`,
  `requirement` (not `description`), `owner`, `thread`, `taskClass` (enum:
  `mechanical`, `pattern`, `implementation`, `correctness`, `difficult` — NOT
  `documentation`), `host`, `scope` (array of paths), `checks` (array of
  `{id, kind, argv, cwd}`, where `kind` is `build` or `acceptance`). `begin`
  requires **at least one check of each kind** (`DELIVERY_ACCEPTANCE_AND_BUILD_REQUIRED`
  otherwise) — for a doc-only change with no real acceptance test, mark cheap
  hygiene checks like `cargo fmt --all --check` / `git diff --check` as
  `kind: "acceptance"` and only the actual build as `kind: "build"` (see any
  `*-doc-*` task in `.metaharness/delivery/` for a working example).
- **Repair-round edit ordering matters.** After a `changes-requested` review
  (or any repair), the correct sequence is `resume()` (if paused) → `bind()` →
  `advance()` (this captures the file's *current* state as the repair round's
  baseline) → **only then edit the file** → `submit()`. Editing before
  `bind()+advance()` makes the fresh request's baseline already include your
  edit, so a subsequent no-further-edit `submit()` trips
  `DELIVERY_REPAIR_NO_PROGRESS` (which auto-pauses the run — call `resume()`
  before `bind()` again).
- **Computing the source digest independently**, to build a `submit()`
  response without waiting on another `advance()` round-trip:
  ```js
  import { sourceSnapshot } from "<repo>/coding-harness/dist/delivery-workspace.js";
  console.log(sourceSnapshot("<repo>").digest);
  ```
  (Use an absolute path for the import if running from outside the repo.)
- **A genuine independent-reviewer REJECT beats your own N passing runs.**
  This session hit a case where 2 of my own live-test passes plus careful
  reasoning still turned out to be an incomplete picture; a fresh reviewer's
  own rerun caught it. Don't dismiss or re-run-until-passing a real REJECT —
  correct the record.
- **Don't propose "raise the limit" from timing-margin reasoning alone.**
  When a fix looks like "just widen a number," and the evidence for why is
  circumstantial (baseline timing + architecture reading), force a cheap,
  reversible reproduction of the actual failure mechanism first (e.g. an
  env-gated artificial delay against the *real* unmodified value, plus a
  server-side log line at the exact decision point) before touching the
  number. This session's G2 fix used exactly this technique instead of
  guessing at margins or stressing the shared host's CPU.
- **This host is genuinely shared/multi-tenant** (other unrelated Docker
  workloads run continuously — confirmed, not hypothetical). Full live
  Docker-backed test runs (`cargo test ... --ignored native_describe_and_recursive_paths_are_exact`)
  take ~660-700s normally; do not assume a slow run is a regression without
  checking `docker ps` / host load first.

## Memory, models and operating rules

- Ruflo only through live structured MCP. Never read managed memory files,
  use SQL/CLI fallbacks, bulk-import or replace sidecars.
- Native subscription authentication only. No provider keys/OpenRouter or
  quota/spend ceilings. Report exact native availability errors; do not
  silently substitute a model or claim a new session is the old executor.
- One writer on `main`. Never create/switch branches or worktrees. Use
  bounded independent read-only review when useful; a native reviewer is not
  by itself proof of a tracked Ruflo swarm.
- No recurring scheduler is currently active for this repository from the
  Claude Code side (any prior cron job was cancelled by the user mid-session).
  This handover does not restart one.
- Node is harness/evidence infrastructure only; runtime stays Rust/Cargo.
- No Product Mock/live database access, flywheel enablement, push, tag,
  deploy, publication, branch/worktree creation or cleanup is granted by this
  handover.
- Commit only verified, authorized slices; keep the two unrelated
  `.claude/helpers/*` files separate from any task commit.

## Handover acceptance

The outgoing session closed G1 and G2, left no harness task active or
paused, and committed only verified, independently-reviewed slices. The
working tree is clean at `37e925b2`. The next session should read the
programme doc's G3 row, scope it the way G2 was scoped on 2026-09-16, and
continue the ledger in order. This is **not an application completion
report** — G3 through G6 are substantial, mostly-unscoped work, not small
follow-ups.
