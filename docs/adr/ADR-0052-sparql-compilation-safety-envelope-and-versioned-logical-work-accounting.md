---
status: proposed
date: 2026-09-03
updated: 2026-09-08
tags: [sparql, compiler, resource-governance, cancellation, cache, dos]
supersedes: []
depends-on: [ADR-0004, ADR-0006, ADR-0007, ADR-0010, ADR-0012, ADR-0023, ADR-0038, ADR-0048]
implements: [ADR-0010, ADR-0038]
---
# SPARQL compilation safety envelope and versioned logical-work accounting

## Status boundary

This ADR is **proposed**. Implemented foundations include the compiler-work
`QueryBudget` dimension and typed terminal causes, mandatory
`QueryControl::terminate`, exact request-budget handoff and compiler-permit
retention, profile-keyed cache entries, and active serving reuse of `Arc<Plan>`.
The raw cache and dormant zero-capacity governed shard are physically distinct.
Primitives provide fallible algebra/Plan measurement, bounded canonical rendering,
compiler reservations and exact clone roots. On 2026-09-08 public serving tree
compilation connects five fan-out/rollback clone sites to request control, binding
checkpoint, exact measurement, reservation and exactly one clone:
nested-subplan rollback branch forests; FILTER-over-UNION preceding-arm
conditions; InnerJoin-over-UNION preceding-arm IQ-node collections and
conditions; LeftJoin-over-left-UNION preceding-arm scalar right nodes and
conditions; and Construction-over-UNION substitution/projection fan-out. Their
exact-limit and pre-mutation rejection tests preserve operation-local charging.
`CompilerWorkMode` is now retained through IQ lowering, including nested
SubPlans and nested `EXISTS`. For `B` resulting branches, a borrowed `EXISTS`
body is cloned only for the preceding `B-1` branches and the final branch owns
the original; rejection of a later clone retains completed earlier charges.
Compiler-measurement work-stack allocation failure has a distinct typed cause
and a redacted `503` mapping without `Retry-After`. Public operation-local clone
limits do not grant whole-plan validity or activate the dormant governed profile.

ADR-0053's normal control-only parser peer, selector-free parser-free peer and separately feature-gated parser-free mutant peer use exact private tuples, the same held ELF, cumulative-cap nonblocking pipes and one immutable spawn deadline. Evidence commits `e55fccd` and `ce5487e` implement the closed child/parent mutant matrix, `d103438` tracks its sources in the development harness, and `fa9d977` proves live request-EOF ordering; these commits are integrated evidence, not shipment or release qualification.

The normal path sends a prepared 96-byte header plus source after `Hello`/`Ready`, closes stdin, and preserves structural preflight → one complete request allocation → exact EOF → nonce/digest/UTF-8 validation. The parent preserves stack header preflight → prospective output cap → one complete result allocation → exact EOF → pidfd waitability/group sweep/exact reap/success → post-reap request replay, correlation, digest, `QueryV1` decode/direct re-encode/static equality. Only unit escapes from either hidden evidence seam.

A closed two-byte big-endian directive before `Hello` selects ten same-executable mutants. Four exit-zero correlation/payload defects fail only after reap; status 78 and deadline dominate wrong correlation after 412 accepted output bytes; a trailing byte fails EOF before semantics while accepted output stays 412; exact-cap output accepts 8,388,920 bytes before post-reap invalid-`QueryV1`; cap+1 fails prospectively at 312 bytes before body allocation; and request-frame allocation refusal means zero result bytes after the required 184-byte `Ready`, followed by EOF, exact reap and raw 78. Live bad nonce, source digest and UTF-8 requests remain alive and silent until stdin EOF, then close output and raw-exit 78. Each case permits a clean next launch.

Commit `38e9c7a` additionally exercises closed zero-, one-, and three-byte plus unknown-`u16` directives against one held executable, proving zero output, exact exit 78, reap and recovery. Commit `5a9919b` reuses one verified held descriptor for the complete ten-case matrix while retaining a fresh child and unchanged controls per case; this removes redundant whole-file hashing but grants no additional authority. The refreshed default dependency receipt at `949cf11` remains distinct from the unqualified parser-profile closure.

The provisional whole-life input cap is exactly 1,048,856 bytes: the normal source ceiling is 1,048,576, while the directive reduces the mutant ceiling to 1,048,574. Output is capped at 8,388,920 bytes (`Ready` 184 + result header 128 + body 8,388,608), independently of the 67,108,864-byte `RLIMIT_FSIZE`. Commit `235084d` adds a separately feature-gated real-parser `QueryV1` evidence peer across the sealed seven-case starter corpus: fresh contained children parse six inputs and return one fixed syntax rejection. Success is accepted only after EOF, exact reap and correlation, worker decode/re-encode byte replay, independent direct decode/re-encode byte replay, and source/profile-bound alpha equivalence. Only aggregate counts escape; no witness or authority is minted. Fresh parser encodings are not required to be byte-identical because generated identifiers may differ.

Public ordinary/security tree misses, preflight and federation retain the request's
`CompileContext`; cache authority remains `Uncontrolled`, not `GovernedV1`.
Tree inner joins reserve candidate pairs and exact scalar left copies; HTTP tests
prove pre-source rejection and all 64 exact VALUES tuples with sufficient work.
Atom resolution and direct lineage also meter map/POM visits, graph comparisons/
filtering/enumeration, class/POM products, parent lookups and exact source copies.
Parent maps are borrowed. Nested contexts retain control; lineage keeps separate
eligibility/recipe charges. Absent-predicate rejection and exact six-triple results
are tested. Path-specific mapping loops, TBox/unifier internals, other copies and
compiler phases remain open. Verified-generation preflight retains its permit
across lease acquisition; live SELECT/ASK/CONSTRUCT keep the 34/33 metadata gate.
Parser/destruction, other owned phases, cache eviction and provisional limits remain open.

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

The pinned `spargebra` 0.4.6 parser first performs global Unicode decoding and
then resolves context-sensitive PEG productions. `<...>` can be an IRI or an
expression; implicit sibling graph patterns create left-deep joins without an
explicit join token; BASE/PREFIX expansion allocates resolved terms; and
property/object lists, collections, annotations, reification and
`CONSTRUCT WHERE` generate or clone terms before a returned algebra can be
checked. Partially built or rejected recursive values must also be dropped.
Consequently an external lexical model and a post-parse traversal cannot prove
parser construction or destruction safe.

The current compiler semaphore limits aggregate concurrency, not work per
request. A deadline can release the async waiter while `spawn_blocking` keeps a
non-cooperative compiler and both its compiler and aggregate admission permits
alive. Cache hits skip much cold work, but the current key has no governance
authority. Physical profile shards prevent raw hits and eviction from entering
governed state. Serving capacity stays raw; dormant governed capacity is zero.
Future governed eviction/drop work, overflow, post-allocation charging and
cache-dependent authorization remain risks.

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
- **Use a grammar-shadow scanner as parser admission authority.** Rejected: its
  language and construction model diverge from the Unicode-decoded contextual
  PEG grammar, effectively creating a second parser contrary to ADR-0004.
- **Instrument the pinned parser or contain it in a bounded Rust process.**
  ADR-0053's source audit selected process isolation for V1 because complete
  hooks require a broad maintained parser/PEG fork. The boundary is required
  before any parser-inclusive governed compiler path may activate; a thread
  timeout or post-parse check is insufficient.

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

### 2. Layer the fixed envelope around the real parser boundary

The governed compiler uses an internal closed `CompileProfileId` whose initial
variants distinguish `Uncontrolled` from `GovernedV1`, and an immutable
`CompileEnvelopeV1`. The envelope is a product safety boundary, independent of
the caller's fuel value and not raiseable by `--max-compiler-work` or another
runtime setting. A profile change requires a new identifier and cache
namespace.

The raw input-byte ceiling is checked before Unicode decoding or allocation.
The current token/lexeme/nesting/operator scanner is diagnostic only: it neither
authorizes nor rejects production input and its counters are not
grammar-complete parser work. The parser-view direct-IRI primitive separately
measures sequential in-query BASE/PREFIX state and direct materialization. It
does not cover contextual PEG choices, parser-generated clones, container or
allocator overhead, or externally configured parser state.

Parser-inclusive governance uses ADR-0053's selected bounded Linux Rust process
boundary: a fresh same-executable worker parses once and emits a bounded flat
wire that the parent validates and decodes iteratively without reparsing. A
successful authoritative parser boundary returns a private admitted-query
witness; only that witness can construct a `GovernedV1` cache key.

`QueryV1` has two different equivalence obligations. Exact replay means that
one worker-produced wire decodes to the exact encoded `Query` structure and can
be re-encoded canonically. A fresh independent parse is different: pinned
`spargebra` generates random internal variables and anonymous blank-node IDs,
so raw `Query` equality or byte-identical wire across two parses of identical
source is not deterministic. Fresh-reparse proof therefore compares same-source/profile
structures with one query-wide variable bijection, one global
query-pattern blank-node bijection and one disjoint CONSTRUCT-template bijection.
Top-level SELECT result names/order and every other discriminant, scalar, vector
order, duplicate, dataset/base and option state remain exact. It must not infer
generated variables from names or AST shapes because implicit GROUP/DESCRIBE
aliases can match authored forms. This conservative comparison never weakens
exact replay, and its receipt grants no admission or cache authority.

Immediately after parse and before canonical rendering, an iterative traversal
enforces algebra node, depth, collection and retained-payload limits. Project-
owned plan construction then reserves finite branch, node, nesting and payload
capacity before growth. `PlanMeasureV1` is a final consistency audit and a
prospective cost for an unavoidable later copy; it cannot retroactively admit
construction already performed.

Candidate constants exist in Rust but are provisional and deliberately
unwired. Implementation must calibrate them jointly with serving fuel against
the frozen conformance/application corpus and adversarial boundary fixtures,
then freeze them in source and this ADR. Activation must also explicitly resolve
the current 1 MiB HTTP query limit versus the private 256 KiB scanner ceiling;
the 256 KiB–1 MiB band must not silently change from accepted input to `429`.

### 3. Reserve deterministic scheduled work at the operation boundary

An internal `CompileMeter` wraps the request's `QueryControl`. Work model V1 is
the checked sum of versioned logical units reserved immediately before each
corresponding operation on the governed path, without refund:

- admitted input/frame bytes and actual in-process controlled-parser operations,
  if a future profile selects them, not diagnostic lexical counters; the selected isolated V1 charges
  exact parent-observable launch, protocol and iterative-decode operations while
  fixed child resource ceilings belong to a separate containment envelope, not
  a fictitious worst-case `CompilerWork` debit;
- algebra nodes and retained payload bytes traversed;
- mapping and ontology candidates inspected;
- branch/product candidates considered, precharged with checked arithmetic
  before reserving or constructing their output;
- expression, term and rewrite-rule comparisons;
- nodes visited in every normalization/cascade round, including unsuccessful
  fixpoint rounds;
- bounded canonical cache-key fragments produced and cache probes performed;
- plan-build nodes, collection slots and owned payload retained; and
- every unavoidable recursive graph copy, synchronous eviction and destruction.

Every potentially super-linear loop charges at its loop boundary, and every
bulk allocation or clone charges its deterministically computed prospective
amount first. Cache insertion, nested-subplan rollback, parser/algebra/IQ/Plan
copies and every failed terminal fixpoint round are included in the operation
audit. Counters never depend on hash iteration order, addresses, thread
scheduling, wall time or whether tracing is enabled. An implementation may
batch adjacent unit charges only when the checked total and exact failure point
are equivalent to individual charging.

The algebra validation traversal, plan audit and a subsequent copy are distinct
operations and each consumes its own units; that is not double charging. The
same concrete operation is never charged twice. After activation, any charge-
schedule or envelope-constant change requires a new compile-profile identity.
An inability to allocate the bounded measurement work stack terminates the
request as redacted `CompilerResourceExhausted`; it is not misreported as an
input envelope violation or accounting overflow. Canonical key production must
be bounded and fallible; unmetered recursive `Query::to_string()` is not
acceptable on the governed path.

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
    ) -> Result<Arc<Plan>>;
}
```

This is initially the only public API that may claim governance of a complete
query because it owns raw admission, authoritative parse, keying and
translation. A
public controlled API accepting an already parsed `Query` would omit parser
work and must not carry the same claim. Existing `compile`, `translate*` and
`parse_and_translate*` APIs remain source-compatible for raw, diagnostic,
oracle and conformance use, but are documented as explicitly uncontrolled.
Their semantic results remain the oracle for governed-path equivalence.

The successful parser boundary returns a private admitted-query witness required
by the governed key and translation path. Project-owned traversals share one
`CompileContext` containing the same `CompileMeter`, checkpoint at bounded
intervals and propagate `sf_sparql::Error::QueryControl` without embedding
query, mapping, schema or driver text.

### 5. Separate governed cache authority and charge the hot path honestly

The implemented cache key includes `CompileProfileId` in addition to the
existing `CompileScope`, structural hash and collision-resolving canonical
content. Commits `19dc567` and `4b3c73d` physically separate raw and governed
caches, so raw compilation cannot seed, hit or evict governed state. Existing binding capacity
remains the aggregate: `N` raw entries and zero dormant governed entries. A future governed constructor must
take explicit raw/governed capacities, checked-add their aggregate, and control
recursive plan eviction/drop; partitioning alone grants no governed authority.

An initial isolated-parser cache may use admitted source bytes plus immutable
compile configuration/scope to locate a candidate bucket, but that index grants
no hit: collision resolution still requires exact validated AST/wire identity.
Because a fresh parse regenerates internal identities, stable hits across parses
require the separately versioned, scope-aware alpha-canonicalizer above. Until
that exists, an independently reparsed raw AST/wire mismatch is a cache miss,
not evidence of semantic divergence and not permission to skip parsing.

The activated profile uses physically separate governed and uncontrolled cache
capacity. Governed values carry their final plan measurement:

```rust
struct CachedPlan {
    scope: CompileScope,
    profile: CompileProfileId,
    plan: Arc<Plan>,
    measure: PlanMeasureV1,
}
```

Serving and execution propagate `Arc<Plan>` so a hit performs no recursive Plan
copy and a miss constructs the Plan once, inserts one shallow handle and returns
another. Scope/profile/witness validation precedes use. Key rendering, hashing,
probe and each shallow handle operation have fixed versioned charges. A legacy
path that genuinely requires a deep copy must reserve the stored prospective
clone work through a non-duplicable operation immediately before exactly one
copy; failed reservation cannot seed or consume a governed entry.

Governed insertion also reserves or otherwise contains every synchronous
eviction and final recursive destruction using the stored measurement. If the
cache implementation cannot expose or prospectively bound that operation, it
must be replaced for the governed profile. Physical separation prevents raw
entries from imposing capacity or destruction work on a governed request.
Cache insertion never creates governance authority.

A hot request may consequently pass with a compiler-work limit that would reject
the same cold request. This is intentional scheduled-work accounting. Both paths
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
compiler measurement allocation failure is a redacted `503 service-overloaded`;
a deadline observed first is `DeadlineExceeded`; and the existing representable
expired-handoff rule still returns public `504` even when an earlier internal
resource cause remains recorded. Cancellation and accounting overflow stay
internal failures.

## Implementation gates

Implementation proceeds as bounded, independently reviewable Rust slices:

1. **Core identity — foundation implemented:** the fourth counter/error/limit
   and explicit terminal semantics are present; final whole-path acceptance
   evidence remains a promotion gate.
2. **Parser/envelope — real-parser starter evidence; profile still unqualified:**
   provisional raw lexical, direct-IRI and algebra/Plan measurements exist.
   ADR-0053 holds and descriptor-launches one ELF under rlimits, a stage-one
   filter, one deadline, cumulative nonblocking-pipe caps and deterministic
   termination/reap. Its first-statement dispatcher recognizes exact private
   tuples before Clap/thread-pool startup and requires raw-empty Linux `environ`.
   The evidence seam correlates a bounded GNU build ID, repairs/verifies the
   post-exec envelope, checks candidate profile/limits, self-probes a default-kill
   candidate and completes the exact handshake. The normal parser-free exchange
   and all ten closed mutant outcomes prove structural/preallocation checks,
   terminal/deadline/trailing-output precedence, exact cap and cap+1 behavior,
   post-EOF request validation, whole-life byte accounting and clean recovery.
   The matrix and source capture are recorded by `e55fccd`, `ce5487e`, `d103438`
   and `fa9d977` as evidence, not shipment. `spargebra =0.4.6` is exact-pinned and
   evidence requires `x86_64-unknown-linux-gnu`; normal loader/Rust startup still
   precedes dispatch. Fresh real parses now travel over `QueryV1` for the sealed
   seven-case starter corpus with exact replay plus direct
   alpha equivalence after reap. The policy/dependency profile remains
   unqualified and adds no credential boundary. Complete qualification and the
   full corpus before any witness, governed cache, serving or admission.
3. **Owned compiler work — five fan-out/rollback sites plus lowering propagation:** the private
   work-mode seam prospectively measures and charges nested-subplan cascade
   rollback branch forests, FILTER-over-UNION preceding-arm conditions,
   InnerJoin-over-UNION preceding-arm IQ-node collections and conditions, and
   LeftJoin-over-left-UNION preceding-arm scalar right nodes and conditions, plus
   Construction-over-UNION substitution/projection fan-out. The same mode now
   reaches nested SubPlans and `EXISTS`; its `B-1` borrowed-clone schedule moves
   the original condition into the final branch and retains charges completed
   before a later rejection.
   Exact `N`/`N-1`, nested-condition, allocation-identity, raw-equivalence and
   pre-mutation tests cover those operations. Completed earlier operations and
   charges are not rolled back when a later recursive operation fails.
   Inner-join and atom-resolution candidate/source-copy accounting are public;
   instrument path-specific mapping loops, other products, merge/right copies and
   normalization/cascade, canonical content, remaining hidden recursive copies
   and plan construction; reserve before work and prove whole-path governed/raw
   semantic equivalence.
4. **Cache — partial:** key-level and physical profile separation, a zero-capacity
   dormant governed shard, `Arc<Plan>` storage and serving propagation are
   present without changing the serving binding's `N`-entry aggregate bound.
   Add the admitted witness, explicit calibrated governed capacity, governed
   writer activation, stored final measurement and eviction/drop control.
5. **Serving — partial:** decoded input charging and owned clone metering now
   use the request control in ordinary/security misses, preflight and federation.
   Exact-limit/cancellation/public-result tests pass; hits share plans without
   clone replay, clone failures cannot populate caches, and preflight retains its permit
   across source acquisition. Parser/build/resolve/remaining work stays open;
   `GovernedV1` still requires the parser, owned-work and cache gates to pass.
6. **Claims:** update capability and operational documentation only after all
   relevant gates pass; keep this ADR proposed until its constants and work
   model receive explicit maintainer acceptance.

## Acceptance gates

- Core tests cover `0`, exact `N`, `N+1`, checked overflow, concurrent consumers
  and every sticky first-cause pairing across all four dimensions.
- Raw-scanner tests record false-positive and false-negative drift without
  granting rejection authority. ADR-0053 proves isolated parser boundaries
  for contextual angles, Unicode-created syntax, implicit joins, BASE/PREFIX,
  collections, property lists, reification, RDF-star, `CONSTRUCT WHERE`, deep
  failure and recursive destruction.
- `QueryV1` tests separate exact encode/decode/re-encode replay from fresh
  reparse equivalence. Fresh differentials bind identical source/profile,
  preserve every ordered structure/option and exact top-level SELECT schema, and
  use one query-wide variable map, one global pattern blank-node map and one
  separate CONSTRUCT-template map. The private typed comparator covers this
  structural core. The sealed seven-case starter now provides paired fresh-worker
  outcomes, exact per-side replay and alpha equivalence; broaden it to the full
  grammar/adversarial corpus and persist replayable receipts that reject split,
  merged, partial and cross-domain renamings. Generated-name heuristics are
  inadmissible.
- Iterative algebra tests cover node, depth, collection and payload boundaries,
  fallible work-stack growth and exact pre-item charging in one pass.
- Compiler tests cover mapping fan-out, `JOIN`/`OPTIONAL`/`MINUS` products,
  normalization/cascade fixpoints and every recursive Branch/IQ/Plan copy, and
  prove rejection occurs before each guarded allocation, mutation or clone.
  Prior completed operations remain charged; whole-compilation failure discards
  its local Plan rather than promising transactional rollback of intermediate work.
- Cache tests pin cold and hot V1 work independently, allow the intentionally
  lower hot budget, reject raw-to-governed reuse, prove raw churn cannot evict
  governed state, contain eviction/drop work, and prove semantic identity.
- Barrier-controlled deadline and disconnect tests prove eventual owned-phase
  worker and permit release without timing-only sleep assertions. They do not
  convert the parser limitation below into a one-second claim.
- HTTP tests prove a pre-source compiler excess performs no backend I/O and
  returns redacted `429`, while an expired representable handoff remains `504`.
- Persisted-corpus fuzzing and generated parser/algebra/compiler properties run
  continuously under ADR-0012; a bounded smoke alone is not acceptance. Full
  format, Clippy, build, workspace-test, conformance and frozen-corpus gates pass.
- The finite serving default and all V1 constants are justified by replayable
  benchmark evidence; an adversarial review finds no uncharged super-linear
  owned loop or cross-profile cache path.

Only then may documentation claim: “Serving enforces a fixed raw-input boundary,
bounded parser process containment, post-parse algebra admission, prospective
plan-construction bounds, and deterministic parent-observable and owned
logical-work accounting across governed cold and cache-hit paths.”

## Known blocker and nonclaims

`spargebra` 0.4.6 parses without `QueryControl`, globally decodes Unicode and
uses recursive PEG productions and allocation-heavy semantic actions. The raw
scanner and call-boundary checkpoints cannot bound construction or destruction.
ADR-0053 now has private launch, bounded pipe I/O, cleanup, a control-policy
candidate, parser-free mutation evidence and sealed-corpus real parsing into
`QueryV1`. Inner wire replay and a fresh direct alpha comparison pass for that
starter corpus. Complete parser/dependency/syscall qualification, comprehensive
paired-corpus receipts, cancellation linkage and an opaque witness remain
blockers for `compile_controlled` and every parser-inclusive boundedness claim.
A post-parse-only mode would not be whole-compiler governance.

This ADR does not claim exact CPU seconds, wall time or heap bytes; database
rows scanned or recursive SQL iterations; source-cost governance; raw or
conformance governance; PostgreSQL/MySQL native cancellation; fairness or
bounded waiting; atomic post-`200` delivery; total M2 closure; or production
admission. It does not change query semantics or authorize a second compiler.

## Consequences

- Good: adversarial compiler work gains a deterministic per-request ceiling in
  the existing governance identity, with exact reproducible boundary tests.
- Good: a fixed structural envelope and physically isolated governed cache
  prevent fuel or raw cache churn from weakening the hard safety boundary.
- Good: the current compiler, `Plan`, bindings and Rust serving architecture are
  extended rather than rewritten.
- Bad: the constructor and public `ServeOptions` shape break source compatibility
  deliberately before 1.0, requiring explicit migration of all call sites.
- Bad: cache warmth can change work-budget admission even though semantics and
  the fixed envelope do not change.
- Bad: the selected isolation boundary carries a Rust worker and versioned-wire
  cost; any future in-process profile would carry broad parser/PEG fork and
  hook-audit maintenance.

## More information

- [ADR-0007 — SPARQL-to-SQL rewriting strategy](ADR-0007-sparql-to-sql-rewriting-strategy.md)
- [ADR-0010 — Security and resource governance](ADR-0010-security-and-resource-governance.md)
- [ADR-0012 — Test strategy](ADR-0012-test-strategy.md)
- [ADR-0023 — Query IR architecture](ADR-0023-query-ir-architecture-flat-ucq-vs-iq-tree.md)
- [ADR-0038 — SOTA application-completion programme](ADR-0038-sota-application-completion-programme.md)
- [ADR-0048 — Rust production and Node evidence runtime boundary](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md)
- [ADR-0053 — Grammar-coupled parser governance and isolation fallback](ADR-0053-grammar-coupled-sparql-parser-governance-and-process-isolation-fallback.md)
- [Application-completion programme](../plans/sota-application-completion-programme.md)
