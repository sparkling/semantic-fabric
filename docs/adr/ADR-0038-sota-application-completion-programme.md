---
status: accepted
date: 2026-08-26
updated: 2026-08-29
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
correctness suite, runtime selector paths for three relational backends, and
credible bounded-streaming and Ontop comparison evidence. Under this ADR's R3,
zero backends are yet production-admitted: a reachable runtime path is not an
admission decision. It is not yet a complete application.
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
not claim implementation: the reusable-harness prerequisite is complete, the M0
foundation remains in progress, and M1–M7 remain gated by executable evidence.

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

Native subscription invocations have no project-imposed provider-dollar spend
ceiling. `subscriptionCostUsd: 0` records zero marginal provider-API charge in
the receipt/routing ledger; it is neither a budget or cap nor a claim of
unlimited subscription capacity. Task, turn, wall-clock, output, concurrency,
first-party rate-limit backoff, retry/repair, resource, and receipt limits remain
operational safety controls. The programme must not set or consume provider API
keys. Native-provider exhaustion or unavailability fails the affected gate
closed and never authorizes an indirect fallback. Product `QueryBudget` cost
limits govern source work and result resources, not model spend. Persisting a
verified engineering outcome does not activate the separate retrieval-policy
flywheel; it remains off without explicit opt-in and its own accepted promotion
transaction.

Accepted ADR status means the decision is adopted; it does not imply the code is
implemented. Each affected ADR must gain a dated implementation-status note as
its increment lands. ADR-0014's previously deferred areas graduate into this
programme, but this ADR does not itself mark them implemented.

### 7. Implementation status (2026-08-29)

The H0a reusable-harness foundation is implemented in `b40dbc6`: frozen v5 gate
law and task/runtime derivation, strict schema dispatch, an externally anchored
policy fingerprint, and authoritative protected-input bindings. H0b is
implemented in `7a1fa24`: the evaluator recomputes the frozen seven dimensions,
the scorer admits only a canonical digest-consistent receipt and awards each
dimension all-or-zero, and the strict v5 envelope requires an external anchor
while v4 replay remains frozen. H0c now has an explicit immutable fresh-ID v5
operator, but not a general/default promotion path. Historical runs `_03` and
`_04` failed honestly at final review. Run
`programme_v5_h0c_20260828_05`, under controller `4b0756f`, then completed a
passing candidate transaction after one patch-admission repair. It proved the
final build, verifier, mutation, QE, protected-input, and dual-native-review
evidence, but the frozen V1 reliability law rejected the programme at 85/100
because an attempt that never reached build had no prior build command.
Provider-free replay verified receipt
`91bfc845961c84237ff7f9ea58e75bc77c76d21913f042e05e77c68bc7ab98d1`, envelope
`4c0897695d9a865ff636088be806b4ce5074d3e3552a15038c58435b0017e05f`, and replay
receipt `ca2bbbb3a909f846b0a443ce21572680a2fc9e930a0cb30d294cf2b4af178cfc`.

The additive schema V6 remedy is implemented through `c8e3f68`, with a V2 outer
policy, full candidate transaction/native sidecar evidence, transition-derived
prior-attempt build semantics, trusted execution, and provider-free replay. It
does not rewrite product architecture or application goals and cannot
reinterpret historical V4/V5 evidence. V6 run
`programme_v6_h0c_20260828_01` failed closed on native-origin policy and replay
verified that failure. It remains immutable historical evidence.

Fresh V6 run `programme_v6_h0c_20260828_02`, under controller `c8e3f68`, then
passed the transaction and every programme gate at 100/100 with six bound
commands, seven native-evidence digests, two final native reviews, and no retry
or repair. Its policy, candidate evidence, receipt, acceptance, envelope,
execution claim, and provider-free replay receipt are
`e71107e522342e1b19206c88d861549d00f9df87f27ed4293b0cc36139b2ae34`,
`a1dc307130c6d3efb42354e0c464fcc66095d016564d5227e6259dc71998ac7f`,
`d9d244ef42c4a914b4b2bec52844b1ddc58a46d1b99759453cde7b34a5940216`,
`1cf36b1d9bcb2c4e6d2c81525baead859b2b62ed9042bfd33991bed8367430f7`,
`02c30ed3bb8f0b0b5a5c10320d64308934ee8438df051626e68e661589939a06`,
`578799eff72dd84cc3b5601754654d1bce33a7b4b1fa56666271fe6833979c86`,
and `f1bcf0fe0720d2851dff219cf9b27563bcf6ff4da317ac9ec259b1fcd505bf02`.
H0c is complete. M0 is now in progress through the verified product/evidence
slices listed below:
`93ae3c2` tracks the root lockfile and freezes dependency resolution;
`374ca99` pins the reviewed CI action, service-image and selected tool inputs;
`1c9bb61` adds a fail-closed, per-file-digested RDB2RDF inventory for 1 suite
manifest, 26 scenarios, 87 exact cases and 189 case-tree files; and `401c1bb`
adds proposed subordinate ADR-0039/0040 while extending the harness-protected
ADR boundary. `31a1164` removes floating readiness execution by installing
MetaHarness 0.3.0 and Darwin 0.2.8 from the committed npm integrity graph and
invoking only local binaries. `a3efb32` makes the SQLite and PostgreSQL runners
consume the sealed inventory in canonical order, rejects count-neutral
per-identity outcome drift, distinguishes local untested PostgreSQL from
CI-required provider failure, and treats missing or malformed sealed inputs as
fatal. `33e202b` introduced the ordered 87-case SQLite outcome baseline;
`a1a6dc9` generates 74 exact capability/backend cells (38 implemented, 34
planned, 2 unsupported, and zero production-admitted); and `81caec2` replaces
the v1 outcome representation with an immutable-snapshot receipt whose
typed status/cause records, bounded parser, read-only production replay, and
atomic generator have mutation coverage. That receipt explicitly does not
attest runner, lockfile, or toolchain provenance. `4ff81b3` runs the inventory,
receipt, and generated-claim checks read-only in CI, protects their complete
authority closure in the controller, and proves generic MetaHarness-fit scores
cannot decide programme acceptance. Proposed ADR status is deliberate: neither
packaging nor federation is accepted or implemented by writing its design lock.

Commit `a84aa05` generalizes that authority to backend-aware v3 receipts and
adds the durable required-live PostgreSQL baseline: 80 pass, one documented
deviation, and six exact skips across all 87 ordered cases. Its receipt, outcome,
and inventory digests are respectively
`c04e6f86f5330ac0534d9c62de6a0cb1cfa12ad3bf1249960a1052ebabc30f52`,
`63eb3bdcd9e7ca172edbb3ef271af8a69d0a4099af0be25a4f95f6a0d4fcb2ac`, and
`4d2eb56e25920c4b8488b47d971cf887902b4e4178257420bf3a7ea44c504c96`.
Required-live replay fails on provider absence, CI checks but cannot regenerate,
and the receipt attests sealed mapping inputs/outcomes—not runner, toolchain,
host, provider, Query/Protocol conformance, or production admission. The SQLite
receipt is regenerated as the same v3 contract with unchanged outcomes.

The current M0 tranche also adds per-test expected SQLite query and Protocol
regression baselines. Those receipts do not attest W3C SPARQL Query/Protocol
conformance, runtime provenance, or backend admission. The default `sf-cli`
dependency receipt closes package resolution, enabled features, and normal/build
dependency edges only; it does not attest binary bytes, build-script output,
linker or system provenance, an SBOM, reproducibility, or production admission.
The tranche now also provides a fail-closed contract and tool for recording and
verifying a **host-observed non-closure observation** of a freshly built current
`sf-cli` executable. CI validates that bounded parser/contract only: it does not
capture, commit, upload, or promote an observation from the mutable hosted
runner. The first external observation was captured from clean commit `5a06eac`
and replayed on 2026-08-28. Its private `0600` receipt binds 363 raw inputs to
357 canonical terminal inputs and three one-hop HostSystem alias records, with
portable/host/receipt digests
`72ce37b4320e5126ef32a693eab80f2f30df3757881c6e2495e8882068079b9a`,
`024fbbbde94471f15a15e65193b1a0c66767f5590b2621f8d9c7b896381a3ad8`, and
`173d0698e7955da881a39574bb5a08d302b80b67fc3e89a95c27d282270e51ca`.
It remains uncommitted, unpublished, noncanonical, and outside
the observed source tree to avoid receipt self-reference.
Authority paths now require root/effective-UID ownership and reject writable
ancestors except root-owned sticky proper ancestors; held directory identities
detect persistent replacement at phase boundaries. The linker-only alias
authority binds alias topology and terminal bytes without weakening generic
symlink rejection, and the structured GNU-note parser binds build-ID owner,
type, declared size, and digest.
The producer still trusts UID 0 and its effective UID and requires that builder
principal to be exclusive and quiescent: resistance to same-principal/root ABA
replacement is an explicit nonclaim, not evidence supplied by this tooling.

An additive runtime-linkage contract canonicalizes strict, bounded glibc
`ld.so --list` output. Commit `863a058` added a private Linux holder for one
discovered input set: descriptor-relative `openat2`/`NO_XDEV` walks, guarded mount
roots, one canonical loader alias, twice-verified source bytes copied into exactly
sealed close-on-exec memfds, canonical identity/order/budgets, ELF roles,
interpreter/SONAME facts, and static `DT_NEEDED` provider equality/reachability.
Start/end fences detect persistent artifact, mount, alias and source replacement.

Commit `c8305c3` adds a private one-shot prepared executor. It independently
requires an exact bubblewrap path, SHA-256, byte length and executable-policy ID,
holds the root-owned non-capability-bearing inode through a guarded descriptor
root, and verifies it before and after execution. Exact CLOEXEC duplicates of the
sealed artifact, loader and DSO sources are identity-, seal-, length-, digest- and
byte-checked at every phase. The child executes only the held bubblewrap inode via
`execveat(AT_EMPTY_PATH)` with an empty environment, `no_new_privs`, dump/core and
per-process resource limits, parent-death signalling, an exact data-FD allowlist,
pidfd/direct-child and process-group cleanup, timeouts, and bounded cancellable
stdout/stderr capture.

Bubblewrap receives no host bind, `/proc`, `/dev` or writable host path. It creates
a fresh size-bounded tmpfs, drops capabilities, unshares user/network/all other
namespaces, copies only the sealed-source bytes into fixed paths, remounts the root
read-only, and invokes the copied loader as `ld.so --inhibit-cache
--glibc-hwcaps-mask "" --list /artifact`. Strict parser output must equal the
candidate view before an in-memory observation is returned. Focused process,
descriptor, policy, phase-fence and substitution mutants pass. The manual
workflow-dispatch lane additionally names the exact test, release-profile binary,
host labels and bubblewrap digest; it is private diagnostic validation, not a
merge, performance, admission or release gate.

Commit `805f413` converts only a completed prepared observation into a distinct
private canonical non-admission record. Fixed `authority=none`,
`admission-result=not-evaluated`, and `non-admission-only` metadata plus 34
`not-attested` fields prevent the record from being read as authority. Separate
domain-separated digests bind the record and receipt bytes; the canonical format
binds the exact semantic view, bubblewrap path/digest/length/policy, ordered
source and destination identities, modes, and bounded raw stdout in lowercase-
hex chunks. Provider-free semantic replay reparses that stdout and requires the
new view to equal the recorded view. It performs no process, filesystem,
network, model, or provider call, and it never recreates the live probe event.
The exact-host workflow renders, parses, and replays this record in memory only.

Commit `9282e60` checks that live path against a caller-supplied expectation for
the closed runtime-ELF dynamic-tag/search/flag policy without adding a second
allowlist or reparsing the immutable object bytes.
Only after every role-specific object and the static `DT_NEEDED` graph validate,
the holder records policy ID
`elf64-le-x86_64-closed-dynamic-tags-safe-search-flags-v1` and the exact five-
source implementation digest
`cd23f2d883c1e99b655395284e7d803e6d00b9eaf90a417560efca7ffde50b0a`.
The exact native diagnostic maintains a separate literal for that pair; the API
authenticates no reviewer and accepts caller-provided expectations. Drift rejects
before probe construction and is checked again during the immediate pre-run
validation phase. The actual pair remains private in-memory holder/observation
metadata and is not serialized. Receipt V1's schema and canonical serialization
are unchanged and continue to record
`runtime-elf-policy-replay=not-attested`.

Commit `73e9864` adds exact late syscall confinement for the prepared observation:
policy `x86_64-prepared-loader-late-cbpf-default-kill-v1` is 55 classic-BPF
instructions (440 bytes), with SHA-256
`0092c69f902c071515f2f82c5aff75bf63f065148f1c0fb51af414787338e80a`.
The holder rechecks a separately sealed, close-on-exec high-numbered policy FD
before and after transfer and requires exactly one `--seccomp <fd>` immediately
before `--`. Bubblewrap installs that late filter for both its namespace PID 1
reaper and the copied loader child. Unknown and forbidden syscalls kill the
process; the narrow allowlist constrains loader I/O, read-only file opening,
non-W+X mapping, PID 1 eventfd signalling, and loader stdout. A native identical-
layout `fstat` control succeeds while a `socket` canary dies by `SIGSYS`, and the
private live observation binds the policy identity. Receipt V1 remains byte-
compatible and deliberately records
`target-seccomp-or-syscall-trace=not-attested`: it contains neither a syscall
trace nor a final-FD inventory and does not replay the live policy proof.

On 2026-08-29, commit `50adc0a` added a pre-construction check over the exact
bounded bubblewrap bytes used for the authorized length and SHA-256: those same
bytes must parse as `RootPie` under the existing expected runtime-ELF policy,
and the held inode, digest, and policy are fenced around the native observation
and canary. Both exact native controls pass. This is static preflight only:
Receipt V1 is unchanged and non-attesting, and bubblewrap host runtime closure
remains unbound.

On 2026-08-29, commit `b34b6d7` added a separate canonical private
`authority=none` inventory under relation
`counterfactual-controlled-name-resolution-not-actual-exec`. The interpreter
path and sorted direct `DT_NEEDED` names derive from the held `RootPie` bytes;
an environment-cleared `LC_ALL=C` process at `/` uses `--inhibit-cache`, an empty
`--glibc-hwcaps-mask`, and `--list <authorized-bwrap-path>`. Pre/post checks revalidate the held bwrap identity,
runtime-ELF policy, and fixed process plan. A domain-separated inventory digest binds that identity/policy metadata,
bounded raw stdout and replayed SONAME/path bindings. The interpreter, reported DSOs, and path-passed target are
unheld and undigested; bwrap is not executed, and provider-free replay proves
only record self-consistency. Receipt V1 remains byte-compatible and unchanged.

This is loader-resolution observation only. Candidate discovery remains prior and
unauthorized; the artifact is not executed, and main-program, relocation,
symbol/version, initialization, `dlopen`, NSS, VDSO and closure completeness are
unproven. The digest detects any byte drift in five embedded policy sources; it
does not classify a change, prove review, or bind compiled bytes, configuration,
dependencies or toolchain. It also does not prove the opaque GNU-property, hash,
symbol, relocation, version, TLS or cross-table payloads outside complete
semantic validation. Bubblewrap copies from sealed
memfds, so the loader does not consume the original held inode capabilities.
Although counterfactual names and paths are now inventoried, the interpreter,
DSOs, and target-path bytes actually consumed by a bwrap launch remain unbound,
as do time-of-use, default cache/hwcaps equivalence, preload and LSM state; the
kernel, bubblewrap, glibc, copy and mount semantics remain trusted. The exact late filter is live observation only: there
is no post-exec final-FD inventory or syscall trace, and Receipt V1 does not
attest it. There is no aggregate cgroup `pids.max`, memory or control-group kill;
the other implemented limits are per process. Same-principal/root ABA, rollback and a
hostile kernel/filesystem remain out of scope under the exclusive, quiescent
builder assumption. The module has no production caller, durable writer/importer,
signature, external witness, authenticated execution/output provenance, SBOM,
reproducibility, minimality, admission, performance or release authority. Its
provider-free semantic replay validates a self-consistent record without
executing anything; a fully reminted self-consistent alternative remains only a
different unauthenticated `authority=none` record.

This observation tooling advances M0 without closing the actual binary-artifact
boundary. It records configured tool identities and a post-build final-link-
dependency-file snapshot with hashes of its listed, mapped inputs; it does not
attest the complete execution closure of any tool or sole configured-linker
authorship of that dependency file. Complete tool-execution, build-script-input,
system and `strace`-grade closure; production-grade collector/containment and
runtime-completeness authority; durable authenticated receipt publication; SBOM
and release provenance; independent reproducibility; the proposed minimal
production artifact and backend admission; and controlled performance evidence
all remain open.
The observation also explicitly leaves linker time-of-use and link-path race
resistance unattested; it is empirical current-artifact evidence, not gates 1–3
of proposed ADR-0039.
Performance production and comparison machinery exists, but no controlled
runner profile, baseline, candidate, or measured numbers exist. Commit
`f2cc800` makes future V5/V6 patch tasks consume exact tracked `Cargo.lock` bytes
from their attested ancestor baseline while retaining an exact task-blob-bound
exception for the two historical fixtures. It does not create measurement
authority. Proposed
[ADR-0041](ADR-0041-manifest-bound-controlled-observational-evidence-capture.md)
keeps the clean-release baseline in a sibling single-attempt observational
transaction rather than weakening V6 patch semantics. A dedicated controlled
runner/profile, capture receipt, replay, and imported baseline remain open.

M0 is not complete until actual complete binary artifact closure, SBOM and
reproducibility/minimality/admission evidence, and the controlled performance
baseline have been performed. Exact commit `ad94cdb` did replay the then-current deterministic
tranche byte-identically in two clean, no-hard-link checkouts under the hardened-builder
`umask 0022`: each rebuilt the controller, passed all 91 harness files, replayed
every current deterministic authority, and remained Git-clean. An initial
`umask 0002` pair was correctly rejected for group-writable inputs, and no trust
check was relaxed. This proves current-tranche repeatability, not actual binary
reproducibility; the complete M0 release gate must run again in two clean
builders after the remaining authorities exist. The generated matrix,
SQLite/PostgreSQL v3 mapping receipts, query/protocol regression receipts, and
package dependency receipt are deterministic scoped authorities, not release or
backend-admission proof. M1–M7 remain gated. These incremental closures do not by
themselves rescore the 44/100 application-readiness baseline.

The pass measured roughly 35 GiB of ephemeral isolated verifier outputs before
successful cleanup. Future harness optimisation may use immutable
content-addressed build reuse and digest manifests, but only with separate
writable lane targets and unchanged isolation, independence, fail-closed cache
validation, evidence binding, and replay semantics.

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
