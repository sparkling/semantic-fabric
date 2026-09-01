---
status: proposed
date: 2026-08-28
updated: 2026-09-01
tags: [metaharness, evidence, performance, benchmark, reproducibility, ruflo, codex, claude]
supersedes: []
depends-on:
  - ADR-0005
  - ADR-0006
  - ADR-0037
  - ADR-0038
  - ADR-0039
  - ADR-0048
implements:
  - ADR-0038
---

# Manifest-bound controlled observational evidence capture

## Status boundary

This ADR is **proposed**. It defines the control-plane contract needed to
capture the first authoritative ADR-0038 M0 performance baseline. It does not
claim that a controlled runner, canonical runner profile, baseline, or capture
receipt exists. The current development host fails the required profile and
cannot produce admissible measurements.

The controller now has a bounded, fixed `/proc`/`sysfs` observer, a conservative
byte-canonical decoder for the product's runner-profile format, and a
negative-only non-admission record. This slice can return only `ineligible` or
`unproven`; both outcomes terminal-fail an input-attested claim with zero
attempts. The protected current source uses an AST-walked, exact-import allowlist
and fixed `openSync` call shape; mutation sentinels reject known command, provider,
socket-creating, dynamic-loader, write-flag, and ambient transport forms. This is
static regression evidence, not an OS capability sandbox. The gate snapshots profile bytes through
typed-array intrinsics without input iteration, property access, or species
construction. Its fixed collector has no injected callback and receives local
controller classification through captured module-private provenance operations;
records built from the separately labelled injected collector retain diagnostic
classification. The durable parser/verifier proves canonical self-consistency,
not independently witnessed collector provenance; positive capture must add that witness.
This remains partial fail-closed implementation, not acceptance of this ADR or proof of a controlled runner.

The controller also now has a capture-specific run-claim codec and a cooperative
same-UID create-new authority adapter. Claim-slot selection varies only with an
independently supplied project-authority digest and run ID; controller, task,
input-attestation, runner-profile, and expected-runner identities are immutable
claim-body bindings. Reserve and replay independently re-attest the pinned
controller store, commit, and task before comparing the whole claim. Only after
descriptor-rooted persistence and exact readback does the adapter return
claim-admission and state-genesis **views**. Those deterministic bytes are not
provenance: the first host consumer and authoritative replay reopen the rooted
claim and re-attest controller authority before exact state derivation. The
records state explicitly that host admission is not evaluated and that no runner
lease, attempt start, or capture is authorized. The adapter requires a
pre-existing, canonical owner-only `0700` authority directory, creates the
derived `0600` claim with `O_EXCL`/`O_NOFOLLOW`, and fsyncs file and directory.
An identical claim is spent rather than idempotent. This proves cooperative
concurrent exclusion within that one root only: it is owner-deletable, has no
external append-only witness or rollback resistance, and is not the future
runner lease. No real project claim has been minted on this ineligible host.

Commits `99fa2e1` and `92f5376` add the bounded canonical registration request, detached-signature acknowledgement, validation/replay seam, and fixed RFC 8032 wire vector.
Commits `7139b05`, `1d33638`, `3ee0ed6`, and `d54518f` add RFC 9162 Merkle verification, shared detached-signature verification, signed checkpoints, and registration inclusion plus checkpoint-consistency replay.
The verifier reconstructs the leaf from the reverified canonical registration envelope, receives the key/log/checkpoint/rooted-claim anchors independently, and snapshots key and proof inputs before its first asynchronous read.
It digest-pins the canonical prior checkpoint, signature-verifies only the new checkpoint, checks one-based sequence to zero-based leaf bounds, consumes the exact inclusion/consistency paths, and rejects key/log/supervisor/epoch substitution.
Its evidence distinguishes `priorCheckpointSignatureVerified: false` from `newCheckpointSignatureVerified: true`; two conflicting signed extensions can both validate while fork, rollback, persistence, global order, admission, lease, attempt, and capture authority remain explicitly unproved or false.
The fixed vector pins leaf, tree, proof, checkpoint, signature, and validation bytes; the validation digest is `eca1b7e1…1ee6`. Capability closure rejects transport, write, signing, process, and dynamic-loader mutants.
The hardened build and all 108 harness files pass (760 tests, two expected skips), and an independent adversarial review found no remaining P0/P1 issue.
There is still no production signer, transport, writer, persistent log, independently administered service, real acknowledgement/checkpoint, lease, runner, attempt, or capture. Proposed [ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md) now defines the next boundary: witnessed transactional run/resource authority plus controlled-runner provisioning and admission.

The first claim consumer before host admission is now implemented: it materializes the exact claimed commit, from either an exact primary or bare
controller store, into a claim-keyed private source root disjoint from both
claim and controller authority. A private Git index is read from the immutable
commit and checked against the controller tree.
Includes, filters, attributes, and group/world-writable Git control or object
authority fail before checkout; the protected object walk is bounded at one
million nodes. Materialized paths, Git modes, Git object IDs, SHA-256 digests,
byte counts, directories, index bytes, Git executable, claim, input attestation, controller,
expected runner identity, and output absence are bound in a deterministic local
view. Directories are sealed `0500`, ordinary files and the index `0400`, and
tracked executable files `0500`. Prepare, verify, and pristine-only disposal
reopen the rooted claim and re-attest the controller. Unsupported modes,
symlinks, gitlinks, hard links, `.git` paths, extras, replacements, and output
injection fail closed. This opaque in-process handle and view grant no host,
lease, attempt, build, execution, capture, persistence, or receipt authority;
same-UID/root mutation, rollback, and path ABA remain explicit nonclaims. Tests use synthetic stores and roots, so no project source tree or measurement was materialized.

The `implements: ADR-0038` relationship denotes a subordinate proposed design
lock, not implementation or M0 evidence. [ADR-0048](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md) governs
this additive harness decision: committed Node/TypeScript observers and
controllers are non-deployable evidence, while any production supervisor or
runner component is Rust. This high-assurance lane runs in parallel with M1–M6
and gates authoritative performance and M7 release claims, not product feature
implementation or the authority of direct Cargo tests.

## Context and problem statement

[ADR-0037](ADR-0037-dual-host-ruflo-engineering-metaharness.md) defines a
candidate-patch transaction. It starts from a red baseline, gives isolated
model workers declared mutable paths, stages a candidate patch, and proves the
result through build, verifier, mutation, QE, and dual-review gates.

ADR-0038 M0 needs a different operation: run an already implemented benchmark
producer once from a clean commit on a controlled host, create a previously
absent baseline, and preserve enough evidence to distinguish a real
measurement from model-authored or relabelled bytes. Reinterpreting V6 would
break both contracts:

- V6 requires existing mutable implementation paths and a non-empty patch;
- its repository preparation creates a dirty candidate and may precreate an
  artifact path;
- its Rust sandbox and generic CPU quota are not the benchmark runner profile;
- a model-written TSV could pass the offline parser without ever being measured;
  and
- patch retry/repair semantics permit selection behavior that a baseline
  capture must prohibit.

The 2026-08-28 probe of the current host could not satisfy the product profile:
its permitted CPUs were not isolated, the governor was `powersave`, swap was
enabled, turbo state was not attestable, and ambient load was high. Affinity
alone cannot repair those conditions; capture-time preflight is authoritative.

## Considered options

- **Treat the baseline as a normal V6 artifact.** Rejected. It permits false
  proof and conflicts with clean-tree/create-new producer invariants.
- **Run the Rust CLI manually and commit the TSV.** Rejected. The bytes would
  lack manifest, source, runner, command, and controller-observed attempt
  provenance.
- **Make hosted CI capture or remeasure.** Rejected. Shared runners do not offer
  the controlled host contract; repeated measurements also create favorable-run
  selection pressure.
- **Add a sibling manifest-bound capture transaction.** Proposed. It can reuse
  controller attestation, native dual review, Ruflo coordination, canonical
  receipts, and provider-free replay without weakening patch transactions.

## Proposed decision

### 1. Add a separate transaction kind

Introduce `programme-capture-v1`, with its own parser, policy, runtime,
envelope, receipt I/O, trusted launcher, operator support, and replay. Preserve
every V4/V5/V6 schema, gate, receipt, replay byte, and meaning.

The existing harness manifest schema already supports multiple tasks. Once the
controlled runner profile exists, register one exact task with
`taskKind: controlled-performance-baseline`. Do not add a default task or infer
task kind from a filename. Capture requires an explicit manifest member and
explicit task path.

The V1 task has exact, closed keys for:

- identity, objective, invariants, exclusions, and authority;
- profile, scenario/workload, root `Cargo.lock`, and source-path bindings with
  SHA-256 digests;
- exact structured capture and offline verification commands;
- one output path, `create-new` mode, canonical media type, and maximum bytes;
- offline/native-first-party/dual-review policy; and
- `routing.evolutionEligible: false`.

Unknown keys, task kinds, schemas, paths, commands, modes, or digest algorithms
fail before any model or benchmark process starts.

The capture parser uses one non-overridable configuration derived from frozen
capture constants. The global harness manifest is always parsed against the
ordinary secure harness configuration; only the observational transaction uses
the task-scoped profile, scenario, lock, and product-source boundary. A caller
cannot supply a weaker protected-path set.

### 2. Separate roles and authority

| Role | Authority |
|---|---|
| Controller | Attest the exact clean commit, task, inputs, tools, build, run ID, and state transitions. |
| External supervisor | Bind a pre-existing claim to an independently administered key/log; later own append-only lease and attempt ordering. |
| Codex and Claude | Review deterministic inputs and the immutable capture record through native subscriptions; never generate or edit measurement bytes. |
| Measurement runtime | Execute the exact release binary once under the admitted profile and write only a controller-owned private output. |
| Product checker | Parse and verify canonical scenarios, workload identity, profile identity, and baseline invariants. |
| Replay | Recompute receipt/digest/control bindings without a provider or live measurement. |
| Integration owner | Import replay-verified bytes into the repository and run the normal product gates. |

Neither Ruflo, MetaHarness, a model review, a score, nor the capture wrapper can
override a failed Rust producer/checker or host-profile gate. The harness has no
commit, push, release, publication, or promotion authority.

### 3. Freeze inputs at one clean controller commit

Before capture, attest one exact commit and tree containing the task, canonical
runner profile, scenario/workload authority, root `Cargo.lock`, every tracked
source in the reachable local Cargo package closure, trusted capture
sources/scripts, and controller build manifest. Bind the resolved external
Cargo/toolchain closure separately. The output path must be absent from the
commit and worktree.

The transaction consumes one verified two-builder pair receipt and its exact
release-artifact binding from [ADR-0039](ADR-0039-minimal-production-serving-artifact.md)'s
reproducibility track; it does not manufacture agreement. That agreement pre-
registers distinct trust roots/run IDs, commits both complete results before
reveal, binds identical deterministic inputs, and accepts only byte-identical
artifacts. Missing, failed, mismatched, retried, selected, or tiebroken results
fail the pair. Directional comparison and clean-checkout repeatability are not
independent agreement. This extends the evidence plane, not runtime architecture
or the application charter.

Materialize that commit into a private clean worktree. Reject dirty or
untracked source, symlink/hardlink substitution, a pre-existing output, path
traversal, non-canonical Git identities, changed protected blobs, or a build
whose source/toolchain identity cannot be bound to the commit.

The implemented partial boundary stops at a sealed, exact-commit source tree.
It uses `read-tree` plus `checkout-index` with an isolated private index. Branch
and ambient-index state is inspected only to reject attribute authority and is
never source or materialization authority; no shell, worktree mutation, remote,
build tool, or producer is invoked.
Before checkout it rejects local include/filter/config and attribute authority,
foreign-owned or cross-UID-writable control/object nodes, and an oversized
object-authority walk; it then preflights every blob size and the path-counted
5 GB aggregate. It validates the
full path set and streams every file through native Git-object and SHA-256
hashing before returning an opaque local handle. Verification
reopens the claim on both sides of the check and repeats tree, index, inventory,
output-absence, controller-store, and pinned filesystem-identity checks.
Cleanup is available only through the still-pristine module-issued handle; a
poisoned tree is preserved for operator inspection. This is source preparation,
not the future execution worktree, attempt lease, durable record, or evidence.

Before materialization, a replay-safe object attestation verifies manifest
membership and records each task-scoped input as
`{path, gitBlobId, sha256, byteLength}` in its fixed semantic order. It binds
the exact commit/tree and raw task bytes, rejects non-blob Git modes, and proves
that the output path is absent from the commit. It does not depend on the
ambient branch or working-tree contents.

The controller store must be either its exact bare repository root or an exact
primary non-bare repository root; ancestor discovery and linked worktrees are
rejected. The controller pins and rechecks the store, Git/common directories,
and object directory, rejects unsafe parent rename authority, queries the
storage object format, and independently rehashes every admitted commit, tree,
and blob body as its native Git object. All protected files use mode `100644`;
tree size and entry count are bounded. This does not claim continuous rollback
resistance against root or a hostile same-UID actor able to swap and restore
paths between checks; the independent bootstrap-store digest and append-only
supervisor remain the authority for that stronger threat model.

The attestation parser is a closed self-consistency codec, not external
authority. Admission and replay receive the controller-store authority
independently, re-attest the pinned commit and task, and require an exact record
match. A syntactically valid record with recomputed self-hashes cannot select
its own authoritative store or commit.

Existing patch tasks read Cargo input as exact raw bytes from their declared
ancestor baseline. The capture task instead binds the tracked root `Cargo.lock`
at the exact controller commit and never uses the legacy embedded fixture. That
fixture remains available only to two exact historical task blobs.

### 4. Use a terminal single-attempt state machine

The only successful transition order is:

```text
admitted
  -> deterministic input attestation
  -> controlled-host preflight
  -> independent Codex + Claude pre-review
  -> terminate model processes and revoke provider egress
  -> one offline capture attempt
  -> canonical output and control-evidence verification
  -> freeze an immutable, digest-bound capture record
  -> restore exact first-party egress for fresh review processes
  -> independent post-review of that record
  -> seal a private final envelope containing the record and reviews
```

Any failed, cancelled, timed-out, uncontrolled, mutated, or ambiguous stage is
terminal for that run ID and produces a failure receipt. There is no automatic
measurement retry, repair, warm rerun, or favorable-result selection. A human
may authorize a later attempt only with a fresh run ID and claim; prior failed
attempt metadata remains linked.

An external append-only runner lease records the claim and attempt start before
the producer runs and rejects a duplicate attempt for that run ID. The receipt
can prove only the controller-observed history inside that lease. It cannot
prove that a person discarded an out-of-band measurement outside the lease;
that remains an explicit nonclaim.

The authority uniqueness key is the project authority plus run ID; controller,
task, profile, and runner identities are immutable claim-body fields, not parts
of the uniqueness key. An existing identical claim is already spent, never an
idempotent launch authorization. A same-UID create-new file prevents concurrent
claims but is owner-deletable; hostile rollback resistance requires a pinned
signing key and a separately administered append-only supervisor.

The implemented local claim binds an **expected** runner identity, not an
observed runner identity. The latter is temporally available only when a future
independently administered supervisor issues the runner lease. Pure claim
parsing proves canonical self-consistency only. Reserve and replay must receive
the authority root, project-authority digest, run ID, controller store, pinned
commit, task path, and expected runner identity independently; they re-attest
the commit and task before exact comparison. Claim-admission and state-genesis
views are deliberately reproducible plain data, not persistence proof or an
external witness. Every authoritative consumer must reopen the rooted claim and
reconstruct exact state; the first host-rejection boundary does so and rejects
missing, deleted, replaced or self-authored authority. There is no local API to acquire
a lease, authorize an attempt, delete, release, renew, reclaim, or retry a claim.

The acknowledgement is claim registration only. Its signature and supplied log
references prove no append-only, monotonic, global-order, fork, or rollback
property and do not create a lease. Positive authority needs a separate service
that durably orders registration, lease, attempt start, terminal outcome, and
final witness transitions.

Within the leased runner and controller process boundary, model processes and
model-network brokers must not exist during the measured interval. Pre-review
may assess the frozen plan; post-review assesses the digest-frozen capture
record before the final envelope is sealed. Neither review may supply
measurement values. Process census does not prove absence on another host or
outside the enforced lease boundary.

A rejected pre-review terminates without consuming an attempt-start
authorization; the run claim remains spent. A rejected or missing post-review
preserves the witnessed attempt as a verified
but import-ineligible terminal outcome; it may not relabel or discard the
measurement. Every started run produces terminal/failure evidence even when no
successful capture record can be formed.

### 5. Bind the controlled execution envelope

The first host-control implementation tranche is intentionally one-way. It
observes two bounded snapshots through fixed read-only kernel surfaces and can
only reject or leave control unproven. Raw source hashes remain auditable while
a separate normalized-control digest prevents irrelevant `/proc` churn from
faking control drift. Profile input is copied through captured typed-array
intrinsics, so hostile iterators, proxies, or species constructors cannot run at
the gate. Node architecture names are translated into the Rust
profile vocabulary; an unknown translation leaves control unproven. Even a
legacy profile match remains `unproven` because microcode, NUMA, cgroup/quota,
filesystem, thermal, runner/lease, binary, affinity, quiescence, and
measurement-boundary closure are not yet positive authority. The verifier
rederives profile observation and reasons from immutable profile bytes and
binds the exact before-state to the returned zero-attempt terminal fail. This
module never emits `pass-host-preflight`; the future evidence-aware positive
authority/replay boundary must authenticate evidence kind before asking the
generic state machine for that transition, immediately before the sole attempt.
An AST-based test recursively resolves runtime imports, permits only exact
read/hash/parser bindings and the fixed read-only open call, and mutation-tests
static, namespace, default, import-equals, dynamic, worker, datagram,
process-loader, write-flag, dynamic-constructor, and ambient-fetch escape forms.
It is a mutation sentinel over protected source, not independent runtime isolation.
Missing CPU-list evidence does not suppress independently observed governor,
turbo, swap, or load disqualifiers. Allowed, online, and isolated CPU lists are
parsed independently, so one malformed list cannot hide relations between the
remaining valid lists.

The runtime executes `sf-performance-receipt capture-baseline` from a unique,
regular, non-hard-linked binary whose bytes, length, and SHA-256 exactly equal
the artifact binding in the verified ADR-0039 pair receipt. It imports source,
toolchain, closure, and build attestation from that pair rather than rebuilding.
The held artifact is revalidated immediately before and after capture; a local
same-commit rebuild is ineligible unless it is one of the agreed artifacts. The
run is offline and shell-free under the exact CPU profile, fresh `/proc`, read-
only CPU-control `/sys`, no generic CPU quota, bounded resources/output, and
parent-controlled timeout/process-group cleanup.

Preflight is repeated immediately before capture and stable controls are
checked afterward. Evidence binds at least:

- verified pair receipt; controller commit/tree; task/profile/scenario/workload/source/lock digests;
- imported Rust toolchain/compiler/target/build/closure bindings and exact artifact length/SHA-256;
- kernel, CPU model/microcode, isolated/allowed CPU sets, affinity, governor,
  turbo observability/state, NUMA, cgroup, swap, filesystem, thermal/load
  limits, and runner identity/lease;
- exact argv/environment allow-list, start/end times, exit/signal/timeout and
  process cleanup;
- output path, mode, permissions, byte length, SHA-256, canonical parse, and
  producer/checker results; and
- native-host review identities, Ruflo task/swarm/trajectory IDs, capture nonce,
  and receipt-chain anchors.

Missing or unreliable controlled-profile, source/build/binary, command, output,
affinity, lease, isolation, or measurement-boundary evidence is always fatal.
Only explicitly ancillary evidence, such as additional thermal or firmware
detail outside the accepted profile, may be recorded as a nonclaim. Self-
reported host text alone is not control evidence.

The build attestation also binds resolved registry archives, proc macros, build
scripts, the exact Rust toolchain, native C compiler/linker inputs, runtime
libraries, and the read-only mount manifest. A broad host `/usr` mount or an
unattested successful build is insufficient.

### 6. Keep output private until replayed import

The private clean capture worktree is the producer's repository layout. Its
fixed baseline path is absent from the attested commit, becomes one untracked
controller-owned output there, and never touches the main/integration checkout.
After deterministic verification, freeze the exact bytes and evidence as the
immutable capture record; after post-review, seal that record and the reviews in
a canonical, bounded, mode-`0600` final envelope.

The producer output is created with mode `0600`, remains a regular single-link
controller-owned file, and is descriptor-reverified before a frozen copy is
made. The record binds the state head before its freeze event; the freeze event
then binds the record digest. The envelope binds the later sealed state, and a
detached authority witness binds the envelope digest and raw file digest last.
No digest-bearing object contains its own digest or a later state head.

Provider-free replay verifies schemas, canonical serialization, digests,
identities, ordering, protected inputs, command results, state transitions,
the one controller-observed attempt for this run ID, and the receipt chain.
Replay proves integrity of the recorded capture; it does not remeasure the host,
exclude out-of-band measurements, or prove that opaque hardware claims were
physically true.

Replay receives the authority key fingerprint independently, verifies the
append-only claim/lease/attempt/terminal/final-witness sequence, and never calls
a provider, probe, or capture command. Portability is limited to hosts able to
reconstruct the pinned controller and product checker closure; verified
integrity is not a claim that another host would reproduce the same timings.

Only the integration owner may import the exact replayed bytes. After import,
the baseline becomes a tracked protected input and ordinary CI runs the
read-only `check-baseline` command. CI must never run `capture-baseline`, probe
a host to claim comparability, or update the baseline automatically.

### 7. Keep learning off until product evidence is eligible

This task is not a GEPA/evolution fitness event. Retrieval tuning remains off.
The frozen 5+5 holdout eligibility and signed promotion rules in ADR-0037 still
apply; capture success cannot bootstrap, train, or promote a memory policy.

Native execution uses only first-party Codex/ChatGPT and Claude subscription
clients. OpenRouter, base-URL substitution, provider API-key transport, and
indirect gateways remain prohibited. `subscriptionCostUsd: 0` means zero
marginal provider-API charge, not a spend cap or a capacity claim.

## Implementation and acceptance sequence

1. Implement and mutation-test the closed task parser and policy state machine.
2. Implement exact input attestation and the negative-only host non-admission
   boundary before any positive runner claim.
3. In parallel, implement the independently administered append-only
   registration/lease authority, provision the dedicated controlled runner and
   tracked profile, and complete ADR-0039 two-builder artifact agreement.
4. After all three pass independently, integrate positive host/capture preflights
   and execution runtime while keeping V4/V5/V6 byte-stable.
5. Register and protect the task/launcher/parser/test controller closure in the
   global manifest and controller build. Bind the profile, scenarios, lock, and
   product-source closure as task-scoped immutable commit blobs so existing
   patch transactions may still modify product implementation paths.
6. Run one fresh-ID capture with native Codex and Claude review outside the
   measurement interval; seal and replay the private receipt.
7. Import only replay-verified bytes, protect the baseline, add CI
   `check-baseline`, and update ADR-0038/programme status with exact evidence.

Acceptance requires parser mutation coverage, dirty/pre-existing/path/identity
rejection, host-control failure tests, timeout/process cleanup, no-network and
no-model-overlap proof, no-retry state tests, mutation of every receipt binding,
provider-free replay, historical V4/V5/V6 characterization, harness build/test,
an explicit V6 capture-schema rejection test, and the existing Rust performance
feature tests and clippy gates.

## Consequences

- Good: a model-authored or copied TSV cannot masquerade as measured evidence.
- Good: patch and observational transactions retain honest, distinct semantics.
- Good: capture is replayable without calling either model provider or rerunning
  a noisy benchmark.
- Cost: a dedicated controlled runner and additional controller code are
  required before M0 performance evidence can close.
- Cost: a failed capture cannot be repaired in place; a later attempt requires
  an explicitly reviewed fresh claim.
- Neutral: this does not implement federation, production admission, packaging,
  or any semantic-fabric runtime feature.

## Rules

- **R1** — never treat patch output, model output, or a hand-authored baseline as
  controlled measurement evidence.
- **R2** — one run ID permits at most one measurement attempt and no automatic
  measurement retry.
- **R3** — no model process or provider egress in the enforced leased runner and
  controller boundary overlaps the measured interval.
- **R4** — exact product producer/checker and controlled-host gates outrank every
  review, score, diagnostic, or coordination signal.
- **R5** — capture output remains private until provider-free replay succeeds
  and an integration owner imports the exact bytes.
- **R6** — hosted CI verifies tracked evidence offline and never captures or
  promotes a baseline.

## More information

- Product harness and benchmark authority: [ADR-0005](ADR-0005-conformance-and-benchmark-harness.md)
- Performance model and boundedness: [ADR-0006](ADR-0006-crate-layout-and-performance-model.md)
- Patch transaction and native-host controls: [ADR-0037](ADR-0037-dual-host-ruflo-engineering-metaharness.md)
- Completion programme and M0 gate: [ADR-0038](ADR-0038-sota-application-completion-programme.md)
- Minimal artifact and two-builder release track: [ADR-0039](ADR-0039-minimal-production-serving-artifact.md)
- Programme plan: [SOTA application-completion programme](../plans/sota-application-completion-programme.md)
