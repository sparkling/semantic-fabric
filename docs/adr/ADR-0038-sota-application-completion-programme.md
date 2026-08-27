---
status: accepted
date: 2026-08-26
updated: 2026-08-27
tags: [programme, sota, completion, correctness, federation, production, release, sparc, ruflo]
supersedes: []
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
implements: []
---

# SOTA application-completion programme

## Context and problem statement

semantic-fabric has a strong virtual knowledge graph core, a large fixed-case
correctness suite, three admitted relational backends, and credible bounded-
streaming and Ontop comparison evidence. It is not yet a complete application.
The 2026-08-26 source audit found five release-level contradictions:

1. recursive property paths return a normal result after a hard-coded 256-hop
   truncation;
2. supported global ordering, grouping, DISTINCT, and CONSTRUCT dedup paths can
   retain source-sized Rust collections despite the bounded-memory invariant;
3. the public runtime owns one source, while the accepted charter includes
   cross-RDBMS federation;
4. resource governance, observability, configuration, identity, provenance,
   lifecycle, and release controls accepted in ADR-0010/0011/0017/0018 are
   incomplete or absent; and
5. the application lockfile is ignored, live/standards gates can skip, and the
   production CLI pulls conformance, benchmark, and prototype-adapter
   dependencies into its normal feature closure.

The current architecture should not be discarded. Its semantic compiler,
mapping IR, dialect emitters, backend streams, and exact-or-unsupported posture
are the right foundation. Completion requires two substantial extensions—a
bounded global physical-plan path and cross-source coordination—plus the
production and evidence layers already anticipated by accepted ADRs.

SPARQL 1.2 Query and Protocol are Working Drafts as of this decision date.
Therefore a timeless claim of “full SPARQL 1.2” is not a stable release contract.
R2RML remains the stable normative mapping baseline.

## Decision

Adopt the issue-independent programme in
[`docs/plans/sota-application-completion-programme.md`](../plans/sota-application-completion-programme.md)
as the accepted route to a charter-complete release.

This decision was accepted on 2026-08-27 by maintainer instruction to complete
the programme with its retained cross-RDBMS federation charter. Acceptance does
not claim implementation: the reusable-harness prerequisite and M0 foundation
are in progress, and M1–M7 remain gated by their executable evidence.

### 1. Definition of complete

“Complete” means all of the following, not merely a green workspace test run:

- every advertised capability is exact on every admitted backend or is rejected
  before execution through a dated, tested capability profile;
- engine memory and request lifetime are bounded for every advertised plan
  shape, not only the simple streaming benchmark;
- one request can query mappings assigned to multiple relational sources with
  correct global SPARQL semantics and explicit consistency/failure behavior;
- accepted operational, policy, and lineage decisions have executable evidence
  or are explicitly superseded before release;
- the released serving artifact is minimal, reproducible, auditable, and
  independently verifiable; and
- all hard product gates pass and the project-owned readiness score is at least
  98/100. No score can average away a failed gate.

A hardened single-source build is a valuable interim release profile. It is not
the charter-complete release while ADR-0002 and ADR-0006 retain cross-RDBMS
federation in scope.

### 2. Preserve the semantic architecture; extend the physical architecture

Keep the existing parser, T-box saturation, R2RML/Direct Mapping compiler,
IQ/cascade, dialect, `SqlBackend`, and reconstruction boundaries. Do not rewrite
the large compiler modules before characterization and generative tests exist.

Add five explicit domain contracts:

- `RuntimeSnapshot`: immutable ontology, mappings, source schemas, epochs, and
  digests, validated before atomic activation;
- `SourceRegistry`: `SourceId`-keyed backends, capabilities, schemas, health,
  identity policy, and cancellation hooks;
- `QueryBudget`: one deadline and work/result/byte/admission budget spanning
  ingress through serialization;
- `FederatedPlan`: per-source SQL fragments plus typed global operators,
  reducers, failure policy, and provenance; and
- `CapabilityProfile`: generated from executable evidence and bound to the
  release, standards snapshot, and backend matrix.

Global blocking operators must be lowered to source/composite SQL where sound.
Where cross-source semantics require coordinator work, the implementation must
use a separately decided bounded spill/merge substrate. Until a shape has such a
proof, it returns a resource-aware unsupported response; source-sized in-memory
collections are not an acceptable production fallback.

The coordinator substrate requires its own design-lock ADR because adopting an
embedded analytical engine would amend ADR-0006, while a purpose-built external
operator layer carries substantial correctness and maintenance cost.

### 3. Exactness and evidence precede feature breadth

Silent partial results are severity-zero release blockers. The first increment
fixes or rejects path closure beyond the current bound and makes every supported
blocking operator meet the memory invariant. Property, metamorphic, differential,
fuzz, mutation, exact-suite-inventory, coverage, load, and soak evidence then
become durable CI/release gates under ADR-0012.

The release profile pins a dated SPARQL/RDF specification snapshot and publishes
the tested delta when a Working Draft changes. W3C RDB2RDF mapping conformance is
reported separately from SPARQL query-language evidence.

### 4. Production runtime is a first-class bounded context

Implement one validated configuration and secret-reference model; verified
source TLS; safe public errors; bounded-cardinality OpenTelemetry-compatible
telemetry; health/readiness; graceful shutdown; source failure classification;
atomic snapshot reload; schema-drift detection; and complete cancellation.

Introduce a provider-neutral authenticated `SecurityContext`. PostgreSQL RLS,
portable ABAC/sensitivity enforcement, and query-time provenance remain aligned
with ADR-0017/0018, but platform-specific identity or taxonomy details do not
leak into the semantic compiler.

### 5. Product, evidence, and engineering control planes remain separate

Split or feature-gate the serving binary so conformance, benchmarks, SQL Server,
REST prototypes, and other experimental adapters do not enter its default
dependency graph. Track the application `Cargo.lock` and run release commands
with `--locked`.

The engineering MetaHarness in ADR-0037 may propose, critique, repair, and verify
work. Ruflo provides the persistent coordination ledger. Deterministic product
tests remain the fitness authority. MetaHarness diagnostics, model confidence,
Agentic-QE summaries, or a readiness score never substitute for application
evidence. Darwin/GEPA remains disabled until five discriminating training tasks
and five sealed holdouts exist.

### 6. Programme governance

Run each non-trivial increment through SPARC specification, architecture,
refinement, and completion gates. Independent work uses a hierarchical,
specialized Ruflo swarm, isolated writer worktrees, native Codex/ChatGPT and
Claude Code subscriptions, bounded critique/repair, independent verification,
and digest-chained receipts. One integration owner promotes verified slices in
dependency order. No OpenRouter or indirect provider transport is permitted.

Native subscription invocations have no artificial dollar ceiling. They remain
bounded by task-declared invocation and turn limits, wall-clock and output
limits, concurrency, first-party provider rate-limit backoff, and receipts. The
programme must not set or consume provider API keys. Native-provider exhaustion
or unavailability fails the affected gate closed and never authorizes an
indirect fallback.

Accepted ADR status means the decision is adopted; it does not imply the code is
implemented. Each affected ADR must gain a dated implementation-status note as
its increment lands. ADR-0014's previously deferred areas graduate into this
programme, but this ADR does not itself mark them implemented.

### 7. Implementation status (2026-08-27)

The H0a reusable-harness foundation is implemented in `b40dbc6`: frozen v5 gate
law and task/runtime derivation, strict schema dispatch, an externally anchored
policy fingerprint, and authoritative protected-input bindings. Schema v4
remains frozen and schema v5 remains disabled. H0b evaluator/scorer/envelope
work is in progress; H0c trusted-launcher activation and every M0–M7 milestone
remain pending their executable gates. This status does not raise the 44/100
application-readiness baseline or claim product completion.

## Acceptance

The acceptance condition was satisfied on 2026-08-27 when the maintainer
directed completion of this programme without removing its cross-RDBMS
federation requirement or gates. It moves to `implemented` only when:

- every programme hard gate is green on an immutable release candidate;
- the charter-complete two-source and three-admitted-backend matrices pass;
- every dependent accepted ADR has current implementation evidence;
- the minimal release artifact, SBOM, signatures, provenance, and clean-machine
  smoke tests verify; and
- the product readiness score is at least 98/100 with no hard-gate failure.

## Consequences

- Good: the existing semantic investment survives; work is sequenced by risk and
  executable proof rather than by feature count.
- Good: “single-source production profile” and “charter-complete federation” are
  honest, separately gated milestones.
- Good: draft-standard drift, backend admission, and unsupported behavior become
  testable release data rather than prose claims.
- Cost: federation, bounded global operators, policy, and lifecycle work are
  substantial and will change core physical-plan interfaces.
- Cost: fail-closed live, fuzz, mutation, load, and release gates need dedicated
  CI capacity and controlled benchmark infrastructure.
- Neutral: materialization, non-relational sources, RML, and external SPARQL
  `SERVICE` remain outside the charter.

## Rules

- **R1** — exact result or explicit pre-execution rejection; never silent
  truncation, partial success, or guessed semantics.
- **R2** — boundedness is proven for every advertised operator shape using heap
  and process-RSS evidence across growing source scale.
- **R3** — a source is not admitted until the complete backend contract, live
  matrix, security, cancellation, and conformance gates pass.
- **R4** — federation uses source identity, one shared budget, typed physical
  operators, and explicit consistency/partial-failure semantics.
- **R5** — all release inputs, dependencies, standards snapshots, tools, and
  evidence are pinned and digest-bound.
- **R6** — no aggregate or model score overrides a failed correctness, security,
  standards, boundedness, supply-chain, or release gate.

## More information

- Programme: [`docs/plans/sota-application-completion-programme.md`](../plans/sota-application-completion-programme.md)
- Stable mapping baseline: [W3C R2RML](https://www.w3.org/TR/r2rml/)
- Draft query baseline: [W3C SPARQL 1.2 Query](https://www.w3.org/TR/sparql12-query/)
- Draft protocol baseline: [W3C SPARQL 1.2 Protocol](https://www.w3.org/TR/sparql12-protocol/)
- Observability baseline: [OpenTelemetry database semantic conventions](https://opentelemetry.io/docs/specs/semconv/db/)
