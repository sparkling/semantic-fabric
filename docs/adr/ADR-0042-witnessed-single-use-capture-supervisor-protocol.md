---
status: proposed
date: 2026-08-29
updated: 2026-08-30
tags: [metaharness, evidence, supervisor, transparency, witness, lease, runner, security]
supersedes: []
depends-on: [ADR-0037, ADR-0038, ADR-0039, ADR-0041]
implements: [ADR-0041]
---

# Witnessed single-use capture supervisor protocol

## Status boundary

This ADR is **proposed**. It defines the authority protocol and deployable
boundaries needed by ADR-0041; it does not claim that an authority service,
transparency log, witness, project principal, controlled runner, lease, attempt,
or measurement exists. Moving this ADR or ADR-0041 to `accepted` requires
maintainer review of real operational evidence, not only repository tests.

The committed V1 registration, checkpoint, Merkle-proof, rooted-claim, and
negative host records remain byte-compatible and nonauthorizing. Production
authority uses a new protocol because V1 embeds a JavaScript safe-integer log
sequence in the signed leaf while an appender assigns the leaf index only after
receiving those bytes. Predicting that index would create a circular or
crash-sensitive contract. V2 separates the service's canonical decimal event
sequence from the transparency log's canonical decimal leaf index.

The committed provider-free V2 configuration, adjacency, event, history,
registration, commitment, transport, result and supplied-reference client modules,
private-package codecs, duplicate-first full/exact-only decisions, and dormant
coordinator implement bounded checks only. Registration/recovery use disjoint roots;
checkout acquisition must reserve/open nothing, while validated per-checkout open/discard closures provide cleanup, full candidate/provenance binding, and commit-gated replies.
One-shot peer consumption is only a contract/test double, not closure-owned enforcement.
The coordinator is build-only; public exports, dependency closure, and manifest stay
unchanged. With no writer, signer, network, or database, it remains nonauthorizing and preserves product goals and historical harness meanings.

## Context and threat model

ADR-0041 requires one controlled observational capture, with no retry or result
selection. The current controller has exact task, input, claim, private-source,
negative-host, state, signature, checkpoint, and RFC 9162 proof seams. It does
not have a writer, signer, network service, linearizable lease store, positive
host authority, runner, or launch capability.

The hostile cases include concurrent controllers, duplicate or changed
requests, a lost response after commit, process failure at every persistence
boundary, stale runner sessions, cross-run contention for one host, replayed
leases, service equivocation, checkpoint forks, same-UID mutation, and a caller
supplying its own trust material. Safety takes precedence over availability: an
ambiguous or partially published run may be permanently spent, but it may not
produce a second grant or attempt.

The `projectAuthorityDigest` in a claim identifies a project namespace; it is
not a credential. A service must authenticate an independently provisioned
project principal and map that principal to the digest through reviewed policy.
Likewise, an expected-runner digest or `controlled: true` text is not runner
identity or host admission.

## Decision

### 1. Separate principals and capability boundaries

Use distinct, independently configured principals:

| Principal | Authority |
|---|---|
| Project client | Submit requests for one configured project namespace; no sequence, runner, lease, or result authority. |
| Capture controller | Re-attest local claim/state/input authority and verify composite evidence; hold no service or runner signing key. |
| Supervisor service | Serialize run and runner-resource state, sign committed semantic events, and return exact stored results. |
| Runner agent | Prove possession of one enrolled runner/session key and enforce a resource fencing token before launch. |
| Transparency log | Publish immutable event bytes and inclusion/consistency material; allocate leaf indices only. |
| Checkpoint witness | Atomically persist and cosign consistent checkpoints under the C2SP witness protocol. |
| Semantic witness | Independently reject invalid per-run or per-resource transitions and persist both high-water marks atomically. |
| Witness initialization anchor | Prove through a deletion-resistant one-time record whether a witness identity/key epoch has ever left genesis; hold no witness signing key. |
| Deployment attestor | Bind reviewed artifacts/configuration to the independently administered live principals; hold no service, log, witness, or runner key. |
| Codex and Claude | Produce distinct native-provider pre/post-review receipts; never author measurement bytes or authority records. |

Trusted project, service, log, witness, witness-initialization-anchor,
runner-enrollment, deployment-attestor, TLS peer, key-epoch, and policy
identities come from a reviewed controller authority configuration.
No response may select or replace them. TLS protects transport; signatures,
policy mapping, persistent state, and proofs establish protocol identity.
Private keys, client credentials, and deployment secrets never enter this
repository or a controller build.

An attestation verified against that pinned deployment-attestor root, epoch, and
policy binds the exact deployed
supervisor and Tessera-personality artifacts, checkpoint- and semantic-witness
artifacts, witness-initialization-anchor and runner-agent artifacts, database
schema and migrations, configuration digests, key epochs, policies, and
administrative principals. Its signer is
administratively separate from every bound principal; a response-supplied or
unpinned signer is rejected. Repository tests without that attestation do not
identify the software holding the live keys.

The controller verifies that exact attestation at startup and immediately before
issuing any positive capability. Every live principal, artifact, configuration,
policy, and key epoch must equal its attested value. Any rotation or change is a
two-phase transition: a replacement attestation under the currently pinned
attestor policy must bind and authorize the complete successor set before any
successor can prepare. Preparation grants no authority; positive capabilities
and ordinary semantic events freeze until the transition protocol below commits
one successor. Attestor-root/epoch/policy rotation additionally needs an old-
policy transition binding the successor root and replacement attestation. An
unattested or mismatched live epoch immediately stops authoritative operation;
there is no grace window or stale-attestation fallback. A replacement attestation
alone can never reset or transfer witness-initialization state.

Controller verifiers stay under `coding-harness/src/`, with their existing
read/hash/verification capability closure. The private, independently deployable
`coding-harness/supervisor-service/` package has its own manifest and tests, but
its committed kernel is nonoperational; its future writer/signer/network adapter
must remain there. A runner agent belongs under a third principal.

### 2. Use one linearizable semantic state transaction

The first service implementation uses a transactional PostgreSQL state store.
One serializable operation must atomically:

1. authenticate the project principal and exact canonical request digest;
2. lock and validate the `{projectAuthorityDigest, runId}` state;
3. lock the independently enrolled resource and every member of its canonical
   physical-parent/overlap set in stable order;
4. allocate a monotonically increasing fence across that whole overlap set;
5. append the next immutable semantic event and signed response identity; and
6. update both run and resource state before any bytes are returned or published.

Database uniqueness constrains the project/run key and every active conflict-set
member. Each closed set contains 1–64 sorted unique resource IDs, includes its
canonical physical parent, and binds the exact runner-enrollment-record digest
before its domain-separated conflict-set digest. Aliases cannot establish safety
unless their signed sets intersect and every intersection is locked. Signer invocation follows locked validation, but a database rollback is never assumed to roll back an HSM or sidecar operation.
A post-sign/pre-commit orphan is nonauthorizing and must never be returned, published, or recovered as a result. Only the database transaction commits the exact envelope/result bytes, raw hashes, status, content type, outbox, run, resources, and counters together.
Signer placement is adapter-specific; crash injection covers every post-sign/pre-commit boundary. Best-effort deduplication or split run/event/resource updates are nonconforming.

After transport authentication maps the caller to one project, an exact project-scoped duplicate may recover exact committed response bytes without appending another event, even after the active authority head advances or reviewed policy rotates the admitted principal while preserving that project mapping. Recovery validates the stored project, current and original request/event/global provenance, canonical request bytes, raw request/response hashes, and canonical status-specific request-bound event envelope; original event digest/global position and changed-replay prior state come from one immutable joined event/run row, never independently synthesized. It then returns the original status, content type and bytes before any current-head, receipt or run read; it never rebuilds or substitutes a response and does not thereby claim signature verification.
On a lookup miss, the service verifies the current authority head and authenticated principal, reads the required predecessor receipt, and then reads run state. If the receipt is pending, the run read is consistency-only: it cannot classify or authorize a mutation; an absent or distinct valid state yields fixed `503`, while a missing exact-result row, malformed state or read ambiguity yields fixed `500`. The locked active head identifies the required global predecessor: authority genesis iff the next global sequence is one, otherwise the immediately prior semantic-event receipt; run state binds the last run event and its global position so a rolled-back or inconsistent head fails closed. The first changed registration terminal-spends through `registration-changed-replay-v2`.
Its evidence digest binds the original request/event digests, changed canonical request digest, authenticated project identity, and active authority head. A later distinct changed request against that closed run receives an eventless closed-run conflict and no semantic result; it never reuses the first terminal result.
No mutating request is accepted or classified while its predecessor lacks the required receipt. Later operation endpoints stay disabled until their changed-replay terminal outcomes exist. Mutations are never transport-retried after ambiguity; request digests are not bearer credentials.

Application-layer preclassification denial collapses malformed credentials,
unmapped projects, and stale heads to identical `403 registration-not-admitted-v2`
bytes; mTLS handshake rejection occurs before this codec.
Receipt pending is `503 registration-authority-pending-v2`; a closed run is
`409 registration-closed-v2`; unknown commit resolution is fixed
`500 transaction-resolution-unknown-v2`. Their fixed
directives respectively require a new authority-bound request, a new such
request after readiness, a new run, or exact-result lookup only. These canonical
unsigned responses contain only protocol/response/outcome/status/content-type/
directive fields, disclose no identity or digest, carry no `Retry-After`, and
must never be parsed as a signed semantic result. Adapter exceptions and malformed, multiple, inconsistent or indeterminate reads use the same internal indeterminate path and fixed `500`; only an explicit valid receipt-pending state uses `503`. A run that records the submitted request digest while its exact committed response row is missing is also indeterminate, never changed replay or a generic closed conflict. Mutating clients disable
automatic transport retries.
Pending is emitted only after authenticated project mapping, exact-recovery miss, current-head validation and one consistency-only run read, never as a namespace oracle. `registration-authenticated-denial-v2` remains a distinct,
signed semantic terminal after a committed registration predecessor.

### 3. Publish committed events without conflating order domains

The full signed event stays authenticated and private. Exact public leaf bytes
are canonical `{domain, commitment}` under
`semantic-fabric/programme-capture/supervisor-public-event-commitment-v2`; the
closed commitment contains only schema/protocol, leaf kind, log-identity digest,
and event digest and has no trailing newline. The log identity is derived under
`semantic-fabric/programme-capture/supervisor-transparency-log-identity-digest-v2`
from the parsed configuration's
complete transparency-log block; the builder accepts canonical configuration
and event-envelope bytes, never caller-selected digests. The stored commitment
digest is the raw SHA-256 of those exact leaf bytes. No public leaf contains
project/run/runner/session identity, credential, raw host/process evidence,
source/command/output detail, nonce, time, lease, fence, or reusable capability.
A sole-purpose personality submits it to Tessera. The private receipt keeps the
log `leafIndex` separate from service `globalSequence` and run `runSequence`;
all are canonical unsigned-64 decimal strings.

The log layer uses Tessera's synchronous publication support and a configured
C2SP witness policy. Tessera's optional antispam is only best-effort and is not
the run or lease uniqueness mechanism. Re-publication after an ambiguous log
write may create a second identical commitment leaf; semantic identity remains
the private stored event digest, and any returned proof must select an exact
matching commitment. No next semantic transition is accepted until the prior
event has a verified published checkpoint and required witness receipts.

A checkpoint witness proves that one tree view extends its persisted prior
view. It does not understand lease semantics and could cosign a consistent tree
containing two conflicting starts. Authoritative mode therefore pins one fixed
checkpoint-witness roster and one fixed semantic-witness roster with explicit
Byzantine threshold `f`. Each uses at least `3f+1` separately administered
witnesses. For roster size `N` and quorum `Q`, it requires `Q >= 2f+1` and
`2Q > N+f`; the latter condition remains mandatory when `N` exceeds `3f+1`, so
any two valid quorums intersect in more than `f` members. The supervisor is not
a roster member. A
semantic witness privately verifies the service signature, public commitment
proof, prior semantic receipt, state transition, run and every overlapping-
resource high-water mark, runner identity, and fence, then atomically persists
its new state before signing. A valid authority composite requires both named
quorums under the exact pinned policies; an arbitrary C2SP `any` policy is not
conforming.

The semantic genesis is canonical, not implicit. Each roster member is
provisioned with the same canonical record under domain
`semantic-fabric/programme-capture/semantic-witness-genesis-v2`, binding the
authority-configuration digest, log origin, service key epoch, both witness
rosters/policies, and typed `no-prior-event` run/resource high-water sentinels.
Its digest is the first registration event's prior semantic-receipt digest.
Only a never-used witness identity may initialize from that genesis and empty
maps. "Never used" is not inferred from an empty or missing local store. Before
first initialization, the witness must atomically consume an independently
provisioned, deletion-resistant one-time record bound to its identity, key and
roster epoch, genesis digest, and authority-configuration digest. The configured
anchor is a separately administered WORM registry or attested non-resettable HSM
monotonic slot with a pinned root and policy; its compare-and-set receipt becomes
part of the witness's durable state. An already-consumed, unavailable, rolled-
back, or unverifiable record forbids genesis initialization. A recovering
identity must authenticate and restore its latest durable
semantic receipt plus complete run/overlap-resource high-water state; missing,
truncated, or stale recovery state fails closed and can never reset to genesis.
A consumed tuple remains permanently ineligible under every future anchor. The
canonical authority-configuration history starts at the independently pinned
genesis configuration, not at the first rotation. Every closed configuration
record contains a canonical uint64-decimal `configurationEpoch`, the sole roster
epoch for every identity it binds, plus the complete service, log, witness, initialization-anchor, runner, key, roster, policy, and
deployment-attestation identities. Genesis alone has typed empty predecessor
sentinels. Its nonzero head digest is derived under
`semantic-fabric/programme-capture/supervisor-authority-genesis-head-v2` from its
epoch and configuration digest. Every successor's `predecessor.configurationDigest`
and `predecessor.headDigest` embed the exact active predecessor head; `headDigest`
is that derived genesis head or the event digest that activated the predecessor.
It never embeds the event that will activate itself. A transition advances the epoch by
exactly one and is itself a published, semantically witnessed event.

An anchor root, artifact, or policy cannot be replaced for an existing witness
identity/key/roster epoch. V2 never migrates an old tuple or consumed-set snapshot
into a successor anchor. Changing the anchor terminally retires the complete old
semantic-witness roster, used or unused, and requires genuinely new identities,
keys, and roster epoch. Before any proposal bytes escape, the serializable service
transaction performs a create-only compare-and-set from the exact predecessor
head to `transition-preparing` with one successor digest. An exact duplicate
retrieves the stored proposal; a different successor permanently conflicts and
reaches no conforming witness. Before signing that committed proposal, each old
semantic witness that contributes a signature atomically persists a
one-successor terminal marker binding the predecessor head and exactly one
successor configuration digest;
afterward it may only retrieve or re-sign those exact bytes, even if no quorum was
formed or it restarts. It can complete that successor or remain terminal, never
authorize another. The required old-policy quorum receipt binds those markers,
the final checkpoint and semantic receipt, complete run/resource high-water
digest, new anchor root/policy and roster, and replacement deployment attestation.

The successor configuration digest is computed first from the exact predecessor
head. The transition event then binds that successor digest, the fixed global
sequence allocated by that transaction, and the same predecessor head. The successor configuration never embeds
the digest of the transition event that activates it. The published event plus its quorum receipt is the
sole activation record; only then does `{successor configuration digest,
transition-event digest}` become the new head. Exact read-only recovery by
transition-request digest is mandatory; gathering the same signatures, storing
the receipt, and materializing that pair are idempotent. A crash after one marker
or a complete quorum can therefore finish only the bound successor.
There is no second controller-write authority and no alternate-successor recovery.
A permanently unavailable successor leaves authority frozen by design.
If a malicious service equivocates before quorum, intersection still prevents
two valid successor quorums, but split terminal markers may make every quorum
impossible. That deliberate denial-of-service outcome permanently freezes the
old authority; markers are never cleared or redirected to regain availability.

Cutover requires no active lease, launch capability, or nonterminal run; otherwise
it stops. A fixed readiness policy requires named service, log, anchor and runner
receipts plus the configured checkpoint- and semantic-witness quorums; an absent
mandatory principal blocks cutover with no smaller-policy fallback. This explicit
availability cost preserves safety. At startup, before accepting each semantic
event, and before issuing any positive capability, the controller replays or
authenticates the chain from pinned genesis through the activation receipt and
durably materializes that sole head. Every request, event, proof, witness state,
fence, and runner capability binds its epoch and digest. Old principals are
terminal-spent before new evidence is accepted, so an outage is allowed but
overlap is not.

The controller and semantic witnesses reject gaps, forks, reordered records,
duplicate successors, every successor semantic-witness identity/key present in
the genesis or any earlier roster, and every old epoch before successor-anchor
lookup. Every genesis-initialization request first proves the candidate is in the
exact active roster and absent from all prior rosters in that complete chain;
anchor lookup happens only afterward. Missing history or proof stops authority.

Without the required semantic quorum the controller may report signature,
commitment inclusion, and checkpoint-consistency facts, but must retain fork,
rollback, exclusive-lease, and state-transition nonclaims. Even a quorum proves
only the transition accepted under the pinned service/witness assumptions; it
does not prove general service honesty.

### 4. Version and order the run-event protocol

V2 events use closed canonical JSON, domain-separated Ed25519 signatures,
bounded canonical UTF-8, immutable byte snapshots before asynchronous work, and
separate signing, event-digest, and validation-digest domains. Proxies,
accessors, sparse or oversized arrays, duplicate keys, noncanonical decimal
strings, unknown fields, and unsafe integers fail closed.

The successful per-run semantic order is:

```text
claim-registered-v2                     runSequence 0
  -> runner-lease-granted-v2             runSequence 1
  -> capture-attempt-start-committed-v2  runSequence 2
  -> capture-attempt-terminal-v2         runSequence 3
  -> capture-final-witness-v2             runSequence 4 (required for success)
```

Before attempt start, an authenticated failure uses `capture-run-terminal-v2` at
the next run sequence and ends the run. Registration admits only changed-replay
or authenticated-denial; pre-lease admits admission, pre-review, runner,
policy, or internal failure; leased-pre-start admits expiry, admission revocation,
preflight, or internal failure, with every wire code carrying the `-v2` suffix.
The closed body binds current state/prior event, nullable lease/fence references,
and, only after lease, `released-unstarted` or `quarantined` evidence. Attempt,
capture, output, and cleanup remain null; post-start failure never uses this type.

Before a lease, the service verifies a one-use admission challenge, enrolled
runner key/session/boot possession, fresh typed positive host evidence, exact
profile/control policy, claim and registration proof, exact controller state
head, and distinct capture-specific Codex and Claude pre-review receipts. The
existing negative-only host module can reject, but can never be relabelled as
this positive evidence.

An authenticated denial, changed replay, admission/pre-review failure, or lease
expiry may append that pre-start terminal event and permanently spend the run.
A lease grant binds the exact project principal,
claim/registration, runner enrollment/session/resource, admission, pre-review,
lease ID, resource-scoped fence, chronologically ordered canonical service-issued/not-after instants,
`maxAttempts: 1`, and explicit `renew: false`, `releaseForReuse: false`,
`reassign: false`, `reclaim: false`, and `retry: false` policy.

The service—not a client wall clock—atomically resolves start versus expiry. If
the lease expires before start, `capture-run-terminal-v2` with the exact expiry
outcome spends the run. Once
start commits, expiry never releases or reassigns the resource. The resource
remains held until a terminal event confirms process/egress/output cleanup; if
cleanup is absent or fails, the resource is quarantined.

Attempt start binds the lease/fence, current state head, quiescence, fresh host
preflight, held source, separately agreed producer artifact, exact argv/env,
output slot, capture nonce, and attempt ID. The runner durably marks the exact
grant consumed and enforces the runner-resource fencing high-water before
executing. A token merely present in a record is not fencing. Without durable
launcher and output-sink enforcement, `resourceFencingVerified` remains false
and no launch capability exists.

Every committed start has exactly one `capture-attempt-terminal-v2`. Its closed
outcome-to-disposition matrix binds timeout, runner loss, missing output,
cleanup failure, egress violation, and fence invalidation to compatible
process/egress/output/cleanup/resource evidence. Unsafe or incomplete cleanup
cannot release a resource. A successful sealed or import-eligible run requires
the next `capture-final-witness-v2`, binding the terminal event, exact frozen
envelope, replay, and post-review digests. A failed run may carry one but never
needs it to remain terminal; neither form grants import, promotion, or release.

### 5. Admit positive controller transitions only through typed composites

The generic local state reducer remains a deterministic structural codec. Raw
digests and service booleans are not positive authority. A new evidence-aware
adapter must reopen and re-attest the rooted claim and state around verification,
then verify the independently pinned authority configuration, service/log/
witness signatures, exact V2 chain, runner admission, fresh host evidence,
resource fence, source/artifact/command/output bindings, and current state head.

Only that composite may produce a module-private, opaque, one-shot capability
for the corresponding local transition. A lease composite can authorize only
`acquire-runner-lease`; it keeps `attemptStartAuthorized: false` and
`captureAuthorized: false`. A later start composite is consumed durably before
the producer process exists. It cannot be serialized, cloned, caller-minted, or
reused after failure.

### 6. Keep the runner and producer agreement explicit

The current host is not a controlled runner. GitHub labels, a self-hosted smoke
job, the legacy performance profile, a signed service response, and local
process census are insufficient. Provisioning must establish the enrolled
runner/session identity, exact CPU and NUMA placement, governor/turbo/swap,
cgroup and no-generic-quota policy, filesystem/mount/runtime closure, thermal
and load bounds, model/provider absence, offline namespace, private output,
process cleanup, and fencing enforcement.

ADR-0039's pair agreement covers the public `semantic-fabric` release artifact;
ADR-0041 currently names the separate `sf-performance-receipt` producer. The
capture must consume a distinct two-builder agreement for that exact producer
and its runtime closure, or a reviewed successor bundle that explicitly binds
both artifacts. An ADR-0039 application receipt alone is not producer authority.

## Acceptance gates

This ADR remains proposed, and no positive transition is admissible, until:

1. ADR-0041 and this record are reviewed and accepted by a maintainer.
2. Project, service, log, checkpoint-witness, semantic-witness, deletion-
   resistant witness-initialization-anchor, runner, and administratively
   separate deployment-attestor roots/epochs, intersecting-quorum policies, and
   canonical semantic genesis are independently provisioned and pinned.
3. Concurrent-run, overlapping-resource, duplicate, changed-replay, denial,
   pre-start-terminal, expiry, start, attempt-terminal, final-witness, and
   crash-point tests prove a durable prefix and one exact response/event per
   semantic request.
4. PostgreSQL restart and fault-injection tests prove no duplicate/skipped run
   event, reused fence, overlapping active resource lease, or reply-before-commit.
5. Independent wire KATs and mutation tests cover every success and failure
   event type plus every identity, domain, ordering, nullable/forbidden field,
   time, fence, policy, state, runner, artifact, command, and output binding.
6. Tessera commitment inclusion/consistency replay, the fixed intersecting C2SP
   quorum, and semantic-witness replay pass from the exact canonical genesis
   digest and independently pinned empty run/overlap-resource high-water maps;
   deletion, rollback, duplicate initialization, and anchor-unavailability tests
   prove that a used witness cannot re-enter genesis, while anchor-rotation tests
   prove genesis-rooted replay, history-fresh successor keys, pre-lookup old-epoch
   rejection, no consumed-set migration, no mixed-epoch cutover, and rejection of
   gaps, forks, reordering, two successors, partial-quorum/successor loss, crash-
   recoverable activation replay, and restart-before-second-sign.
7. One separately administered runner proves enrollment, fresh admission,
   resource fencing, isolation, execution boundary, output freeze, and cleanup.
8. A capture-specific dual-review verifier accepts distinct native Codex and
   Claude receipts and rejects opaque or recycled review digests.
9. A two-builder agreement exists for the exact producer bytes and consumed
   runtime closure.
10. A deployment attestation verified by the independently pinned attestor
    root/epoch/policy binds the exact live service, log/witness personalities,
    initialization anchor, runner agent, database migrations, configuration,
    key epochs, policies, and administrative principals to reviewed artifacts;
    startup, positive-capability, and every epoch/change transition fail closed
    on stale attestation, with successor activation impossible before a valid
    replacement attestation and any required old-policy transition.
11. The full harness, product tests, hardened build, protected-path, capability,
    historical-anchor, replay, and adversarial gates remain green.

## Explicit nonclaims

A provider-free transition codec proves only exact configuration adjacency. It
does not prove prior global semantic state, full historical roster freshness,
old-policy quorum, publication, or activation.
Even after implementation, evidence is limited to the configured principals,
policies, keys, service transaction, log, witnesses, runner agent, and proved
checkpoint. It does not prove secure or accurate service time; physical host,
TPM, kernel, root, or hypervisor integrity; absence of out-of-band or discarded
measurements; process execution merely from a start record; model/network
absence outside the enforced namespace; physical fencing without downstream
high-water enforcement; persistence, non-equivocation, or rollback beyond the
named service and witness assumptions; uncompromised or non-colluding keys;
future currentness; or benchmark/output correctness from logging alone.

No event, witness, model review, harness score, or receipt grants commit, push,
import, publication, promotion, release, deployment, or learning authority.
The MetaHarness recall flywheel remains separately opt-in and off until its
frozen train/holdout gate produces a replayable strict-promotion receipt.

## Alternatives rejected

- **Keep V1 and predict the next log index** — circular and unsafe across
  concurrency, batching, duplicate publication, and crash recovery.
- **Use a local file, Git commit, same-UID lock, or harness memory as authority**
  — owner-rewritable and not independently administered.
- **Use Tessera antispam as uniqueness** — explicitly best-effort; it cannot
  replace a linearizable run/resource transaction.
- **Use only RFC 9162 consistency or a C2SP checkpoint witness** — proves an
  extending tree view, not valid single-attempt or exclusive-resource semantics.
- **Use only a transactional database** — can serialize semantics but lacks
  independently replayable public inclusion and checkpoint evidence.
- **Use Trillian for a new log** — it is in maintenance mode and recommends
  Tessera for new deployments.
- **Release or recycle on TTL, retry, renew, or reclaim** — a stale controller
  could then create a second attempt or overlap one physical runner.
- **Treat labels, profile text, or negative preflight as positive admission** —
  none proves the enrolled runner/session, enforcement, or fresh host state.
- **Put signing, storage, or transport in the controller verifier** — breaks the
  least-capability boundary and makes self-issued evidence possible.

## Consequences and rules

- Good: semantic uniqueness, privacy-minimized public commitment inclusion,
  checkpoint consistency,
  semantic witnessing, runner fencing, and controller transitions are distinct,
  testable claims.
- Good: V1 fixtures and historical harness anchors remain stable.
- Cost: a PostgreSQL service, Tessera personality, intersecting checkpoint and
  semantic witness quorums, runner agent, trust provisioning, and controlled
  host must be operated independently.
- Cost: safety-first crash handling can spend a run or quarantine a runner and
  require a fresh run ID or operator remediation.

- **R1** — service sequence, log leaf index, run sequence, and fencing token are separate canonical uint64-decimal domains.
- **R2** — one transaction owns per-run state, every overlapping-resource state, the signed event, and response identity; no reply or publication precedes commit.
- **R3** — no same-run renewal, retry, reclaim, reassignment, or second attempt.
- **R4** — a started resource is released only after proved cleanup, otherwise quarantined; TTL never releases it.
- **R5** — positive local transitions require typed composite evidence and an opaque one-shot capability; raw digests and booleans are insufficient.
- **R6** — C2SP checkpoint witnessing and semantic high-water witnessing are separate, fixed, intersecting-quorum requirements rooted at canonical genesis.
- **R7** — repository fixtures, current-host observations, and V1 records stay nonauthorizing.
- **R8** — the public log contains only domain-separated event commitments; private evidence and reusable capability material never enter a public leaf.

## Primary references

- [Tessera](https://github.com/transparency-dev/tessera) — tile-log publication and witness integration; antispam is explicitly best-effort.
- [C2SP witness protocol](https://c2sp.org/tlog-witness) — consistency and atomic persist-before-cosign rules.
- [Transparency witness](https://github.com/transparency-dev/witness) — interoperable C2SP service.
- [Trillian](https://github.com/google/trillian) — maintained legacy log recommending Tessera for new deployments.
