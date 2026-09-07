---
status: accepted
date: 2026-09-07
updated: 2026-09-07
tags: [programme, v1, completion, release, governance, ruflo]
supersedes:
  - ADR-0038
depends-on:
  - ADR-0002
  - ADR-0006
  - ADR-0010
  - ADR-0011
  - ADR-0012
  - ADR-0014
  - ADR-0017
  - ADR-0018
  - ADR-0024
  - ADR-0037
  - ADR-0048
  - ADR-0050
implements: []
---

# V1 product completion and release profile

## Status boundary

This decision is **accepted**. It replaces ADR-0038 as the controlling
definition of product completion and release work for v1. ADR-0038 remains an
auditable record of the broader SOTA programme; its research and advanced-
assurance work becomes a labelled post-1.0 backlog.

This is an explicit priority and evidence-scope change, not an implementation
claim. Moving an item to post-1.0 does not make it complete, supported, or
production-admitted. ADR-0002's virtualisation and cross-RDBMS product charter,
ADR-0048's Rust runtime boundary, and the accepted security, governance, and
operability contracts remain in force. ADR-0037 remains the accepted
engineering control-plane design, but its full transaction is no longer a gate
on every v1 commit.

## Context

The application programme mixed three different outcomes:

1. a usable, secure, bounded semantic-fabric product;
2. the minimum evidence needed to release that exact product responsibly; and
3. research-scale quality trains, advanced provenance infrastructure, and
   autonomous harness evolution.

Treating all three as one serial completion gate delayed product integration
without making incomplete runtime features safer. The code already contains
substantial exact-query, mapping, snapshot, lifecycle, tracing, and narrow
federation foundations. It is not complete: public authorization, general
reload and drift handling, the remaining runtime controls, useful bounded
cross-source execution, product packaging, backend admission, and release
evidence remain open.

V1 therefore needs one explicit product profile, one short integration path,
proportionate verification during development, and full evidence at integration
and release boundaries.

## Decision

### 1. V1 remains a real product completion milestone

The v1 release is the virtualisation-only Rust application described by
ADR-0002 and ADR-0048. Its release profile must enumerate, by exact version:

- query forms, operators, result formats, RDF/SPARQL snapshot, and R2RML/Direct
  Mapping behavior;
- admitted SQLite, PostgreSQL, and MySQL source profiles;
- admitted single-source and cross-RDBMS plan shapes; and
- configuration, security, lifecycle, and resource limits.

Every advertised cell is executable and exact. A cell outside the release
profile rejects before source execution with a stable public error. A backend
or charter capability may be removed from v1 only by an explicit superseding
ADR; editing prose, changing a matrix label, or omitting a test is not a scope
decision.

The cross-RDBMS charter remains product work. The v1 profile must include the
current exact bounded multi-source UNION and at least one useful exact bounded
cross-source join path with explicit consistency, cancellation, and failure
semantics. Wider operator combinations may stay excluded when their rejection
is tested and accurately published.

### 2. Critical product and runtime guarantees

The following remain release blockers for every admitted v1 path:

- **Exactness:** no truncation, guessed semantics, partial-success label, or
  source result presented as a complete SPARQL answer.
- **Bounded execution:** request bytes, active work, compiler work, source work,
  results, retained state, deadlines, and cancellation have finite validated
  controls appropriate to the advertised shape.
- **Snapshot integrity:** one immutable ontology/mapping/schema/source/policy
  generation is pinned through a request; off-path validation, atomic
  activation, drift detection, cache invalidation, and readiness fail closed.
- **Security:** authenticated requests create an explicit provider-neutral
  `SecurityContext`; policy mismatch fails closed; cache partitions cannot cross
  policy, subject, or attributes; authorization is enforced before results are
  released; pooled source context is transaction-scoped and cleaned on every
  terminal path. The external sensitivity taxonomy is consumed when available,
  never invented locally.
- **Operability:** typed layered configuration and secret references, verified
  TLS for remote sources, safe errors, bounded-cardinality logs/metrics,
  liveness/readiness, source-failure classification, and bounded shutdown work
  on the release artifact.
- **Federation:** every source shares the request snapshot, budget, security
  context, cancellation, and explicit partial-failure policy.
- **Lineage:** bounded identifiers tie a result or access decision to its
  mapping, source, snapshot, plan, and policy without logging source values or
  raw identity.
- **Runtime closure:** the deployable product and every deployable dependency
  are Rust/Cargo artifacts; development, benchmark, conformance, Node, Ruflo,
  and model tooling do not enter the production dependency closure.

An accepted ADR applicable to these guarantees must be implemented for the v1
profile or explicitly superseded. An implementation foundation or private seam
does not satisfy a public runtime gate.

### 3. Minimum release evidence

An immutable v1 release candidate must have all of the following:

- locked full-workspace format, build, test, and strict Clippy results;
- focused semantic, negative, collision, cancellation, redaction, and
  fail-before-I/O proofs for each changed critical boundary;
- the dated capability/standards profile and required live matrix for every
  admitted backend and federated shape, with no unexpected skip or deviation;
- boundedness and overload evidence for every admitted blocking or recursive
  path, without requiring a research-grade comparative benchmark;
- a minimal versioned serving artifact, clean-machine smoke against every
  admitted backend, and a non-root/read-only reference deployment;
- locked dependency and licence review, reachable critical/high advisory
  disposition with owner and expiry, an SBOM, checksums, signature, and
  provenance bound to the exact packed artifact; and
- independent native Codex and Claude review of the exact release delta. Model
  review is corroboration; deterministic product evidence remains authority.

One clean, controlled release build is the v1 minimum. A second independent
byte-identical builder, transparency service, exhaustive dynamic-loader proof,
and research benchmark publication are post-1.0 assurance unless another
accepted ADR promotes one into the v1 threat model.

### 4. Work explicitly moved to post-1.0

The following stay open, visible, and eligible for later scheduling, but do not
block v1 once the critical profile above passes:

- the 100,000-case generated train; cross-product NoREC/MR1 expansion; long
  fuzz campaigns; global coverage and mutation-score ratchets;
- comparative Ontop optimization research, research-grade controlled
  performance publication, one-hour soak expansion, and speculative operator or
  backend breadth outside the declared v1 profile;
- two-builder bit-for-bit reproducibility, complete ELF/syscall/runtime-closure
  attestation, public transparency/witness quorums, and the advanced capture
  authority described by proposed ADR-0041 through ADR-0047; and
- MetaHarness V7 expansion, Darwin/GEPA, AVO, autonomous retrieval-policy
  tuning, and other harness evolution. The flywheel remains off.

Existing focused generated, mutation, live, or harness evidence is retained and
may satisfy a directly corresponding v1 gate. Deferral applies to expanding the
assurance programme, not to deleting evidence that protects a shipped boundary.

### 5. Integration and writer topology

V1 work uses one canonical integration branch and one integration owner. At
most two code-writing worktrees are active at once, including the integration
owner when that owner is writing. Their mutable paths and dependency order must
be disjoint and stated before work begins. Read-only analysis, test execution,
and review may run concurrently without becoming additional writers.

Long-lived milestone branch forests are not an integration plan. A verified
coherent commit is integrated promptly, and the next work is based on that
canonical head. Reusable work found on an older branch is ported deliberately
and reverified; branch age or a green historical run grants no authority.

### 6. Proportionate gates

Each coherent product commit runs formatting plus focused affected-crate tests,
doctests, build/check, strict Clippy, feature combinations, and named negative or
mutation proofs appropriate to its boundary. A dependency, shared-contract,
unsafe, release, security-enforcement, or uncertain-impact change escalates to
the affected integrated gate before commit.

After each integrated product slice, the canonical integration branch runs the
full locked workspace format, test, build, check, and strict-Clippy gates plus
the relevant feature, live-backend, generated-authority, cache-isolation, and
cross-source matrices. Unknown impact or selector failure runs the full set.

The immutable release candidate runs every v1 product and minimum-release gate
in this ADR. A per-commit success cannot substitute for the integrated or
release run, and a previous-head result cannot attest a later commit. Pure
documentation status changes use structural, link, line-count, and diff checks;
they do not require an unrelated product rebuild.

Ruflo may retain coordination and receipt identifiers, and ADR-0037's harness
may be invoked for a high-risk boundary or final release review. Neither is a
product oracle. Full harness evolution and its research score are post-1.0.

### 7. Status and claim discipline

Status is evidence-scoped:

- `implemented` means the named behavior exists at the cited commit and its
  required executable evidence passes;
- `complete` means every gate of the explicitly named profile passes on one
  immutable candidate;
- `deferred-post-1.0` means work remains undone outside the v1 gate; and
- `unsupported` means an exact rejection is implemented, not that unfinished
  work was renamed.

Moving a task between those sets requires an ADR or a dated implementation note
that names the changed authority. Capability tables, README projections, and
release notes must be generated or updated from the same truth after code lands.

## Completion sequence

1. Freeze the v1 release profile and current canonical integration head.
2. Finish public security enforcement, snapshot reload/drift, total request
   controls, configuration/TLS/metrics, and cross-backend cleanup.
3. Complete the bounded cross-source join/profile and its live differential.
4. Split and version the minimal production artifact and close admitted backend
   matrices.
5. Run the full integrated gate, repair only from the resulting exact head, and
   freeze an immutable release candidate.
6. Produce and verify the minimum release-evidence bundle, then tag only that
   exact candidate.

## Current implementation status

The v1 profile is accepted and **not complete** on 2026-09-07. Existing code has
exact-path, request-admission, immutable-snapshot, bounded-shutdown, partial
tracing, mapping-evidence, private security-context/cache, and narrow
multi-source UNION foundations. A default-off, three-family Prometheus candidate
is verified but pending promotion. Public authorization, general reload/drift,
complete total governance, verified TLS/layered configuration/OTLP, the full
metric catalogue, useful cross-source join execution, production packaging,
backend admission, and the minimum release bundle remain open.

## Consequences

- Product completion has a shorter, explicit critical path without weakening
  the advertised runtime contract.
- Full integration and release verification remain mandatory while redundant
  full-workspace runs no longer serialize every small commit.
- Advanced assurance remains valuable and visible, but its absence cannot be
  misreported as either implemented product behavior or a v1 failure.
- The original SOTA score and research programme cease to be release authority;
  they may be resumed post-1.0 against the shipped profile.

## Acceptance

This decision is accepted by maintainer direction. The v1 application is
complete only when the critical product/runtime guarantees and minimum release
evidence above pass on the same immutable candidate, all advertised capability
cells are exact, and every remaining post-1.0 item is labelled without being
counted as complete.

## More information

- Historical SOTA programme: ADR-0038.
- Engineering control plane: ADR-0037.
- Living programme: `docs/plans/sota-application-completion-programme.md`.
