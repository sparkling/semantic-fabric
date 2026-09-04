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
A private parent-side supervisor, fixed-size `Hello`/`Ready` codec and
control-only evidence exchange are implemented. The supervisor holds one opened
ELF, records a bounded full-file SHA-256 diagnostic, launches that exact
descriptor under a stage-one `x86_64-unknown-linux-gnu` policy, and owns pidfd/process-group
termination and reap. Its nonblocking parent pipes are cumulatively byte-capped
under the immutable spawn deadline. A hidden Rust dispatcher is the literal
first statement of `sf-cli::main`, before Clap or application thread-pool
initialization. It byte-compares only the exact reserved `argv[0]`/`argv[1]`
tuple, rejects malformed reserved-position invocations, and requires the raw
Linux `environ` vector—not Rust's filtered iterator—to be empty. A non-default
Rust evidence seam can launch a prepared worker, correlate independently
observed bounded GNU build-ID digests, repair and verify its post-exec control
envelope, install and self-probe a default-kill control-ready policy candidate,
and complete exact `Hello`/`Ready`/EOF. Malformed or unprepared reserved
invocations still exit silently with status 78 via raw Unix `_exit`; a prepared
evidence exchange can exit 0. The partial dependency/profile digest and policy
remain unqualified candidates. A private pure-Rust `QueryV1` codec is now
implemented but is not connected to either process: its fixed 32-byte header
and records, explicit tag table and provisional record/edge/scalar/wire caps
feed an allocation-free borrowed-wire preflight, exact postorder-tree and scalar
validation, fallible iterative reconstruction, post-decode algebra validation
and byte-exact canonical replay. Private 96-byte `ParseRequestV1` and 128-byte
`ParseResultV1` outer codecs now bind exact kinds, lengths, nonce, source and
payload digests, encoding/wire versions, zero flags/reserved fields and closed
outcomes. Independent raw-frame caps run before header access; rejection
encoding is allocation-free and carries no parser text. Twenty-two focused
inner-wire tests and seventeen outer-frame tests pass. These establish dormant
codec foundations only: no request/result transport, parser execution,
fresh-parse alpha oracle, worker-produced wire, admitted-query witness,
independent release/runtime attestation, permit integration or serving exists. No UID/GID,
supplementary-group, capability, namespace, LSM or privilege-transition
qualification is claimed.
These foundations do not enable `CompileProfileId::GovernedV1`, change serving,
or change capability status or admission.

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

The 2026-09-04 source audit and workspace manifest exact-pin the dependency as
`spargebra =0.4.6`,
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

The runtime closure is wider than those four crates. `SparqlParser::new()`
constructs randomized standard collections, while aggregate and anonymous-node
generation uses `rand`/`rand_chacha`/`getrandom`. On the pinned GNU toolchain the
observed forms include a `GRND_INSECURE` standard-library seed, a zero-length
flags-zero probe and bounded flags-zero seed reads. The final policy must bind
the complete resolved feature graph, Rust standard library, allocator, GNU libc
and loader, admit only corpus-qualified `getrandom` argument forms, and keep any
`/dev/urandom` fallback fail-closed. This observation does not yet qualify a
syscall policy.

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

V1 selects one exact 64-bit `x86_64-unknown-linux-gnu` process-isolation
profile. The parser-isolation and evidence compile gates now require that exact
target triple; musl or another ABI requires an independently qualified profile.
This closes target selection, not GNU/glibc, toolchain, dependency, syscall,
loader or release qualification. A prepared parent holds the same
opened `sf-cli` executable, records its observed identity and SHA-256,
launches it by descriptor, and invokes a private parser-worker mode. Worker
dispatch must be the first user-code statement, before Clap or application
thread-pool initialization. Normal dynamic-loader and Rust runtime startup
necessarily precede it; this is not a zero-runtime or zero-allocation startup
claim. The worker reads no peer bytes until it has installed and locally
distinguished its default-kill control-ready policy candidate. Starting with the same held binary keeps the worker inside
the Rust/Cargo product boundary and prevents a mutable path from choosing
another executable. The parent's full-file fingerprint and the bounded GNU
build-ID digest used by the handshake are diagnostic continuity/correlation
evidence, not release authority, executable authentication or dynamic-runtime-
closure attestation; those remain ADR-0039 release gates. Other targets
remain buildable but return `UnsupportedPlatform` before launch, so this profile
fails closed rather than becoming a weaker fallback.

The private parent foundation opens one regular ELF, validates and hashes its
bounded full bytes through the held descriptor, and never reopens a derived
path. It separately requires exactly one bounded GNU build ID; the worker
observes the same form through `/proc/self/exe` before its policy-candidate
transition. This build-ID comparison neither rehashes the entire image in each
child nor authenticates provenance or dynamic dependencies. Child setup uses
only prebuilt POD/C-string state and raw or
async-signal-safe operations. It applies exact hard and soft rlimits, an empty
environment, a new process group, parent-death signal, no-new-privileges,
non-dumpable pre-exec state, a filled signal mask and close-on-exec descriptor
allowlisting. The executable is duplicated to one exact descriptor at or above
the post-setup `RLIMIT_NOFILE` ceiling. A stage-one seccomp policy permits only
the one `execveat(AT_EMPTY_PATH)` using that descriptor and its exact static
empty-path pointer, and denies descendant creation, process-group escape and
limit mutation. The original launch descriptor closes on successful exec. The
worker intentionally opens `/proc/self/exe` temporarily at a low descriptor to
observe its bounded GNU build ID, then closes that descriptor before installing
the final policy; no stronger descriptor-non-recreation claim is made.

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
allowances: the final query ceiling needs framing headroom and calibration. The
evidence seam exchanges one exact `Hello`/`Ready` pair and then requires stdout
EOF; the private query/result codecs are defined but no transport path accepts
or emits them.

The dormant `ParseRequestV1` header is exactly 96 bytes: magic `SFPREQ01`,
protocol version, request kind, zero flags, header length, source length,
handshake nonce, SHA-256 source digest, UTF-8 encoding tag, QueryV1 version and
zero reserved bytes. The dormant `ParseResultV1` header is exactly 128 bytes:
magic `SFPRES01`, protocol version, success/rejection kind, zero flags, header
length, payload length, the correlated nonce and source digest, SHA-256 payload
digest, QueryV1 version, closed rejection code and zero reserved bytes. All
integers are big-endian. Success requires a non-empty canonical QueryV1 payload;
rejection requires an empty payload, zero QueryV1 version and exactly one of
`Syntax`, `QueryEnvelope` or `ResourceExhausted`. Nonce/digest mismatch,
unknown values, nonzero flags/reserved bytes, truncation, trailing bytes or a
noncanonical QueryV1 payload fails closed.

With the existing 184-byte handshake, a full 1 MiB source needs at least
1,048,856 cumulative input bytes (`184 + 96 + 1,048,576`), so the current
1,048,576-byte candidate input cap cannot carry its stated source maximum. A
full 8 MiB result needs at least 8,388,920 cumulative output bytes
(`184 + 128 + 8,388,608`). Worker integration must bind those direction totals
and define whether the one-byte EOF/trailing-output probe consumes budget; it
must not silently reduce the public query ceiling to hide framing overhead.

The eventual one-shot terminal contract is stricter than receipt of a valid
prefix: the parent accepts exactly one canonical result frame only after stdout
EOF and successful child exit/reap. Trailing bytes, a valid frame followed by a
panic or signal, a nonzero exit, truncation, nonce/digest mismatch or malformed
framing discard the entire result and mint no witness.

That stage-one policy is deliberately default-allow and is not a general
sandbox. Its blanket `clone`/`clone3` denial preserves the single-threaded
construction of worker entry. After exec and before reading `Hello`, worker
entry closes unintended descriptors; verifies stdin/stdout pipes, `/dev/null`
stderr, PID/TID/group/parent, parent-death signal, no-new-privileges, filter
mode, exact rlimits and stage-one mutation denial; repairs/verifies signal
dispositions, pending set, alternate stack and empty mask; restores umask and
non-dumpability; observes its bounded GNU build ID; then installs with TSYNC and
self-probes the default-kill control-ready policy candidate. `Ready` is derived
from that build ID, a locally pinned partial profile candidate, local limit
constants and the parent nonce. Only rlimits are kernel-observed; wall,
direction-byte and concurrency values are local candidate constants matched to
the parent frame. This does not qualify the candidate for parser workloads or
establish a credential, namespace or LSM boundary.

Each parse uses a fresh child. Before announcing readiness, the session
establishes fixed stack, address-space, CPU, descriptor and descendant-process
controls and closes every unintended inherited descriptor. Pipe-output and
wall/concurrency limits are parent-owned; `RLIMIT_FSIZE` does not bound pipes.
The parent retains
compiler and aggregate permits through deterministic kill and reap. Timeout,
cancellation, panic, abort, malformed output, output overflow or protocol
failure destroys no parent-side recursive parser value, and a subsequent request
must start successfully.

The fixed child resource ceilings are immutable per-request OS caps in a
separately named parser-containment envelope, not an accounting reservation;
refund semantics do not apply. Work model V1 charges only operations the parent can observe and schedule exactly: admitted
input/frame bytes, process launch, protocol frames, iterative validation/decoding
and owned compilation. It does not charge a fictitious worst-case amount for
child work that might not occur. Before activation, exact executable/release
identity, the complete resolved Cargo dependency-and-feature graph,
parser-qualified OS policy, containment limits, parent charge schedule and wire
version must form the governed compile profile. The current four-crate/checksum/
feature digest is only a partial control-ready marker and does not satisfy that
gate.

The child parses once and returns a bounded flat, index-based `QueryV1` wire. The
parent never accepts SPARQL/SSE text that would require reparsing. Frame lengths,
counts, indices and aggregate bytes are validated before allocation; decoding is
iterative and fallible, so generic recursive Serde is not an admissible shortcut.
A successful decode and post-parse algebra validation may eventually mint a
private `AdmittedQuery`; the current control-only handshake cannot.

The implemented inner-wire foundation freezes magic `SFPQW001`, version 1, a
32-byte header, fixed 32-byte records, big-endian `u32` edges and a terminal
scalar section. Provisional maxima are 65,536 records, 131,072 edges, 2 MiB of
scalar bytes, an independent 8 MiB raw-input envelope checked before header
access, and a separate 256-record reconstruction-depth guard. The component
maxima currently imply a tighter 4,718,624-byte canonical structural maximum;
the independent raw envelope rejects oversized arbitrary child output and
remains authoritative if later profile-bound component limits change. The
existing tighter algebra depth is rechecked after reconstruction.
Every tag has one exact flags/fields/arity shape. Records form one canonical
postorder tree: the root is last, child roots precede owners, reverse traversal
visits every record exactly once, edge/scalar ranges are contiguous and fully
consumed, and `NO_INDEX` is legal only for `VALUES` `UNDEF`. Borrowed UTF-8,
IRI, variable, blank-node, datatype and language-tag validation precedes
input-sized decode allocation. Recursive `Box` nodes use a reviewed fallible
global-allocator helper; successful reconstruction must re-encode to the exact
input bytes. These constants and bytes remain provisional until transport
integration, corpus/fuzz calibration and profile binding are complete.

`QueryV1` separates exact replay from fresh-reparse equivalence. Exact replay
requires a worker wire to decode to the exact encoded AST and re-encode to the
same canonical wire. A separate pinned-parser invocation cannot use raw AST or
wire equality as its oracle: `spargebra` creates random internal variables and
anonymous blank-node identifiers, so equal source may parse to unequal raw
identities. Fresh comparison is valid only when both sides bind the same source
SHA-256 and complete parser-profile digest, including initial base IRI, prefix
map and custom aggregate-function set. Two syntax rejections may match; parsed
versus rejected fails, and resource, protocol, panic, kill or limit failures
never count as syntax equivalence.

The versioned structural comparator preserves every query/algebra/expression/
path/term discriminant, scalar, vector order, duplicate, dataset/base and option
state exactly, modulo three independent bijections: one global query-wide
variable map, one global query-pattern blank-node map including nested `EXISTS`,
and one separate CONSTRUCT-template blank-node map. Before traversal, top-level
SELECT result variables are fixed to their exact names and order. The global
variable map then remains shared through patterns, subqueries, `EXISTS`,
aggregates and the template so joins and correlations cannot split or merge.
The conservative global pattern blank-node map may reject some equivalent
hand-built ASTs, but cannot hide an identity change in same-source parser output.
CONSTRUCT template and pattern blank nodes never share identity authority,
including `CONSTRUCT WHERE`. No commutative, set, join or order normalization is
permitted. PREFIX spelling/history is erased by parsing and remains bound only
through the source digest.

Complete generated-variable provenance is not structurally recoverable from
`spargebra::Query`: implicit GROUP aliases and DESCRIBE IRI variables can have
the same AST shapes as authored `AS`/`BIND` forms, and users may legally spell
the random hexadecimal name shape. The comparator must not implement an
`is_generated_variable` name or shape heuristic. It alpha-maps every
non-observable variable under the one global bijection while keeping top-level
SELECT output exact. If generated/source provenance later becomes policy
authority, the parser must emit a versioned provenance sidecar or use a
maintained fork and a new governed wire/profile; it does not require an
application-architecture rewrite.

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

1. **Partial:** the workspace manifest exact-pins `spargebra =0.4.6`, and the
   parser/evidence compile gates require `x86_64-unknown-linux-gnu`. Complete the
   parser/PEG/runtime dependency-and-feature closure, exact same-executable
   worker identity, GNU control profile, fixed containment limits and
   parent-observable work schedule.
2. **Handshake and parent-supervisor control foundations implemented:** fixed
   framing, magic, version, nonce, build/parser profile and exact contract-value
   acknowledgement are canonical. Only the rlimit fields are child-observed;
   wall time, cumulative direction bytes and concurrency are parent-owned, while
   descendant denial enforces the process count. The private Linux x86-64
   launcher holds the executable descriptor, installs exact parent-side pre-exec controls, requires
   a pidfd and owns deterministic process-group kill/reap. Parent pipe ends are
   nonblocking; prospective cumulative direction caps, fixed-buffer partial and
   `EINTR`/`EAGAIN`/hangup/error handling share the immutable spawn deadline.
   Live-process I/O errors enter the termination/reap containment path under the
   required ignored-`SIGPIPE` contract. Fixture and non-default evidence seams
   prove held identity, limit validation, environment/descriptor closure,
   stage-one spawn/group/exec denial, canonical wall timeout, pipe-limit/failure
   containment, exact `Hello`/`Ready`/EOF, trailing-output rejection, reap and
   clean next launch. The handshake correlates one bounded GNU build-ID digest;
   the parent's separate full-file SHA remains diagnostic. This grants no
   production worker, parser-qualified policy/profile, query wire or admitted
   witness authority.
3. **Partial; control-ready candidate evidenced:** the hidden private-entry discriminator is the first user-code
   statement before Clap/application thread-pool initialization. Exact two-token
   routing, malformed-reserved rejection, raw-empty-`environ` enforcement, ordinary
   CLI fallthrough and silent status 78 via raw Unix `_exit` are tested, including
   a malformed raw environment entry that Rust's iterator filters. The prepared
   evidence path verifies/repairs the enumerated post-exec control envelope,
   installs/self-probes a TSYNC default-kill policy candidate before reading
   `Hello`, and emits independently derived `Ready`. Descendant prevention does
   not rely on per-user `RLIMIT_NPROC`. The candidate must still be replaced or
   qualified against the complete parser corpus/syscall surface, and the partial
   dependency marker must become a complete governed profile before QueryV1
   integration, profile admission or serving.
4. **Partial:** parent I/O tests inject stalls, truncation, closed pipes and
   cumulative-limit rejection, proving attempted kill/reap and successful next
   launch. Add cancellation, panic, abort, stack/address-space exhaustion,
   malformed protocol output and forced death; trailing control output is now
   contained. Prove no
   PID/FD/permit leak and a successful next request after each.
5. **Partial; inner and outer codecs implemented:** the fixed flat index-based
   `QueryV1` tag/schema table, fallible iterative encoder/reconstructor,
   allocation-free structural/scalar preflight, canonical ownership proof and
   exact decode/re-encode replay are covered by 22 focused tests over every
   pinned query/algebra/function/aggregate family plus malformed headers,
   ranges, indices, sharing, scalars and component bounds. Exact dormant
   96-byte request and 128-byte result codecs add 17 focused golden, mutation,
   correlation, redaction and raw-cap `0`/`N`/`N+1` tests. Add transport-level
   direction `0`/`N`/`N+1`, allocation-failure injection, persisted
   fuzz/property corpora and worker-produced replay; provisional limits are not
   accepted calibration.
6. Differentially prove decoded `Query` semantics and syntax outcomes against
   a fresh direct pinned parse with the same source/profile. Use one global
   variable bijection with exact ordered top-level SELECT outputs, one global
   query-pattern blank-node bijection and a disjoint CONSTRUCT-template
   bijection; do not infer generated-variable provenance from names or AST
   shapes. Cover SELECT/ASK/DESCRIBE/CONSTRUCT over application, W3C, Unicode and
   adversarial corpora, including contextual angles, implicit joins, long
   BASE/PREFIX, collections, property lists, reification, RDF-star,
   `CONSTRUCT WHERE`, deep failure and recursive-drop fixtures. The comparison
   receipt binds source/profile and both exact wire digests but grants no
   admission, cache or serving authority.
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
- Good: the private parent now has descriptor-exact launch,
  deadline/cumulative-cap nonblocking pipe primitives and deterministic
  pidfd/process-group cleanup plus an evidence-only post-exec control check,
  policy-candidate transition and exact control handshake; the hidden dispatcher
  fails closed without exposing a parser service or widening product capability.
- Cost: a fresh worker adds launch/IPC latency, a bounded wire protocol and
  Linux-specific operating-system qualification.
- Cost: the flat wire must explicitly cover the full admitted `Query` algebra;
  generic recursive serialization is intentionally unavailable.
- Neutral: diagnostic scanner and direct-IRI measurements remain useful for
  differential evidence but grant no product capability.

## Nonclaims

This decision does not claim an accessible production parser worker, complete
containment or a general syscall sandbox. The evidence-only control peer proves
only the enumerated post-exec observations/repairs, candidate-policy installation
probe, exact `Hello`/`Ready`/EOF exchange and cleanup. It does not prove an
accepted/final parser policy, parser-syscall completeness, a complete governed
dependency profile, UID/GID/groups/capability/namespace/LSM confinement, query
IPC or transport, parser execution, worker-produced `QueryV1`, a fresh-parse alpha oracle, an
admitted witness, independent release or runtime attestation, permit integration
or serving activation. The dormant inner/outer codecs grant none of those authorities. The full-file
fingerprint and bounded GNU build-ID digest do not authenticate a release or its
dynamic closure; and pidfd acquisition assumes integration excludes a
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
