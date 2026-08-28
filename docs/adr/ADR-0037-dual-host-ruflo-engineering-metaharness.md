---
status: accepted
date: 2026-08-25
updated: 2026-08-28
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
verification, use reliability and elapsed time to break routes with equal
marginal provider-API spend, and freeze the route snapshot for each run or
evaluation epoch. `subscriptionCostUsd: 0` records zero marginal provider-API
charge in the receipt/routing ledger; it is neither a budget or cap nor a claim
of unlimited subscription capacity. Native subscription invocations have no
project-imposed provider-dollar spend ceiling, and no fabricated token price is
supplied to force a route. Task, turn, time, output, concurrency, first-party
rate-limit backoff, retry/repair, resource, and receipt limits remain operational
safety controls.

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

**CI capability amendment (2026-08-26).** GitHub-hosted Ubuntu runners cannot
create the network namespace required by the production `bwrap --unshare-net`
contract. The hosted Node 20 baseline therefore runs only portable,
provider-free unit and contract coverage with native integration explicitly disabled. The
same native tests remain fail-closed when required and are dispatched through
the labelled `self-hosted`, `linux`, `bwrap`, `systemd-user` workflow, which
preflights both bubblewrap and the user-systemd boundary. A hosted green check
does not claim that native isolation ran.

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

The frozen issue-#8 acceptance path remains schema v2 with the
`exact-reference` oracle. The reusable contract foundation also accepts schema
v3 `verifier-only` tasks whose admitted paths, generated outputs, command
evidence and QE profiles are derived from protected task data and bound through
a versioned Rust runtime profile. Unknown schemas and oracle modes fail closed.
The legacy schema-v4 `launch-issue-8.mjs` path remains bound to
`config/issue-8-acceptance.json`. The explicit immutable fresh-ID schema-v5
operator uses `config/programme-v5-acceptance.json`, which remains an issue-8
H0c activation fixture; no general next-product-task or default promotion path
exists. H0c is accepted only when an actual fresh run passes every gate, seals
an accepted envelope, and provider-free replay verifies it.
**Proposal note (2026-08-28):** [ADR-0041](ADR-0041-manifest-bound-controlled-observational-evidence-capture.md) proposes a sibling observational plane without amending this boundary.

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

### 9. Retrieval-policy flywheel

Ruflo retrieval-policy tuning is separate from Darwin/GEPA code evolution. This
repository prepares the evaluation surface but does not enable an autonomous
loop:

- `.claude/eval/semantic-fabric-relevance-anchor-v1.json` is a 48-task,
  ADR-derived candidate relevance anchor with balanced deterministic train and
  holdout halves. Its task-canonical SHA-256 is pinned by
  `.claude/eval/flywheel-anchor.manifest.json`; changes require a new versioned
  anchor, not an in-place edit.
- The anchor, its manifest, `.claude/settings.json`, the inherited proven-config
  pointer, and the active framework policy are protected harness inputs.
- `RUFLO_HARNESS_LOOP` and `RUFLO_FLYWHEEL_LEGACY_APPLY` are absent from the
  tracked configuration. The 2026-08-28 post-H0c operational check also found
  them absent from the live daemon environment and confirmed that the daemon
  worker set excludes `harness`; that condition must be rechecked after every
  restart. Both worker entry points return `opt-in required` without loading or
  mutating flywheel state.
- A deliberately overridden *explicit evaluation* may run only against the
  project anchor and the current `neural_patterns` store. It returns
  `applied: false`; promotion is a separate, confirmed transaction requiring a
  trusted Ed25519 key, frozen gates, sequential evidence, stale-head protection,
  and ledger compare-and-swap.

The tracked active policy is an inherited `framework/node-cli` champion that
predates this project-local evaluation surface. Its proven-config pointer and
parameters agree, but it is not a semantic-fabric flywheel promotion receipt and
must not be presented as one. A future explicit evaluation may use it as the
baseline it must beat.

The current owner-reported learning stores are inconsistent: historical global
statistics and the legacy ReasoningBank `patterns.json` contain activity, while
the `neural_patterns` store consumed by the flywheel contains zero patterns.
Those counts are not interchangeable. With an explicit one-call override the
real runtime validates the project anchor and returns `store too small to
harvest a corpus`, creating no evaluation receipt. Eight owner-visible records
with at least four harvestable records are only enough to start evaluation; they
do not imply production readiness. Activation also requires a pinned
non-fallback embedding provider, an immutable store snapshot, maintainer review
and live calibration of the candidate relevance anchor, and a model-call-free
local evaluation producing a non-regressing result. Its 24-task promotion half
has useful sequential-evidence headroom, but ties and losses can still make a
trial non-promotable. History must enter through the owning Ruflo ingestion path
with provenance; benchmark labels must never be copied into the retrieval store
to manufacture a win.

The older daemon generation worker is not approved here. It persists its own
generation lineage and can serve a prior champion on a later tick without using
the explicit ADR-322A promotion transaction. It remains disabled until upstream
routes background adoption through the same signed, confirmed, fail-closed
transaction and this ADR is amended.

The tracked active-policy assertion deliberately permits only the inherited
`framework/node-cli` baseline. A legitimate project-local promotion is therefore
not authorized by the current state model: it requires an ADR amendment, a
transaction-verified local pointer model, and durable receipt/ledger retention
outside the ignored local flywheel directories.

Receipt replay has a precise meaning: it verifies Ed25519 signatures, lineage,
gate fingerprint, and re-executes the frozen gate over sealed scores. It does
not re-run retrieval or the benchmark. User-facing text may say that a signed
receipt is replay-verifiable for integrity and gate decisions; it must not imply
that replay independently reproduces benchmark execution or that the disabled
daemon path “provably” promotes winners.

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

Accepted architecture, framework, and H0c activation implemented. Harness
commits through `a84aa05` establish the private supply chain, dual-host routing,
patched-candidate transaction, exact-origin and filesystem/resource isolation,
bounded retry/cancellation, provider-free QE/SAST, frozen dependency closure,
schema-V6 evidence law, trusted execution/replay, protected dormant retrieval
tuning, and the PostgreSQL receipt authority. The harness passes 635 provider-
free tests across 91 files with two expected skips. It has no publication or
evolution path, and all harness source files remain under 500 lines.

The reusable-harness prerequisite then advanced through `ef10001`, `c3a3e99`,
`f8db1e0`, `7a5244a`, `6e7c153`, `b40dbc6`, and `7a1fa24`. H0a freezes schema-v5 gate
law, strict duplicate-key parsing, versioned task/runtime derivation, an
externally anchored policy fingerprint, and authoritative protected-input
bindings for the harness manifest, task, controller build, controller lock and
frozen Cargo lock. Frozen schema-v4 replay and AQE 3.13.10 identity remain
unchanged. H0b adds the replay-complete evaluator, canonical scorer, strict v5
envelope, and externally expectation-aware dispatcher. Its pinned ACCEPTED test
vector has policy, assessment, and envelope digests
`0d5505e4952c87bd12204ffb11caf40ae31351b29a950358d3ea54f3b85161bb`,
`4f4fe45c2c9ce9a0a30c95519769bc1a3607c71b2e6526f78914ce42e331a977`, and
`fdab0843eeffad9ce75d8b730d4977ccd3149bf72d0af748bb6d3e6cd10065e7`.
The explicit v5 operator can now execute a freshly claimed run, but it is not a
general/default promotion path and does not make H0c accepted.

Development runs 03–24 emitted fail-closed receipts while exposing and closing
native containment, egress, response-bound, frozen-registry, timeout, and
verifier-infrastructure defects; none is semantic acceptance evidence. Runs
25–27 reached a healthy red baseline but exhausted pre-admission repair. Their
evidence led to bounded rejected-patch context, a fresh base-relative repair
contract, and strict diff normalization without weakening exact context, path,
index, lane-tree, or expected-candidate checks. Run 28 then failed the exact-
origin gate on an intermittent denied connection. One unchanged-controller
repeat was permitted to classify that result; run 29 did not reproduce it.

Run `issue8_dual_native_20260826_29`, under packed private controller commit
`c3834e5`, used native `gpt-5.6-sol` and `claude-sonnet-4-6`, proved the red
baseline, and admitted the initial patch only to
`crates/sf-sparql/src/unfold.rs`. A post-admission verifier-directed repair was
attempted once and exhausted its configured one-repair operational allowance.
The sealed programme result is
`REJECTED`, 40/100 against the required 98, with hard gates failed and no fitness
eligibility. Its receipt, assessment, and envelope digests are respectively
`065134f2a0ad03a6067d31e1dde3d8fa4b7a87c73f46716e8bb2625f049b0b15`,
`56221a498c7d08af1bab911f6723f2e6dce7dd4c8720f0b197321b65ddf3a554`, and
`d54a598464c4bde3195ef37953078ceb74cb18c726622c072e7230486a6236c4`.
The independently verified product commits remain valid, but their direct
evidence cannot be substituted for a passing sealed transaction.

H0c runs `programme_v5_h0c_20260827_03` and `_04`, both under packed controller
`ad32d4a`, proved the red baseline, build, three verifier lanes, generated EARL,
and four mutation sentinels, then failed at the final-review boundary before any
review digest was sealed. Both recorded `status: fail`; their programme
assessments were `REJECTED` at 40/100, and provider-free replay verified the
recorded failure. Run `_04` bound policy fingerprint
`7d6e3a25966a56472fc4504cf4f15dee32c5188395f45ab8bc09c377730cb21e`, receipt
`d888c175f54ce39f5e9c62837487758ed0410d5b57591564a11a771dd9937bc7`, envelope
`66618b0bd8d74992bc485af6fe8af5bd14e0f8e8d5ce0b214f732e2e84849a32`, and replay
receipt `c588022d606009b9d15777fbf70c843085d783e3f8dd579544c19b0d78fab833`.
The historical generic `HARNESS_TRANSACTION_FAILED` receipt cannot attribute the
failure to spend, rate limits, or either model. Commit `f82c2b5` now maps future
opaque native review-operation exceptions to `HARNESS_NATIVE_REVIEW_FAILED`
while preserving explicit cancellation, privacy, current v5 codes, and frozen
v4 bytes. Another run requires a new controller policy, claim, and run ID;
neither H0c run opts into the retrieval flywheel.

Fresh run `programme_v5_h0c_20260828_05`, under controller `4b0756f`, then
completed a passing candidate transaction after one patch-admission repair. It
proved the final build, three verifier lanes, generated EARL, four mutation
sentinels, QE, protected inputs, and dual native reviews. The frozen V1
reliability law nevertheless rejected the programme at 85/100 because the
pre-build repair had no prior build command. Its policy, receipt, assessment,
envelope, and replay digests are
`c4411178334b54620da099ba7e2c9e029ebcc6873a0bc85e9dfca93bacbfeb79`,
`91bfc845961c84237ff7f9ea58e75bc77c76d21913f042e05e77c68bc7ab98d1`,
`cf7734d88eaa05e91916992c8ad8e1c2dfdc285e7a14fe14858bed3744288930`,
`4c0897695d9a865ff636088be806b4ce5074d3e3552a15038c58435b0017e05f`,
and `ca2bbbb3a909f846b0a443ce21572680a2fc9e930a0cb30d294cf2b4af178cfc`.

The additive schema V6 correction is implemented through `c8e3f68`; V1, V4,
and V5 stay frozen. Run `programme_v6_h0c_20260828_01` failed closed at the
native-origin gate and replay preserved the failure. Fresh run `_02` passed the
transaction and every gate at 100/100 with six bound commands, seven native-
evidence digests, two final native reviews, and no retry or repair. Its policy,
receipt, envelope, and provider-free replay receipt are `e71107e5…ae34`,
`d9d244ef…0216`, `02c30ed3…9a06`, and `f1bcf0fe…bf02`. H0c is complete; product
milestones remain gated. The run cleaned about 35 GiB of ephemeral verifier
outputs, leaving immutable build reuse and digest manifests as a constrained
efficiency target. Neither run enabled the retrieval flywheel.

The repository's `.mcp.json` is tracked and protected. Root
`.harness/mcp-policy.json` is static input to the exact pinned MetaHarness scanner,
whose clean/info result is a blocking harness test. That scanner evidence is not
runtime tool authority: task `PolicyGate`, native isolation, and direct controls
remain authoritative; a diagnostic unable to inspect the effective root surface
is still `INCONCLUSIVE`.

Its dependency-scoped Rust closure pins the historical lock, target package
set, verified crate archives, and minimized exact sparse-index records while
excluding the mutable shared registry source tree. Upstream MetaHarness
diagnostics classify the repository/harness at 75/67 and
the genome as `needs-work`; those values are non-authoritative for the reasons in
section 8. Darwin/GEPA stays disabled. No receipt is semantic acceptance until
a later explicitly authorized dual-host transaction passes every hard gate and
the project-owned score threshold.

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
  publication, API-key-backed provider transport, or promotion authority.

## Rules

- **R1** — no product runtime crate depends on a harness, Darwin genome,
  Agentic-QE, or provider-specific development artifact.
- **R2** — no real evolution run executes in CI, a scheduler, or a background
  worker; no evolution command exists before the 5+5 eligibility gate.
- **R3** — no aggregate, synthetic, diagnostic, or model score overrides a
  failed product oracle or enters an evolution fitness signal.
- **R4** — no publication, API-key-backed provider transport, or harness
  promotion occurs without explicit authorization and complete verifier evidence.
- **R5** — retrieval tuning stays off by default; neither a daemon generation
  nor an unsigned/implicit apply may activate a repository-local policy. A
  receipt replay proves integrity and gate reproducibility, not benchmark
  re-execution.
