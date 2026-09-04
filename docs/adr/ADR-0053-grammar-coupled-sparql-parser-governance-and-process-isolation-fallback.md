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
A dormant parent-side supervisor foundation and the fixed-size `Hello`/`Ready`
handshake codec are implemented. The supervisor holds the opened current ELF,
records a bounded observed SHA-256 fingerprint, launches a test-only fixture by
that exact descriptor under a stage-one Linux x86-64 policy, and owns required
pidfd/process-group termination and reap. Its parent pipe ends are nonblocking,
cumulatively byte-capped and share the immutable spawn deadline. It is private
and unreachable from the product CLI. No private worker dispatch at the first
user-code statement, before Clap/application thread-pool initialization;
post-exec final-policy verification; final default-deny policy; `Ready`
exchange; bounded query-protocol IPC; parser invocation; `QueryV1` result wire;
admitted-query witness; independent release/runtime attestation;
concurrency-permit integration; or serving binding exists. These foundations do
not enable `CompileProfileId::GovernedV1`, change serving, or change any
capability status or admission.

The raw lexical scanner, parser-view direct-IRI measurement, fallible post-parse
algebra validator, bounded cache-key writer, exact clone roots, `CompileContext`
and Plan measurement remain private development foundations. Five dormant
fan-out/rollback sites prospectively meter nested-subplan rollback branch
forests, FILTER-over-UNION preceding-arm conditions, InnerJoin-over-UNION
preceding-arm IQ-node collections and conditions, and
LeftJoin-over-left-UNION preceding-arm scalar right nodes and conditions, plus
Construction-over-UNION substitution/projection fan-out. `CompilerWorkMode`
now propagates through lowering, including nested SubPlans and `EXISTS`, with
the tested preceding-`B-1` clone/final-owner schedule and retained completed
charges. All public compiler paths select uncontrolled mode. These primitives
may produce calibration and adversarial evidence, but none is parser admission
authority; remaining recursive-copy sites are still unmetered.

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

V1 uses a Linux x86-64-only process-isolation profile. A prepared parent holds
the same opened `sf-cli` executable, records its observed identity and SHA-256,
launches it by descriptor, and invokes a private parser-worker mode. Worker
dispatch must be the first user-code statement, before Clap or application
thread-pool initialization. Normal dynamic-loader and Rust runtime startup
necessarily precede it; this is not a zero-runtime or zero-allocation startup
claim. The worker reads no untrusted IPC until it has installed and verified its
final default-deny policy. Starting with the same held binary keeps the worker inside
the Rust/Cargo product boundary and prevents a mutable path from choosing
another executable. The observed fingerprint is diagnostic continuity
evidence, not release authority, executable authentication or attestation of
the dynamic runtime closure; those remain ADR-0039 release gates. Other targets
remain buildable but return `UnsupportedPlatform` before launch, so this profile
fails closed rather than becoming a weaker fallback.

The dormant parent foundation opens `/proc/self/exe` once, validates and hashes
that bounded regular ELF through the held descriptor, and never reopens a
derived path. Its child setup uses only prebuilt POD/C-string state and raw or
async-signal-safe operations. It applies exact hard and soft rlimits, an empty
environment, a new process group, parent-death signal, no-new-privileges,
non-dumpable pre-exec state, a filled signal mask and close-on-exec descriptor
allowlisting. The executable is duplicated to one exact descriptor at or above
the post-setup `RLIMIT_NOFILE` ceiling. A stage-one seccomp policy permits only
the one `execveat(AT_EMPTY_PATH)` using that descriptor and its exact static
empty-path pointer, and denies descendant creation, process-group escape and
limit mutation. The launch descriptor closes on successful exec and cannot be
recreated below the enforced descriptor ceiling.

Both parent pipe ends are `O_NONBLOCK`. Prospective cumulative input/output
caps reject an operation before I/O; fixed-buffer loops handle partial work,
`EINTR`, `EAGAIN`, hangup and error under the one deadline created before spawn.
Worker exit wins a pending write, while a read first drains pipe bytes already
buffered at exit. Each live-process I/O error enters a containment path that
tries process-group and exact-worker signaling before reap; if every signaling
route fails, it surfaces that containment error rather than blocking on an
uncontained wait. Tests prove limit, stall, truncation, closed-pipe and
next-launch recovery. The
parent contract requires ignored `SIGPIPE` so a raced close becomes a contained
`EPIPE`, not process termination. `max_input_bytes` and `max_output_bytes` count
all bytes in their direction for the whole worker lifetime, including future
`Hello`/`Ready` and every later frame header. They are not raw-query or payload
allowances: the final query ceiling needs framing headroom and calibration.
These primitives exchange no protocol bytes.

That stage-one policy is deliberately default-allow and is not a general
sandbox. Its blanket `clone`/`clone3` denial makes the future worker
single-threaded. After exec, a future worker must verify/reset inherited state,
install its final allowlist policy, verify the child-observed kernel limits and
acknowledge the exact parent-owned contract values before `Ready`; it must
consume no untrusted bytes before that transition. Executing a
new image may reset dumpability, so the parent-side setting is not a post-exec
claim. Those worker-side controls do not yet exist.

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
private `AdmittedQuery`; the current dormant handshake/supervisor foundation cannot.

`QueryV1` separates exact replay from fresh-reparse equivalence. Exact replay
requires a worker wire to decode to the exact encoded AST and re-encode to the
same canonical wire. A separate pinned-parser invocation cannot use raw AST or
wire equality as its oracle: `spargebra` creates random internal variables and
anonymous blank-node identifiers, so equal source may parse to unequal raw
identities. The independent differential must instead use a versioned,
scope-aware alpha mapping and form-specific oracles: SELECT, ASK and DESCRIBE
preserve their dataset/base and pattern semantics; CONSTRUCT additionally
preserves template scope and blank-node relationships. User-named variables and
source-spelled blank nodes remain bound by their query scopes and cannot be
treated as unconstrained generated identities.

The first governed cache may use admitted source plus immutable compile
configuration/scope only to find candidates. A hit still requires exact
validated AST/wire collision equality. Stable reuse across fresh parses needs a
separately versioned scope-aware alpha canonicalizer; until it exists, regenerated
identity is an honest miss and never authority to bypass parsing or validation.

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
2. **Dormant handshake and parent-supervisor foundations implemented:** fixed
   framing, magic, version, nonce, build/parser profile and exact contract-value
   acknowledgement are canonical. Only the rlimit fields are child-observed;
   wall time, cumulative direction bytes and concurrency are parent-owned, while
   descendant denial enforces the process count. The private Linux x86-64
   launcher holds the executable descriptor, installs exact parent-side pre-exec controls, requires
   a pidfd and owns deterministic process-group kill/reap. Parent pipe ends are
   nonblocking; prospective cumulative direction caps, fixed-buffer partial and
   `EINTR`/`EAGAIN`/hangup/error handling share the immutable spawn deadline.
   Live-process I/O errors enter the termination/reap containment path under the
   required ignored-`SIGPIPE` contract. Test-only fixture
   seams prove held identity, limit validation, environment/descriptor closure,
   stage-one spawn/group/exec denial, canonical wall timeout, pipe-limit/failure
   containment, reap and clean next
   launch. This grants no production worker, query-wire or admitted-witness
   authority.
3. **Incomplete:** dispatch the private worker at the first user-code statement,
   before Clap/application thread-pool initialization, and prove its post-exec
   verification, signal reset, final default-deny syscall policy and verified
   controls before `Ready`. Qualify descendant prevention without relying on
   per-user `RLIMIT_NPROC` as a per-worker boundary. No untrusted bytes may be consumed
   under the stage-one/default-allow gap.
4. **Partial:** parent I/O tests inject stalls, truncation, closed pipes and
   cumulative-limit rejection, proving attempted kill/reap and successful next
   launch. Add cancellation, panic, abort, stack/address-space exhaustion,
   malformed/trailing protocol output and forced death; prove no
   PID/FD/permit leak and a successful next request after each.
5. Define a complete flat index-based `QueryV1` wire and iterative fallible
   encoder/decoder. Prove exact decode/re-encode replay plus frame, input, output,
   count, index and aggregate-byte `0`, exact `N` and `N+1` rejection before
   allocation or access.
6. Differentially prove decoded `Query` semantics and syntax outcomes against
   a fresh direct pinned parse using the versioned scope-aware alpha mapping and
   SELECT/ASK/DESCRIBE/CONSTRUCT-specific oracles over application, W3C, Unicode
   and adversarial corpora, including contextual angles, implicit joins, long
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
- Good: the private dormant parent now has descriptor-exact launch,
  deadline/cumulative-cap nonblocking pipe primitives and deterministic
  pidfd/process-group cleanup without exposing an incomplete worker mode or
  widening the product API.
- Cost: a fresh worker adds launch/IPC latency, a bounded wire protocol and
  Linux-specific operating-system qualification.
- Cost: the flat wire must explicitly cover the full admitted `Query` algebra;
  generic recursive serialization is intentionally unavailable.
- Neutral: diagnostic scanner and direct-IRI measurements remain useful for
  differential evidence but grant no product capability.

## Nonclaims

This decision does not claim an accessible production worker, complete
containment or a general syscall sandbox. In particular, it does not claim a
private worker dispatch at the first user-code statement, before
Clap/application thread-pool initialization; post-exec final-policy verification;
final default-deny policy; `Ready` exchange; bounded query-protocol IPC; parser
invocation; `QueryV1` wire; admitted witness; independent release/runtime
attestation; concurrency-permit integration; or serving activation. The current
fingerprint does not authenticate a release or its dynamic closure; pre-exec dumpability is
not asserted after exec; and pidfd acquisition assumes integration excludes a
competing wait-any reaper or hostile `SIGCHLD` mutation. Kernel uninterruptible
sleep can still delay reap. This ADR also does not claim exact CPU time or heap
bytes, governance of raw/conformance APIs, complete owned compiler-phase
accounting, database/recursive SQL work, source-native cancellation, atomic
post-`200` delivery, backend admission, total M2 completion or production
readiness.

## More information

- [ADR-0004 — Oxigraph crates as the RDF/SPARQL substrate](ADR-0004-oxigraph-rdf-sparql-substrate.md)
- [ADR-0010 — Security and resource governance](ADR-0010-security-and-resource-governance.md)
- [ADR-0012 — Test strategy](ADR-0012-test-strategy.md)
- [ADR-0038 — SOTA application-completion programme](ADR-0038-sota-application-completion-programme.md)
- [ADR-0048 — Rust production and Node evidence boundary](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md)
- [ADR-0052 — SPARQL compilation safety and work accounting](ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md)
