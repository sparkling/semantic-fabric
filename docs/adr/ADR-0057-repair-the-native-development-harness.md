---
status: proposed
date: 2026-09-24
updated: 2026-09-24
tags: [dev-process, metaharness, native-subscriptions, routing, frozen-evaluators]
depends-on:
  - ADR-0037
  - ADR-0048
  - ADR-0055
---

# ADR-0057: Repair the native development harness

- **Status**: proposed
- **Date**: 2026-09-24
- **Updated**: 2026-09-24 (verified shared-runtime contract)
- **Deciders**:
- **Tags**:

## Context

Semantic Fabric is a Rust-native, virtualisation-only OBDA application. It rewrites
an admitted SPARQL subset through R2RML mappings to live SQLite, PostgreSQL and MySQL
sources. Correct RDF identity, query results, policy enforcement, bounded execution,
source-generation lifecycle, cancellation and public serving behavior define success.
It is not a SWE-bench runner or generic software-repair benchmark.

The workspace contains `sf-core`, `sf-sql`, `sf-mapping`, `sf-validation`,
`sf-sparql`, `sf-conformance`, `sf-bench`, `sf-capture-supervisor`, `sf-serve`
and `sf-cli`. Cargo.lock and Rust 1.96 pin the Rust build. Node/npm tools in
`coding-harness/` remain private development/evidence infrastructure under ADR-0048;
they must not become dependencies of the deployable application.

ADR-0037 established the customised MetaHarness direction. ADR-0055 replaced its
closed-candidate per-commit ceremony with mandatory main-only delivery through
`DeliveryHarness`, the existing native conversation and ready-stage verification.
Repair must preserve that successful boundary and finish useful application work.
It must not revive historical worktree launchers, a competing model daemon or
the ignored `semantic-fabric-harness` experiment.

### Observed baseline, 2026-09-24

Physical checkout was `/home/claude/src/hm/semantic-fabric` on `main`, commit
`0f2902fc1f7f94f63cc61d9dcc422d2f6e0996da`. Existing changes were present in
`crates/sf-serve/tests/query_budget/reconstruction.rs`,
`crates/sf-sql/src/backend/sqlite/owned.rs`, and untracked
`crates/sf-sql/src/backend/sqlite/owned_demand_tests.rs`. They are outside this ADR's
scope and were not adopted, tested as accepted work, staged or changed.

| Evidence | What it proves and leaves open |
| --- | --- |
| `coding-harness/package.json` and lock | Active private package declares harness/router and both native adapters with `latest`; installed harness 0.2.0, Router 0.4.0 and host adapters 0.1.2 are inspected resolutions, not registry-currency claims. |
| `src/delivery-stage.ts` | Real upstream `HarnessKernel`, `AgentPool`, `AlgorithmRouter`, `PolicyGate` and `VerifierRegistry` verify a supplied source-bound native response. The callback itself invokes no model. |
| `src/delivery-runtime.ts` and `delivery-workflow.ts` | Durable native requests, declared checks, feedback-directed repair and source/evidence invalidation exist outside the kernel. Preserve this controller. |
| `src/delivery-contracts.ts` | `selectDeliveryRoute` uses task-class defaults or explicit routes; delivery routing is not demonstrated measured Router selection merely because Router is installed elsewhere. |
| `coding-harness/README.md` | `begin`, `bind`, `advance`, `submit`, `verify`, `finish` run within the existing native conversation. CLI never starts another host or commits. `scripts/slice.sh` wraps this lifecycle. |
| Current AGENTS.md | Claude-only since 2026-09-22, authorized 9router subscription transport, distinct Claude review executor; Opus replaces Fable. Codex is paused until explicit reauthorization. |
| Latest handover | G1-G6 remain incomplete; reports baseline harness/test failures, source-wide digest sensitivity, catalogue ownership mistakes and an explicit stopped loop. These are dated reports to reproduce, not current test results. |

Source-bound stage acceptance and an immutable hash chain are useful, but they do not
prove a native model was called by that kernel, direct product correctness, independent
review quality, restart safety or measured delivery acceleration. Each claim needs
its own evidence at its actual boundary.

## Decision

Keep the project entrypoint a thin adapter over the existing/customised shared
`@metaharness/harness` runtime in `coding-harness/`. Use Semantic Builder's verified
delegation contract without copying its product gates or operating authorization.
Keep one source integration owner, direct Cargo/product evaluators, immutable
outcomes and proportional per-outcome checks.

The shared runtime owns Router selection/escalation, API/native transports,
workspaces, receipts, repair/review, GEPA/evolution and learning. Fabric owns tasks,
frozen evaluators, mutation/ownership bindings, transport policy and sole integration.
The project entrypoint must not add a local Router, retry state machine, workspace
manager, receipt reducer, GEPA, Flywheel or AgenticOW controller. Reuse the existing
delivery lifecycle behind this boundary, not a second controller beside it.

Semantic Builder verified this boundary on main at
`df74b4911cf05fa7ecdcdfe13c5e0d8533e97b12`, run
`run-2026-09-24T18-24-37-723Z-4f9af644`, receipt
`sha256:2f2c08d4008bd7100f152056b0172157ad034783a9a7fa3561e6110b87a6c8a2`.
The successful receipt is Development evidence, not release authority or proof
that Fabric has implemented this Proposed ADR.

This Proposed ADR authorizes no implementation, scheduler restart, Codex restoration,
publication or deployment. It specifies a reviewable repair contract for a subsequent
authorized session. ADR-0055's current v1 priorities and evolution deferral remain
binding; this proposal does not narrow any product acceptance obligation.

### 1. Verify upstream APIs before changing the adapter

Inventory the effective installed packages, lockfile integrity, exports, declarations,
source and tests. Check live registry metadata before upgrading; do not treat a Brain
snapshot or another project's lockfile as current package truth. Compile a minimal
fake-worker exercise against each API actually selected before designing around it.

| Component | Use | Verification requirement |
| --- | --- | --- |
| `metaharness` factory | Scaffold and diagnostics | Inspect CLI help and generated artifacts in a temporary non-Git directory. A generated kernel/host scaffold is not a running delivery control plane. |
| `@metaharness/harness` | Ready-stage control plane | Prove installed `HarnessKernel`, `AlgorithmRouter`, `AgentPool`, `VerifierRegistry`, `PolicyGate` and receipt APIs on pass, fail and policy-denied cases. |
| `@metaharness/router` | Shared-runtime model/effort ranking | Verify delegation and downstream-verifier labels, not another adapter-local Router or route ledger. |
| Host adapters | Native host integration | Verify their actual configuration/output role; retain repository native callbacks and actual executor binding. Adapter presence is not invocation proof. |
| Darwin/GEPA | Deferred policy evolution | Verify application evaluator/native reflection injection before use; no stock `real`/synthetic score can promote Fabric policy. |
| AVO/Flywheel/AgenticOW | Deferred shared-runtime variation/learning | Require a named authorized objective and recorded runtime use; no adapter-local controller or v1 expansion. |

This table describes shared-runtime responsibilities, not dependencies to wire again
inside the adapter. Test its public delegation boundary and the runtime's actual
callbacks; an installed package or copied scaffold does not establish execution.

Source-grounded Brain results inspected for this ADR are
`metaharness/packages/harness/src/kernel.ts`,
`metaharness/packages/harness/__tests__/kernel.test.ts` and
`metaharness/packages/harness/README.md`. Inspect exact resolved code again during
repair. Upstream examples contain cost/retry defaults that cannot be imported as
subscription budgets. The current Fabric stage already uses `costUsd: Infinity`
and no replay retries for a submitted response; retain its no-quota semantics.

Keep the package private and manifests on `latest` with exact tested lock resolution.
Choose Node/tool declarations from the actual development runtime; if standardizing
on Semantic Builder's Node 24, use `@types/node: "24"` and verify the full type
closure instead of relying on `latest` to select that major. Rust runtime packaging,
Cargo.lock, strict checks and dependency-security controls remain independent.

### 2. Preserve the active native transport and executor

Both Codex and Claude Code are supported interface targets. Only currently authorized
hosts may execute. Current policy permits Claude Code through the configured 9router
subscription route and pauses Codex. Keep disabled-host contract tests offline.
Record requested and resolved native model, effort, host/client version and distinct
executor ID; preserve an explicit requested model and the selected main model.

Retain authorized `CLAUDE_CONFIG_DIR`, `ANTHROPIC_BASE_URL` and
`ANTHROPIC_AUTH_TOKEN` handling without reading token values into logs or evidence.
The local 9router route is an explicit subscription transport exception to older
generic no-proxy prose, not permission for arbitrary API fallback. Use the working
repository launcher/host contract; verify native readiness when a new invocation
needs it, and never replace transport silently.

Current AGENTS.md forbids OpenRouter, Requesty and provider API-key routes. The
prospective author route is isolated OpenRouter `deepseek/deepseek-v4.1-flash`,
followed on confirmed output rejection by declared native subscription repair.
It stays disabled until explicit repository authorization and transport-policy
amendment admit it. Neither historical `evolve-openrouter.mjs` nor Semantic Builder's
API allowance transfers authority. Native repair/review remains separate from API
authoring, with current native-host restrictions intact. No cost, token, request,
invocation, seat or provider-quota budget may gate, retry, route or score native
subscription work. Reported tokens describe efficiency, never availability.

If the requested native client/model is unavailable, preserve task state and report
its exact client/model/error for account recovery. Do not substitute another host,
lower effort or reinterpret a failed authentication as a normal source defect.
Metadata claiming direct login is neither required nor sufficient for the configured
subscription route; the actual native client result is authoritative.

### 3. Reuse the main-only outer delivery lifecycle

Retain the existing conversation as executor/coordinator and sole integration writer
on canonical `main`. Never create/switch branches, new worktrees or detached checkouts.
Historical recovery worktrees are read-only evidence. Responses go through the
existing `begin`/`bind`/`advance`/`submit`/`verify`/`finish` lifecycle. Any future
authorized API author is isolated and owned by that runtime, not a second integration
writer or automatically resumed background loop. Native-only execution remains
current policy until that separate authorization exists.

Register coherent public outcomes using `DeliveryTask`, exact scope, source identity,
owner/thread, declared build/acceptance argv and review route. Keep frozen evaluator
and task registration ownership separate from candidate mutation ownership. Record
baseline evaluator/configuration hashes and a prior evaluator revision; do not make
a task manifest self-reference its own commit.

For new files, repair admission safely if required: the current handover notes that
scoped paths must exist before `begin`. Do not create empty source files outside an
admitted writer merely to bypass that rule. An authorized harness repair must add a
tested absent-leaf path contract or the integrator must use an already supported
admission mechanism. Reject traversal, symlink and hardlink escapes.

Adopt pre-existing in-scope changes only through explicit `adoptExistingChanges`
and recorded starting hashes. Leave unrelated work untouched. Whole-source checks
and formal review need stable source, including unrelated files; agree a stable
interval with other owners before collecting evidence. Do not claim a moving
worktree is the reviewed candidate.

Retain kernel verification per ready stage and the runtime's durable request sequencing.
The entrypoint supplies task/configuration and returns runtime evidence references;
it does not implement retry, escalation, workspace management or another state store.
The upstream kernel's sequential loop is not a dependency scheduler, crash-resume
store or failure-feedback transport. A replay of the same submitted response is
not a repair. A fresh repair request carries the precise failed commands/review
findings and source/evidence digests.

### 4. Bind frozen acceptance to Fabric behavior

Before dispatch, prepare declared Rust builds, fixtures and external-source services.
The verifier owner freezes a deterministic regression and proves the original
candidate fails for the named behavior. Missing prerequisites, panic/timeout without
diagnosis and skipped live-database tests cannot substitute for that red baseline.

| Outcome area | Required evaluator evidence when affected |
| --- | --- |
| Rewriting/RDF identity | Exact terms, datatypes, blank nodes, bags/sets, joins, filters, NULL/error semantics and existing W3C/Ontop/differential fixtures; test admitted and rejected cases. |
| Query work/cancellation | Independent fixed or justified bounded expectations, exact/N-1 tests, sticky stop, producer/worker ownership, cap-one recovery and public endpoint tests. |
| Source generations | Actual SQLite/PostgreSQL/MySQL schema/role/DDL/reload behavior, verified lease lifetime, policy separation and refusal of unqualified guarantees. |
| Serving/security | Authentication, row policy, redaction, request/byte/work limits, parser isolation, native TLS, shutdown/reaping and post-prefix terminal-failure behavior. |
| Integration/release | Impacted crate builds and public regressions; combined G1-G6 and exact-artifact checks only at their declared integration/release boundary. |

Pin expected behavior independently of the implementation. A test that computes
its own threshold or copies a catalogue label can stay green with the feature
disconnected. Demonstrate discriminating mutations in isolated test fixtures or
controlled scoped candidates; do not temporarily revert concurrent user work.
An intentional backend no-op skip must be labelled not-run, never qualification.

Keep `tests/capabilities/catalog-v1.json`, generated capability matrices and README
in scope only when affected source hashes or supported claims require updates.
Use the established refresh tools, review the resulting claim boundary and run
the capability-matrix evaluator. A digest refresh is not capability qualification.
Do not add this engineering ADR to published navigation without specific instruction.

Do not equate Oxigraph/library support with Fabric's supported surface or import
Semantic Builder's Jena/ontology-creation gates wholesale. Fabric's actual native
oracles and public query behavior remain fitness authority.

### 5. Delegate routing and escalation to the shared runtime

Pass authorized task/transport policy through the runtime's existing route boundary.
Do not add a project-entrypoint Router, parallel route ledger or local escalation loop.
Candidate identity includes host, exact model, effort, adapter/client version and
task class. Separate `low`, `medium`, `high`, `xhigh`, `max` and supported `ultra`
identities; never clamp an explicit native effort to fit stale code.

Current Claude-only defaults remain Haiku for bounded mechanical work, Sonnet for
normal work/review and Opus for a named unresolved difficult question. Preserve
the current user restrictions and main model. Disabled Codex candidates may have
offline contract coverage, but cannot gain fabricated availability/performance
evidence. If reauthorized, admit their actual native model/effort identities.

After explicit API authorization, the shared runtime selects DeepSeek V4.1 Flash
authoring and declared native subscription repair/review. No same-task model race
or GLM intermediate tier is introduced. Confirmed output rejection carries exact
verifier feedback to declared repair; API account/authentication failure and unknown
completion stop that lane. Native subscription/requested-model failure pauses for
account recovery, not fallback. Structured-output failure is not native unavailability.

The runtime preserves capability floors, failed-route exclusions and outcome
attribution. All required checks and review precede successful quality evidence;
whole-outcome time remains distinct from model latency. Ordinary different-input
outcomes do not become comparative training pairs. The adapter neither trains a
separate Router nor restarts architecture/registration after routine findings.

### 6. Review, repair and finish the exact outcome

Run focused deterministic checks and required builds before formal review. An
independent reviewer receives the current green source, scope, authorities and
exact prerequisite evidence. Under current policy it is another Claude executor,
different from every implementation handoff; same-provider review must be labelled
honestly. A stronger review route answers a named question, not every routine edit.

Future opposite-vendor or dual-consensus review requires actual host authorization
and route availability. Never mark same-vendor evidence as dual-vendor or rewrite
a registered policy to evade an outage. Per-outcome review is sufficient unless
scope or risk requires more; do not repeat release-level ceremony per small commit.

Retain each failed run and add the smallest useful regression. Attribute failures:
confirmed author/output defect to declared runtime repair; evaluator mutation/defect
to verifier owner;
missing artifact or inconclusive process execution to integrator; unavailable model
to account recovery. A plain exit 1 stays unlabelled until diagnosed. No-progress
repair pauses for a concrete diagnosis rather than consuming repeated identical turns.

The sole integrator commits only the verified scope on main and calls `finish`
against that exact next commit. Recheck source and receipt identity before accepting
the result. Candidate completion alone does not release source-dependent work.
Preserve incremental commits; no push, tag, deployment or publication is implied.

### 7. Keep immutable evidence and optional memory

Consume existing shared-runtime delivery records rather than creating adapter
receipts. Preserve task/evaluator/source
digests, requested/actual routes, client and executor identities, prerequisite
hashes, commands/exits/durations, private output witnesses, review findings,
repair parentage, integration commit and unavailable-host state. Actual native
invocations and underlying provider model calls are different counts; unknown
provider calls stay unknown.

Retain interrupted, rejected and negative evidence append-only. Atomic writes,
durable stage IDs, duplicate/stale submission rejection and process-group cleanup
must survive restart. Latest-status indexes may change; immutable receipts may not.
Hash-chain verification checks evidence integrity, not product correctness or
remote-memory synchronization.

The shared runtime emits and reduces immutable routing outcomes exactly once;
the project entrypoint must not maintain its own reducer or Router snapshot.
Keep one integration owner and freeze selection policy per run. Revalidate mutable
read dependencies before accepting external/sibling evidence.

Use structured Ruflo MCP for optional recall and outcome/task state. Confirm the
connection's repository binding before storing ADR graphs or project facts.
`ruflo_user` stores only reusable validated lessons and exact retrieval verifies
each promoted key. Never use CLI wrappers, direct SQL or memory database files.
A memory outage is reported once and cannot block local product delivery.

This session's user-memory search/list worked. Exact retrieval of
`metaharness-phase-gating-proportionality` and `documentation-system-quality-rules`
failed with `Mcp error: -32603: Failed to execute MCP tool 'memory_retrieve':
policy-state-lock-timeout`; `ruflo/guidance_brain` returned the same failure.
Operational-harness and Flywheel-boundary patterns were retrieved. The current
project MCP served Semantic Builder, so Fabric ADR graph registration is deferred
to a Fabric-bound connection. Changing a namespace does not change its database.

### 8. Defer evolution and gate future promotion

ADR-0055 forbids expanding Darwin/GEPA/AVO, retrieval tuning and benchmark trains
during v1 completion. Keep that restriction. Ordinary delivery repair does not
require an evolution experiment or minimum training-set ceremony.

Any later explicitly authorized evolution delegates to the shared runtime's
GEPA/evolution/learning path. Fabric supplies frozen native evaluators, admitted
tasks and policy constraints, never a local GEPA, Flywheel, AgenticOW, AVO or
promotion controller. Its product source, requirements, tests, safety clauses,
Cargo dependencies and holdouts stay outside harness-policy mutation surfaces.

The runtime owns train/selection/holdout isolation, frozen execution identity,
negative evidence, promotion and rollback. Synthetic ranking, stock `real` scores,
small smoke runs or another project's benchmark cannot establish Fabric promotion.
Runtime `@metaharness/flywheel` evolution is distinct from optional Ruflo memory
learning; neither gets independent acceptance or integration authority. Package
installation and this Proposed ADR enable no evolution or autonomous generation.

### 9. Preserve security and real resource limits

Use strict input schemas, argv arrays, validated absolute/contained paths and
actual filesystem/process restrictions. Test out-of-scope writes, symlink/hardlink
escape, malicious tool output, log overflow, cancellation and surviving process
groups. Prompt instructions and post-hoc hashes are not a sandbox.

Allow only required subscription configuration in native environments. Exclude
provider API keys and production/cloud/source credentials; grant test-only source
access to the exact owned fixture. Protect and redact logs. Do not copy tokens,
connection strings, confidential source data or raw prompts into shared memory.
Preserve SQLite authorizer/defensive protections and native TLS/role boundaries.

Use disposable Fabric-owned database fixtures on loopback ports; never touch
Semantic Product's containers or volumes. Reuse the authorized fixture lifecycle
and record ownership/cleanup. Parallel read-only analysis is bounded by current
user controls; the 2026-09-23 handover records at most one spawned agent until
lifted. This is separate from historical independent-session capacity.

Serialize shared generated outputs, Git operations and fixture mutations. Measure
effective CPU capacity, memory and I/O before heavy work and during long checks;
adjust Cargo/test workers to observed host pressure. Native session limits and
local build concurrency are distinct. Resource ceilings are not provider quotas.

## Implementation sequence and executable handoff

1. Read current AGENTS.md/CLAUDE.md, harness README, ADR-0037/0048/0055 and the G1-G6
   ledger. Verify main, physical path, dirty files, live delivery owner and explicit
   scheduler holds. This ADR does not release the stopped loop.
2. Reproduce the chosen harness defect in focused fake-native tests. Record exact
   packages/APIs and classify reported baseline failures on a stable source view.
   Do not change product tests or broaden the repair to unrelated open G1-G6 work.
3. Freeze discriminating evaluator tests under separate ownership. Register one
   scoped repair outcome using the existing delivery contract and an acceptance
   plus build check; fix stale scope once before dispatch.
4. Make the entrypoint delegate to the existing shared runtime; prove injected API
   rejection/native repair without enabling a real API route. Bind authorized native
   execution through the current lifecycle, verify focused checks/build and obtain
   independent read-only review.
   Commit the verified slice and finish against its exact commit.
5. Prove one real admitted Fabric outcome through that path, including a meaningful
   negative oracle and restart/failure evidence. Report accepted behavior and elapsed
   time, not harness score or task count as product progress.
6. Retain existing evaluator/security joins, document remaining limitations and
   hand back exact evidence. Defer routing calibration expansion and all evolution
   until an authorized objective benefits from them.

Existing command surfaces below must be filled with real task/evidence paths. They
do not invent a new CLI or authorize execution of a stopped application programme:

```bash
cd /home/claude/src/hm/semantic-fabric
pwd -P
git branch --show-current
git status --short
npm --prefix coding-harness run build
npm --prefix coding-harness test -- __tests__/delivery-runtime.test.ts __tests__/delivery-workflow.test.ts
cd coding-harness
npm run delivery -- /home/claude/src/hm/semantic-fabric begin /absolute/task.json
npm run delivery -- /home/claude/src/hm/semantic-fabric bind task-id owner /absolute/native.json
npm run delivery -- /home/claude/src/hm/semantic-fabric advance task-id owner
npm run delivery -- /home/claude/src/hm/semantic-fabric submit task-id owner /absolute/response.json
# Continue returned implementation/check/repair/review stages, then:
npm run delivery -- /home/claude/src/hm/semantic-fabric verify task-id owner
# Sole integration owner makes the scoped commit before finish.
npm run delivery -- /home/claude/src/hm/semantic-fabric finish task-id owner FULL_COMMIT_SHA
```

Run `npm ci` only in the separate dependency-resolution stage when necessary.
Harness builds synchronize/harden generated manifests, so include intended generated
changes in scope and inspect them. For Rust slices declare affected `cargo build`
and focused `cargo test`/supported `cargo nextest run` commands. Use full workspace
checks at the integration/release boundary required by ADR-0055, not every ADR edit.

### Validation and acceptance criteria

- Offline tests prove upstream API use, exact native route forwarding, configured
  transport retention, API-key isolation, unavailable-model pause and no fallback.
- Adapter tests prove shared-runtime delegation, prospective DeepSeek rejection to
  declared native repair, no same-task race and no duplicate routing, workspace,
  receipt or evolution state. Real API calls remain separately unauthorized.
- Shared-runtime acceptance tests keep focused frozen checks per attempt and require
  impacted build/regression/review joins before integration. Green tests cannot hide
  failed native execution, invalid structured output or an empty patch.
- Delivery tests reject wrong branch/path, unowned mutations, stale/replayed
  submissions, forged review identity and missing/failed build or acceptance evidence.
- Recovery tests preserve failures, pending requests and locks, stop/reap child
  processes and avoid duplicate execution or a second integration writer.
- Frozen regressions fail for the named Fabric defect and pass through actual public
  behavior; live-source skips remain not-run. Existing Rust/runtime safety survives.
- Routing tests preserve explicit choices/capability, separate effort identities,
  require verifier quality before latency and reduce immutable learning once.
- One real outcome through an authorized author route is reviewed, committed and
  finished against exact evidence. Same-vendor review stays explicit while Codex is paused.
- Harness dependencies stay outside deployable Cargo artifacts; optional memory,
  plugins and evolution cannot change acceptance or become new v1 gates.
- Documentation-only validation checks ADR syntax, sequential number, links and
  line count. It does not claim the unrelated dirty Rust candidate was built or
  accepted and does not require publishing this engineering decision.

### Rollback and handback

Before repair, record working runtime/policy/lockfile hashes and active delivery
state. On regression, stop new dispatch and preserve candidate changes and all
evidence. Revert only the offending verified harness slice through normal scoped
main commits after owner reconciliation; never reset main, delete another writer's
work, rewrite receipts or drop historical recovery refs. Resume a valid pending
stage only when its exact source and prerequisites still match.

A mandatory delivery failure is repaired through its existing ownership boundary;
it does not authorize direct source authoring outside the harness. Optional memory
or learning failures leave delivery usable with honest local-only evidence. Preserve
current scheduler holds, host authorization and publication restrictions on rollback.

Hand back source/evaluator/task hashes, actual native binding, dirty-file disposition,
package/toolchain identity, commands/results/witness paths, independent review,
accepted commit/receipt, routing observations, fixture cleanup, remaining product
obligations and next ready action. Distinguish historical reports, reproduced defects,
unrun proposals and current accepted evidence.

## Consequences

### Positive

- Repair strengthens the active runtime without replacing its working outer lifecycle.
- Rust semantics and public source behavior remain the acceptance authority.
- Quality-first routing can improve accepted-outcome time using honest evidence.

### Negative

- Whole-source evidence requires coordination with unrelated active writers.
- Native transport/API drift and recovery contracts need explicit compatibility tests.
- Historical candidate/evolution assets remain as potentially confusing evidence.

### Neutral

- Claude-only native repair/review, prospective API authorization and deferred v1 evolution remain explicit.
- Harness, native workers, deterministic evaluators and publication retain separate authority.

## Links

- [ADR-0037: engineering MetaHarness](ADR-0037-dual-host-ruflo-engineering-metaharness.md)
- [ADR-0048: Rust production boundary](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md)
- [ADR-0055: v1 completion and release](ADR-0055-v1-product-completion-and-release-profile.md)
- [Mandatory delivery lifecycle](../../coding-harness/README.md)
- [Native stage implementation](../../coding-harness/src/delivery-stage.ts)
- [Task and route contracts](../../coding-harness/src/delivery-contracts.ts)
- [G1-G6 application ledger](../plans/sota-application-completion-programme.md)
- [Dated session handover](../plans/session-handover-2026-09-23.md)
- Reference: Semantic Builder `docs/adr/ADR-0051-upstream-metaharness-application-delivery.md`
  and `src/tools/semantic-builder-harness/` delegating to
  `src/tools/application-development-harness/`, verified at
  `df74b4911cf05fa7ecdcdfe13c5e0d8533e97b12`; run/receipt identity is recorded above.
