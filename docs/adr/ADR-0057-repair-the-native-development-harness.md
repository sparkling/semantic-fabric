---
status: proposed
date: 2026-09-24
updated: 2026-09-25
tags: [dev-process, metaharness, native-subscriptions, routing, frozen-evaluators]
depends-on:
  - ADR-0037
  - ADR-0048
  - ADR-0055
---

# ADR-0057: Repair the native development harness

- **Status**: proposed
- **Date**: 2026-09-24
- **Updated**: 2026-09-25 (source-audited implementation handoff)
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

### Observed baseline, 2026-09-24 and 2026-09-25

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
| Policy/implementation gap | At `84304bbd8baa2df1380e685a8da427c1de0caff8`, `delivery-contracts.ts:56,103` accepts Codex routes/tasks despite AGENTS.md:14 requiring Claude. Explicit reviewers also lack an enabled-host guard. Fix admission; offline interface compatibility is not execution authority. |
| Custody and path behavior | `delivery-runtime.ts:61-66` replaces digest-bound task JSON; it is not append-only storage. `begin` already uses `allowMissingLeaf: true` at line 106; the dated new-file admission complaint is not a current defect. |
| Latest handover | G1-G6 remain incomplete; reports baseline harness/test failures, source-wide digest sensitivity, catalogue ownership mistakes and an explicit stopped loop. These are dated reports to reproduce, not current test results. |

Source-bound stage acceptance and an immutable hash chain are useful, but they do not
prove a native model was called by that kernel, direct product correctness, independent
review quality, restart safety or measured delivery acceleration. Each claim needs
its own evidence at its actual boundary.

## Decision

Keep `delivery-cli.ts` thin over the existing `DeliveryHarness`; retain its external
native executor contract. Share published upstream mechanisms, not a fictional
portable Builder runtime. Builder's private
`@semantic-builder/application-development-harness` depends on workspace contracts:
do not import/copy it, add a sibling `file:` dependency or introduce a cross-repo service.

`@metaharness/harness` owns ready-stage kernel/pool/policy/verifier/recovery primitives
and hash-linked stage receipts. `@metaharness/router` owns any later model-quality
selection; Darwin owns any later authorized evolution. Fabric's outer controller
retains task dependencies, source/checkout/lease checks, requests, native response
binding, check/review/repair sequencing, durable task custody and exact-commit evidence.
The current conversation launches models; the delivery CLI does not. No new daemon,
workspace manager, Router engine, receipt reducer, GEPA, Flywheel or AgenticOW loop.

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

### 1. Preserve concrete upstream and project seams

Inventory the effective installed packages, lockfile integrity, exports, declarations,
source and tests. Check live registry metadata before upgrading; do not treat a Brain
snapshot or another project's lockfile as current package truth. Compile a minimal
fake-worker exercise against each API actually selected before designing around it.

| Component | Use | Verification requirement |
| --- | --- | --- |
| `metaharness` factory | Scaffold and diagnostics | Inspect CLI help and generated artifacts in a temporary non-Git directory. A generated kernel/host scaffold is not a running delivery control plane. |
| `@metaharness/harness` | Ready-stage control plane | Prove installed `HarnessKernel`, `AlgorithmRouter`, `AgentPool`, `VerifierRegistry`, `PolicyGate` and receipt APIs on pass, fail and policy-denied cases. |
| `@metaharness/router` | Model-quality prediction/selection | Installed 0.4.0 is cost-optimizing; `AlgorithmRouter` is stage routing, not this model Router. Delivery currently uses static policy. Do not claim measured selection or add a second custom engine. |
| Host adapters | Native host integration | Verify their actual configuration/output role; retain repository native callbacks and actual executor binding. Adapter presence is not invocation proof. |
| Darwin/GEPA | Deferred policy evolution | Verify application evaluator/native reflection injection before use; no stock `real`/synthetic score can promote Fabric policy. |
| AVO/Flywheel/AgenticOW | Deferred variation/learning | Not required by active delivery. Require a named authorized objective and verified upstream API before adding any package; no v1 expansion. |

Dependencies remain in `coding-harness/package.json`/`package-lock.json`, private and
outside Cargo. Preserve `DeliveryHarness` public methods and
`verifyNativeStage(request,response,reasons)`: request/response are immutable stage
inputs; returned `NativeStageResult` includes kernel receipt evidence. No API redesign
or new package is needed for this repair. `models/routing.ts` is a separate custom
history/ranking path, not imported by delivery; leave it and legacy v5/v6/issue-8
execution paths alone unless an active dependency proves an in-scope defect.

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
external native host contract; verify readiness when a new invocation needs it.
`delivery-process.ts` runs deterministic checks with model credentials removed; it
is not a native-launcher environment and must not be changed to inject gateway tokens.

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

Use exact `DeliveryTask` schema v1, not invented evaluator fields: required
`id`, `requirement`, `owner`, `thread`, `taskClass`, `host`, `scope`, `checks` plus
`schemaVersion:1`. Each check supplies `id`, `kind`, argv array and repository-relative
`cwd`; at least one `acceptance` and one `build` are mandatory. `requested` needs
`selectionReason`; `reviewer` carries `{host,model,effort}`. Keep evaluator revision
and hash pins in an integrator-owned handoff until a tested schema extension exists.
New leaf files under existing parents are already admitted; preserve that contract.
`resolveWorkspacePath` can reject hardlinks, but delivery currently does not request
that option. Add exact path/adversarial tests before claiming full mutation isolation.

Adopt pre-existing in-scope changes only through explicit `adoptExistingChanges`
and recorded starting hashes. Leave unrelated work untouched. Whole-source checks
and formal review need stable source, including unrelated files; agree a stable
interval with other owners before collecting evidence. Do not claim a moving
worktree is the reviewed candidate.

Retain kernel verification per ready stage and the outer controller's request sequence.
The entrypoint supplies task/configuration and returns existing evidence references;
it does not add a second retry controller, workspace manager or state store.
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

### 5. Keep routing policy separate from execution

Keep `selectDeliveryRoute` as current static bootstrap, with enabled-host admission
enforced before task state changes and rechecked on bind/resume/submit. Historical
Codex records remain readable, but cannot execute while paused. Do not add a new
Router, parallel route ledger or escalation loop to fix this admission defect.
Keep structural `parseDeliveryTask`/`route` parsing separate from enabled-host
execution guards so `read`/status can still inspect old Codex evidence unchanged.
Candidate identity includes host, exact model, effort, adapter/client version and
task class. Separate `low`, `medium`, `high`, `xhigh`, `max` and supported `ultra`
identities; never clamp an explicit native effort to fit stale code.

Current Claude-only defaults remain Haiku for bounded mechanical work, Sonnet for
normal work/review and Opus for a named unresolved difficult question. Preserve
the current user restrictions and main model. Disabled Codex candidates may have
offline contract coverage, but cannot gain fabricated availability/performance
evidence. If reauthorized, admit their actual native model/effort identities.

Prospective API authoring is a separate, blocked design slice: DeliveryTask/native
response schema currently requires native-subscription identity. Do not squeeze
DeepSeek through a native alias or fabricate a subscription witness. Future transport
authorization must define typed API provenance, isolated credentials and native
repair/review handoff before code or real API calls. No model race or GLM tier.

Current failures retain exact check/review feedback in the same delivery outcome.
Unavailable native models pause for account recovery, never fallback. Future Router
integration requires zero subscription prices, no price/usage ranking, downstream
quality labels and paired evidence before latency claims. Its cost-oriented defaults
are not policy. Do not add calibration/evolution to complete this repair.

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

### 7. Preserve evidence and optional memory

Retain project-owned `DeliveryRun` custody and upstream stage receipts as different
evidence. Existing latest task JSON is atomically replaced with accumulated history;
that is not append-only disk custody or cryptographic native attestation. Preserve
task/evaluator/source
digests, requested/actual routes, client and executor identities, prerequisite
hashes, commands/exits/durations, private output witnesses, review findings,
repair parentage, integration commit and unavailable-host state. Actual native
invocations and underlying provider model calls are different counts; unknown
provider calls stay unknown.

Retain interrupted, rejected and negative entries; never rewrite them as passes.
Atomic writes, durable stage IDs, duplicate/stale submission rejection and process-group
cleanup must survive restart. Latest task state may change; retained stage results may not.
Hash-chain verification checks evidence integrity, not product correctness or
remote-memory synchronization.

Delivery has no active learned-routing outcome reducer; do not claim one from package
presence. Keep one integration owner and stable route per run. Revalidate mutable
read dependencies before accepting external/sibling evidence. Upstream `ReceiptLog`
does not replace task records, command logs, leases or exact-commit custody.

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

Any later explicitly authorized evolution uses upstream Darwin/GEPA with Fabric's
frozen evaluators and native reflection callbacks. No generic installed shared
evolution service exists here. Do not add a local optimizer, Flywheel, AgenticOW,
AVO or promotion controller. Product source, requirements, tests, safety clauses,
Cargo dependencies and holdouts remain outside policy mutation scope.

Any future evolution design must assign train/selection/holdout isolation, frozen
execution identity, negative evidence, promotion and rollback to explicit owners.
Synthetic ranking, stock `real` scores or another project's benchmark cannot establish
Fabric promotion. Upstream evolution and optional Ruflo memory learning are distinct;
neither gets independent acceptance or integration authority. This Proposed ADR and
package installation enable no evolution or autonomous generation.

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

All paths below are relative to `coding-harness/`. Read current instructions and
reconcile live owner/dirty files before admission; no stopped programme resumes.
One registered harness-repair outcome owns F0-F3; keep separate frozen evaluator and
candidate owners. Correct scope once, not a new task for every review finding.

| Slice / owner | Exact files and decision | Required exit evidence |
| --- | --- | --- |
| F0 integrator + verifier | `__tests__/delivery-runtime.test.ts`, `delivery-workflow.test.ts`, fixtures; freeze assertions for Claude-only admission before candidate edits. | Red: Codex task, explicit Codex reviewer and resumed old Codex run cannot execute. Historical records still inspect. No native/API calls in tests. |
| F1 existing native writer | `src/delivery-contracts.ts`, `delivery-runtime.ts`, `delivery-workflow.ts`; retain `delivery-stage.ts` kernel seam and `delivery-cli.ts` lifecycle. | Guard before `begin` writes state and before bind/resume/submit; same-provider distinct reviewer identity preserved; no silent route conversion. |
| F2 writer + verifier | `src/delivery-workspace.ts`, `workspace.ts`, `delivery-process.ts` only for reproduced safety/recovery defects; same two test suites. | Existing missing-leaf admission, denied symlink/hardlink paths, source/prerequisite drift, interrupted check, no-progress pause and exact-lock recovery tests. Scope hashes are not a process sandbox. |
| F3 integrator + independent Claude review | Build/test complete patch; `package.json`/lock only for proven dependency fixes. Keep output synchronizers, not legacy programme execution, in build closure. | Current green candidate, exact separate review executor, scoped main commit and `finish`; no unresolved failed check hidden by baseline failures. |
| F4 current product owner | One already admitted ready G1-G6 outcome and its frozen Cargo checks; no new product scope from harness repair. | Public behavior and meaningful negative oracle, declared build, separate review and exact committed evidence. |

F0 is the first phase of the same admitted task, not a separate task or required red
commit. Verifier records base commit, frozen test patch hash and intended failing
assertion before F1; candidate cannot edit that frozen authority. The final green
commit includes those tests. Preserve mandatory-green `verify`/`finish`; no invented
evaluator-revision field, red acceptance or separate maintenance exception is needed.
Use task checks with `cwd:"coding-harness"`: build argv `["npm","run","build"]`;
acceptance argv `["npm","test","--","__tests__/delivery-runtime.test.ts",
"__tests__/delivery-workflow.test.ts"]`. Set host/reviewer `claude-code`, bind actual
model/effort/executor, and name exact existing source/test files in `scope`.

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
Build invokes `scripts/sync-harness-manifest.mjs` and `scripts/harden-build.mjs`:
inspect `.harness/manifest.json`, `.harness/controller-build.json` and `dist/` output.
Declare tracked generated changes in scope. `harden-build.mjs` still records
`dist/issue-8-program.js`; that legacy digest is not proof of delivery execution.
For Rust slices declare affected `cargo build`
and focused `cargo test`/supported `cargo nextest run` commands. Use full workspace
checks at the integration/release boundary required by ADR-0055, not every ADR edit.

### Validation and acceptance criteria

- Offline tests prove existing upstream stage API use, permitted native route
  admission, unavailable-model pause and no fallback. CLI launches no native model.
- Existing kernel seam runs once per submitted response; task custody and repair
  remain in the outer controller. No API transport or duplicate state is added.
- Delivery acceptance tests keep focused frozen checks per attempt and require
  impacted build/regression/review joins before integration. Green tests cannot hide
  failed native execution, invalid structured output or an empty patch.
- Delivery tests reject wrong branch/path, unowned mutations, stale/replayed
  submissions, forged review identity and missing/failed build or acceptance evidence.
- Recovery tests preserve failures, pending requests and locks, stop/reap child
  processes and avoid duplicate execution or a second integration writer.
- Frozen regressions fail for the named Fabric defect and pass through actual public
  behavior; live-source skips remain not-run. Existing Rust/runtime safety survives.
- Routing tests preserve allowed explicit choices and distinct effort identities;
  static bootstrap never claims measured learning or immutable-outcome reduction.
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
