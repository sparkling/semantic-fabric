# semantic-fabric coding harness

Private, development-only control plane for verified repository changes. It
coordinates native Codex/ChatGPT and Claude Code subscriptions, but direct
repository evaluators decide whether a candidate passes.

The package has no CLI, MCP server, publication path, commit/push authority, or
evolution command. It returns evidence and a patch to an explicit integration
owner.

## Local verification

```bash
npm ci
npm run build
npm test
```

CI tests use fake model processes and make no provider calls. Runtime model
execution requires successful native subscription preflights. Provider API
keys, ambient proxy variables, base-URL overrides, OpenRouter, and Requesty are
rejected. The controller injects only a loopback CONNECT endpoint backed by its
exact-origin Unix-socket broker.

## Transaction

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
unlimited subscription capacity. Task, turn, time, output, concurrency,
first-party rate-limit backoff, retry/repair, resource, and receipt limits remain
operational safety controls.

## Main modules

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
fixture, its test-only bounded scanner/one-parse reader and opaque
non-authorizing byte brand, independent completeness oracle, immutable V1
receipt, and additive V2 OID/attribute-aware candidate-matrix/effective-privilege
witness receipt. The reader's 92 hostile/limit/private-brand KATs plus five
exact-fixture baseline tests reject noncanonical or forged fixture authority.
V1's raw oracle independently expands direct ACL atoms. V2 binds 13,603 fresh
no-membership-role checks across six populated classes; FDW/server remain
explicit zero classes. It also binds matching captures from two distinct fresh
volumes. These are test evidence only: they are excluded from
supervisor-service source and build inputs and authorize neither migration nor
runtime activation. A pure lexical mutator, extracted fail-closed replay support
and 17 KATs protect the exact eight top-level class branches plus return-zero,
one-atom omission, one typed-sentinel addition and count-neutral substitution.
Its live verifier uses one rollback-only serializable transaction to populate
FDW/server, match original and normalized projection bags to the independent
raw-catalogue oracle, execute all 12 mutants, prove every exact mutant output,
and require the full-bag key or multiplicity rejection. Held-descriptor
`O_NOFOLLOW` reads, exact process arguments and mutated container predicates
protect the local boundary; 60-second sessions and 15-second probes/inspections
leave 180 seconds beneath the 300-second parent ceiling. Two fresh networkless
runs prove deterministic evidence and owned cleanup. Unpublished developer-local
Node.js 20.0.0 and 24.14.1 runs report 602 supervisor tests, V1/V2 plus the
12-mutant live replay, and the 892-test parent harness with two intentional
skips. No version-bound run receipt is tracked; CI is configured to run the same
gates, but hosted evidence and the remaining predicate/value/nullability/order/
duplicate/array/`UNION ALL` mutation cases, runtime `Plan` and live-observation
brands remain open.

The parent harness replaces Node 20's asynchronous recursive watcher with
explicit root/nested directory watches, three agreeing tree digests and an
8,192-watcher fail-closed ceiling that closes every partially opened watcher.

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
evaluation-only. It currently no-ops because the flywheel-visible neural store
has no eligible patterns; legacy counters and ReasoningBank data are a different
store. Eight owner-visible records, four harvestable records, and a pinned
non-fallback embedding provider are only the threshold to begin evaluation, not
production readiness. If a future trial emits a signed receipt, replay verifies
receipt integrity, lineage, the gate fingerprint, and the gate decision over
sealed scores. Replay does not re-run retrieval. The separate daemon generation
path is not approved until it uses the same confirmed promotion transaction.
Production receipts will also require durable retention outside the ignored
local state directories.
