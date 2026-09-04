---
status: proposed
date: 2026-09-04
tags: [sparql, parser, resource-governance, isolation, rust, dos]
supersedes: []
depends-on: [ADR-0004, ADR-0010, ADR-0012, ADR-0038, ADR-0048, ADR-0052]
implements: [ADR-0010, ADR-0052]
---

# Grammar-coupled SPARQL parser governance and process-isolation fallback

## Status boundary

This ADR is **proposed**. It selects the minimum architecture to investigate and
prove before any parser-inclusive compiler-governance claim may be activated.
It does not accept a `spargebra` fork, add a production worker process, enable
`CompileProfileId::GovernedV1`, or change the capability catalogue.

The raw lexical scanner, parser-view direct-IRI measurement, fallible post-parse
algebra validator, bounded cache-key writer, exact clone roots, `CompileContext`
and Plan measurement remain private development foundations. One exact
nested-subplan rollback clone is metered through a dormant raw/metered seam; all
public compiler paths select uncontrolled mode. These primitives may produce
calibration and adversarial evidence, but none is parser admission authority.

## Context

ADR-0004 deliberately reuses the pinned Oxigraph `spargebra` parser instead of
building a second SPARQL parser. ADR-0052 proposes one governed path from raw
query bytes through parsing, compilation and cache handling. Source inspection
and adversarial fixtures show that a scanner outside the pinned grammar cannot
establish that boundary:

- the `standard-unicode-escaping` feature decodes the whole input before the PEG
  grammar sees it, so decoded syntax can differ from submitted bytes;
- PEG context decides whether `<...>` is an IRI or a relational expression;
- sibling graph patterns create implicit left-deep joins without explicit
  scanner-visible join operators;
- sequential `BASE` and `PREFIX` resolution materializes expanded IRIs; and
- property/object lists, collections, annotations, reification and
  `CONSTRUCT WHERE` allocate, generate or clone terms before a returned algebra
  can be validated.

A partially constructed or rejected recursive value must also be dropped. A
post-parse node/depth check therefore cannot retroactively protect parser stack,
intermediate allocation, amplification, work, or recursive destruction. Query
length and a thread timeout do not close those gaps.

## Decision

### 1. Keep the raw scanner outside production authority

The fixed raw-byte check runs before any decoding or allocation. All other raw
scanner dimensions are diagnostic proxies only: they neither admit nor reject a
production query and are not charged as if they were parser work. They may be
retained only while a maintained corpus uses them to expose parser/scanner drift.

The parser-view direct-IRI primitive remains separately named defense in depth.
It measures sequential in-query `BASE`/`PREFIX` state and direct materialization,
but does not bound parser-generated clones, containers, allocator overhead,
contextual PEG choices, or externally configured parser state.

### 2. Prefer a pinned, grammar-coupled controlled parser

The first implementation candidate is an upstreamable control interface or a
narrow, exact-revision fork of `spargebra` and, where required, its generated PEG
runtime. It continues to use the Oxigraph term and algebra model; it is not a
new grammar or a second semantic parser.

The parser control surface is provider-neutral and does not depend on
`sf-core`. `sf-sparql` adapts its events to the request's one `QueryControl`.
Before the corresponding operation, the controlled parser must:

- bound raw bytes and Unicode-decoder output and use fallible allocation;
- reserve deterministic work and checkpoint every recursive PEG family at a
  bounded interval;
- enforce recursion and pending-work limits before descent;
- reserve before parser-owned `String`, `Vec`, map, algebra-node, expansion,
  generated-term and clone growth;
- cover direct IRIs, prefix/base resolution, implicit joins, paths, collections,
  comma/semicolon sugar, annotations, reification and `CONSTRUCT WHERE`; and
- keep every partial/rejected tree within a destruction-safe structural bound.

A checked-in grammar-production and construction-helper inventory binds the
exact parser revision to its control hooks. Adding or changing an uncovered
production fails the gate. Parser limits and charge schedules form part of the
governed compile-profile identity.

### 3. Fail over architecturally, never dynamically

If review cannot prove that every recursive entry, allocation, syntax-sugar
clone, formatter step and rejected-tree destruction path is controlled before
the operation, in-process governed parsing is prohibited. The production design
must then use a bounded Rust process-isolation boundary.

That boundary runs the parse and all work needed to avoid reparsing hostile text
in the parent. It returns a bounded, versioned AST or Plan wire format; the
parent never treats child-produced SPARQL text as safe input to the same parser.
The worker has fixed stack, address-space, CPU/wall, output, process and
concurrency limits; deterministic kill/reap, permit retention, crash recovery
and next-request success are acceptance requirements. It receives either a
defined shared accounting channel or a parent-reserved monotonic work grant;
unused work is not refunded.

This is an implementation-time fallback, not runtime failover. A release profile
chooses and attests one boundary. It never silently switches after an error.
Node, MetaHarness and model hosts remain development/evidence infrastructure;
any product worker and wire implementation is Rust/Cargo under ADR-0048.

### 4. Preserve the semantic compiler architecture

The accepted parser emits the same `spargebra::Query` semantics consumed by the
existing T-box, IQ, normalization, lowering, cascade and SQL paths. Raw,
conformance and oracle APIs remain explicitly uncontrolled unless they receive a
separately named contract. This decision adds a safety boundary; it does not
change the virtualisation-only product goal, introduce a second query language,
or authorize materialization.

Immediately after a controlled parse, ADR-0052's iterative algebra validator
runs before canonical key rendering or project-owned recursive rewriting. Its
successful witness is required to form a governed cache key. Prospective
plan-construction bounds and owned-phase metering remain separate later gates.

## Considered options

- **Raw scanner plus post-parse validation.** Rejected as production authority:
  its accepted language and construction model differ from the decoded,
  contextual PEG grammar.
- **Pinned grammar-coupled instrumentation.** Preferred: it preserves the
  substrate and returns the existing algebra without a wire translation.
- **Bounded Rust process isolation.** Required fallback when complete in-process
  coverage cannot be proved; stronger containment costs IPC, packaging and a
  versioned wire contract.
- **A grammar-shadow estimator or replacement SPARQL parser.** Rejected under
  ADR-0004: it creates a second grammar whose drift must itself be proved away.
- **Post-parse validation alone, a crash probe, stack sizing, or thread timeout.**
  Retained only as evidence or defense in depth; none prevents or contains an
  unbounded parser operation.

## Implementation and acceptance gates

1. Pin the parser source/revision and freeze a complete production/hook inventory.
2. Differentially prove controlled and upstream parser results and errors over
   the checked-in application, W3C, Unicode and adversarial corpora.
3. Prove exact `0`, `N` and `N+1` rejection before decode, descent, expansion,
   allocation, clone and node construction for every hook family.
4. Include contextual-angle, Unicode-created syntax, implicit-join, long
   BASE/PREFIX, collection, property-list, reification, RDF-star,
   `CONSTRUCT WHERE`, deep failure and recursive-drop fixtures.
5. Mutation-test the hook inventory so removing or bypassing a hook fails.
6. Run persisted-corpus fuzzing and generated parser/algebra properties under
   ADR-0012; a bounded smoke run is not acceptance.
7. For in-process parsing, prove stack, heap and work plateaus at all fixed
   boundaries and show cancellation checkpoints use the request's exact control.
8. For process isolation, inject timeout, OOM, panic, malformed/truncated output
   and forced death; prove bounded kill/reap, no leaked permit/process, and a
   successful next request.
9. Prove parser rejection is redacted, performs no cache/backend I/O and cannot
   mint an admitted-query witness.
10. Re-run full semantic equivalence, conformance, build, Clippy, harness and
    independent native Codex/Claude adversarial review before promotion.

Until every applicable gate passes, ADR-0052 remains proposed and serving uses
the current explicitly uncontrolled compiler path.

## Consequences

- Good: the safety boundary follows the actual parser grammar and allocations
  instead of inferring them from a divergent scanner.
- Good: the current semantic compiler, plan and execution architecture remain
  intact.
- Good: process containment is a fail-closed architectural escape hatch when
  complete in-process instrumentation cannot be proved.
- Cost: a narrow parser/PEG patch carries revision, upstreaming and hook-audit
  maintenance.
- Cost: the isolation fallback adds a Rust worker, bounded wire protocol and
  operating-system qualification.
- Neutral: diagnostic scanner and direct-IRI measurements remain useful for
  differential evidence but grant no product capability.

## Nonclaims

This ADR does not claim exact CPU time or heap bytes, pre-emption without the
tested process boundary, governance of raw/conformance APIs, owned compiler
phase accounting, database/recursive SQL work, source-native cancellation,
atomic post-`200` delivery, backend admission, total M2 completion, or production
readiness.

## More information

- [ADR-0004 — Oxigraph crates as the RDF/SPARQL substrate](ADR-0004-oxigraph-rdf-sparql-substrate.md)
- [ADR-0010 — Security and resource governance](ADR-0010-security-and-resource-governance.md)
- [ADR-0012 — Test strategy](ADR-0012-test-strategy.md)
- [ADR-0038 — SOTA application-completion programme](ADR-0038-sota-application-completion-programme.md)
- [ADR-0048 — Rust production and Node evidence boundary](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md)
- [ADR-0052 — SPARQL compilation safety and work accounting](ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md)
