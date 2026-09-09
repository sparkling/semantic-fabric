---
status: proposed
date: 2026-09-04
updated: 2026-09-09
tags: [sparql, parser, resource-governance, isolation, rust, dos]
supersedes: []
depends-on: [ADR-0004, ADR-0010, ADR-0012, ADR-0038, ADR-0048, ADR-0052]
implements: [ADR-0010, ADR-0052]
---

# Grammar-coupled SPARQL parser governance and process-isolation fallback

## Status boundary

**2026-09-08 correction (refined 2026-09-09):** ADR-0055 requires repair of a reproduced authenticated parser abort. Public serving now reuses a prepared Rust worker, request-control-aware waits, exact reap and bounded QueryV1 decode without parent source reparse; missing embedding runtime fails closed. Runtime omits only diagnostic full-file SHA and its 512 MiB scan ceiling: bounded ELF/build-ID reads, held identity and pre-launch metadata checks remain. A sparse oversized ELF regression retains evidence-mode rejection and runtime drift rejection; debug sections cannot independently disable serving. Required local ordinary/lineage input, cap-one exact recovery, cancellation and scope tests cover this slice. This ADR remains proposed: GovernedV1, complete syscall/dependency/ELF attestation and its broader corpus are not promoted. The following evidence history describes the earlier private-only state.

This **proposed** ADR selects bounded Linux Rust process isolation for V1 after a source audit; complete in-process hooks require a broad maintained fork. A private supervisor validates and holds one ELF before source preparation, records a bounded full-file SHA-256 diagnostic, descriptor-launches it under a stage-one `x86_64-unknown-linux-gnu` policy, owns pidfd/process-group termination and exact reap, and caps nonblocking pipes under one immutable deadline. First-statement `sf-cli::main` dispatch recognizes only exact raw-empty-environment private tuples: the normal control-only parser peer, a selector-free parser-free synthetic peer, and separately feature-gated parser-free mutant and real-parser `QueryV1` peers.

The normal synthetic exchange sends a prepared 96-byte request header plus source after `Hello`/`Ready`, then closes stdin. The child stack-preflights structure/body limits before its sole complete request allocation, requires exact EOF, and only then validates nonce → source digest → UTF-8 and writes a correlated 128-byte result header plus an independent static 100-byte empty-ASK `QueryV1`. The parent stack-preflights the result and prospectively caps its body before one complete allocation, reads exact body and EOF, establishes pidfd waitability, sweeps the process group, exactly reaps and requires success, then replays/correlates the request, checks digests, decodes/directly re-encodes and compares static bytes. Only unit escapes from either hidden transport-evidence seam.

Commits `e55fccd` and `ce5487e` add the closed same-executable mutant child and parent evidence, `d103438` adds both new modules to the development-harness source inventory, and `fa9d977` adds live request-EOF ordering evidence. They are integrated repository evidence, not shipped or release-qualified functionality. One exact two-byte big-endian directive before `Hello` selects ten mutants: wrong nonce, source digest, payload digest or self-consistent invalid `QueryV1` exit zero and fail only post-reap; status 78 and deadline dominate a complete wrongly correlated result at 412 accepted output bytes; and an extra trailing byte fails the EOF probe before semantic checks while accepted output remains 412.

The remaining mutants bind boundaries precisely. Exact-cap output accepts `Ready` 184 + result header 128 + body 8,388,608 = 8,388,920 bytes and then fails invalid `QueryV1` post-reap. A cap-plus-one declaration is rejected prospectively after 312 accepted bytes and before body allocation. Injected request-frame allocation refusal means zero result bytes after the required 184-byte `Ready`; stdout EOF and exact reap then expose raw status 78. The provisional whole-life input cap is 1,048,856 bytes: normal source ceiling 1,048,576, mutant ceiling 1,048,574 because the directive consumes two bytes. `RLIMIT_FSIZE` remains independently 67,108,864 bytes.

Live bad nonce, source-digest and UTF-8 requests remain alive and silent beyond `Ready` while stdin stays open; only after exact EOF do they close output and raw-exit 78. Every mutant and malformed-request probe permits a clean next launch. Commit `235084d` adds a separate real-parser peer over the sealed seven-case starter corpus: six fresh children return parser-produced `QueryV1`, one returns a fixed syntax rejection, and the parent accepts only after EOF, exact reap/correlation, exact per-side replay and fresh-direct alpha equivalence. Only aggregate counts escape. The parser policy/profile and dependency/syscall closure remain unqualified; no witness, cache authority, attestation, admission, permit, serving, release or `CompileProfileId::GovernedV1` authority exists. No credential, namespace, LSM or privilege-transition boundary is claimed; production activation remains later.

Commit `38e9c7a` closes the raw directive proof gap with internally fixed zero-, one-, and three-byte and unknown-`u16` cases: each emits zero bytes, raw-exits 78, is exactly reaped and permits recovery on the same held descriptor. Commit `5a9919b` makes the ten-mutant matrix use one preparation/fingerprint and one held descriptor, preserving a fresh child and all controls per case while reducing the focused all-feature run from 192.31 to 24.24 seconds. The default-only dependency receipt refreshed at `949cf11` remains baseline evidence. Commit `1e320a2` adds a parser-free, `authority=none` qualification-input receipt for 368 packages, 382 target/host contexts and 1,042 edges, binding the lock, workspace manifests, reported tool versions, exact target cfg and root-specific feature tree. It excludes source/build/proc-macro/artifact bytes, ambient Cargo configuration, transient races, runtime linkage and syscalls, so it is not a governed parser profile.

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
next-launch recovery. The parent requires ignored `SIGPIPE`, so a raced close is a contained
`EPIPE`, not process termination. `max_input_bytes` and `max_output_bytes` count
all bytes in their direction for the whole worker lifetime, including future
`Hello`/`Ready` and every later frame header. They are not raw-query or payload
allowances: the final query ceiling needs framing headroom and calibration. The
normal parser peer remains control-only. Separately named, feature-gated
parser-free normal and mutant peers use the same held executable and lifecycle;
the normal API has no selector, while the mutant tuple consumes one closed
two-byte directive before `Hello`.

The private `ParseRequestV1` header is exactly 96 bytes: magic `SFPREQ01`,
protocol version, request kind, zero flags, header length, source length,
handshake nonce, SHA-256 source digest, UTF-8 encoding tag, QueryV1 version and
zero reserved bytes. The private `ParseResultV1` header is exactly 128 bytes:
magic `SFPRES01`, protocol version, success/rejection kind, zero flags, header
length, payload length, the correlated nonce and source digest, SHA-256 payload
digest, QueryV1 version, closed rejection code and zero reserved bytes. All
integers are big-endian. Success requires a non-empty canonical QueryV1 payload;
rejection requires an empty payload, zero QueryV1 version and exactly one of
`Syntax`, `QueryEnvelope` or `ResourceExhausted`. Nonce/digest mismatch,
unknown values, nonzero flags/reserved bytes, truncation, trailing bytes or a
noncanonical QueryV1 payload fails closed.

The candidate limits bind exact whole-life totals: 1,048,856 cumulative input
bytes and 8,388,920 cumulative output bytes. Normal input is `184 + 96 +
1,048,576`; mutant input is `2 + 184 + 96 + 1,048,574`. Maximum output is
`184 + 128 + 8,388,608`. The independent 67,108,864-byte `RLIMIT_FSIZE` does
not stand in for pipe accounting. A trailing byte is emitted but rejected and
not counted as accepted output. A cap-plus-one result is rejected from its
stack header at 312 accepted bytes before any body allocation. These values
remain provisional and must not silently reduce the public query ceiling.

The eventual one-shot terminal contract is stricter than a valid prefix. EOF is
checked before semantics; child waitability, group sweep, exact reap and success
precede request replay or result interpretation. Consequently trailing output,
deadline and nonzero exit dominate correlation/payload errors. Exit-zero
correlation/payload faults are classified only post-reap. Any failure discards
the result and mints no witness.

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
child work that might not occur. Before qualification execution, exact
executable identity and target, a complete statically resolved Cargo
dependency/feature closure, containment limits, parent charge schedule and wire
version must form a control-ready qualification envelope. That envelope permits
only the separately named evidence peer; it is not a parser-qualified syscall
profile and grants no admission or serving authority. A bounded observation may
derive only a candidate post-policy parser-workload syscall-argument profile;
loader/Rust startup and dynamic-runtime closure require separate held-runtime
evidence. Only independent fresh-corpus replay under the immutable default-deny
profile can qualify it for the later governed compile profile and activation.

The eventual qualified child will parse once and return a bounded flat, index-based `QueryV1` wire. The
parent never accepts SPARQL/SSE text that would require reparsing. Frame lengths,
counts, indices and aggregate bytes are validated before allocation; decoding is
iterative and fallible, so generic recursive Serde is not an admissible shortcut.
A successful decode and post-parse algebra validation may eventually mint a
private `AdmittedQuery`; neither the control handshake nor parser-free normal or
mutant evidence can do so.

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
input bytes. These constants and bytes remain provisional until parser transport
integration, corpus/fuzz calibration and profile binding are complete.

`QueryV1` separates exact replay from fresh-reparse equivalence. Exact replay
requires a parser-produced worker wire to decode to the exact encoded AST and re-encode to the
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

1. **Partial; static qualification inputs implemented:** the workspace exact-pins
   `spargebra =0.4.6`; commit `1e320a2` binds the root-specific locked Cargo
   package/feature/context graph and exact GNU target cfg. Complete source,
   build/proc-macro, standard-library, allocator, artifact and dynamic-runtime
   closure remain outside that `authority=none` receipt; no sealed SPARQL syntax
   corpus with a grammar-family coverage manifest exists yet.
2. **Handshake and parent-supervisor control foundations implemented:** fixed
   framing, magic, version, nonce, build/parser profile and exact contract-value
   acknowledgement are canonical. Only the rlimit fields are child-observed;
   wall time, cumulative direction bytes and concurrency are parent-owned, while
   descendant denial enforces the process count. The private Linux x86-64
   launcher holds the executable descriptor, installs exact parent-side pre-exec controls, requires
   a pidfd and owns deterministic process-group kill/reap. Parent pipe ends are
   nonblocking; prospective cumulative direction caps, fixed-buffer partial and
   `EINTR`/`EAGAIN`/hangup/error handling share the immutable spawn deadline.
   Live-process I/O errors enter termination/reap containment under ignored
   `SIGPIPE`. Fixture and non-default seams prove held identity, limit checks,
   environment/descriptor closure, stage-one spawn/group/exec denial, deadline,
   pipe-failure containment, handshake/EOF/reap and clean recovery. The normal
   synthetic exchange and ten closed same-executable mutants additionally prove
   exact whole-life accounting; terminal/deadline/trailing precedence; cap and
   cap+1 behavior; request-allocation refusal after `Ready`; post-EOF nonce,
   digest and UTF-8 validation; and a clean next launch. `e55fccd`, `ce5487e`,
   `d103438` and `fa9d977` record that integrated evidence, not shipment. The
   bounded GNU build-ID and full-file SHA remain diagnostic. A separate sealed
   starter-corpus peer now returns real parser-produced `QueryV1` evidence, but
   this grants no production parser worker, qualified profile or witness.
3. **Partial; control-ready candidate evidenced:** the hidden private-entry discriminator is the first user-code
   statement before Clap/application thread-pool initialization. Exact two-token
   routing, malformed-reserved rejection, raw-empty-`environ` enforcement, ordinary
   CLI fallthrough and silent status 78 via raw Unix `_exit` are tested, including
   a malformed raw environment entry that Rust's iterator filters. The prepared
   evidence path verifies/repairs the enumerated post-exec control envelope,
   installs/self-probes a TSYNC default-kill policy candidate before reading
   `Hello`, and emits independently derived `Ready`. Descendant prevention does
   not rely on per-user `RLIMIT_NPROC`. Gate 3 qualifies launch identity, target,
   framing, ceilings, lifecycle and parent-observable accounting only; it does
   not require parser execution or claim a complete parser syscall profile. The
   static closure and qualification envelope must still be completed. The
   parser-free normal/mutant peers grant no parser, corpus, profile, witness,
   cache, release or admission authority.
4. **Partial:** parent I/O tests inject stalls, truncation, closed pipes and
   cumulative-limit rejection. The ten-mutant matrix adds output corruption,
   nonzero/deadline/trailing precedence, exact cap/cap+1, request-allocation
   refusal, exact reap and clean normal recovery. Add cancellation, panic, abort,
   stack/address-space exhaustion and broader forced-death/leak evidence.
5. **Partial; real-parser starter-corpus wire evidenced:** the fixed flat index-based
   `QueryV1` tag/schema table, fallible iterative encoder/reconstructor,
   allocation-free structural/scalar preflight, canonical ownership proof and
   exact decode/re-encode replay are covered by focused tests over every
   pinned query/algebra/function/aggregate family plus malformed headers,
   ranges, indices, sharing, scalars and component bounds. Exact private
   96-byte request and 128-byte result codecs add focused golden, mutation,
   correlation, redaction and raw-cap `0`/`N`/`N+1` tests. Parser-free normal and
   mutant peers exercise the exact transaction and stated boundary cases,
   including one request-frame allocation refusal. Persisted fuzz/property and
   broader allocation-failure evidence remain. **5a — bounded observation:** a
   separately named, feature-gated peer parses the sealed seven-case starter
   corpus inside the Rust envelope, one fresh child per case, and returns only
   bounded aggregate outcomes. **5b — parser wire evidence:** the same starter
   inputs run in fresh children and return real parser-produced
   `QueryV1` or a fixed rejection, then requires post-reap correlation, exact
   per-side replay and source/profile-bound fresh-direct alpha equivalence.
   Complete grammar coverage, persisted receipts, policy replay, loader/runtime
   closure and cancellation evidence remain. This grants no `AdmittedQuery`,
   cache, permit, witness, readiness, release, serving or `GovernedV1` authority.
6. **Partial; typed comparator foundation implemented:** eight focused tests
   cover correlation/outcome classification, bounded fallible traversal, exact
   ordered top-level SELECT outputs, one global variable bijection, one global
   query-pattern blank-node bijection and a disjoint CONSTRUCT-template
   bijection without generated-name heuristics. The starter Gate 5b corpus now
   compares decoded worker semantics and syntax outcomes with a fresh direct
   pinned parse bound to the same source/profile. Extend this proof using one global
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

Until every applicable governed-profile gate passes, ADR-0052 remains proposed;
public isolated parsing does not promote the existing uncontrolled cache profile.

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
containment or a general syscall sandbox. The normal control peer proves the
enumerated post-exec observations/repairs, candidate-policy probe, handshake and
cleanup; the parser-free normal/mutant peers additionally prove transport and
failure-order behavior only. None proves an accepted parser policy,
parser-syscall completeness or a complete governed dependency profile;
UID/GID/groups/capability/namespace/LSM confinement, complete parser corpus and
policy replay, loader/Rust-startup syscall tracing, production parser execution,
an admitted witness, independent release/runtime attestation, permit integration
or serving activation. Gate 5 peers execute only as bounded evidence after Gate
3; production activation remains later. The private codecs
and mutant evidence grant none of those authorities. The
full-file fingerprint and bounded GNU build-ID digest do not authenticate a release or its
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
