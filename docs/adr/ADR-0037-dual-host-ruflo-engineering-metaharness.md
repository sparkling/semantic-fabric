---
status: accepted
date: 2026-08-25
updated: 2026-08-25
tags: [dev-process, ruflo, metaharness, dual-host, codex, claude, agentic-qe, darwin, avo]
supersedes:
  - ADR-0030
depends-on:
  - ADR-0026
  - ADR-0027
implements: []
---

# Dual-host Ruflo engineering MetaHarness

## Context and problem statement

Open-issue remediation crosses query semantics, mappings, dependencies,
connectors, conformance, and live sources. It benefits from parallel analysis
and independent model review, but product correctness must remain a direct,
deterministic property of the patched repository.

The tracked `coding-harness/` is currently a legacy single-host smoke scaffold:
it loads `@metaharness/kernel`, declares only Claude, and offers only `init`,
`doctor`, `--version`, and `--help`. It has no task runner, verifier transaction,
routing, repair loop, receipt chain, or Codex adapter. Its manifest hashes
untracked host files, and its lockfile contains plaintext private-registry
resolution. The ignored
`semantic-fabric-harness/` and its historical OpenRouter experiment are not an
upgrade source.

The Oxigraph and semantic-query MetaHarness structures in `semantic-builder`
show the relevant boundary: the engineering harness, direct product evaluators,
Ruflo coordination ledger, and promotion authority are separate systems.
MetaHarness diagnostics or model confidence cannot substitute for Cargo, W3C,
differential, security, mutation, or live-source evidence.

## Decision

Upgrade the versioned `coding-harness/` in place into a private, development-
only engineering control plane. It returns a patch and evidence; it has no
product-runtime, commit, merge, push, publish, deploy, or promotion authority.

### 1. Runtime and orchestration

Use the current `@metaharness/harness` runtime, including:

- `HarnessKernel`, `AlgorithmRouter`, a persistent run-scoped `AgentPool`,
  `PolicyGate`, and `VerifierRegistry`;
- path, tool, network, authority, and protected-input gates;
- bounded critique, independent cross-vendor review, verifier-directed repair,
  retry budgets, circuit breakers, cancellation, and output/time ceilings;
- outcome memory plus append-only, digest-chained receipts.

The authoring baseline verified the public packages
`@metaharness/harness@0.2.0`, `@metaharness/router@0.4.0`,
`@metaharness/host-claude-code@0.1.2`, and
`@metaharness/host-codex@0.1.2`. The implementation pins exact versions after
export and compatibility verification; it does not retain the obsolete direct
`@metaharness/kernel` dependency merely because the old scaffold used it.

Use the real `@metaharness/router` for quality-first routing. At cold start,
select the least-observed capable native host. Record quality only after direct
verification, use reliability and elapsed time to break equal subscription-cost
routes, and freeze the route snapshot for each run or evaluation epoch.
Subscription programme cost is recorded honestly as `$0`; no fabricated token
price is supplied to force a route.

Logical workers retain route, breaker, and outcome state for a run. Individual
model invocations are ephemeral so task context and credentials do not leak
between candidates.

### 2. Native dual-host model execution

Architecture, implementation, repair, review, and GEPA reflection are available
to both native host families:

- OpenAI Codex/ChatGPT through the native Codex client and ChatGPT subscription;
- Anthropic Claude through the native Claude Code client and Claude subscription.

Preflight verifies both first-party authentication classes. Child processes get
an explicit environment allow-list; provider API keys, base-URL overrides, and
proxy transport variables are removed. OpenRouter, Requesty, and any indirect
gateway are prohibited for execution, routing, fallback, retry, or mutation.
Hard decisions require distinct-host proposals and independent cross-vendor
review; absence of either host fails closed.

CI uses fake native executables and never calls a model provider.

### 3. Candidate transaction

Every task follows this dependency graph:

```text
prepare isolated worktrees and frozen evaluator
  → architecture + bounded critique
  → implementation
  → patch/path admission
  → apply patch to clean candidate
  → build the patched candidate
  → public + independent + regression verifiers
  → independent Codex and Claude reviews
  → digest-bound receipt
```

Semantic failures enter verifier-directed repair from a preserved candidate
state. Every repaired patch is re-admitted, rebuilt, and re-verified. A baseline
build or pre-patch artifact is never candidate evidence. One same-host retry is
allowed only for a classified transient transport/process failure. Cancellation
propagates to process groups and produces a failure/cancellation receipt.

### 4. Policy and isolation

Each writer owns one branch and worktree; a single integration owner applies
accepted patches in dependency order. The task contract contains exact mutable
paths and protects at least:

- evaluator tests, sealed fixtures, standards expected results, and oracle law;
- ADRs, harness policy/configuration, lockfiles, CI, release, and publication
  files unless the task explicitly targets one of them;
- every unrelated manifest and user/concurrent worktree change.

Reject traversal, symlinks/hardlinks that escape the worktree, shell
metacharacters in structured command fields, undeclared tools, oversized
output, and newly created files over 500 lines. Deterministic stages execute
structured argv without a shell. Model tools are read/edit oriented with no
browser, arbitrary network, nested agents, or MCP control. Native clients may
reach only their first-party services; candidate tools remain offline except
for a separately authorized dependency-resolution stage.

### 5. Ruflo coordination ledger

Ruflo coordinates rather than writes product code:

- hierarchical, specialized swarm topology with explicit Raft consensus at the
  hive/decision layer;
- persistent agent/task records, dependencies, health, and trace identifiers;
- project and user memory search before work, task/model routing hooks before
  dispatch, verified outcomes after completion, and reusable pattern storage;
- named researcher/architect, writer, tester, security, and independent-review
  roles mapped to isolated worktrees and native host invocations.

No metered Ruflo provider executor is used. Ruflo identifiers and route/outcome
records are bound into the receipt but remain coordination evidence, not proof
of product correctness.

### 6. Agentic-QE boundary

ADR-0026 remains authoritative. Agentic-QE is invoked only through named,
task-bound local profiles with provider variables removed: real-LCOV gap
analysis, Rust test generation with AI enhancement disabled, quality/contract
assessment, and SAST where applicable. It may propose tests and risks; generated
tests become authoritative only after human/model review, freezing, commit, and
direct execution. Cargo, W3C, spareval/materialization differentials, live DBs,
and mutation gates remain the oracles.

### 7. Receipts and authority

Each canonical receipt records:

- baseline, evaluator, and candidate commits/trees plus protected-input digests;
- route snapshot, host/model/role, native client versions, and auth class;
- admitted paths, patch digest, tool versions, structured build commands, exits,
  post-patch artifact digests, and verifier output digests;
- critique, independent reviews, retry/breaker/cancellation/repair evidence;
- Ruflo swarm/task/hook IDs and task-bound Agentic-QE evidence;
- prior-receipt digest and `development-only-no-promotion` authority.

Digest integrity proves tamper evidence, not authorship or correctness.

The old `.harness/manifest.json` is not the protected-input digest source.
Protected digests are computed at task start over an explicit tracked-path list
containing at least the harness manifest and lock, evaluator task, ADRs, and all
task-contract protected paths. A listed path that is untracked is a hard
failure. Regenerate the factory manifest to cover tracked files only or remove
it.

### 8. Factory, diagnostics, and evolution

Before any install, query every required package against the approved HTTPS
registry in a disposable directory and verify its exports. If a package is not
available, stop for a separate vendoring/runtime decision; never fall back to a
private HTTP mirror. Generate and validate a candidate lock in that disposable
directory first: no HTTP or private-address origin and an integrity hash for
every fetched entry.

Only after that supply-chain preflight, run the current Ruflo/MetaHarness factory
into separate disposable directories for `codex` and `claude-code`. Compare
doctor, genome, score, manifests, and executable commands; copy no generated
host state into the repository. Replace the tracked lock with the verified HTTPS
lock, pin selected packages exactly, and use `npm ci`.

Set `"private": true`, delete `publishConfig` and the publish-oriented `files`
list, and add a failing `prepublishOnly` guard. Until evolution becomes eligible,
remove `evolve`/`evolve:dry`, the Darwin dependency, and the invalid tracked
`suite.json`; CI asserts their absence. The current suite is not evidence: it
has one task, identical public/hidden/regression commands, an empty mutation
allowlist, a foreign absolute path, and no sealed holdout.

Doctor, genome, score, OIA, threat model, MCP scan, and drift are diagnostics.
A scanner that cannot inspect the repository's actual `.mcp.json` surface is
`INCONCLUSIVE`, never clean.

Darwin/GEPA is ineligible until at least five discriminating training tasks and
five sealed holdouts exist. Task IDs are opaque; evaluator law, thresholds,
authority, and holdout truth stay outside the genome. Models and route snapshots
are frozen for comparison. Promotion requires held-out improvement with no
safety or regression loss; otherwise the seed remains.

Diagnostic score/genome/similarity outputs, fitness weights, command allowlists,
and tool policy remain outside the mutation surface and every promotion signal.
Each evolution receipt includes a reward-hack scan; a candidate correlated with
diagnostic score manipulation is rejected.

AVO is eligible only for a named hard-tail with several plausible strategies,
a reliable independent evaluator, and ordinary repair unable to decide. It uses
copied workspaces, protected inputs, bounded actions/time/invocations, and stops
at an independently verified winner or honest null. Routine #8, #9, #10, and #6
work is not AVO-eligible.

## Acceptance

Harness-programme acceptance requires every hard gate below and a non-degraded
MetaHarness score of at least 98/100:

| Dimension | Points | Hard evidence |
|---|---:|---|
| Policy and supply-chain safety | 20 | path/tool/network/authority tests; HTTPS lock-origin test; private package |
| Evaluator integrity | 15 | sealed inputs; red-baseline proof; oracle isolation; tamper tests |
| Evolution containment | 5 | no executable path before 5+5; distinct holdout test; reward-hack scan; evaluator law outside genome |
| Patched-candidate verification | 20 | apply → rebuild → verify ordering; repair reruns; artifact digests |
| Dual-host control plane | 15 | native auth/env tests; routing; critique; independent reviews; no fallback |
| Reliability and receipts | 15 | retry/breaker/cancel tests; outcome memory; receipt-chain validation |
| Ruflo and QE integration | 10 | real coordination IDs; task-bound QE evidence; authority separation |

No aggregate score can average away a failed correctness, standards, security,
supply-chain, or provider-authentication gate. The first real acceptance task is
issue #8 end to end; expansion to the full corpus waits for that receipt and its
direct product gates. The score is a post-gate readiness diagnostic and is never
a Darwin/GEPA fitness, promotion, or repair reward.

## Implementation status

Accepted architecture, not yet implemented. The Ruflo MetaHarness score tool on
2026-08-25 scored the repository 71 and tracked `coding-harness` 67; the tracked
harness genome was `needs-work`, and OIA/MCP security diagnostics were blind to
the configured surface. These are baselines on that tool's 0–100 scale, not
acceptance evidence or evolution fitness.

## Consequences

- Good: hard changes receive parallel, independent native-host reasoning while
  deterministic repository evidence retains final authority.
- Good: verifier ordering, worktree isolation, policy gates, and receipts make
  repair reproducible and auditable.
- Good: evolution and search activate only when their evaluators can distinguish
  real semantic-fabric quality.
- Cost: the current scaffold needs a secure package foundation and a real
  control plane before it can run product tasks.
- Neutral: nothing in this ADR changes an `sf-*` runtime dependency or grants
  publication, provider API spend, or promotion authority.

## Rules

- **R1** — no product runtime crate depends on a harness, Darwin genome,
  Agentic-QE, or provider-specific development artifact.
- **R2** — no real evolution run executes in CI, a scheduler, or a background
  worker; no evolution command exists before the 5+5 eligibility gate.
- **R3** — no aggregate, synthetic, diagnostic, or model score overrides a
  failed product oracle or enters an evolution fitness signal.
- **R4** — no publication, provider API spend, or harness promotion occurs
  without explicit authorization and complete verifier evidence.
