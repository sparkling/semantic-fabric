---
status: proposed
date: 2026-09-26
updated: 2026-09-26
tags: [dev-process, metaharness, ruflo, parallel-delivery, openrouter]
depends-on: [ADR-0037, ADR-0048, ADR-0055, ADR-0057]
---

# ADR-0058: Upstream-first parallel hybrid delivery

- **Status**: Proposed; review and repair plan only
- **Date**: 2026-09-26
- **Deciders**:

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
