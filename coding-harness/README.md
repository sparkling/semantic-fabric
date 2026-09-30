# semantic-fabric coding harness

Optional, private development-only harness with native Codex/ChatGPT and Claude Code executors under
[ADR-0055](../docs/adr/ADR-0055-v1-product-completion-and-release-profile.md).
The older closed-candidate experiment remains separate and optional.

All Node/TypeScript here—including the historically named
`supervisor-service/` package—is non-deployable evidence/oracle infrastructure.
The product runtime is Rust; any future production supervisor is a separately
packaged Rust service under ADR-0048.

The delivery CLI runs declared checks and records task/handoff/evidence state;
ready `mode: "run"` invokes configured model adapters. It never creates worktrees,
commits, pushes, publishes or deploys. Ruflo is accessed only through MCP.

## Delivery path

Owner amendment 2026-09-28 (ADR-0058): direct implementation, repair, tests and
builds are allowed for all work. Both native hosts and an isolated direct
OpenRouter adapter are authorized. The historical Claude-only/native-only and
mandatory-dispatch descriptions below do not govern current ordinary delivery.
Retain scoped acceptance, independent review, one integrator and data isolation.
Application programme remains paused. Unit tests use injected transports; the
explicitly authorized initial proof uses real configured models.

### Native parallel execution

GCP resumption and this harness deployment are owner-authorized. Repairs never
accept application WIP or authorize other publication. Stop the previous coordinator
writer before resuming its existing UUID; never launch a competing conversation.
Set `SEMANTIC_FABRIC_COORDINATOR_SESSION_ID`, then run `npm run coordinator` here.
Use `npm run coordinator -- --dry-run` to inspect without launching. The launcher
pins Codex `gpt-6-astra`/`medium` on local `main`; it never resumes a paused goal.
Preserve configured subscription gateways, ordinary Codex `gpt-6.1-sol`/`high` and existing Opus repair.

Before dispatch, record dependencies, source revision, owned scope, read-only
status, deliverable, acceptance checks, native model/effort and result recipient
in the coordinator's task queue (Ruflo MCP for persistent tracking). These queue
fields are coordination metadata, not additional `DeliveryTask` JSON fields.
Run `npm run delivery -- /absolute/repo ready /absolute/manifest.json` in `run` mode.
Upstream refills queued independent outcomes immediately, including after failure.

JSON `delivery-pool-progress` on stderr links durable starts, candidates, settlements and drain; stdout retains final results.
`fulfilled` means callback success, not candidate acceptance or integrated product progress.
Settled outcomes may integrate before unrelated siblings finish: sole owner serially
integrates/checks/verifies/commits/finishes, subject to active read/write/resource
reservations. Only accepted parents release children, including concurrent cohorts.
Manifests remain explicit, not an automatic DAG scheduler. Whole-source/Cargo reads
still block conflicting acceptance; preserve cancellation custody and fresh review. At an explicitly authorized owner boundary, `integrate` may add `revalidateAgainst: {commit, sourceDigest}` pinning exact clean canonical HEAD/snapshot for evaluator or declared-input migration. Default hard-pin rejection remains. Scope overlap, active reservations and invalid candidate evidence still refuse; original receipts stay immutable. This opt-in reruns every declared check and requires a fresh distinct reviewer over all changed inputs, even without Cargo. It never replans/replays the author or accepts the candidate by itself.

`selectDeliveryRoute` already supplies task-based model/effort defaults. Preserve
an explicit route and the selected main model; escalate only a named unresolved
question. Measure accepted-outcome latency, review/repair failures and total
reported tokens (including context and rework); unavailable token data stays
unknown. Sample effective CPU, interval utilization, memory/I/O before heavy work, every 30 seconds and at refill.
Size local test jobs separately; report active/ready/blocked queues and idle reasons. No fixed native-session cap.

### Scoped delivery lifecycle

Use one task per coherent requirement closure. The existing native conversation
is the executor; `DeliveryHarness` supplies durable sequencing and the installed
`@metaharness/harness` kernel verifies each ready native stage. The outer controller
owns prerequisites, repair feedback and recovery. Upstream kernel steps are sequential,
its internal retries do not supply failed-verifier feedback, and its receipt chain
alone is not crash-resume. We therefore reuse it per stage, not as another build daemon.

1. Use Ruflo MCP to recall relevant context and create an assigned task. Define
   exact file scope, requirement, owner/thread and acceptance/build commands.
2. `begin` claims the one writer on `main` and selects a model/effort. Existing
   changes inside the scope require an exact `adoptExistingChanges` list; their
   starting hashes are recorded, not treated as verified. Other dirty work stays
   outside the slice and must remain unchanged.
3. The native host starts the selected subscription agent (or retains the selected
   main model), then `bind` records its actual model, effort, native executor ID
   and observation. A mismatched handoff is rejected; no silent substitution.
4. `advance` returns a durable `native` implementation request. The existing host
   executes it with normal tools; `submit` binds the result to that request, actual
   route, executor and resulting source digest. The sole writer edits only its scope.
   Subsequent `advance` calls run eligible declared checks automatically, stopping at
   the next native request or a hold. Individual checks remain available via `check`.
   Checks execute sequentially, without shell expansion, and retain each actual
   result, duration, source/environment hashes and private output logs—including
   failures. Each check defaults to 30 minutes and 10 MB of combined output;
   positive `timeoutMs`/`maxOutputBytes` fields override these process-safety
   controls up to 24 hours/100 MB, never subscription budgets. Cancellation,
   timeout or overflow stops the process group and records failure. An unconfirmed
   surviving group retains the operation record for recovery. Log hashing is streamed.
5. Failed checks return an implementation repair request containing the failure and
   hash-bound diagnostic log paths. Failed review returns the reviewer's actual issues.
   Dependent stages are not dispatched until their prerequisites pass. A repair with
   no source progress pauses for explicit intervention. After passing checks, the host
   assigns the returned review request to an independent executor, using the API
   default or the task's explicit `reviewer: {host, model, effort}`.
   Review is read-only, must use a different executor ID from every implementation
   handoff, and is bound to the source and exact prerequisite results. `verify`
   requires both stages and every check; missing, failed or stale evidence fails.
6. The integration owner commits only the verified scope on `main`.
   `finish` requires that exact next commit, matching source and passing evidence.
   Mirror the resulting commit, check results and measured durations to the Ruflo
   task and memory through MCP; read back the stored result. Memory outages are
   reported, but the local record remains available and delivery can continue.

Run commands from `coding-harness/` after its normal local build:

```bash
npm run delivery -- /absolute/repo begin /absolute/task.json
npm run delivery -- /absolute/repo bind task-id owner /absolute/native.json
npm run delivery -- /absolute/repo advance task-id owner
# Existing native host performs the returned request, then writes its response.
npm run delivery -- /absolute/repo submit task-id owner /absolute/response.json
# Repeat advance/submit for any repair and the independent review.
npm run delivery -- /absolute/repo advance task-id owner
npm run delivery -- /absolute/repo verify task-id owner
# Integration owner commits the scoped changes using normal Git tools.
npm run delivery -- /absolute/repo finish task-id owner FULL_COMMIT_SHA
npm run delivery -- /absolute/repo status task-id
```

`scripts/slice.sh` in the repository root wraps the same steps (`start`, `impl`,
`checks`, `review`, `verdict`, `close`) and retries transient `index.lock` races.
It commits only the task scope and never pushes. A check may use
`cargo nextest run`, which ran the `sf-serve` library tests in half the time of
`cargo test` on 2026-09-23; other `nextest` subcommands are refused. After editing
a file that the capability catalogue hashes, run
`scripts/refresh-capability-digests.py` to refresh only the stale digests and
regenerate the matrix.

Example task (paths/checks must match the actual change, not this example):

```json
{
  "schemaVersion": 1,
  "id": "task-id",
  "requirement": "Exact public query behavior for the named regression",
  "owner": "native-owner",
  "thread": "existing-conversation-id",
  "taskClass": "implementation",
  "host": "codex",
  "scope": ["crates/sf-sparql/src/example.rs"],
  "checks": [
    {"id": "build", "kind": "build", "argv": ["cargo", "build", "--locked", "-p", "sf-sparql"], "cwd": "."},
    {"id": "public", "kind": "acceptance", "argv": ["cargo", "test", "--locked", "-p", "sf-sparql", "specific_test"], "cwd": "."}
  ]
}
```

Every `native` action returns a request with its ID, task/thread/base commit, stage,
attempt, requested route, starting source digest, exact scope, prerequisite digests
and failure feedback. `next` inspects/persists the next action without running checks;
repeating it or restarting the CLI returns the same still-valid pending request.
There is no polling process or second conversation resume. Submit this strict shape
(replace placeholders with the returned ID, actual source digest and native metadata):

```json
{
  "schemaVersion": 1,
  "requestId": "64-character-request-digest",
  "sourceDigest": "64-character-resulting-source-digest",
  "native": {"host": "codex", "model": "gpt-5.6-sol", "effort": "medium", "executorId": "observed-native-executor", "authentication": "native-subscription", "observation": "actual native host metadata"},
  "outcome": "completed",
  "summary": "Changes made or independent review conclusion",
  "issues": []
}
```

Obtain the current digest with the exported `sourceSnapshot(repo).digest` from
`dist/delivery-workspace.js` (not a Git tree hash). `changes-requested` requires
nonempty `issues`; `completed` requires none. `unavailable` and `cancelled` persist
the exact native error in `summary` and pause; neither triggers a fallback.
Rejected/duplicate/stale submissions do not advance the task. Source drift invalidates
downstream results; a newer incomplete check cannot reuse an older pass. Completed
pre-workflow records remain historical, not retroactive claims of kernel execution;
`next` explicitly enrolls a still-active legacy run into the workflow.

Native binding fields are `host`, `model`, `effort`, `executorId`,
`authentication: "native-subscription"`, and `observation` (the actual host
metadata/error, never credentials). Default Codex routes use `gpt-6.1-sol`/`high`
for all ordinary task classes. Explicit task/reviewer model and effort pins stay
unchanged; configured Opus/high remains capability repair. Explicit Claude
routes use Haiku/Sonnet/Opus at native-default effort; an explicit Claude route
may record the actual Claude Code effort (`low`..`max`; `ultra` is Codex-only).
Opus high is the bounded top-tier escalation (ADR-0055; it replaced Fable on
2026-09-23) and, like Astra max, requires an explicit `requested` route. These are explicit project policy, not learned
quality estimates. New tasks without a host default to native Codex
`gpt-6.1-sol` at high effort; explicit native/API assignments stay
unchanged. Review requires a distinct executor from every author, not an implied
cross-vendor consensus. Native unavailability pauses with exact client/model/error.
`requested: {host, model, effort}` plus
`selectionReason` preserves an explicit choice; `preserveMainModel: true`
retains the active main model and requires its explicit `requested` route.
Ultra otherwise requires `explicitUltra: true`.
Max/ultra are forwarded unchanged. Subscriptions have no usage/spending quota gate.
Isolated API calls enforce at most $1 per request, with no cumulative ceiling.
Check commands receive the sanitized environment unchanged. Evidence hashing
deduplicates `PATH` in first-match order and omits only Codex's volatile
`.codex/tmp/arg0/codex-arg0*` launcher entries, so a native-host transition does
not stale checks while meaningful toolchain or environment changes still do.

`pause <id> <owner> <exact reason>` releases the claim without discarding work;
`resume <id> <owner>` requires unchanged base/outside scope and a new native
handoff. On native unavailability, pause and report exact client/model/error.
Resume invalidates pending native requests and requires a fresh bound executor;
it preserves prior failures and only reuses unchanged evidence. A review transport
failure resumes at review, not as a request to edit already-passing source.
`supersede <id> <owner> <successor-id> <exact reason>` terminally reconciles a
paused historical run only when the named successor is complete at an exact commit
that descends from the paused run's base. It records the successor task, commit,
reason and timestamp while preserving every prior check, handoff, event, workflow
result and verdict; it never promotes failed evidence to passing.
On an explicit user review hold, stop: neither a scheduler nor an active goal
releases that hold. A stale operation lock after a hard crash requires checking
the recorded process and any child before explicit recovery; it is never
automatically stolen. Use `inspect`, then `reconcile <id> <owner> <lock-nonce-or-none>
<reason>` to repair an interrupted claim transition. Recovery checks the exact
nonce, PID/start identity and child process group, archives the old lock and
records the reason. A live owner/child or an uncertain spawn window refuses
recovery. An unfinished run needs a new handoff afterward. `status` and records
survive process restarts; no failed/intended command is turned into a pass.

Records/logs live under ignored `.metaharness/delivery/`, separate from managed
Ruflo stores. They bind the working source including pre-existing outside work;
they are scoped development evidence, not a clean release-candidate attestation.
Linux `flock` and `/proc` provide operation exclusion and process identity;
the OS lease also covers reconciliation and releases on owner exit.
The native identity observation is trusted host/operator input, not a provider
signature. Request hashes and receipts establish local consistency, not proof that
a model actually performed a review. Host dispatch and the commit decision remain
the accountable integrator's responsibility. This cooperative harness does not sandbox arbitrary trusted build
scripts or require harness use for direct work. Acceptance-command selection remains the owner's correctness
responsibility; a zero exit code alone cannot prove a meaningful test selection.
Build environments preserve selected Cargo/Rust/owned-fixture settings but omit
provider-specific configuration variables, API keys and proxy overrides. Trusted
build scripts still have ordinary local filesystem access, including `HOME`;
this is not native-credential isolation. Never put credentials in task arguments,
handoff observations, or logs. Native and isolated API transports stay explicit.
The real closed-candidate isolation and replay checks below are not weakened.

### API proposals and isolated ready callbacks

`propose <id> <owner>` dispatches the pending API packet without editing source.
The sole integrator applies proposed bytes, builds generated metadata, then uses
`submit` with the stored proposal. Submission verifies paid evidence, original
source, exact proposed bytes and separately attributed root-generated paths.
`packet` renders the same no-tools contract for native execution. Invalid output
uses `fallback` with fresh Opus/high; failed checks/review use `repair` without
another API author call. Only confirmed nonexecuted HTTP402 permits Sonnet5/Sol
medium. Authentication, model mismatch, known HTTP infrastructure errors and
unknown completion do not authorize that fallback. Costs and negative evidence persist.

`runDeliveryPool` wraps the actual upstream bounded pool for caller-selected ready
callbacks. It refuses same/ancestor mutation paths and shared named resources,
holds canonical lease only for admission/snapshot/integration, and drains every started
callback before releasing its reservation, retaining unconfirmed child custody.
Recorded candidate/evidence paths survive failed callbacks. `createDeliveryCandidate`
reuses immutable private-source materialization and the ordinary `DeliveryHarness`
lifecycle; candidates cannot commit or supersede canonical outcomes. A dependent
requires a complete integrated parent and exact declared input bytes, never candidate success.
These are cooperative source snapshots outside the checkout; scopes name files.
Lifecycle detects out-of-scope writes; this is not hostile-process isolation.
Contract tests use explicit fixture identities; they do not prove live model overlap.
The CLI package is host-only development tooling: only `delivery-pool` imports its
standalone pool leaf. The frozen `dist/issue-8-program.js` controller's 92-module
local import closure contains neither this pool nor candidate adapter. It does
not acquire a new CLI/MCP entry or dependency inside its protected runtime.
Ordinary pool use requires the full development install; validation records its
installed version against the lockfile plus actual module SHA-256, not a per-request gate.
Production dependency hash coverage and symlink protections remain unchanged.

## Local verification

Package manifests deliberately use `latest` (user direction, 2026-09-10).
The committed lockfile records the exact dependency resolution used by `npm ci`,
builds and receipts; do not replace those manifest selectors with exact version pins.
Dependency updates need a fresh lockfile and compatibility checks, not automatic
installation during a task. Current resolution: harness 0.2.0, router 0.4.0,
both native-host adapters 0.1.2, and the development CLI `metaharness` 0.4.16.
The updated scanner exposed a broad `.claude` script allowance; that grant and the
obsolete Ruflo CLI allowance are removed. Live Ruflo access remains MCP-only.

```bash
umask 0022
npm ci
npm run build
npm test
```

CI tests use fake model processes and make no provider calls. Historical candidate model
execution requires successful native subscription preflights. Provider API
keys, ambient proxy variables, base-URL overrides, OpenRouter, and Requesty are
rejected. The controller injects only a loopback CONNECT endpoint backed by its
exact-origin Unix-socket broker.

## Historical closed transaction

This describes the implemented experiment, not permission to run its legacy
worktree-creating launchers. Current work has one writer on `main`, with no new
branches/worktrees. Keep the experiment's isolation checks intact; do not
retrofit or expand it merely to finish the product.

```text
frozen baseline + evaluator worktrees
  → distinct-host architecture and bounded critique
  → implementation
  → exact-path patch admission
  → clean candidate apply and offline build
  → public + independent + regression verification in parallel
  → independent Codex and Claude reviews
  → protected-input check and digest-chained receipt
  → receipt-bound seven-dimension programme envelope
```

Every repair resets the candidate and repeats admission, build, all verifier
lanes, and both reviews. Candidate commands require an OS network namespace;
dependency installation is a separate registry-pinned stage. Missing isolation
or either native host fails closed.

Native model execution uses independently enforced exact-origin, filesystem, and
resource boundaries. The trusted runtime exposes only a Git-masked candidate
snapshot, private output channel, empty private home, copied credential
capability, and one broker socket; it hides the controller, evaluator, verifier,
common Git object store, and other host paths. A systemd cgroup-v2 transient unit
enforces CPU, memory, PID, file-size, descriptor, and runtime ceilings. Timeout,
cancellation, or output overflow stops and verifies the exact unit before a
result can be accepted, and execution failure revokes active broker sessions.
Native subscription invocations have no project-imposed provider-dollar spend
ceiling. `subscriptionCostUsd: 0` records zero marginal provider-API charge in
the receipt/routing ledger; it is neither a budget or cap nor a claim of
unlimited subscription capacity. No subscription cost, token, request,
invocation or quota ceiling is permitted. Experimental time/output/resource
controls do not authorize quota-based routing or execution after native failure.

`NativeModelCandidate.reasoningEffort` optionally forwards Codex `low`, `medium`,
`high`, `xhigh`, `max` or `ultra` unchanged. Astra max and ultra are available;
absence uses the native default, not implementation-high/review-low overrides.
Use distinct candidate IDs for distinct model/effort configurations. See
ADR-0055 for task-based allocation. Unknown effort fails input validation;
native subscription/model unavailability pauses with the exact client error.

## Main modules

- `delivery-runtime.ts`, `delivery-workflow.ts`, and `delivery-cli.ts` own the daily
  durable task/check/handoff loop; `delivery-stage.ts` composes the actual one-stage
  kernel/pool/policy/verifiers. Internal replay of an unchanged response is disabled;
  this is not a ceiling on subscription requests. The host uses live structured
  Ruflo MCP for outcome synchronization; this CLI is not an MCP server and does not
  open managed memory or pretend local receipts prove remote synchronization.
- `kernel.ts` composes the real `HarnessKernel`, `AlgorithmRouter`,
  `VerifierRegistry`, `PolicyGate`, persistent routed pool, critique, consensus,
  and memory hooks.
- `candidate.ts`, `git-worktrees.ts`, and `repository-operations.ts` enforce the
  patched-candidate transaction and identity checks.
- `native-process.ts`, `network.ts`, and `models/` implement bounded native
  execution, first-party-only transport, routing, retry, breakers, and review
  independence.
- `acceptance-task.ts`, `contracts.ts`, `policy.ts`, `evidence.ts`, and
  `receipts.ts` validate schema-v2 exact-reference and schema-v3 verifier-only
  tasks, Ruflo, Agentic-QE, protected-input, and receipt boundaries. Objective,
  invariants, exclusions, route metadata, commands, generated outputs, QE
  profiles, and oracle mode are protected task data, not controller literals.
- `issue-8-programme-envelope.ts` binds the project-owned acceptance score to
  the frozen schema-v4 issue-8 receipt and makes a rejected score a non-zero
  launcher result.
- `programme-policy-v5.ts`, `programme-gate-contract-v1.ts`, and
  `programme-task-runtime-v1.ts` freeze schema-v5 replay law and protected task
  derivation. `programme-gates-v5.ts`, `programme-score-v5.ts`, and
  `programme-envelope-v5.ts` recompute every gate, score dimensions all-or-zero,
  and require an external policy-fingerprint anchor. `programme-envelope.ts`
  preserves frozen v4 replay while dispatching v5 only against an exact runtime
  expectation. The explicit fresh-ID v5 operator/launcher now derives a trusted
  pre-execution anchor and complete evidence contract; it is not a general or
  default promotion path.
- `programme-gates-v6.ts`, `programme-v6-program.ts`, and the V6 operator,
  policy-anchor, receipt, and replay modules bind transition-aware repair
  evidence and full native-runtime sidecars without changing frozen V4/V5 law.
- `frozen-cargo-lock-fixture.ts` reads V5/V6 patch-task locks as raw bytes from
  the exact attested ancestor baseline; only two exact historical task blobs may
  use the embedded legacy fixture.
- `metaharness-diagnostics.ts` parses the protected native Ruflo score snapshot;
  its exact Git blob digest must match the candidate receipt.
- `effective-config-command.ts` gives the two tracked project launcher files a
  strict, scoped admission gate; it does not claim to resolve ambient host config.
- `programme-v5-ruflo-runtime.ts` copies the two exact coordination-status files
  into a private networkless runtime instead of mounting live Ruflo directories.
- `.harness/manifest.json` is the canonical tracked control-plane manifest and
  identifies the repository's actual `.mcp.json` coordination surface.

The first acceptance definition lives in `config/issue-8-acceptance.json`.
V5 run `programme_v5_h0c_20260828_05` completed a passing transaction after one
pre-build repair, then was honestly rejected at 85/100 by its frozen V1 law.
The sibling schema-V6 path does not upgrade that historical evidence. V6 run
`programme_v6_h0c_20260828_01` failed closed at exact-origin enforcement and
replay preserved the failure. Fresh run `_02` passed at 100/100 with six bound
commands, seven native-evidence digests, two final native reviews, and no retry
or repair. Receipt `d9d244ef…0216`, candidate evidence `a1dc3071…ac7f`, envelope
`02c30ed3…9a06`, and provider-free replay `f1bcf0fe…bf02` complete H0c.

The legacy schema-v4 `launch-issue-8.mjs` path is bound to
`config/issue-8-acceptance.json`. The explicit schema-v5 operator uses
`config/programme-v5-acceptance.json`, which remains an issue-8 H0c activation
fixture; no general next-product patch launcher exists. Proposed
[ADR-0041](../docs/adr/ADR-0041-manifest-bound-controlled-observational-evidence-capture.md)
describes a separate single-attempt observational transaction, but no capture task
or controlled profile is registered yet. Neither patch task is an evolution
suite or promotion signal. Darwin/GEPA remains disabled until five training
tasks and five sealed holdouts satisfy the independent evaluator gate.

That capture plane now includes a non-authorizing, claim-rooted private-source
boundary. It checks out only the claimed commit through an isolated Git index,
after rejecting include/filter/attribute authority, cross-UID-writable Git
controls, and an unprotected bounded object store. It seals and re-verifies the
full tree, and returns an opaque local view with every
lease, attempt, and capture authority field false, host admission unevaluated,
and no build or execution API. Its tests use synthetic
primary and bare stores; no controlled profile, project run, source tree,
receipt, or measurement is created by the repository test suite.

Supervisor-service verification also protects the PostgreSQL 16.15 baseline
fixture, bounded reader, completeness oracle and immutable V1/V2 receipts. The
additive V3 replay contract pins those receipts and historical replay implementations
byte-exact and never invokes the historical runners. Its only invoked V3 profiles are
`baseline-v1`, `baseline-v2`, `branch` and `final-where`; each owns two distinct
fresh networkless containers with anonymous volumes and no published ports.
Acceptance requires `/proc/1/comm` to be exactly `postgres\n` and `pg_isready`
to succeed. The reader's 92 hostile/limit/private-brand KATs plus five exact-fixture
tests reject forged authority. V1's raw oracle expands direct ACL atoms; V2 binds 13,603 fresh
no-membership-role checks across six populated classes, with FDW/server explicit
zero classes. The V1/V2 receipts and V3 contract are test evidence only: they are excluded from
the Node oracle build and authorize neither migration nor runtime activation.
A pure lexical mutator and fail-closed replay support protect
eight branch deletions plus four record-set mutants. A separate source-pinned
quartet freezes 19 final-`WHERE` deletions and two batches of ten and nine. Each
rollback-only batch derives its expected bag from the unchanged raw oracle,
adds 15 independently counted hidden predicate witnesses, and proves the
unchanged projection is still equal before executing every mutant through a
loose closed eight-field decoder. The result is exactly 19 executed, 15/15
non-equivalent killed, four independently proved guard-equivalent, and zero
unresolved; two fresh networkless runs reproduced deterministic 11,963,849-
and 11,608,234-byte transcripts and complete rollback/cleanup. The sealed Node
oracle and public bundle remain byte-identical, so no product runtime input or
export changed. Hosted run [`33636424967`](https://github.com/sparkling/semantic-fabric/actions/runs/33636424967)
at exact `d0cc5fb938a1ff8b70859c19882934461fe23c5a` passes the exact Node 20/24 V3 matrix; no hosted run receipt is
tracked. Live bridge/store/runner
work belongs to Rust; Node execution expansion is closed.

The parent harness replaces Node 20's asynchronous recursive watcher with
explicit root/nested directory watches, three agreeing tree digests and an
8,192-watcher fail-closed ceiling that closes every partially opened watcher.

## Local Ruflo status boundary

Tracked `.mcp.json` is exact-empty and `.agents/config.toml` declares no project
MCP launcher. Both files and their current-state KAT are protected inputs. The
collector snapshots only `tasks/store.json` and `swarm/swarm-state.json` into
`0400` files beneath `0500` directories, then starts the exact pinned Ruflo MCP
package under bubblewrap with `--unshare-net`. Tests distinguish the sealed path
from shared-network and whole-directory mutants using network-namespace, TCP,
local-DNS, and AF_UNIX canaries. Hardlinks, symlinks, sockets, FIFOs, world-write,
and concurrent source mutation fail closed. Same-euid/same-gid `0664` inputs are
accepted only as cooperative, non-authoritative status evidence.

The private package snapshot reconstructs two historical Ruflo memory files from
protected deterministic gzip assets. A canonical manifest pins compressed and
decoded size/digest, exact target, compression and non-executable mode; bounded
descriptor-stable reads reject group/world-writable or executable protected
sources, links, swaps, traversal and drift, and bind each accepted source mode
and object across captures. Ambient target bytes are never opened or copied,
regardless of mode. Decoded buffers remain inside an opaque materializer snapshot;
the manifest and both blobs are exact Git-backed runtime-tree resources for the
Issue 8, V5 and V6 packed launchers. This executes the already-attested schema-v2
CLI identity; it creates no new identity, runtime export, fetch path or authority.
The Linux-only reader has byte/count bounds but no compressed aggregate or
decompression deadline, and its descriptor fallback still assumes `O_NOFOLLOW`.

## Retrieval flywheel boundary

Ruflo's retrieval-policy flywheel is governed outside the candidate transaction.
The project tracks a 48-task, hash-pinned candidate relevance anchor with
balanced deterministic halves, and the harness protects those files, the opt-in
settings, and inherited active-policy pointers. The candidate is not approved
for activation until maintainers review its labels and calibrate them against a
live retrieval baseline. Background tuning is disabled in tracked configuration.
The 2026-08-28 post-H0c check found no opt-in variables or `harness` worker in
the live daemon; that must be rechecked after each restart. H0c execution and
ordinary verified-outcome persistence do not opt into this flywheel.

The explicit Ruflo evaluation path is model-call-free, local, and
evaluation-only. At the dated 2026-08-28 check it no-opped because the visible
neural store had no eligible patterns; legacy counters and ReasoningBank are a different
store. Eight owner-visible records, four harvestable records, and a pinned
non-fallback embedding provider are only the threshold to begin evaluation, not
production readiness. If a future trial emits a signed receipt, replay verifies
receipt integrity, lineage, the gate fingerprint, and the gate decision over
sealed scores. Replay does not re-run retrieval. The separate daemon generation
path is not approved until it uses the same confirmed promotion transaction.
Promotion/operational receipts will also require durable retention outside the ignored
local state directories.

A fixed-seed 2×1 generic Darwin Shield diagnostic passed only 9/12 gates. It is
not project retrieval evidence and cannot activate or promote the flywheel.
