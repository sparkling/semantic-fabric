---
status: accepted
date: 2026-09-07
updated: 2026-09-08
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

**Updated 2026-09-07:** implement the delivery review's main-only integration, native-builder/model-effort, proportional-check and queued course-correction rules. Public portable equality-row authorization, safe layered configuration and verified remote-source TLS close narrow product boundaries, not general ABAC or operability. A standalone serving-only Cargo build excludes conformance/benchmark crates and extra backend features while preserving SQLite/PostgreSQL/MySQL and all serving controls. Opt-in authored reload now validates and publishes complete generations off-path, immediately fences detected drift, preserves request leases and fixed caller policies, and cannot heal shutdown/worker panic. Required real-CLI evidence covers SQLite and encrypted PostgreSQL/MySQL single/mixed-source reload and invalid-input recovery. Backend DDL leases, policy/configuration hot reload, product completion and the exact release bundle remain open.

This decision is **accepted**. It replaces ADR-0038 as the controlling
definition of product completion and release work for v1. ADR-0038 remains an
auditable record of the broader SOTA programme; its research and advanced-
assurance work becomes a labelled post-1.0 backlog.

**2026-09-08 public Direct lifecycle:** the closed PostgreSQL 16.9/16.15 PK-backed profile is now wired through `serve --direct-mapping-base`, with independent review and required owned-TLS CLI evidence for exact authenticated SELECT/ASK/CONSTRUCT/lineage, traffic-independent drift and successor activation, fixed startup ontology, incompatible DDL rejection/recovery, bounded shutdown and wrong-CA/no-PK rejection. Independent request/control pools retain resolved trust; mandatory five-second observation (nonzero reload interval overrides), 30-second control and 60-second startup bounds are fixed independently of requests. Native candidate and cleanup ownership prevents overlapping retries and late publication; shutdown counts both lanes under the existing drain plus three-second forced allowance. Row-policy profiles, additional sources and other backends remain excluded from this Direct profile, not from authored serving. Other backend-generation guarantees, total compiler/source controls and exact-release qualification remain open; no hard CPU-preemption or production-admission claim is made.

**Lineage update (2026-09-08):** ADR-0017 now has a public opt-in, compiler-proved
constant mapping/source SELECT and CONSTRUCT profiles with per-solution PROV-O,
native graph reification, pinned snapshot/logical-plan/policy identifiers and bounded
fail-terminal streaming. Graph metadata stays in named bundles outside the product
graph, preserving response-wide blank-node identity. Its required
SQLite HTTP, isolation, reload and failure tests plus owned pinned PostgreSQL
16.15/MySQL 8.4.11 serving-only CLI lineage/portable-policy checks are incremental
results, not full lineage, all native-profile or exact-release qualification, or
application completion. Native evidence parses the actual returned metadata and
reification, observes encrypted sessions and rejects unsupported lineage while
a source-table lock remains held; those constant-origin cases do not establish
lineage-specific reload/cancellation.
An additional bounded multi-mapping profile now propagates actual origins through
positive RDF-matched BGP/JOIN/UNION and root projection/dedup/slice, merging late
duplicate witnesses before output. Its finite witness buffer fails on overflow;
it is not an unbounded graph materialization or a candidate-map list. Broader
operators, federation, authorized row keys and native/release qualification remain
open. The capability catalog separately retains that incomplete release gate;
ADR-0017 records the precise additional profile and required commands. Twelve
required pinned native multi-map SELECT/CONSTRUCT cases now prove exact-target
TLS/native stop under deadline/disconnect/forced SIGTERM, held-lock and unrelated
sibling isolation, cap-one recovery and fail-terminal completion. This does not
qualify lineage UNION/JOIN cancellation, source-RLS, reload or every operator.
The separate bounded two-source UNION lineage profile now binds actual origins to
their source under shared snapshot/security/budget and native cleanup owners.
Required HTTP and owned TLS CLI checks cover its bags, entailed source affinity,
source-scoped blank nodes, portable callers, pinned activation, limits and native
deadline/disconnect/forced-shutdown failures. ADR-0017 records the exact profile;
the following join slice adds coverage; row-key authority and broader qualification
remain separate.
The existing bounded two-source join now emits actual origins for both mandatory
contributors, using compiler-sealed source/map pairs and its existing capped
pre-200 executor. Explicit map-to-source links disambiguate identical authored IDs;
hidden keys, filtered rows and empty joins acquire no inferred row provenance.
Required HTTP tests cover bags, limits, policy, activation and recovery; the pinned
TLS CLI aggregate adds both-order exact bags and twelve join-lineage native
deadline/disconnect/forced-shutdown cases. The required owned TLS aggregate now
also qualifies authored lineage reload: constant/multi-map SELECT/CONSTRUCT,
mixed UNION and nonempty both-order join results carry changed actual mapping
documents; invalid input fences new queries while native-held requests complete
with pre-invalid results/provenance, and repaired input restores readiness.
Held-query cases are multi-map SELECT on both providers and mixed UNION/forward
join on PostgreSQL, not every graph/operator/order. This closes that reload evidence
slice. Required owned PostgreSQL public-router tests now also qualify source-RLS
lineage: exact A/B/A constant/multi-map SELECT/CONSTRUCT and federated UNION/join,
actual source/map proof, empty results, concurrent caller isolation and clean cap-one
pool reuse. Constant-lineage body-drop, policy-error and deadline cases fail terminally
and recover with isolated caller state. ADR-0017/0018 retain the precise scope;
these are not every failure permutation, full historical lineage or exact-release gates.
The aggregate gate concerns coverage of declared v1 paths, not every historical
ADR-0017 combination. Row-key transport is conditional on explicitly declared or
verified authority: ADR-0017 permits mapping/source-only lineage when no authorized
stable key exists. Tested wider operator exclusions are not promoted to v1 gates.
**Profile qualification (2026-09-08):** the recorded public/native lineage,
lifecycle, reload and source-RLS evidence closes the declared lineage-profile gate.
`l-lineage` is now non-blocking; `query-lineage-generic` remains the incomplete,
non-advertisable historical broader profile. This changes no supported shape,
backend or conditional row-key authority. Query-budget, backend-admission and
exact-release gates remain separately blocking.
The catalog validator keeps implementation state separate from release scope:
planned work requires a documented limitation and architecture-plan evidence,
but not a fabricated release blocker. Planned capabilities cannot be advertised;
required checks and production-admission guards are unchanged.

**Compiler-input update (2026-09-08):** public compilation now precharges decoded
UTF-8 bytes for lineage preparation, preflight and authoritative compilation,
including authenticated cache hits, under the same cumulative request budget.
Required HTTP/unit tests cover exact bounds, security precedence, no first-pass
source admission and two-pass permit recovery. This closes a reproduced zero-budget
bypass, not total compiler CPU/catalog-growth governance; `l-query-budget` remains
blocking. The later parser-lifetime correction below leaves governed-cache admission dormant.
Public tree compilation now also carries existing owned normalization/lowering/
nested-cascade clone metering through ordinary/security misses, preflight and
bounded federation. Public `EXISTS` tests prove input-only work cannot fund tree
cloning and sufficient work preserves the exact bag; cache hits avoid clone replay.
Operation-local measurement limits grant no whole-plan admission authority.
Required exact-bound, cancellation, no-cache-on-clone-failure and partition checks
protect this wiring. Tree inner-join lowering now precharges checked candidate
products and exact left-branch copies, with between-pair cancellation. Authenticated
HTTP tests prove pre-source rejection and all 64 exact VALUES tuples on success;
pruned/empty products and inclusive bounds are test-locked. Merge internals/right
copies, parsing/build and other compiler work remain open. Atom resolution now
reserves map/POM visits, graph comparisons/filtering, class/POM products and parent
lookups, and meters actual logical-source copies. Direct lineage retains the same
control alongside its existing eligibility/recipe charges; nested contexts preserve
ownership. Public absent-predicate queries cannot bypass this work by yielding no
branches; sufficient work preserves all six exact triples. Path predicate searches
now also reserve mapping candidates, graph work and exact term/source copies;
negated paths charge complement comparisons/visits and preserve exact duplicate
pairs. Required pre-source rejection, cancellation, cache and raw-equivalence checks
pass. Graph inventory/reflexive checks now reserve visits, graph comparisons and
copies, and enumeration slots on the same control identity. Empty named-graph
answers cannot bypass mapping work; required HTTP/preflight tests prove pre-source
rejection and permit recovery, with exact paid `+`, `*` and `?` results. Shape
construction, TBox/unifier internals and other payload copies/phases remain
unqualified; the total-work gate stays open.

**Parser-lifetime correction (2026-09-08):** an authenticated, sub-ingress-limit
query reproduced a server-process abort. Public compilation now uses a prepared
Rust parser process on Linux x86_64 GNU, including security, lineage, preflight
and bounded federation. The held executable is verified before readiness;
embeddings explicitly supply `ParserRuntime`, or readiness/compilation fail closed.
Exact EOF, process cleanup/reap and bounded canonical QueryV1 validation precede
parent AST ownership; the parent never reparses source text. Request control
interrupts pipe/exit waits while existing compiler/request permits retain ownership.
Required local CLI tests cover nested, Unicode-created, additive and malformed
inputs on ordinary/lineage paths, server survival and cap-one exact-result recovery.
Focused tests cover scope restoration, typed errors and owned cancellation/reap.
This is necessary defect repair under this accepted contract, not promotion of
proposed ADR-0052/0053, GovernedV1, a complete syscall sandbox or release admission.
Full dependency/ELF attestation remains post-1.0; required remaining compiler/source
work controls and exact release qualification remain blockers.

The catalog's wider global-operator and atomic/no-prefix-stream limitations are
non-blocking scope exclusions under this decision: proposed ADR-0040 cannot add
v1 requirements, and a failed bounded prefix is not a complete answer. Existing
admitted-operator exactness, bounds, rejection, terminal-failure, redaction and
cancellation tests remain required; no capability or backend is removed.

Native PostgreSQL/MySQL serving cancellation now holds dirty connections and
request capacity through bounded stop/discard, with acknowledged cleanup before
reuse. Required owned TLS CLI tests observe stopped database work and pool
recovery, including forced SIGTERM during ASK/SELECT/CONSTRUCT on both backends.
Shutdown now preserves the runtime for owned cleanup: the original drain deadline
is followed, only when forced, by a three-second cleanup allowance; exhaustion
is an error, not a clean exit. PostgreSQL generation/RLS regressions remain green.
Required mixed-source UNION/join CLI tests also observe both providers stop under
timeout/disconnect/SIGTERM while server-side lock witnesses remain held, preserve
separate same-credential CLI siblings and recover full cap-one federated results.
ADR-0010/0011 record the endpoint, constructor and shutdown limits. This closes
the pinned native cancellation slice, not wider backend qualification, total
governance, protected generations or exact-artifact release admission.

This is an explicit priority and evidence-scope change, not an implementation
claim. Moving an item to post-1.0 does not make it complete, supported, or
production-admitted. ADR-0002's virtualisation and cross-RDBMS product charter,
ADR-0048's Rust runtime boundary, and the accepted security, governance, and
operability contracts remain in force. ADR-0037 remains the accepted
engineering control-plane design, but its full transaction is no longer a gate
on every v1 commit.

## Context

**Query-profile delta (2026-09-08):** required owned-TLS PostgreSQL 16.15/MySQL 8.4.11 CLI checks now cover one-hop DESCRIBE graph sets, duplicate-edge cycles and complete 258-edge `P+`/`P*` closures. The native path fixture uses decimal-digit VARCHAR keys, not arbitrary native collation/type qualification. SQLite NOCASE path joins reproduced false reachability and lost IRIs; the blanket comparison prototype was rejected for decoding/correlated-path regressions. The integrated repair instead preserves CHARACTER padding and DATE types in explicitly collated authored SQL projections through guarded, prepare-only alias-preserving metadata recovery. Unnamed computed outputs that make recovery ambiguous fail closed. Path-key comparison remains open. Native/correlated key equality, total source controls and exact release remain open; ADR-0049 records the evidence and limitations.

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

The serving build is `cargo build --locked --release -p sf-cli --no-default-features`.
Build only that package: workspace feature unification can re-enable development
backends. The default `development-tools` feature preserves the existing
`conformance`/`bench` CLI for developers; it is excluded from the serving artifact.
`bash scripts/check-serving-profile.sh` checks the root-specific normal/build
graph; `cargo test --locked -p sf-cli --no-default-features` retains public serving
regressions and requires developer-command rejection. Required-live source TLS
also runs with defaults disabled. Dependency graphs and local tests are not an
SBOM, clean-machine smoke, signed release, or backend admission.

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

V1 work uses canonical `main` and exactly one integration writer. Never create,
switch to, or develop in another branch or worktree. Historical recovery refs
are read-only integration inputs, not new execution lanes; preserve them and
all unrelated working changes. Read-only investigation, review and compatible
tests may run concurrently when they shorten the critical path without resource
contention. Native agents execute; a Ruflo record alone does not launch a worker.

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

At a coherent public-feature integration boundary, `main` runs the full locked
workspace format, test, build and strict-Clippy gates plus relevant feature,
live-backend, cache-isolation and cross-source checks. Adjacent micro-commits do
not each rerun that full set without a new reason. Unknown impact or selector
failure requires broader verification; passing focused checks is not release
qualification. Preserve source-bound evidence without repeatedly regenerating
historical receipts merely because HEAD moved.

The immutable release candidate runs every v1 product and minimum-release gate
in this ADR. A per-commit success cannot substitute for the integrated or
release run, and a previous-head result cannot attest a later commit. Pure
documentation status changes use structural, link, line-count, and diff checks;
they do not require an unrelated product rebuild.

Native Codex/Claude agents build through normal edit/test/inspect loops. Ruflo
MCP retains useful coordination and verified outcomes. ADR-0037's closed
candidate evaluator is an optional experiment, not the default builder, a
per-commit gate or a product oracle. Its legacy worktree-creating launchers are
incompatible with the main-only rule and must not run in this programme. Keep
existing isolation checks intact; do not expand or retrofit that harness merely
to deliver v1. Research scores and harness evolution remain post-1.0.

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

1. Resolve interrupted integration, bring verified recovery work onto `main`,
   and freeze the exact v1 profile. Do not start another unintegrated lane.
2. Finish public security enforcement, snapshot reload/drift, total request
   controls, configuration/TLS/metrics, and cross-backend cleanup.
3. Retain the implemented bounded join/live differential; finish its native cancellation, consistency/admission and exact-release qualification.
4. Split and version the minimal production artifact and close admitted backend
   matrices.
5. Run the full integrated gate, repair only from the resulting exact head, and
   freeze an immutable release candidate.
6. Produce and verify the minimum release-evidence bundle. Tag, push or publish
   that candidate only with explicit authorization in the current task.

### Model and reasoning-effort allocation

Use the selected main model without asking for a downgrade. Choose supporting
native subscription agents by task, with no spend/token/request/invocation/quota
ceilings, API keys or OpenRouter. This is an initial execution policy, not a
claimed model-performance benchmark:

| Task | Starting choice |
|---|---|
| Deterministic operations | Ordinary tools; no model delegation required |
| Bounded mechanical work | Luna or Haiku |
| Established-pattern implementation | Terra |
| Normal feature work and test-driven repair | Sol or Sonnet |
| Difficult cross-component reasoning/review | Astra high or Opus |
| Hard unresolved semantics, concurrency or integration | Astra max/ultra |
| Exceptional long-running hard problem | Fable when it adds value |

Astra max and ultra are available. Explicit effort must pass unchanged through
the Codex candidate/client/adapter; omission uses the native model default, not
operation-specific high/low overrides. Distinct model/effort configurations need
distinct candidate IDs when recording routing outcomes. Native client errors
are authoritative: pause and report the exact client/model/error on subscription
or requested-model unavailability; never silently clamp, downgrade or substitute.
Optimize observed time to verified integration and rework. Do not invent monetary
savings or equate API list prices with subscription billing. Independent native
Codex and Claude review remains required for the release delta, not every edit.

### Six-hour course correction

The [scheduled prompt](../plans/programme-six-hour-review-prompt.md) compares
promised requirement-level outcomes and the previous correction with actual
`main` evidence, then changes execution and continues one primary outcome.
No progress in an active interval triggers integration/public-path work instead
of further decomposition, optional research or harness expansion. A missed
outcome requires an execution change, not an unsupported replacement deadline.
React to immediate blockers without waiting for the timer. The launcher uses
native `codex queue` for the pinned conversation, never a competing resume;
queue failure propagates with no execution fallback. Ruflo MCP memory is
optional and individually updated; malformed/failed recall cannot block delivery.

## Current implementation status

The v1 profile is accepted and **not complete** on 2026-09-08. Existing code has
exact-path, request-admission, immutable-snapshot, bounded-shutdown, partial
tracing, mapping-evidence, public bearer query admission/security-partitioned cache, and narrow
multi-source UNION and bounded join paths. Default-off three-family Prometheus metrics are
integrated with their public CLI/HTTP tests. ADR-0018's explicit bearer profile
now defaults closed, authenticates before body/source work, preserves context
through execution, and isolates single-source caches; protected UNION is uncached.
The optional PostgreSQL source-RLS profile now adds same-transaction trusted
identity, role/table checks and acknowledged rollback or session discard, with
required-live public-query and two-fragment UNION isolation/cleanup evidence.
The public provisioned-subject registry now authenticates distinct callers on one
server/pool, retains each identity/settings bundle atomically, and proves cache,
execution and live SELECT/ASK/CONSTRUCT/UNION isolation. It does not install
business policies, issue credentials or validate external identity issuers.
The schema-version-2 registry now also enforces a bounded portable equality-row
subset for exact source/table/column rules using bound parameters, with public
SQLite query and two-source UNION isolation plus fail-before-I/O rejection.
General ABAC/sensitivity, live cross-backend portable-policy qualification,
external issuer integration and policy/configuration hot reload, protected backend generations,
complete total governance, exact-artifact TLS qualification/OTLP, the full
metric catalogue, total cross-source cancellation, exact production packaging,
backend admission, and the minimum release bundle remain open.

The Rust serving connectors now enforce certificate/hostname verification for
remote PostgreSQL/MySQL, resolve bounded private trust references, prevent TLS
downgrade/socket fallback and retain PostgreSQL trust for cancellation. Required
loopback protocol peers and public startup rejection tests verify these boundaries.
Required native CLI tests now verify authenticated exact queries on digest-pinned
PostgreSQL 16.15/MySQL 8.4.11, including their mixed UNION, actual encrypted
sessions and independent-CA/hostname failures. Exact-release-artifact evidence
and backend admission remain required; this narrow result does not complete M3/M6.

The public bounded two-pattern join now implements ADR-0006's fixed-cap merge:
128 distinct complete driving triples, 4,096 probe triples, conservative bound
key parameters and exact RDF comparison. Independent graph-oracle and public
negative tests cover collation, padding, language, NULLs, bags, authorization,
caps and pre-200 failure. Required CLI tests prove SQLite reload and encrypted
PostgreSQL/MySQL joins in both triple orders. Each source has its own statement
view under one application generation; no distributed snapshot is claimed.
Wider algebra rejects before I/O; native cancellation, protected backend
generations and exact-release qualification remain required. General DISTINCT,
spill, Bloom/temp-table optimizations and ADR-0040 are not silently adopted.

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
