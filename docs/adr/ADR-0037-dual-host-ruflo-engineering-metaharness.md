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

At decision time, the tracked `coding-harness/` was a legacy single-host smoke
scaffold:
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
ambient proxy transport variables are removed. The controller then supplies only
its own loopback CONNECT endpoint, backed by an exact-origin Unix-socket broker,
as the enforcement mechanism; it is not a provider route or fallback. OpenRouter,
Requesty, and any indirect gateway are prohibited for execution, routing,
fallback, retry, or mutation.
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
task-bound, provider-free local profiles. The task contract selects the relevant
profiles rather than requiring every profile for every change: issue #8 requires
real-LCOV gap analysis and SAST. `rust-testgen-no-ai` and `quality-contract` are
permitted future profile labels, but issue #8 implements no collectors or runners
for them. Agentic-QE may propose tests and risks; generated tests become
authoritative only after review, freezing, commit, and direct execution. Cargo,
W3C, spareval/materialization differentials, live DBs, and mutation gates remain
the oracles.

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
The trusted launcher seals that receipt inside a second, digest-bound programme
envelope containing the seven-dimension assessment. A passing candidate with a
rejected programme assessment is returned as `gated`, never as success.

The factory `.harness/manifest.json` is not, by itself, the protected-input
digest source. Protected digests are computed at task start over an explicit
tracked-path list containing at least the harness manifest and lock, evaluator
task, ADRs, CI/publication controls, every Cargo manifest, and all task-contract
protected paths. A listed path that is untracked is a hard failure. The current
canonical manifest mirrors this list and the trusted build checks that every
declared harness TypeScript source has a corresponding attested output. Other
governance, ADR, Cargo, CI, and publication-control paths are bound as Git blob
digests rather than build outputs.

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

**Decision amendment (2026-08-25).** The earlier accepted criterion requiring a
non-degraded upstream MetaHarness score of at least 98 is replaced by the
project-owned rubric below. Exact source audit of the owning Ruflo wrapper and
its active `metaharness@0.3.2` cache showed that `harnessFit` is
`round(plan.confidence * 100)` over shallow repository metadata: it does not
execute tests or inspect the custom harness source. The selected Rust archetype
tops out near `0.8003 → 0.80 → 80`; even the global archetype ceiling is
`0.967 → 0.97 → 97`. Repository/harness values such as 71/67 are therefore
version-bound context, not programme scores. Upstream degraded execution or
failed hard constraints still fail the diagnostic gate; `harnessFit` itself
contributes no points.

The exact native Ruflo score results and owning implementation hashes are stored
in the parsed `config/metaharness-diagnostics.json` snapshot. That tracked blob
is a protected task input, and the programme envelope must match its receipt
digest before deriving diagnostic status. A literal score or hard-constraint
claim embedded only in controller code is not evidence.

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

Harness-programme acceptance requires every hard gate below and at least 98/100
on this project-owned, seven-dimension evidence rubric:

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
supply-chain, or provider-authentication gate. Upstream diagnostic values do not
contribute points. The first real acceptance task is issue #8 end to end;
expansion to the full corpus waits for that receipt and its direct product gates.
The project-owned score is a post-gate readiness summary and is never a
Darwin/GEPA fitness, promotion, or repair reward.

## Implementation status

Accepted architecture; framework and trusted runtime implemented; final
programme transaction pending. The incremental harness commits through
`947253d` establish the private supply chain, dual-host routing, patched-candidate
transaction, exact-origin broker, mount namespace, copied credential
capabilities, systemd cgroup-v2 quotas, bounded retry/cancellation, provider-free
QE/SAST, protected governance inputs, and digest-chained receipts. The harness
builds and passes 298 tests across 45 files. It has no publication or evolution
path, and all harness source files remain under 500 lines.

Development runs 03–16 emitted failure receipts while exposing and then closing
native runtime containment, optional-egress, long-lived-tunnel, response-bound,
and shared Cargo-registry drift defects; none is acceptance evidence. Run 16
proved the frozen dependency closure but its first Codex architecture invocation
exhausted one terminal 20-minute deadline. That behavior contradicted this ADR's
accepted one-retry rule for classified transient process failures. The runtime
now classifies a complete native deadline exhaustion as transient and permits
exactly one fresh same-host retry, with two 10-minute attempts preserving the
same 20-minute worst-case lane budget. The next run remains the first eligible
issue-#8 acceptance transaction on the reconciled protected snapshot.
Dirty user-owned `.mcp.json` and untracked `coding-harness/.claude/` state remain
unstaged and are masked from model sessions; diagnostics that cannot inspect
their effective surfaces remain `INCONCLUSIVE`, not clean and not a substitute
for direct controls.

Its dependency-scoped Rust closure pins the historical lock, target package
set, verified crate archives, and minimized exact sparse-index records while
excluding the mutable shared registry source tree. Upstream MetaHarness
diagnostics classify the repository/harness at 71/67 and
the genome as `needs-work`; those values are non-authoritative for the reasons in
section 8. Darwin/GEPA stays disabled. No receipt may be described as semantic
acceptance until the real dual-host transaction, every hard gate, and the
project-owned score threshold pass.

## Consequences

- Good: hard changes receive parallel, independent native-host reasoning while
  deterministic repository evidence retains final authority.
- Good: verifier ordering, worktree isolation, policy gates, and receipts make
  repair reproducible and auditable.
- Good: evolution and search activate only when their evaluators can distinguish
  real semantic-fabric quality.
- Cost: secure native execution adds systemd, mount-namespace, broker, frozen
  dependency-closure, and independent-evidence complexity and latency.
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
