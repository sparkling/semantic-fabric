---
status: proposed
date: 2026-09-26
updated: 2026-09-28
tags: [dev-process, metaharness, ruflo, parallel-delivery, openrouter]
depends-on: [ADR-0037, ADR-0048, ADR-0055, ADR-0057]
---

# ADR-0058: Upstream-first parallel hybrid delivery

- **Status**: Proposed; review and repair plan only
- **Date**: 2026-09-26
- **Deciders**:

## September 28 implemented transport slice

Owner authorized direct implementation, repair, builds and tests for all work,
with both configured native hosts and isolated OpenRouter execution. The harness
remains optional orchestration/learning, not a compulsory application gate.
Application programmes remain paused; no push or publication is authorized.
This supersedes the narrower maintenance-only wording in the historical plan.

The existing delivery CLI now exposes `propose`, using source-bound pending
requests and an isolated API adapter. New tasks without an explicit host default
to DeepSeek/high; explicit native assignments remain unchanged. API handoffs
record actual API authentication and usage, never pretend native execution.
Known malformed output is task-held; unknown completion and abandoned process
reservations stop dispatch. Live process-owned reservations permit concurrent
calls. Source application, checks and acceptance stay with the existing main-only
integrator and MetaHarness verifier seam. No second scheduler was introduced.

Node 24 engines/declarations and strict library checking are active. Validation
passed the existing build (TypeScript, manifest sync, runtime hardening) and all
46 focused delivery API/runtime/workflow tests. Generated outputs require non-group-writable
modes; the runtime hardening guard remains unchanged.

This is partial transport evidence, not full ADR completion. Native fallback,
shared native/API planning, initial live cascade, compatible pool/source handoff
and restart admission still require proof. Failed native attempts remain evidence;
injected transport tests do not stand in for real model execution.

## September 28 implementation handoff (current plan)

This section supersedes conflicting September 26 proposals below. Those sections
remain historical source findings, not another executable checklist. Status remains
Proposed: documentation refresh does not claim this repository implemented Builder.
Implement only after the owner authorizes this ADR; reconcile explicit local host
and concurrency restrictions in the same first slice. No publication or application
programme resumption follows from documentation or harness acceptance.

### Finish criteria and scope discipline

Compose upstream tools with thin project adapters, not a second framework.
Preserve working acceptance, parallelism and learning. Do not copy Builder's entire
private runtime, add a scheduler/service, regenerate a working harness, or install
unused packages. Before changing a seam, show its actual local defect or missing
contract; reuse the existing implementation when it already satisfies the contract.
Keep one task/outcome through repair; no new task, calibration, inventory or repin
for each finding. Focused regression, independent review and impacted build once
per coherent slice; broader join at acceptance, not between every small repair.
Propose removal of mandatory worker-only maintenance repair where it causes
self-hosting loops; amend local instructions with owner approval before using direct
scoped repair. Application execution still uses the project harness. Direct edits
never inherit old worker acceptance receipts.

Completion means: configured routes work, deterministic checks and independent
review pass, failure/repair and source-handoff contracts are proved, and restart
admission passes without starting the programme. Successful cheap/frontier cascade
counts as success. Perfect DeepSeek-only execution, model bake-offs, highest harness
score, a PR system, or new evolution benchmarks are NOT completion gates.
Preserve/exclude pre-existing managed helper files. Concurrent updates are expected
only where owner-confirmed; never absorb them into scoped harness commits.
Unexpected owned source changes still require reconciliation.

### Reuse and remove map

| Responsibility | Required composition / removal |
| --- | --- |
| Stage execution | Retain actual `@metaharness/harness` kernel/pool/verifier callback seam and project acceptance. Factory/templates scaffold; they do not prove active execution. |
| Concurrent ready callbacks | First assess `runBoundedPool` in Ruflo `v3/@claude-flow/cli/src/services/bounded-worker-pool.ts`, as documented in Builder ADR-0054 R8. Wrap existing outcome callbacks, not a new agent platform. |
| Export compatibility | Pool is exposed through `./dist/*`, not stable dedicated high-level API. Resolve installed export/declarations on Node 24, record package/version/lock and cancellation behavior. Declare any dependency actually used; no reliance on an accidental global install. |
| Scheduling authority | Pool bounds callbacks; project retains dependency acceptance and same-file/ancestor-path/named-resource exclusion. Do not remove them or claim pool handles them. Root releases children only after integrated predecessor source reaches their input. |
| Source isolation | Non-Git candidate snapshots/patches, one integrator on the authorized checkout specified below. No new branches/worktrees/PR machinery is required for this repair. Earlier worktree proposal needs separate future authorization. |
| Planning | Share architecture renderer across native/API wrappers; packet-only mode explicitly has no tools, retains admitted source and requests a plan instance, not schema definition. Native contract stays unchanged. |
| Review | One existing coordinator for ordinary/recovery paths; one production prompt renderer shared by native/API wrappers and qualification. Remove duplicate active prompts/coordinators only after preserving stronger checks. |
| Evidence | Reviewer sees current admitted source, patch, task and sanitized deterministic results/file-policy facts; never hidden evaluator, expected verdict or author rationale. Bind qualification to original source/evaluator, not today's checkout. |
| Progress | Use existing process/progress seams for PID/start, phase, throttled activity, safe tool names and completion. Bound capture; callback errors cannot orphan children. No raw prompts, tool bodies, credentials or reasoning in ordinary logs. |
| Repair | Invalid completed output retains actual usage/cost and safe diagnostic, not completion-unknown. Infrastructure/stale-source belongs to integrator; evaluator mutation to evaluator owner. Identical failed patch stops, changed repair remains eligible. |
| Learning | Preserve existing Router/outcome/memory/evolution connections and sole-writer reduction. Keep explicit local promotion/qualification boundaries; packages installed or settings present are not operational-learning proof. No new benchmark programme or removal of learning to simplify delivery. |

Builder inspected pool versions 3.38.20 locally and 3.45.0 on GCP, not this repo.
Its pre-aborted signal may still start callbacks; timeout can return before a
noncooperative callback stops. Check cancellation before dispatch and retain resource
ownership until child termination is observed. Verify on the intended server before
claiming deployment parity. Existing supported pool may be retained if equivalent.
Keep remote inference concurrency separate from Cargo/.NET/Node job workers.
Sample effective CPU, interval utilization, memory and I/O pressure before heavy
work, every 30 seconds and at refill; account for all jobs on that host. Use supported
worker controls, isolated targets/DBs/ports; do not invent CPU-based model-session caps.

Inventory factory/kernel, Router, Darwin/GEPA, AVO/Flywheel, AgenticOW and QE only
where present or required by accepted local decisions: label installed, configured,
operational and deferred separately. Reuse existing hooks/adapters rather than
local substitutes. Preserve ordinary outcome capture and learned policy reduction;
promotion still needs real evaluator and held-out proof. Missing optional promotion
evidence neither strips learning nor blocks delivery, and does not authorize a
new evolution campaign.

### Approved target routing; local activation remains an implementation step

- Eligible planner/implementer and separately qualified fresh-context reviewer:
  `deepseek/deepseek-v4.1-flash` via direct OpenRouter, not 9router API forwarding.
  Qwen/GLM are not this target. Same-model review uses a separate qualified reviewer identity and fresh API request,
  without author conversation/rationale; it is not cross-model/developer consensus.
  Preserve stronger declared review policies.
- Capability/output failure: configured capable native subscription repair,
  Builder reference Opus/high `cc/claude-opus-5-5[1m]`. Verify exact client alias
  locally; no model rename presented as runtime proof.
- Only proven nonexecuted HTTP402 credit rejection permits lighter Sonnet/Sol
  subscription fallback. Admit Sol only where Codex is authorized; Claude-only
  repositories use eligible Sonnet after policy amendment. Never Opus solely for
  exhausted credits. Authentication, unknown completion and local request-limit
  failures do not authorize fallback. Native unavailable pauses with exact
  client/model/error; never switch transport silently.
- Enforce maximum $1/request before dispatch; no cumulative/task/session ceiling.
  Preserve actual versus unknown charges, reservation/replay protection and actual
  fallback author/reviewer identities. Keep API keys out of native/tool/verifier
  environments. Existing native subscription gateway settings stay intact.
- Builder defaults: high reasoning, `reasoning.exclude=true` (hide returned thoughts,
  not disable computation), 131072 max output tokens, 1800000ms API timeout,
  2000000 response bytes, provider ceilings $0.50 input/$2 output per million.
  These are configurable defaults, not test-sized budgets or performance promises.
- New confirmed-output holds bind task digest, policy and packet class, never
  disable unrelated tasks. Retain same-task exclusions and unknown-charge stops.
  Old unscoped failures remain effective until explicitly evidence-migrated;
  never erase receipts or rotate a policy merely to bypass a hold.

### Builder donor evidence and bounded acceptance

Read Builder ADR-0051/0053 updated September 28 and ADR-0054 R8, then inspect donor
diffs rather than cherry-picking across unrelated runtimes:
`cf00eb06b` progress/cancellation and maintenance-authoring amendment;
`3430c6365` shared review/file-policy evidence/unchanged-repair stop;
`803cff426` production-context reviewer qualification;
`e759396a6` confirmed-credit Sonnet/Sol fallback;
`fa82f77fa` shared planner and completed-proposal diagnostics;
`701ea878b` task-scoped holds and fixed finish criteria.
Donor path prefix: `src/tools/application-development-harness/src/`;
inspect `native-worker-contracts-v1.ts`, `control-plane-review-v1.ts`
(if renamed, follow `ordinaryReviewAgentV1`), `native-model-invocation-v1.ts`,
`hybrid-worker-implementation-v1.ts`, `hybrid-suspension-v1.ts`,
`hybrid-credit-fallback-v1.ts`, and qualification/runtime adapters.

Builder live `deepseek-full-cascade-live-20260927-r2` passed in 236410ms:
DeepSeek plan, rejected implementation, Opus repair, DeepSeek approval; all
verifiers passed, API cost $0.02369829258. Receipt
`sha256:9f7324f0d77de7b0622e788a96a2e783f56f4f997c9d8455f34df58a3be8729e`
verifies; final scoped regression was 181 tests/build/restart audit.
This proves Builder cascade, not sibling adoption or perfect DeepSeek authorship.
Upstream ADR-127 (Accepted June 18) uses exact search/replace sentinel blocks;
Builder uses JSON edits. Keep safe exact matching; neither transport is proof
of model correctness. Do not blame JSON without captured evidence.

Acceptance in this repository, within one repair outcome:
1. Reconcile policy/config once, then implement only local gaps mapped below.
2. Inject transport/process tests for source/review binding, failed and unchanged
   repair, safe progress, cancellation, task A held/task B eligible, HTTP402 versus
   auth/unknown failures, credit fallback independence, $1 bound and no total cap.
3. Exercise two genuinely independent existing outcomes through compatible upstream
   pool with actual overlap evidence; retain conflict exclusion. Reuse accepted local
   proof when source-current. Prove one dependent receives accepted parent source.
   If programme graph is truly serial, report that constraint; do not invent
   application independence, strip business dependencies or launch filler tasks.
4. Run one local end-to-end cascade with independent review and authoritative
   checks, accepting legitimate frontier repair. Preserve failed attempts.
5. Integrate verified source serially; run read-only restart admission and document
   active/ready/manual/blocked work separately from harness defects. Record
   installed versions, exact commands, source/receipt identities, costs and limits.
   Update this ADR to Implemented only for proven scope; leave programme stopped.

## Fabric-specific implementation map (September 28)

Rechecked clean main `99af9439121ea0879496b8bac61a349a3caa1cf2`.
Fabric is Rust semantic-data delivery; preserve public-query and conformance
acceptance plus SQLite/PostgreSQL/MySQL isolation. Harness is development-only.

- Keep `coding-harness/src/delivery-cli.ts` begin/bind/advance/submit/check/verify/
  finish/pause/resume lifecycle and `delivery-stage.ts` kernel seam. Its supplied
  callback result is not proof a model was launched; wire executor only where absent.
- `delivery-contracts.ts` native handoff rejects slash-qualified model IDs.
  Add explicit API/native transport variant, not a fake native OpenRouter identity.
  `assertHostEnabled` currently has no execution callsite: wire admission without
  breaking historical receipt parsing.
- Reuse `delivery-runtime.ts` / `delivery-workflow.ts` source/check/review join;
  consolidate shared prompts/review coordination, bounded progress, exact error
  attribution and unchanged-failed-candidate stop at those seams.
- Keep `delivery-workspace.ts` OS lease/PID custody and main-only integrator;
  separate non-Git candidate observations from canonical integration. Global
  `active.json` writer lease must not be removed before candidate ownership exists.
- `delivery-process.ts` inherits target/job/DB settings, not resource allocation.
  Allocate private Cargo targets, SQLite files, database schemas and ports;
  validate locks across invocations. Serial learning reduction keeps one writer.
- Local AGENTS remains Claude-only/native-only; amend owner-authorized routing
  policy and relevant ADR-0037/0055/0057 instructions together before API activation.
  Preserve Fabric's hold on expanding evolution research; retained learning and
  honest local outcome capture do not require that separate programme.
- Package engines currently admit Node20 and `@types/node: latest` is not a
  Node24 guarantee. Pin major24 declarations and prove strict dependency closure.

Future implementer validation: commands below are check payloads under the existing
delivery task, not permission to bypass it. Direct maintenance commands require the
explicit scoped maintenance amendment first.
```bash
npm --prefix coding-harness run build
npm --prefix coding-harness test -- --run __tests__/delivery-runtime.test.ts __tests__/delivery-workflow.test.ts
```
Build already includes TypeScript, manifest sync and hardening; retain all three.
Add focused route/admission, process and pool contracts in existing test layout.
Use affected Rust/database tests and ADR-0055 integrated public-feature join;
skipped database tests stay not-run, not passing. No broad release programme.

## Historical September 26 assessment (superseded instructions)

Source findings below retain their original date and evidence limits. Earlier Qwen,
worktree/PR prerequisites and heavier orchestration proposals are historical, not
current implementation direction. Use September 28 handoff above for execution.

## Purpose and authority

Make independent Fabric outcomes execute concurrently through the existing
delivery harness, with isolated source/build/database state and exact integration
evidence. Preserve Rust product acceptance, native escalation and independent
review. Neither role records nor an installed kernel prove parallel execution.

The requested target is Node 24, direct OpenRouter
`deepseek/deepseek-v4.1-flash` authoring and independent
`qwen/qwen3-235b-a22b-2507` review, native subscription escalation through configured routes,
at most $1 per metered request and no cumulative spending cap. Native subscriptions
have no cost, token, request, invocation or provider-quota budget. Model admission
follows ready dependencies and measured host resources, not a fixed session cap.
The requested source workflow uses task branches/worktrees and reviewed PRs.

That target conflicts with current main-only personal instructions and Fabric's
Claude-only/native-only policy. This proposed ADR records those conflicts as
repair prerequisites; it does not abandon the target or silently authorize
worktrees. First reconcile the governing instructions explicitly, then change
guards and dispatch. This review changes no execution policy, source, package,
runtime state, scheduler, publication or application pause.

## Exact local baseline

Inspected `/home/claude/src/hm/semantic-fabric`, canonical `main`, clean at
`f8d8acf817f888b6d63dd5960ac56cc78de4db30` on 2026-09-26. Local Node is
`v24.14.1`. Installed `coding-harness/node_modules` contains
`@metaharness/harness` 0.2.0, `@metaharness/router` 0.4.0 and `@types/node`
20.19.43. `@claude-flow/codex` is absent at that package location; this is not
a host-wide absence claim. Manifests use `latest`, but engines admit Node 20.

ADR-0037 is Accepted, dated 2026-08-25, updated 2026-09-10. ADR-0055 is
Accepted, dated 2026-09-07, updated 2026-09-23. ADR-0057 remains Proposed,
dated 2026-09-24, updated 2026-09-25. Their historical assertions are not
substitutes for current source inspection or runtime proof.

This is local source/package evidence, not GCP inspection or server validation.
No model job, build, database test or throughput experiment ran. No Fabric-scoped
Ruflo MCP connection is available in this session; Builder's connection must not
hold Fabric facts. ADR graph registration is pending the correct project connection.
User memory was searched and the exact phase-gating pattern retrieved; its
superseded native-only guidance cannot override the current requested target.

## Findings: active, miswired and missing

Paths below are repository-relative at the baseline commit.

| Area | Evidence | Classification and implication |
| --- | --- | --- |
| Active entry | `coding-harness/package.json:11`; `coding-harness/src/delivery-cli.ts:8` | `delivery` invokes the existing external-host lifecycle, not a background worker launcher. Keep this public entry. |
| Real kernel | `coding-harness/src/delivery-stage.ts:8` | `verifyNativeStage` uses `HarnessKernel`, `AgentPool`, `AlgorithmRouter`, `PolicyGate`, `VerifierRegistry`. Its worker returns an already supplied response; it invokes no model or concurrent task scheduler. |
| Outer controller | `coding-harness/src/delivery-runtime.ts:92`; `coding-harness/src/delivery-workflow.ts:51` | Source-bound begin/bind/check/repair/review/finish exists. Declared checks are sequenced; task state is not a DAG dispatcher. |
| Writer exclusion | `coding-harness/src/delivery-runtime.ts:114`; `coding-harness/src/delivery-workspace.ts:15` | One `active.json` writer and main-only root guard deliberately reject independent writing lanes. Increasing model settings cannot remove this serialization. |
| Operation custody | `coding-harness/src/delivery-workspace.ts:87` | OS lease plus PID/start identity and child checks exist. Preserve recovery; do not replace with an unowned lock file. |
| Host admission defect | `coding-harness/src/delivery-contracts.ts:69` | `ENABLED_HOSTS` and `assertHostEnabled` exist but have no source callsites. Structural parsing accepts Codex; begin/bind do not enforce the declared Claude-only guard. This is miswired code, not a missing constant. |
| API route missing | `coding-harness/src/delivery-contracts.ts:53`; `coding-harness/src/delivery-contracts.ts:31` | Model validation rejects slash-qualified IDs, hosts are native-only, handoff authentication is `native-subscription`. Direct OpenRouter requires an explicit transport variant, not relabelling a native handoff. |
| Runtime mismatch | `coding-harness/package.json:22`; `coding-harness/package.json:30` | `@types/node: latest` resolves to major 20 and engines allow 20. Running Node 24 does not establish a Node-24-only contract. |
| Resource controls | `coding-harness/src/delivery-process.ts:8`; `coding-harness/vitest.config.ts:9` | Cargo job/target settings are inherited and Vitest has a local two-worker bound. These are not shared resource admission or an evidence-based server allocation. |
| Output and DB isolation | `coding-harness/src/delivery-runtime.ts:141`; `coding-harness/src/delivery-process.ts:11` | Check logs are attempt-scoped, but caller-provided `CARGO_TARGET_DIR` and `SF_TEST_*` are not allocated per task. Independent checks need explicit target, database/schema, port and cleanup ownership. |
| Historical worktrees | `coding-harness/src/git-worktrees.ts:319`; `coding-harness/README.md:283` | Detached-worktree machinery exists in the older experiment and is explicitly prohibited for current delivery. Do not revive it as an unreviewed parallel adapter. |
| Configuration wiring | `coding-harness/src/delivery-cli.ts`; `.agents/config.toml` | Active delivery does not consume `loadSwarmAutomationConfig`; no `[swarm.automation]` block was found in the two inspected Codex config files. Adding one alone changes no delivery behavior. |

The source-wide snapshots also mean a disjoint edit can invalidate another task's
review. A path ownership table alone cannot make parallel writers safe against
that evidence model. Bind each candidate to an immutable source identity before
checks, then revalidate the combined integration source.

## Reusable upstream interfaces

Grounding: Brain lookup on 2026-09-26 returned Ruflo
`v3/@claude-flow/codex/src/dual-mode/cli.ts` and its tests. Detailed upstream
inspection is pinned in Semantic Builder ADR-0054 at Ruflo
`a91db6768ba5e8c9e31c840ad25c1c96478f3f0f` and MetaHarness
`7bffe2f58ee72c367cea07beb62ee00fc2bee755`. These are source pins, not proof of
npm parity or installation on this host.

- [`DualModeOrchestrator` and configuration loader][orchestrator] provide native
  Claude/Codex execution with dependency graphs and separate worker/writer bounds.
  Use `runCollaboration` only after a compatibility test proves effective config,
  native gateway preservation and permitted source roots. The loader chooses the
  first existing `.agents/config.toml` or `.codex/config.toml`; it does not merge.
- [`CodexWorktreeCoordinator`][worktrees] already supplies `prepare`, `status`,
  `integrate`, `cleanup`. Reuse after policy reconciliation; do not build another
  Git workspace manager. `prepare` defaults to HEAD; pin an accepted base SHA.
  `integrate` is a local merge, not a GitHub PR workflow or automatic predecessor
  source transfer. Do not call it as an implicit merge authorization.
- [Ruflo's CLI][cli] chains repeated workers unless `--parallel-workers` is set.
  The default feature template is sequential. Dependency batches have barriers;
  neither increasing bounds nor declaring `dependsOn` proves continuous refill
  or accepted source handoff into already prepared worktrees.
- Keep [MetaHarness kernel][kernel] verification and public pool/router APIs at
  the current ready-stage seam. Its serial step loop is not the missing outer
  concurrent executor. Ruflo ADR-324 is Accepted (2026-07-28); ADR-327 remains
  Proposed (2026-07-28), and MetaHarness ADR-047 remains Proposed (2026-06-16).
  Proposed distributed fencing is not a prerequisite for ordinary local delivery.

Two real integration gaps remain: this upstream dual-mode worker interface is
native-only, and its memory bootstrap uses Ruflo CLI. The requested API transport
and MCP-only memory policy therefore need a tested adapter seam or an upstream
change. Do not assert they are existing options or launch a forbidden CLI fallback.
Keep the existing external-host contract usable while establishing compatibility.

## Bounded repair sequence and acceptance

1. **Resolve policy and freeze the adapter contract.** Record effective provider,
   model, worktree, integration and PR authority in one consistent instruction
   update. Preserve historical records without permitting disabled execution.
   Freeze tests showing all admission entrypoints reject disabled hosts; wire
   the existing guard rather than duplicate it. Pending explicit worktree policy
   reconciliation, use read-only research and existing main-only delivery only.
2. **Establish Node 24 and transport types.** Set development engines/tooling to
   Node 24 and `@types/node: "24"`; test the full strict type closure. Add an
   isolated OpenRouter author/reviewer contract with exact model IDs verified
   against current provider metadata. Verify availability and pricing of the
   exact requested IDs above; do not silently reselect either model.
   Bound maximum input/output cost before every call; reject unknown price or a
   bound above $1, retain unknown-charge accounting and same-request replay
   protection, and represent unlimited cumulative allowance as `null`.
   API credentials stay only in the API adapter, never native/check environments.
   Rejection-driven native escalation preserves exact route/effort; native
   unavailability reports client/model/error and pauses, without fallback.
3. **Adapt upstream execution and source ownership.** Keep `DeliveryHarness`
   task/check/review evidence; add only integration needed to invoke verified
   upstream executor/worktree APIs. Independent candidates own non-overlapping
   paths and pinned bases. Accepted predecessor commits must be materialized and
   checked before dependent dispatch. One integration owner resolves manifests,
   locks and accepted PR order; Fabric integration and documentation target
   canonical `main`. PR creation/push/merge remain separately authorized.
   Test dirty-state retention, failed dependencies, stale handoff rejection,
   cancellation and no writes outside the assigned source root.
4. **Add shared heavy-work admission.** Allocate build and test resources separately
   from model sessions, using current cgroup/affinity CPU capacity, interval CPU,
   memory peaks and I/O pressure across projects sharing that host. Each server
   has separate admission; do not pool CPU capacity across hosts. Account for Cargo jobs, Rust test
   threads, nextest and Vitest children. Give each candidate a private Cargo target,
   SQLite files, PostgreSQL/MySQL database/schema, ports and output directory.
   Test conflicting claims, crash recovery and cleanup of owned fixtures only.
   A skipped database suite remains not-run. Never equate model count to CPU count.
5. **Prove one useful parallel outcome.** Record overlapping native/API worker
   timestamps and PIDs/request IDs, exact source and accepted handoff hashes,
   effective routes, resource samples and independent review. Compare whole
   verified-outcome time with a serial baseline on the same work; do not claim
   speedup from isolated model latency. Run focused Fabric semantic/public-query
   tests and impacted builds, then integrated regression at the outcome join.
   Preserve per-task receipts and serial learning reduction. No GCP claim until
   the same evidence is collected on the named server.

Keep implementation in the existing harness workflow with frozen regressions and
incremental verified commits. Do not introduce a second scheduler, generic service,
benchmark programme, API fallback or evolution gate merely to repair these seams.

## Consequences

Upstream reuse avoids duplicating native execution and workspace management while
preserving Fabric's acceptance boundary. Explicit source and resource ownership
costs integration work; upstream batching and API/MCP mismatches remain visible.
This proposal earns no product-completion points and changes no published routes.

## Links

- [Existing repair proposal](ADR-0057-repair-the-native-development-harness.md)
- [Current delivery decision](ADR-0055-v1-product-completion-and-release-profile.md)
- Cross-project research: `semantic-builder/docs/adr/ADR-0054-upstream-first-parallel-harness-execution.md`.

[orchestrator]: https://github.com/ruvnet/ruflo/blob/a91db6768ba5e8c9e31c840ad25c1c96478f3f0f/v3/@claude-flow/codex/src/dual-mode/orchestrator.ts
[cli]: https://github.com/ruvnet/ruflo/blob/a91db6768ba5e8c9e31c840ad25c1c96478f3f0f/v3/@claude-flow/codex/src/dual-mode/cli.ts
[worktrees]: https://github.com/ruvnet/ruflo/blob/a91db6768ba5e8c9e31c840ad25c1c96478f3f0f/v3/@claude-flow/codex/src/worktrees/coordinator.ts
[kernel]: https://github.com/ruvnet/agent-harness-generator/blob/7bffe2f58ee72c367cea07beb62ee00fc2bee755/packages/harness/src/kernel.ts
