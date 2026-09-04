---
status: proposed
date: 2026-09-04
updated: 2026-09-04
tags: [sparql, parser, resource-governance, isolation, rust, dos]
supersedes: []
depends-on: [ADR-0004, ADR-0010, ADR-0012, ADR-0038, ADR-0048, ADR-0052]
implements: [ADR-0010, ADR-0052]
---

# Grammar-coupled SPARQL parser governance and process-isolation fallback

## Status boundary

This ADR is **proposed**. A source-level feasibility audit selects the bounded
Linux Rust process-isolation fallback for V1; complete in-process hooks would
require a broad maintained fork, not the narrow extension originally preferred.
A dormant fixed-size `Hello`/`Ready` handshake codec is implemented, but no
process supervisor or launch, enforced containment, parser invocation,
`QueryV1` result wire, admitted-query witness or controlled binding exists.
This codec does not enable `CompileProfileId::GovernedV1`, change serving, or
change the capability catalogue.

The raw lexical scanner, parser-view direct-IRI measurement, fallible post-parse
algebra validator, bounded cache-key writer, exact clone roots, `CompileContext`
and Plan measurement remain private development foundations. Four dormant
fan-out/rollback sites prospectively meter nested-subplan rollback branch
forests, FILTER-over-UNION preceding-arm conditions, InnerJoin-over-UNION
preceding-arm IQ-node collections and conditions, and
LeftJoin-over-left-UNION preceding-arm scalar right nodes and conditions. All
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

The 2026-09-04 source audit binds the current dependency to `spargebra` 0.4.6,
Cargo checksum `46715eb9…f656`, with `peg`, `peg-macros` and `peg-runtime` 0.8.6
and the `sparql-12`, `sep-0002`, `sep-0006` and
`standard-unicode-escaping` features. The parser module is private. Its Unicode
decode precedes grammar entry; generated repetition allocates hidden vectors;
PEG rule and precedence recursion have no hook surface; semantic actions expand
IRIs, synthesize joins/terms and construct recursive algebra; and a failed parse
runs a separate expected-token traversal. A mechanical inventory found 257
grammar rules, approximately 224 reachable from `QueryUnit`, but cannot prove
construction, allocation, failure and destruction coverage. A credible hook
implementation would span the parser/actions, PEG macros/runtime and controlled
representation or allocation. V1 therefore selects process containment.

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

### 2. Do not mislabel a broad parser fork as a narrow hook

Grammar-coupled hooks remain technically possible, but V1 does not select them.
The source audit could not find a boundary that covers Unicode decode, generated
PEG recursion/repetition, semantic-action allocation, error replay and recursive
drop without coordinated changes across several upstream implementation layers.
A future upstream control interface or owned fork must repeat the exact-revision
inventory and prove every construction and failure path; it cannot inherit
authority from the diagnostic scanner or from this audit.

### 3. Select a bounded fresh Rust process per parse

V1 uses a Linux-only process-isolation profile. A prepared parent holds and
authenticates the same `sf-cli` executable, launches it by descriptor, and invokes
a private parser-worker mode before normal CLI parsing. Starting with the same
held binary keeps the worker inside the Rust/Cargo product boundary, pins the
main/parser bytes and prevents a mutable path from choosing another executable;
it does not attest the dynamic runtime closure, which remains an ADR-0039 release
gate. Non-Linux builds fail closed for this profile.

Each parse uses a fresh child. Before announcing readiness, the child applies
fixed stack, address-space, CPU, output, descriptor and descendant-process
controls and closes every unintended inherited descriptor. The parent retains
compiler and aggregate permits through deterministic kill and reap. Timeout,
cancellation, panic, abort, malformed output, output overflow or protocol
failure destroys no parent-side recursive parser value, and a subsequent request
must start successfully.

The fixed child resource ceilings are immutable per-request OS caps in a
separately named parser-containment envelope, not an accounting reservation;
refund semantics do not apply. Work model V1 charges only operations the parent can observe and schedule exactly: admitted
input/frame bytes, process launch, protocol frames, iterative validation/decoding
and owned compilation. It does not charge a fictitious worst-case amount for
child work that might not occur. Exact executable identity, Cargo
source/checksum and features, OS-control profile, containment limits,
parent-side charge schedule and wire version form the governed compile-profile
identity.

The child parses once and returns a bounded flat, index-based `QueryV1` wire. The
parent never accepts SPARQL/SSE text that would require reparsing. Frame lengths,
counts, indices and aggregate bytes are validated before allocation; decoding is
iterative and fallible, so generic recursive Serde is not an admissible shortcut.
A successful decode and post-parse algebra validation may eventually mint a
private `AdmittedQuery`; the current handshake-codec-only foundation cannot.

This is an implementation-time selection, not runtime failover. A release
profile attests one boundary and never switches after an error. Node,
MetaHarness and model hosts remain development/evidence infrastructure; the
worker and wire are Rust/Cargo product code under ADR-0048.

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
- **Pinned grammar-coupled instrumentation.** Not selected for V1: it preserves
  the returned algebra, but the audited hook surface is a broad maintained fork
  and still needs complete construction, failure and destruction proof.
- **Bounded Rust process isolation.** Selected after the source audit. It
  contains uninstrumented upstream parser behavior at the cost of launch/IPC, a
  Linux control profile and a bounded versioned wire contract.
- **A grammar-shadow estimator or replacement SPARQL parser.** Rejected under
  ADR-0004: it creates a second grammar whose drift must itself be proved away.
- **Post-parse validation alone, a crash probe, stack sizing, or thread timeout.**
  Retained only as evidence or defense in depth; none prevents or contains an
  unbounded parser operation.

## Implementation and acceptance gates

1. Pin parser/PEG sources, checksums and features plus the exact same-executable
   worker identity, Linux control profile, fixed containment limits and
   parent-observable work schedule.
2. **Handshake-codec foundation implemented:** fixed framing, magic, version,
   nonce, build/parser profile and exact effective-limit acknowledgement are
   canonical; failures are closed and non-reflective. Land the dormant supervisor next; this codec has no
   process, containment, query-wire or admitted-witness authority.
3. Prove child controls and descriptor/environment allowlists are installed
   before `Ready`; qualify descendant prevention without relying on per-user
   `RLIMIT_NPROC` as a per-worker boundary.
4. Inject timeout, cancellation, panic, abort, stack/address-space exhaustion,
   malformed/truncated/trailing/oversized output and forced death; prove bounded
   kill/reap, no PID/FD/permit leak and a successful next request after each.
5. Define a complete flat index-based `QueryV1` wire and iterative fallible
   encoder/decoder. Prove frame, input, output, count, index and aggregate-byte
   `0`, exact `N` and `N+1` rejection before allocation or access.
6. Differentially prove decoded `Query` semantics and syntax outcomes against
   the direct pinned parser over checked-in application, W3C, Unicode and
   adversarial corpora, including contextual angles, implicit joins, long
   BASE/PREFIX, collections, property lists, reification, RDF-star,
   `CONSTRUCT WHERE`, deep failure and recursive-drop fixtures.
7. Run persisted-corpus fuzzing, generated protocol/algebra properties and
   protocol/control mutation tests under ADR-0012; a bounded smoke is not
   acceptance.
8. Add an opaque private `AdmittedQuery` and controlled-binding typestate only
   after containment, wire, differential and algebra-validation gates pass.
9. Prove parser rejection is redacted, performs no cache/backend I/O and cannot
   mint a witness; parent cancellation/deadline observes the same sticky request
   control while permits remain held through reap.
10. Re-run full semantic equivalence, conformance, build, Clippy, harness and
    independent native Codex/Claude adversarial review before promotion.

Until every applicable gate passes, ADR-0052 remains proposed and serving uses
the current explicitly uncontrolled compiler path.

## Consequences

- Good: the safety boundary follows the actual parser grammar and allocations
  instead of inferring them from a divergent scanner.
- Good: the current semantic compiler, plan and execution architecture remain
  intact.
- Good: process containment is a fail-closed boundary for the audited upstream
  parser paths that cannot be completely hooked in-process.
- Cost: a fresh worker adds launch/IPC latency, a bounded wire protocol and
  Linux-specific operating-system qualification.
- Cost: the flat wire must explicitly cover the full admitted `Query` algebra;
  generic recursive serialization is intentionally unavailable.
- Neutral: diagnostic scanner and direct-IRI measurements remain useful for
  differential evidence but grant no product capability.

## Nonclaims

This decision does not claim that the selected supervisor/worker launch,
enforced containment profile, parser invocation or `QueryV1` wire is
implemented. It does not claim exact CPU time or heap bytes, governance of
raw/conformance APIs, complete owned compiler-phase accounting,
database/recursive SQL work, source-native cancellation, atomic post-`200`
delivery, backend admission, total M2 completion, or production readiness.

## More information

- [ADR-0004 — Oxigraph crates as the RDF/SPARQL substrate](ADR-0004-oxigraph-rdf-sparql-substrate.md)
- [ADR-0010 — Security and resource governance](ADR-0010-security-and-resource-governance.md)
- [ADR-0012 — Test strategy](ADR-0012-test-strategy.md)
- [ADR-0038 — SOTA application-completion programme](ADR-0038-sota-application-completion-programme.md)
- [ADR-0048 — Rust production and Node evidence boundary](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md)
- [ADR-0052 — SPARQL compilation safety and work accounting](ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md)
