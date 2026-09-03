---
status: proposed
date: 2026-09-03
tags: [sparql, compiler, resource-governance, cancellation, cache, dos]
supersedes: []
depends-on: [ADR-0006, ADR-0007, ADR-0010, ADR-0012, ADR-0023, ADR-0038, ADR-0048]
implements: [ADR-0010, ADR-0038]
---

# SPARQL compilation safety envelope and versioned logical-work accounting

## Status boundary

This ADR is **proposed**. It records the next M2 design slice; it does not claim
that compilation is currently bounded, cancellable or governed by
`QueryBudget`. The current serving path limits query bytes and concurrent
blocking compiler jobs, but its compiler closure cannot consume the request
budget. `QueryLimits` has only source-work, result-item and serialized-byte
dimensions, and `CompilerBinding::compile` remains uncontrolled.

No capability catalogue entry, readiness signal or production-admission claim
may cite this ADR until the implementation and acceptance gates below pass.
Product implementation and deployable dependencies remain Rust/Cargo
artifacts. Node and MetaHarness may exercise or preserve evidence only, in
accordance with ADR-0048.

## Context and failure model

An admitted SPARQL request can spend substantial work before any source is
opened. Adversarial invalid or valid text can stress recursive parsing, prefix
and payload construction, algebra traversal, mapping-candidate fan-out,
branch-product allocation, normalization fixed points, canonical cache-key
construction and plan cloning. Checked query length bounds input bytes but does
not express structural limits or the work performed within that length.

The current compiler semaphore limits aggregate concurrency, not work per
request. A deadline can release the async waiter while `spawn_blocking` keeps a
non-cooperative compiler and both its compiler and aggregate admission permits
alive. Cache hits skip much cold work, but the current key has no governance
profile: an uncontrolled caller can populate the same semantic cache namespace
later used by serving. Integer overflow, charging after allocation and
cache-dependent authorization are additional fail-open risks.

The threat model includes an unauthenticated client choosing query text and
timing, repeated cache priming or eviction, and concurrent requests racing a
deadline or cancellation. It does not assume that a logical work unit equals a
CPU cycle, elapsed nanosecond or allocated byte.

## Considered options

- **Rely on query length, compiler admission and the wall deadline.** Rejected:
  they neither bound work within one admitted query nor stop detached blocking
  work.
- **Meter process CPU or elapsed time as the request budget.** Rejected as the
  primary contract: scheduler, machine, build and cache variation make exact
  boundary tests and portable configuration impossible.
- **Add logical fuel without a structural envelope.** Rejected: some recursive
  parser and formatter calls are outside project-owned loops, and charging only
  after they return is not a safety boundary.
- **Charge a cached plan's historical cold compilation cost on every hit.**
  Rejected: existing `QueryBudget` dimensions account for work actually
  performed. Replaying skipped work would silently turn accounting into a
  distinct complexity-authorization policy.
- **Fork the parser immediately or isolate every compiler in a process.**
  Deferred. Either may close the remaining pre-emptibility gap, but neither is
  required for the narrower, honest envelope-and-logical-work capability.

## Proposed decision

### 1. Extend the one request identity with an explicit fourth limit

`sf-core` adds `QueryCharge::CompilerWork`,
`QueryControlError::CompilerWorkExceeded`, a compiler-work atomic counter and
getter, and changes the public constructor deliberately to:

```rust
pub const fn new(
    max_compiler_work: u64,
    max_source_work: u64,
    max_result_items: u64,
    max_serialized_bytes: u64,
) -> Self
```

There is no three-argument compatibility constructor, implicit unlimited
compiler value or builder that hides the new dimension. The workspace is
pre-1.0 and its crates currently report version `0.0.0`; making every struct
literal and constructor call fail at compile time is the desired migration
mechanism. Raw tests and explicitly uncontrolled adapters may choose
`u64::MAX`, but serving must supply a finite value.

Compiler work belongs to the existing linearizable `QueryBudget`, not a second
budget. Its inclusive, checked, no-refund and sticky-first-terminal laws remain
unchanged across all four counters. Checked addition or multiplication failure
terminates with `AccountingOverflow` before allocation or mutation.

### 2. Put a fixed, versioned envelope in front of logical fuel

The governed compiler uses an internal closed `CompileProfileId` whose initial
variants distinguish `Uncontrolled` from `GovernedV1`, and an immutable
`CompileEnvelopeV1`. The envelope is a product safety boundary, independent of
the caller's fuel value and not raiseable by `--max-compiler-work` or another
runtime setting. A profile change requires a new identifier and cache
namespace.

Before `spargebra` parsing, an allocation-bounded, token-aware linear scan
enforces finite limits for tokens, individual lexemes, string/comment/IRI and
escape payloads, resolved prefix expansion, delimiter nesting, RDF-star
nesting, and right-recursive operator chains. It must distinguish syntax
contexts rather than count punctuation inside comments or literals. After
parse, an iterative traversal enforces finite algebra node, depth and retained
payload limits before any project-owned recursive rewrite. Plan builders reserve
finite branch, node, nesting and retained-payload envelope capacity before each
growth operation; a final iterative validation must pass before cache insertion
or return. A post-construction check alone is not admission.

The exact V1 constants are not invented in this design record. Implementation
must calibrate them against the frozen conformance and application corpus, add
adversarial boundary fixtures, freeze them in Rust source and this ADR, and
obtain review before the ADR can be accepted.

### 3. Charge deterministic work actually performed

An internal `CompileMeter` wraps the request's `QueryControl`. Work model V1 is
the checked sum of one unit for each elementary item actually visited or
created on the governed path:

- admitted input bytes and lexical tokens scanned;
- algebra nodes and retained payload bytes traversed;
- mapping and ontology candidates inspected;
- branch/product candidates considered, precharged with checked arithmetic
  before reserving or constructing their output;
- expression, term and rewrite-rule comparisons;
- nodes visited in every normalization/cascade round, including unsuccessful
  fixpoint rounds;
- canonical cache-key bytes produced; and
- plan nodes and owned payload bytes copied for cache insertion or return.

Every potentially super-linear loop charges at its loop boundary, and every
bulk allocation or clone charges its deterministically computed prospective
amount first. Counters never depend on hash iteration order, addresses, thread
scheduling, wall time or whether tracing is enabled. An implementation may
batch adjacent unit charges only when the checked total and exact failure point
are equivalent to individual charging.

This work model is a portable defensive proxy, not a measurement of total CPU,
heap, I/O or source execution. If cache-independent query-complexity
authorization is later required, it must be a separately named static plan
complexity limit rather than a fictitious debit for work not performed.

### 4. Add one whole-query controlled compiler API

`sf-sparql` adds this public entry point:

```rust
impl CompilerBinding {
    pub fn compile_controlled(
        &self,
        sparql: &str,
        control: &dyn QueryControl,
    ) -> Result<Plan>;
}
```

This is initially the only public API that may claim governance of a complete
query because it owns lexical admission, parse, keying and translation. A
public controlled API accepting an already parsed `Query` would omit parser
work and must not carry the same claim. Existing `compile`, `translate*` and
`parse_and_translate*` APIs remain source-compatible for raw, diagnostic,
oracle and conformance use, but are documented as explicitly uncontrolled.
Their semantic results remain the oracle for governed-path equivalence.

Project-owned traversals take `&dyn QueryControl` or `&CompileMeter`, checkpoint
at bounded intervals and propagate the existing `sf_sparql::Error::QueryControl`
without embedding query, mapping, schema or driver text.

### 5. Separate governed cache authority and charge the hot path honestly

The governed cache key includes `CompileProfileId` in addition to the existing
`CompileScope`, structural hash and collision-resolving canonical content. A
raw/uncontrolled compile therefore cannot seed or hit a governed entry. Cached
values become equivalent in shape to:

```rust
struct CachedPlan {
    scope: CompileScope,
    profile: CompileProfileId,
    plan: Arc<Plan>,
    clone_work: u64,
}
```

`clone_work` is the V1 structural cost of the one prospective deep plan copy,
not the historical cold compilation total. Cache lookup clones only the `Arc`,
validates scope and profile, charges actual lookup/key work plus `clone_work`,
then performs at most one deep clone. A cold path charges all compilation work
and the actual copy retained or returned by caching. Cache insertion never
creates governance authority.

A hot request may consequently pass with a compiler-work limit that would reject
the same cold request. This is intentional actual-work accounting. Both paths
remain inside the identical cache-independent V1 envelope, and admitted
semantic output must be identical.

### 6. Preserve deadline, cancellation and HTTP precedence

`sf-serve::deadline::run_compiler` changes its internal closure contract from
`FnOnce() -> T` to `FnOnce(RequestBudget) -> T`. The blocking closure receives a
clone of the same request identity that owns the deadline, all counters and the
aggregate admission permit; `sf-sparql` sees it only through `QueryControl`.
There is no compiler-local deadline or reset.

Serving adds `ServeOptions::max_compiler_work` and `--max-compiler-work`. The CLI
has a finite default selected by the calibration gate. `ServeOptions` struct
literals and `QueryLimits::new` call sites deliberately fail to compile until
they state the new policy; ordinary CLI callers that omit the flag keep working
through the finite default.

Owned compilation phases checkpoint before and after parse and at bounded loop
intervals. The first terminal cause remains sticky internally. A compiler-work
failure before a live HTTP handoff is a redacted `429 query-budget-exceeded`; a
deadline observed first is `DeadlineExceeded`; and the existing representable
expired-handoff rule still returns public `504` even when an earlier internal
resource cause remains recorded. Cancellation and accounting overflow stay
internal failures.

## Implementation gates

Implementation proceeds as bounded, independently reviewable Rust slices:

1. **Core identity:** add the fourth counter/error/limit; migrate every caller
   explicitly; retain exact concurrency, overflow and sticky-terminal laws.
2. **Envelope:** implement the lexical scanner and iterative algebra validator;
   freeze constants only after corpus and adversarial calibration.
3. **Owned compiler work:** instrument mapping expansion, branch products,
   normalization/cascade, canonical content and plan construction; charge before
   allocation and prove governed/raw semantic equivalence.
4. **Cache:** add the profile discriminator and `Arc<Plan>` storage; prove raw
   entries cannot hit governed compilation and that no deep copy precedes its
   charge.
5. **Serving:** pass the exact `RequestBudget` into the blocking worker, add the
   finite CLI/config limit and redacted HTTP mapping, and retain both admission
   permits until the worker really exits.
6. **Claims:** update capability and operational documentation only after all
   relevant gates pass; keep this ADR proposed until its constants and work
   model receive explicit maintainer acceptance.

## Acceptance gates

- Core tests cover `0`, exact `N`, `N+1`, checked overflow, concurrent consumers
  and every sticky first-cause pairing across all four dimensions.
- Lexical tests cover exact/max-plus-one strings, comments, IRIs, escapes,
  prefixes, Unicode, RDF-star, delimiters and right-recursive operator chains;
  iterative algebra tests cover node, depth and payload boundaries.
- Compiler tests cover mapping fan-out, `JOIN`/`OPTIONAL`/`MINUS` products,
  normalization and cascade fixpoints, and prove rejection occurs before the
  guarded allocation or clone.
- Cache tests pin cold and hot V1 work independently, allow the intentionally
  lower hot budget, reject raw-to-governed reuse, and prove semantic identity.
- Barrier-controlled deadline and disconnect tests prove eventual owned-phase
  worker and permit release without timing-only sleep assertions. They do not
  convert the parser limitation below into a one-second claim.
- HTTP tests prove a pre-source compiler excess performs no backend I/O and
  returns redacted `429`, while an expired representable handoff remains `504`.
- Bounded fuzz/property smoke tests cover scanner/parser/algebra boundaries
  under ADR-0012, and the full format, clippy, build, workspace-test,
  conformance and frozen-corpus regression gates pass.
- The finite serving default and all V1 constants are justified by replayable
  benchmark evidence; an adversarial review finds no uncharged super-linear
  owned loop or cross-profile cache path.

Only then may documentation claim: “The serving compiler enforces a fixed,
versioned syntax/algebra/plan envelope and deterministic logical-work accounting
on cold and governed-cache paths.”

## Known blocker and nonclaims

`spargebra` 0.4.6 exposes parsing without `QueryControl` and uses a PEG grammar
with recursive expression productions. The V1 pre-parser envelope bounds what
enters it, and checkpoints observe cancellation before and after it, but cannot
interrupt a parser call already running. The same caution applies to any
remaining upstream recursive formatter or allocator call until replaced or
instrumented. Cooperative/pre-emptive cancellation throughout parsing, or a
guaranteed one-second release of a compiler permit for every input, requires an
upstream/forked native Rust parser control hook or process isolation. This is
an unavoidable blocker to that stronger claim, not a blocker to the
narrower envelope-and-logical-work slice.

This ADR does not claim exact CPU seconds, wall time or heap bytes; database
rows scanned or recursive SQL iterations; source-cost governance; raw or
conformance governance; PostgreSQL/MySQL native cancellation; fairness or
bounded waiting; atomic post-`200` delivery; total M2 closure; or production
admission. It does not change query semantics or authorize a second compiler.

## Consequences

- Good: adversarial compiler work gains a deterministic per-request ceiling in
  the existing governance identity, with exact reproducible boundary tests.
- Good: a fixed structural envelope and disjoint cache profile prevent fuel or
  cache state from weakening the hard safety boundary.
- Good: the current compiler, `Plan`, bindings and Rust serving architecture are
  extended rather than rewritten.
- Bad: the constructor and public `ServeOptions` shape break source compatibility
  deliberately before 1.0, requiring explicit migration of all call sites.
- Bad: cache warmth can change work-budget admission even though semantics and
  the fixed envelope do not change.
- Bad: the upstream parser remains bounded but non-pre-emptible within one call;
  the stronger cancellation SLA remains open.

## More information

- [ADR-0007 — SPARQL-to-SQL rewriting strategy](ADR-0007-sparql-to-sql-rewriting-strategy.md)
- [ADR-0010 — Security and resource governance](ADR-0010-security-and-resource-governance.md)
- [ADR-0012 — Test strategy](ADR-0012-test-strategy.md)
- [ADR-0023 — Query IR architecture](ADR-0023-query-ir-architecture-flat-ucq-vs-iq-tree.md)
- [ADR-0038 — SOTA application-completion programme](ADR-0038-sota-application-completion-programme.md)
- [ADR-0048 — Rust production and Node evidence runtime boundary](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md)
- [Application-completion programme](../plans/sota-application-completion-programme.md)
