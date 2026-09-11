# Application-completion programme

- **Status:** In progress — ADR-0055 v1 completion profile active
- **Date:** 2026-08-26
- **Updated:** 2026-09-11
- **Controlling decision:** [ADR-0055](../adr/ADR-0055-v1-product-completion-and-release-profile.md) (accepted v1 profile)
- **Historical programme:** [ADR-0038](../adr/ADR-0038-sota-application-completion-programme.md) (superseded; post-1.0 SOTA backlog retained)
- **Supporting decisions:** [ADR-0037](../adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md), [ADR-0039](../adr/ADR-0039-minimal-production-serving-artifact.md), [ADR-0040](../adr/ADR-0040-bounded-federated-global-operators-and-spill.md), [ADR-0048](../adr/ADR-0048-rust-production-and-node-evidence-runtime-boundary.md), [ADR-0049](../adr/ADR-0049-exact-recursive-property-path-fixed-points.md), [ADR-0050](../adr/ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md), [ADR-0051](../adr/ADR-0051-postgresql-16-public-observed-schema-profile.md), [ADR-0052](../adr/ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md), [ADR-0053](../adr/ADR-0053-grammar-coupled-sparql-parser-governance-and-process-isolation-fallback.md), and [ADR-0054](../adr/ADR-0054-bounded-stable-root-order-windows.md)

**Scope:** Repository source, tests, accepted ADRs, CI, and measured benchmark evidence; GitHub issues and pull requests are deliberately not programme inputs.

**Status note:** This programme now separates v1 product completion from the post-1.0 SOTA and advanced-assurance backlog. Reclassification is explicit; nothing moved to post-1.0 is relabelled implemented, supported, or complete.

## Remaining release gates (2026-09-11)

This is the finite completion ledger under ADR-0055, not a new programme. At the review baseline
`a09e772a66080002574b92804b3dda4b0e64c62a`, sixteen overlapping catalogue
limitations mapped to the six outcomes below; they are not sixteen equal tasks.
All remain open until their missing evidence is attached. The sole main integrator
owns each outcome; native read-only Sol/Sonnet review supports it. Command IDs
resolve to exact commands in [the catalogue](../../tests/capabilities/catalog-v1.json).
Existing passes qualify their recorded source/profile, not a later release candidate.

| Gate / required outcome | Existing public evidence | Missing closure / dependency / acceptance |
|---|---|---|
| G1 — finite admitted request governance: `l-query-budget`, `l-deadline-cancellation`, `l-sqlite-admission` | Shared deadline/admission, isolated parser, selected compiler controls, public caps, SQLite VM interrupt and native stop/recovery. Projection/UNION integration is at `a09e772`. G1a base/VALUES now has exact/N-1/every-stop and ordinary/bearer phase-refusal, duplicate/UNDEF cold/key-only warm and capacity-recovery evidence; its harness binds the exact resulting commit. G1b's DESCRIBE form now has a checked whole-block envelope, paid fresh-name collisions/output copying and exact ordinary/bearer pre-source/cold-warm recovery evidence; initial RDF-star remains open. | Finish owned compiler/cache and source-work boundaries for admitted shapes, then prove ordinary/authenticated cold/warm exactness, early refusal and full capacity recovery at unchanged defaults. Use `cmd-query-budget-http`, `cmd-compiler-input-admission`, `cmd-sqlite-active-vm-identity`, `cmd-sqlite-connection-admission-backend`, `cmd-sqlite-connection-admission-serving`, `cmd-native-query-controls`. Compiler sub-outcomes are below; no research-grade CPU/allocator claim. |
| G2 — admitted RDF/query exactness: `l-native-ref-witness-identity`, `l-path-resource`, `l-postgresql-synthetic-row-identity`, `l-describe` | Public SQLite/native numeric, text/CHAR, static-template, path and bounded one-hop DESCRIBE cases have scoped evidence. | Close declared natural/typed-template value construction, arithmetic, mixed-descriptor/pooled and base-resolved IRI cases; resolve the PostgreSQL synthetic/real-rowid boundary for the declared profile. Validate exact bags/NULL/identity and existing unsupported-shape rejection with `cmd-property-path-exact`, `cmd-property-path-key-equality`, `cmd-postgresql-rowid-boundary`, `cmd-describe-compile`, `cmd-describe-sqlite`, `cmd-native-query-profile-live`. Do not add unadvertised DESCRIBE breadth or remove a promised feature. G1 controls must cover these admitted paths. |
| G3 — coherent generations/semantic admission: `l-metadata-toctou`, `l-schema-lifecycle`, `l-semantic-admission-scope`, `l-snapshot` | Immutable reload leases and protected PostgreSQL Direct/authored generations are public; invalid generations fence readiness/new requests. | Complete required backend/policy generation and DDL-race guarantees, binding M ⋈ T/source validation and cache identity to the same lease. Run `cmd-runtime-snapshot-state`, `cmd-runtime-snapshot-http`, `cmd-runtime-snapshot-body`, `cmd-public-authored-generation-live`, `cmd-public-direct-lifecycle-live`, `cmd-semantic-admission-runtime`, `cmd-semantic-admission-validation`, `cmd-semantic-admission-mapping`. Existing profile exclusions remain explicit; no new general ABAC or policy-hot-reload programme. |
| G4 — declared cross-source execution: `l-federation` | Two-source UNION and fixed-cap two-pattern join, actual lineage, native stop/sibling isolation/recovery are implemented. | Qualify source consistency, protected generations and every admitted backend combination for those public shapes; depends on G1–G3. Run `cmd-federated-union-sparql`, `cmd-federated-union-serve`, `cmd-federated-union-cli`, `cmd-federated-join-sparql` and their required owned mixed-source TLS cases. No Bloom/spill/global-algebra research prerequisite. |
| G5 — required native transport/live matrix: `l-transport-security`, `l-live-optional` | Verified remote TLS and native authentication/exactness suites have owned-fixture evidence. | Run the required fail-closed matrix against the immutable candidate after G1–G4; record exact artifact/profile/log digests and resolve candidate-specific failures. Use `cmd-verified-source-tls`, `cmd-verified-source-tls-cli`, `cmd-verified-source-tls-live`, `cmd-native-query-profile-live`. The TLS/authentication selector is not numeric-profile evidence. No new Product Mock/live-database access is implied. |
| G6 — exact candidate/admission verdict: `l-release-artifact`, `l-production-admission` | Rust-only serving boundary and versioned developmental non-root/read-only image smoke exist. | After G1–G5, one immutable candidate passes the full locked workspace and serving-only checks, backend/profile matrix, clean-machine smoke, licence/advisory review, SBOM, checksums, signature/provenance and independent native Codex/Claude exact-delta review. Use ADR-0055 §3 and M7/Definition of done below; record the actual commands/digests. These are aggregate gates, not two more feature programmes. Publication requires separate current approval. |

G1's finite accounting sub-outcomes are: **G1a**, base/VALUES controls now have
independent phase-cut and public exactness/recovery evidence; **G1b**, close the remaining
admitted compiler phases and acquired-cache operations as one end-to-end controlled
compilation profile; **G1c**, close source/recursive work and cleanup ownership for
the admitted SQLite/PostgreSQL/MySQL and two-source paths; **G1d**, run their combined
public/default-corpus acceptance and record the profile verdict. Before G1b edits,
enumerate the remaining phase entry/exit boundaries and exact missing proof once.
Group compatible work under that outcome; do not turn each next unmetered helper
into a new completion prerequisite or require `GovernedV1` merely as a label.
This inventory is not proof of closure, and known failures are not scope exclusions.

**Execution correction:** after G1a's source-bound verification/commit, take G1b's
one-time missing-phase/proof inventory and the smallest unblocked gate
outcome above. Audit prerequisite tests before freezing each harness scope; preserve
negative tests' intended phase rather than raising defaults or measuring away the
failure. Keep exact prospective controls at fanout, copy and retained-state growth
boundaries. A conservative phase envelope is an option only with checked finite
inputs, a bound covering its entire admitted work, bounded cancellation checkpoints
and unchanged default-corpus acceptance; it is not a substitute for source controls
or a permission to delete existing checks. Update one authoritative gate row and
only ADR decisions/status that actually change, not four implementation diaries.

**Forecast and stop rule:** six recent completed outcome families took 168.3 minutes
first-start-to-finish wall time; recorded checks took 10.6 minutes (6.3%). The remainder
is not all overhead: implementation, review and interruptions were not separately
timed. Nine recent commits touched 177 files, 61 of them ADR/catalogue/generated
evidence; file touches are not effort measurements. No whole-programme ETA is justified
until G1b/G2/G3 missing-proof inventories and candidate matrix runtimes are measured.
A correction has not worked merely because another helper/receipt is green: compare
the remaining named obligations and actual user-visible acceptance. Once G1–G6 pass
on the candidate, stop completion work and recommend ending the six-hour reviews.

## Historical implementation evidence

The prior queue and detailed H0/M0–M7 implementation diary remain locally in Git:
`git show a09e772:docs/plans/sota-application-completion-programme.md`.
That local commit is not asserted to be published. Historical passing/failing
receipts keep their exact claims; they are not current release gates or estimates.

## Outcome

Complete and release semantic-fabric v1 as a virtualisation-only knowledge graph over live relational systems of record. Completion means exact-or-fail semantics, bounded execution, real cross-source query execution, production security and operability, and the ADR-0055 minimum release evidence. The wider SOTA research and advanced-assurance programme resumes post-1.0.

The semantic compiler does **not** need a wholesale rewrite. Three evolutionary
product changes remain on the v1 critical path:

1. lower every advertised global operator to a bounded physical execution path;
2. finish total request controls, public security, reload/drift and operability;
3. complete source identity, the registry, and an admitted federated plan.

If the federation item is removed, the application charter must change from systems of record/cross-RDBMS federation to one source per deployment. This programme retains the accepted charter.

## Historical SOTA baseline

This frozen 2026-08-26 assessment is retained for audit and has not been
rescored. Its **44/100** and former 98-point target are post-1.0 SOTA diagnostics,
not v1 release authority. ADR-0055's explicit product and release gates control.

| Dimension | Weight | Baseline | Evidence-led finding |
|---|---:|---:|---|
| Correctness and standards | 25 | 17 | Strong fixed differential/W3C coverage and sealed mapping inputs/runners; path truncation, one R2RML deviation, incomplete backend receipts, and no generative/fuzz layer |
| Security and governance | 20 | 8 | Bound parameters and partial timeout/pooling exist; no total budget, result/cost cap, TLS, identity, or policy enforcement |
| Federation and architecture | 15 | 3 | Reusable backend abstraction and semi-join cost model; runtime and mapping IR are single-source |
| Performance and boundedness | 15 | 10 | Strong measured simple-streaming and Ontop evidence; global sort/group/dedup retain source-sized state |
| Operability and reliability | 15 | 2 | No production config, telemetry, health/readiness, graceful shutdown, reload, or drift handling |
| Release and product evidence | 10 | 4 | At the frozen snapshot, CI/audit/harness existed but the app lockfile was ignored; broad binary closure, version 0.0.0, and product release proof were absent |
| **Total** | **100** | **44** | **Historical SOTA target ≥98; not a v1 gate** |

Material gaps found directly in the current tree:

Former P0 constraint-authorized serving drift is closed by `24a0e20` for the authored-R2RML lane. A typed `CompilerSchema` strips unverified PK, UNIQUE, FK, functional-dependency and NOT-NULL proofs before compiler/cache construction. The 2026-09-02 extension adds non-forgeable `ColumnTypeAuthority::Unverified`: cached serving cannot use mutable startup types to prove positional PostgreSQL pooling across different physical columns, and missing facts fail closed. Poison controls cover stale constraints plus frozen/cached type-authority counterfactuals. This deliberately forgoes key/FD/type optimisations: D1 deduplicates conservatively; each fallback arm captures its full BGP-boundary key, including an active graph variable, before projection, remaps it only through physically key-preserving pure unary wrappers after all rewrites, and overlays it on a private execution clone. Joined/OPTIONAL/path/aggregate, modifier-bearing or multi-branch nested wrappers, key-dropping nested projections, orphaned/multiply owned markers and groups with fewer than two executable arms return `501`; runtime repeats the shape proof before I/O. Fully ground overlapping arms instead use an exact SQL unit-relation pool. Serving rejects the remaining source-sized fallback before I/O.

Live execution now recursively probes base Table/Query sources, overlays those catalogs for base-source references in nested SubPlans, rejects missing, duplicate or ambiguous result columns, allocates fresh aliases above nested IQ/SQL uses, validates and emits every branch before opening any cursor, and maps post-commit SELECT/CONSTRUCT executor failures to one stable body error. Offline/synthetic alias emission and translate-time immediate wrappers retain the bounded, non-SQL-token-aware lexical heuristic; it is never live metadata authority. Conformance `rr:sqlQuery` metadata errors are no longer ignored. Compiler controls preserve the synthetic Direct-Mapping `rowid` name across Table→Query and read PostgreSQL `ctid` only at a base table, but a real `rowid` collision and snapshot-local CTID keep the no-PK path non-authoritative. Shared fail-fast admission now bounds application work before Router/body polling; its permit follows active internal workers instead of completed response bytes. Each physical serving SQLite connection also has a permanent cap-one async identity. The public PostgreSQL PK-backed lifecycle now rebuilds coherent rich observations off-path and fences/heals through one control coordinator; request paths never fence. Raw/foreign SQLite mutex access, general lifecycle, no-PK identity and production admission remain open.

| Priority | Gap | Current evidence | Required disposition |
|---|---|---|---|
| P1 | Recursive-path resource qualification | `5c379f6` computes an exact finite-pair fixed point beyond 256 and rejects unproved dialects; serving counts observable probe/open/pull attempts, not source rows or recursive iterations; owned SQLite interrupts only active VM work | Charge recursive/source work to total `QueryBudget`; qualify a common source-native cancellation contract before backend admission |
| P0 | Bounded global operators are incomplete | `639134d` rejects source-reading fallbacks pre-I/O; ADR-0054 qualifies finite root variable-key ORDER; `43399ce` adds only streaming two-source `UnionAll`, which is non-blocking | Composite SQL or accepted bounded spill/merge for GROUP, DISTINCT, graph dedup, wider ORDER and cross-source blocking shapes; retain `501` until each is proved |
| P0 | Cross-source charter is not delivered | Immutable snapshots now serve exact two-source UNION and a fixed-cap two-pattern join, with conservative key reduction, exact RDF set/bag semantics, pre-200 failure and required SQLite/encrypted PostgreSQL/MySQL public evidence; native guards now protect fragments; the pinned native cancellation matrix now passes; wider backend/profile combinations, protected backend generations and release admission remain open | Extend the physical algebra one exact bounded shape at a time; qualify consistency, failure, cancellation, performance and the full backend matrix without accepting proposed ADR-0040 implicitly |
| P0 | Minimum release closure is incomplete | `Cargo.lock`, pinned inputs and non-authorizing diagnostic closure evidence exist; no exact v1 artifact bundle exists | Produce the minimal Rust artifact, SBOM, licence/advisory disposition, checksum, signature, provenance and clean smoke required by ADR-0055. Complete dynamic closure, witnesses and two-builder identity remain post-1.0 |
| P0 | Standards evidence is not yet release-complete | Backend-aware v5 receipts bind all 87 ordered SQLite, required-live PostgreSQL, and required-live MySQL mapping outcomes. MySQL records 74 pass, one documented deviation and 12 exact typed unsupported outcomes under its conformance-only SQL-2008 profile; provider provenance is unbound and zero production admission remains explicit. Per-test SQLite query/protocol baselines and the exact one-target-expression, one-hop SQLite DESCRIBE endpoint detect regression without claiming full W3C conformance | Add pinned supported-surface SPARQL/Protocol manifests and wider DESCRIBE qualification; keep mapping/query/protocol, provider provenance and backend-admission evidence disjoint |
| P1 | Governance covers only part of a request | One serving identity spans the deadline, fail-fast aggregate active-work admission, and observable source/result/byte work. Its default 64 is finite governance, not capacity evidence; active internal workers retain the permit. SQLite adds cancellable per-member admission and active-VM interruption. Fairness/bounded waiting, raw mutex and submitted/running-work cancellation, busy/UDF/VFS/I/O, compiler CPU, database/recursive work, raw/conformance, full native cancellation/admission qualification and atomic post-`200` responses remain outside total governance | Measure and configure the gate per deployment; extend the same control into total `QueryBudget` and native cancellation for every backend |
| P1 | Production secret/transport exposure | `484a4b4` adds bounded redacted `SourceRef`, exclusive `--source`/`--source-env`, typed driver parsing and pre-I/O inline-password rejection. Typed bounded TOML/environment/CLI layering now carries only secret references through the same redacted startup boundary. Remote PostgreSQL/MySQL certificate/name verification, bounded private roots and same-policy PostgreSQL cancellation have required peer/CLI tests. Required digest-pinned live CLI tests cover both encrypted backends and their mixed UNION. Exact-release-artifact qualification, a direct external secret-store protocol, metrics/OTLP and broader secret-corpus coverage remain open | Exact-artifact TLS qualification, complete telemetry and secret-corpus tests |
| P1 | Accepted runtime ADRs are not fully delivered | ADR-0011 now has typed layered startup configuration, fixed health/readiness probes, bounded signal shutdown and the exact partial structured-tracing slice described above, but still lacks full metrics/OTLP, SLO/overhead qualification, exact-artifact TLS qualification, source-health policy and adapter-internal `sf-sql` spans; ADR-0017/0018 remain incomplete | Implement or supersede the remaining clauses with dated status/evidence |
| P1 | V1 release-profile tests are incomplete | The exact 5,000-case SQLite SELECT train is integrated; the dated advertised/live release matrix remains open | Close the focused v1 matrix; schedule the nightly 100,000, broad NoREC/MR1, long fuzz/shrinking and global coverage/mutation trains post-1.0 |
| P1 | Serving artifact is too broad | `sf-cli` imports conformance/bench; conformance enables REST and SQL Server | Minimal serve artifact; opt-in evidence/developer features |
| P1 | Remaining lifecycle and admission work | Immutable snapshots, request leases, fixed probes, bounded shutdown and authored-R2RML `M ⋈ T` admission exist. The public closed PostgreSQL-16 Direct profile now builds fully validated candidates off-path, compares the rich qualified generation on a distinct max-size-one control pool, fences only completed control failures, retries while not ready, activates by full-state CAS and fails closed on worker loss. Exact-patch public CLI startup and fixed-policy authored reload are qualified; wider lifecycle and production admission remain open | Preserve the public closed-profile evidence; close remaining required compiler/source controls and exact release gates without weakening the sealed boundary |
| P2 | Maintainability risk | `exec_core`, `build`, PostgreSQL introspection and the normalizer are characterized/decomposed; every `iq/normalize` production/test file is below 500 lines. Eighteen product-source files remain above 500 lines: 12 in `sf-sparql` (`iq/lower.rs`, `cascade/mod.rs`, `unfold.rs`, `emit.rs`, `unify.rs`, `lib.rs`, `iq/resolve.rs`, `iq.rs`, `cascade/joinelim.rs`, `path.rs`, `leftjoin.rs`, `cascade/ws_st.rs`), four in `sf-sql` (`backend/rest.rs`, `backend/pg.rs`, `backend/sqlserver.rs`, `backend/monetdb.rs`) and two in `sf-mapping` (`r2rml.rs`, `direct_mapping.rs`). `sf-bench/workload.rs` and multiple test/evidence files are also oversized | Split only lane-blocking stages, preserving behavior, API, test identities and evidence selectors |

The 2026-09-01 graph-scope slice closes the former P0 generated-blank-node
identity gap for the evidenced single-source profile. R2RML mapping nodes are
reconstructed from `(effective target graph, generated identifier)`; default,
constant and row-derived graph maps normalize consistently; direct and
reference objects, class atoms, fixed-graph paths, dump/query execution and
DISTINCT subplan remapping share the same IQ identity; and CONSTRUCT template
nodes retain a disjoint fresh-per-solution label domain. Same-graph equality
between differently shaped identifier recipes remains a sound `501`, as do
dynamic-graph paths and row-dependent rendered-width pooling. PostgreSQL/MySQL
still need direct named-graph execution matrices, and globally bounded graph
dedup remains open.

`cargo audit` passes with six configured advisory exceptions and three
unmaintained crates: `paste 1.0.15` (`RUSTSEC-2024-0436`),
`proc-macro-error2 2.0.1` (`RUSTSEC-2026-0173`), and `rustls-pemfile 1.0.4`
(`RUSTSEC-2025-0134`). This is not clean supply-chain closure: every exception
still needs reachable-feature, owner, expiry, and compensating-control evidence.

## Domain model and target architecture

| Bounded context | Aggregate or port | Current home | Target responsibility |
|---|---|---|---|
| Semantic contract | `RuntimeSnapshot`, T-box, mapping IR, capability profile | `sf-core`, `sf-mapping` | Versioned T/M/schema/source/constraint-policy identity and fail-closed validation |
| Query compiler | IQ, optimizer, dialect-neutral physical operators | `sf-sparql` | Exact supported-profile rewrite to single/federated physical plan |
| Source runtime | `SourceRegistry`, `SqlBackend`, backend capability contract | `sf-sql`, `sf-serve` | Source lifecycle, binding, streaming, cancellation, health and admission |
| Federation | `FederatedPlan`, fragment, reducer, global operator | sealed `sf-sparql`/`sf-serve` two-source `UnionAll`; wider nodes planned | Per-source fragments, bounded data movement, merge/spill and failure semantics |
| Request governance | `QueryBudget`, `SecurityContext` | `sf-serve` | Total deadline/work/result budget, identity, policy and safe public errors |
| Lineage | query receipt/provenance vector | `sf-sparql`, `sf-serve` | Mapping/source/row-key lineage without persisted A-box state |
| Operations | runtime config and lifecycle | `sf-serve`, `sf-cli` | Secrets, TLS, telemetry, probes, reload, shutdown and drift |
| Evidence and release | immutable evidence bundle | `sf-conformance`, `sf-bench`, CI | Standards, oracle, QE, load, security and exact-artifact proof |
| Engineering control | task contract and receipt | `coding-harness/`, Ruflo | Dual-host proposals/repair/verification; never product authority |

```text
HTTP / CLI
   │  SecurityContext + QueryBudget
   ▼
Query session ───────────────► CapabilityProfile (exact or reject)
   │
   ├── RuntimeSnapshot { T, M, schemas, constraint authorities, epochs, digests }
   │                    │
   │                    └── SourceRegistry { SourceId → backend/capabilities }
   ▼
semantic compiler → federated physical plan
                         │
            ┌────────────┼────────────┐
            ▼            ▼            ▼
       source fragment  reducer   global bounded operator
            └────────────┴────────────┘
                         ▼
              reconstruction/serialization

Evidence plane: standards + differential + QE + load + release receipts
Engineering plane: Ruflo + native Codex/Claude MetaHarness (no promotion authority)
```

The shared-contract gate now includes opaque `SourceId`/`SourceMapping`, neutral
schema DTOs, atomic request accounting with fail-fast aggregate admission, an
immutable source registry/snapshot with request-lifetime generation leases, and
the sealed two-source `UnionAll` vertical. Next add validated candidate
construction, drift/reload and each remaining bounded federated node;
decomposition targets only modules blocking those lanes.

## Programme dependency graph

```text
M0A typed seams + release-profile truth
  ├─► M1 bounded physical execution
  ├─► M2 total-governance contract
  ├─► M3 secure observable runtime
  ├─► M4 v1 standards/live matrix
  └─► M5 snapshot lifecycle, source identity, policy + lineage
M1 operator seam + M5 SourceId ─► M6 contracts/fixtures; M2 governance + M4 QE ─► M6 promotion
M1 + M2 + M3 + M4 + M5 + M6 ───► M7 minimal v1 release
M0E + advanced M4 + SOTA proof + harness evolution ───► post-1.0
```

M1–M5 product work uses one integration writer with independent read-only support
once M0A supplies typed seams. M6 follows operator/source-identity dependencies. M7 waits for
all ADR-0055 product and minimum-evidence gates, not the post-1.0 branch.

## Milestones and QA gates

These sections explain G1–G6; they are not a second backlog. Only accepted
ADR-0055 profile guarantees are prerequisites. Proposed ADRs describe options.

### M0 — Architectural truth and deterministic foundation

Outcomes:

- publish a generated, dated capability/backend/standards matrix;
- distinguish `accepted` decision status from implementation status in every
  touched ADR;
- track `Cargo.lock`; use `--locked`; pin CI actions, installed tools, W3C suite
  inventory, fixtures, expected outcomes, skips, deviations, and spec snapshots;
- split RDB2RDF mapping conformance from SPARQL query/protocol evidence;
- freeze backend-aware SQLite and required-live PostgreSQL receipts over every
  ordered mapping outcome without promoting either backend;
- **SPARQL baselines:** freeze per-test expected SQLite query and Protocol
  outcomes as regression receipts, without treating them as W3C conformance,
  runtime provenance, or backend admission;
- retain focused boundedness evidence while keeping diagnostic records distinct
  from artifact and release authority; and
- retain the existing production-artifact decision; broader global-operator/spill
  design remains post-1.0, not a prerequisite to the admitted federation shapes.

QA gate:

- one clean release checkout resolves the locked dependency graph and exact
  candidate; independent byte-identical builder proof remains post-1.0;
- every public claim maps to a test/profile entry or is labelled planned;
- missing/malformed standards inputs fail; no new skip can hide behind a count;
- the programme backlog remains derived solely from the charter, source,
  decisions, standards and executable evidence.

### M1 — Exactness and bounded physical execution

Outcomes:

- replace silent 256-hop truncation with exact cycle-safe closure for each
  admitted dialect, or reject before returning a success response;
- preserve exact bounded execution of admitted ORDER, GROUP/aggregate, DISTINCT
  and graph/CONSTRUCT dedup, with no accidental in-memory fallback; broader
  global/external operator development remains post-1.0;
- close the PostgreSQL delimited-identifier deviation or retain it as an explicit
  release-profile exclusion; and
- keep changed files manageable; decompose a compiler/executor hotspot only when needed for the current required fix, not as an independent completion gate.

QA gate:

- chains and cycles at 1, 255, 256, 257 and >1,000 hops are exact or explicitly
  rejected, never silently partial;
- every advertised blocking shape respects its configured retained-state caps
  and has targeted source-growth evidence; formal comparative RSS qualification
  remains post-1.0;
- flat/tree/unoptimized/optimized/materialized-oracle results agree;
- unsupported-shape tests assert the exact pre-execution failure class.

### M2 — Admitted request governance and cancellation (ADR-0055)

Outcomes:

- preserve one linearizable identity for active application work from the absolute Tower `Service::call` deadline through admission, compile/acquire/execute, observable source work, semantic results and serializer bytes; representable expired handoffs are public `504` while internal first-cause accounting remains sticky; fixed health and query-less discovery metadata intentionally bypass query-work accounting;
- preserve strict media-specific request admission and its raw `n`/checked form `3n+16` wire/decoded `n` caps; it is a subset, not full Protocol conformance;
- preserve the shared fail-fast active-work gate: finite default 64, startup range `1..=Semaphore::MAX_PERMITS` before I/O, shedding in `call` before Router/body polling, deadline/control precedence, stable `503 service-overloaded` plus `Retry-After: 1`, and closed-gate `500`;
- retain its permit through active producer/compiler/backend clones but not completed-byte draining; keep terminal state out-of-band so a full channel finishes at its deadline and a streamed failure yields buffered prefix, one stable `result stream failed` error and fused EOF without `Content-Length`;
- retain the 2026-09-08 compiler-input floor: decoded UTF-8 bytes are precharged at every public compilation entry, including lineage, preflight and authenticated cache hits; exact-bound HTTP and cumulative two-pass tests pass. Public tree compilation also now meters normalization/lowering/nested-cascade clones and checked tree inner-join candidate products/left-branch copies plus atom-resolution candidates/logical-source copies and path mapping-search/complement work with exact term/source copies and mapping-wide graph inventory/reflexive checks, with cancellation, cache isolation and exact-result tests; cache hits avoid clone replay. Finish required admitted-profile compiler/catalog-growth and source-work controls without equating fuel to exact CPU; raw/conformance extensions are not silently v1 prerequisites under ADR-0055;
- keep lexical/direct-IRI scanners diagnostic. The reproduced server parser abort
  now requires the Rust isolated runtime under ADR-0055: public ordinary/lineage,
  preflight and federation use bounded QueryV1, request cancellation and exact reap;
  cap-one survival/exact recovery pass. Full ELF/syscall attestation stays post-1.0;
- admit post-parse algebra before bounded canonical rendering, and prove finite compiler/catalog growth and cancellation for every admitted phase using the shared request identity. Retain checked fanout, copy and retained-state admission; ordinary traversal may use a proved conservative phase envelope with bounded checkpoints. Proposed [ADR-0052](../adr/ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md) is not independent scope authority; logical work is not an exact CPU/allocator measurement;
- retain exact cache identity/partitioning and shared `Arc<Plan>` ownership; bound acquired-lock operations, capacity/eviction and owned cleanup for the admitted profile. Stable cross-process alpha-equivalence expansion remains post-1.0. Do not claim physical allocator/drop isolation from logical accounting alone;
- close cooperative cancellation/work bounds for the cap-four compiler profile; activate `GovernedV1` only if its declared gates pass. Its name is not a product requirement, and no default-admitted query may be silently removed to obtain a green gate;
- retain per-physical-connection cap-one admission and active-VM cancellation for owned SQLite; then cover raw mutex/submitted-work/busy/UDF/VFS/I/O gaps and retain the implemented native PostgreSQL/MySQL guards and the pinned mixed-source timeout/disconnect/SIGTERM matrix; finish wider backend/profile qualification without claiming fairness or bounded waiting;
- propagate disconnect/cancellation to all tasks, streams and connections; and
- retain generated RFC 9457/trace correlation, bounded payload-free logs/metrics and seeded-secret rejection; add adapter visibility only to close a demonstrated admitted operability defect, without driver/schema/credential strings.

QA gate:

- for active application work, one deadline covers Tower `Service::call` after request-target parsing but before Axum route/method dispatch, then admission, parse, compile, acquire, execute and serialize; fixed health and query-less discovery metadata remain available without entering that work path;
- timeout/disconnect releases worker and connection capacity within the declared
  bound for every advertised path; focused parser cancellation/reap is required
  defect-repair evidence, not the deferred full containment-attestation programme;
- exact `0`, `N` and `N+1` parser/algebra/build/work/cache tests plus focused
  release-profile differential and malformed-input proofs pass; long corpus
  fuzzing and cross-process alpha-equivalence expansion remain post-1.0;
- aggregate overload sheds immediately without Router/body polling or an internal queue; provider pools retain their separately configured bounds;
- exact result/byte caps and finite recursive/source work hold for every admitted format/path. A failed bounded prefix is followed by terminal failure and never labelled a complete answer; atomic/no-prefix streaming is explicitly post-1.0 under ADR-0055.

### M3 — Secure, observable, operable runtime

Outcomes:

- retain the implemented bounded typed startup layers and environment-only
  secret injection; direct external secret-store transport remains separate work;
- retain implemented verified remote-source TLS and credential-free argv; retain required live PostgreSQL/MySQL TLS/UNION checks and rerun on the exact release artifact;
- retain ADR-0011's request/compiler spans, Serve-only JSON logs, governance events
  and three-family default-off Prometheus profile with bounded volume/redaction;
  full OTLP/SLO expansion is post-1.0, not an admitted operability prerequisite;
- retain implemented `/livez`, snapshot-state `/readyz`, and three-phase bounded SIGTERM/Ctrl-C shutdown (drain preserves admitted work; forced expiry cancels survivors and allows three seconds for owned cleanup, failing on exhaustion); add automatic source-health/failure policy and complete cross-backend cleanup qualification; and
- publish finite operational limits and alert thresholds; research-grade SLO
  calibration remains post-1.0.

QA gate:

- a seeded secret corpus appears nowhere in argv, logs, traces, metrics or errors;
- telemetry has bounded event/label volume and passes a release smoke; formal
  controlled overhead qualification remains post-1.0;
- no metric label contains query text, IRIs, source values or other unbounded data;
- readiness already fails for explicit invalid/not-ready runtime state; automatic unavailable-mandatory-source detection remains open;
- existing admitted work can complete during drain, newly minted budgets reject, exact forced expiry cancels survivors, and owned PostgreSQL/MySQL TLS CLI tests observe stopped native work, clean exit and closed ingress after forced ASK/SELECT/CONSTRUCT and mixed UNION/join SIGTERM; server-side lock/session witnesses, separate same-credential CLI sibling safety and full cap-one recovery pass; wider backend/profile and release admission remain open.

### M4 — V1 standards and live-release profile

V1 outcomes:

- retain the exact fail-closed RDB2RDF inventory and run the required live
  matrix for each admitted SQLite, PostgreSQL, and MySQL profile;
- publish one dated supported-surface SPARQL/Protocol/result-format manifest and
  prove every advertised cell or its exact pre-I/O rejection;
- retain the static Product Mock `M ⋈ T`, DESCRIBE, mapping receipts, and exact
  5,000-case generated SQLite train as directly applicable regression evidence;
- run focused generators, malformed-input cases, mutants, and fault injection
  for the critical boundaries changed in the v1 lane; and
- make every required release service fail closed when unavailable.

V1 QA requires the exact inventory, no unexpected failure/skip/deviation, and
the advertised live/capability matrix on the release candidate. The nightly
100,000 train, broader NoREC/MR1 and cross-backend generation, long fuzz and
shrinking campaigns, global coverage/mutation ratchets, Agentic-QE expansion,
and one-hour controlled soak are labelled post-1.0 work.

### M5 — Snapshot lifecycle, identity, policy and lineage ([ADR-0050](../adr/ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md))

Current bounded slice (2026-09-08): `PgDirectLifecycleV1` now has public Rust/CLI startup and exact 16.9/16.15 owned-TLS lifecycle proof; its types stay sealed and production admission remains separate. The public Rust/CLI bearer service-principal profile now defaults closed, validates an environment-referenced credential before query body/source work, retains the provider-neutral `SecurityContext` through execution, partitions the single-source cache and compiles protected UNION uncached. Real allow/deny traces emit. The default bearer profile grants read access to all mapped data. Its optional PostgreSQL source-RLS profile binds explicitly configured claims in read-only transactions, guards every mapped public table and role, and rolls back or discards connections. Public SELECT/ASK/CONSTRUCT and both UNION fragments pass disposable PostgreSQL 16.15 row-isolation, concurrency, failure, deadline and pool-cleanup tests. The schema-version-2 provisioned registry additionally supplies a narrow portable equality-row profile for exact source/table/column rules; values remain bound parameters, uncovered or complex plans reject before source I/O, and public SQLite queries plus the two-source UNION isolate callers. General end-user identity, general ABAC/sensitivity, live cross-backend portable-policy qualification, policy-aware hot reload and access-decision metrics remain open. Rotation requires a new server. ADR-0018 remains incomplete and no backend gains production admission. The opt-in ADR-0017 constant-mapping/source SELECT and CONSTRUCT profiles now emit final-solution PROV-O and native RDF 1.2 graph reification with snapshot/logical-plan/policy IDs through the normal authenticated request/stream path. CONSTRUCT frames one response-wide N-Quads dataset with product/default graph separated from named provenance bundles, shared mapped nodes and fresh template nodes, and occurrence counts rather than unique-graph claims. Required owned pinned PostgreSQL 16.15/MySQL 8.4.11 serving-only CLI tests now parse actual SELECT PROV-O and graph reification, preserve empty/allowed portable-row isolation and reject unsupported lineage while a native source-table lock remains held. These cases do not establish all native lineage lifecycle/operator combinations or release admission. Required SQLite tests cover native reification, empty/invalid templates, nested/directional terms, escaping, bags, dedup/slice, UNION, saturation, portable policy, pinned reload, exact bytes and failure; wider multi-origin operators, authorized row keys, wider federation and wider native-profile/exact-release qualification remain open. The additional bounded multi-mapping profile carries only actual origins through RDF-matched positive BGP/JOIN/UNION and root projection/DISTINCT/slice; overlapping witnesses merge origins before LIMIT while explicit UNION preserves bags. It caps each relation at 1,024 witnesses and fails on overflow. Constant-predicate/graph-map and other precise restrictions are in ADR-0017; its mappingCatalog is not provenance. Ordinary query optimization remains separate from the lineage recipe. Twelve required native multi-map SELECT/CONSTRUCT cases now observe the exact encrypted target stop under deadline/disconnect/forced SIGTERM, preserve a separately locked sibling, and verify cap-one lineage recovery or bounded clean process exit. This does not qualify lineage UNION/JOIN, portable/source-RLS cancellation or reload. The separate bounded two-source UNION lineage profile now carries actual source-keyed mapping origins through the public path, preserving bags and blank-node scope with one snapshot/security/budget. Required HTTP checks cover reversed/unbound arms, entailed affinity, portable callers, pinned activation, witness and exact byte/result limits, and cap-one failure recovery. Six additional pinned native TLS UNION cases cover exact-target deadline/disconnect/forced-shutdown stop, unaffected siblings and recovery. Federated lineage CONSTRUCT and wider qualification remain open. The separate bounded federated join lineage profile now seals the actual map/source pair per mandatory arm, follows cost-side swapping, and emits both contributors only for each final matched bag occurrence. It reuses the 128-build/4096-probe executor and capped pre-200 serializer, with explicit map-to-source links and no hidden keys. Required public checks cover exact/projected bags, policy, activation and limits; the pinned TLS CLI aggregate adds twelve both-order join-lineage deadline/disconnect/forced-shutdown cases with exact encrypted target, held-lock, sibling and cap-one recovery witnesses. The recorded source-RLS qualification follows below; exact-release qualification remains open. Required owned TLS lineage reload now verifies changed/restored actual mapping documents and exact results for constant/multi-map SELECT/CONSTRUCT, mixed UNION and nonempty both-order joins. Native-held multi-map SELECT on both providers plus mixed UNION/forward join on PostgreSQL completes with pre-invalid results while readiness and new queries fail closed; repair restores readiness. These are authored-generation checks, not policy/configuration reload or every held graph/operator/order. Required owned PostgreSQL16.15 public-router source-RLS evidence now parses actual constant/overlapping-map SELECT/CONSTRUCT and two-source UNION/join lineage for A/B/A callers: exact authorized products/bags, actual source/map origins, no denied values or raw identities, empty results, concurrent constant SELECT and clean cap-one PID reuse after each complete response. Constant-lineage SELECT/CONSTRUCT body-drop, policy-error and deadline cases fail terminally and recover with isolated caller state. This is the recorded RLS profile, not every failure permutation, remote TLS, policy installation/configuration reload or exact-artifact admission. Under ADR-0055 the declared lineage-profile gate is now qualified and l-lineage is non-blocking. Full historical ADR-0017 remains incomplete; separate runtime-budget, backend-admission and exact-release gates stay blocking.

Outcomes:

- build immutable `RuntimeSnapshot {T, M, schemas, sources, epochs, digests}`;
- validate `M ⋈ T` and source capabilities off-path, then atomically swap; existing
  queries retain their original snapshot;
- fingerprint source schemas, detect drift, invalidate affected plans, roll back
  invalid snapshots and expose readiness state;
- retain the public query-admission context, isolated cache, PostgreSQL transactional source-RLS and portable equality-row profiles; qualify their required generation/concurrency/cleanup behavior. General ABAC/sensitivity, external end-user issuers and policy-hot-reload expansion remain post-1.0 under ADR-0055;
- retain bounded allow/deny traces and protect their redaction; extra decision telemetry is not an independent v1 gate; and
- retain the qualified opt-in lineage profiles tying actual mapping/source, snapshot,
  plan and policy identifiers to results; wider row-key/operator lineage remains outside the admitted profile. Never persist instance data or source values in telemetry.

QA gate:

- invalid reloads never activate; valid reloads are zero-downtime and invalidate
  stale plans exactly once;
- drift is detected within the declared interval and blocks affected readiness;
- tenant noninterference and pooled-context cleanup pass across concurrency,
  cancellation and failure;
- admitted provenance identifies the expected actual origins and pinned identifiers,
  bounded by its declared retained-state/result/request controls; no wider row-key claim.

### M6 — Charter-complete cross-source federation

Outcomes:

- retain the implemented `SourceId`, mapping/source affinity and immutable source registry, then complete capability and backend-generation contracts;
- retain the implemented bounded two-pattern join and one-triple-per-arm `UnionAll`;
  finish native cancellation and exact-release qualification with no silent source omission;
- preserve exact bag/NULL semantics, source consistency, partial-failure handling,
  shared cancellation and provenance for every advertised join/operator;
- use the simplest proven bounded strategy; broader operators, temp-table/Bloom
  reducers and research spill optimizations stay post-1.0 unless required by an
  advertised feature; admit source/backend/shape cells through the public profile.

QA gate:

- every released source-count/shape differential equals a trusted materialized
  reference under its admitted failure schedules;
- coordinator retained state obeys the M1 caps independently of source size;
- any implemented reducers prove exactness and safe bypass; comparative
  transfer-efficiency qualification remains post-1.0;
- cancellation reaches every source and spill artifact; no partial result is
  labeled successful.

### M7 — Minimal v1 release

Outcomes:

- split the production server from conformance, benchmark and experimental
  connector closures; give the application a non-zero semantic version;
- define supported platforms/backends, upgrade/rollback/runbooks and a non-root,
  read-only OCI reference deployment;
- gate licences, semver/API compatibility, the Rust product audit, time-bounded
  advisory waivers, SBOM, checksums, signature and exact-artifact provenance;
- build once in a clean controlled environment and smoke the exact packed digest
  against every admitted backend and federated profile; and
- retain release commands and outputs as the minimum replayable evidence bundle.

QA gate:

- production dependency tree contains only admitted functionality;
- zero unwaived reachable critical/high vulnerability and every waiver has owner,
  dependency path, feature/target reachability, controls and expiry;
- signatures, SBOM and provenance verify independently; clean-machine smoke uses
  the exact packed digest;
- all ADR-0055 hard gates pass and independent native Codex and Claude release-
  delta reviews agree. A harness diagnostic or model score contributes no points.

Two-builder byte identity, transparency/witness publication, exhaustive runtime-
closure proof, comparative Ontop publication and harness evolution are not
renamed complete; they remain post-1.0.

## Ruflo and MetaHarness execution model

ADR-0037 remains available as the engineering control plane, but ADR-0055
governs its v1 use:

1. One integration owner writes and commits directly on `main`; no new branches
   or worktrees. Preserve unrelated changes and historical recovery refs.
2. Every building task uses the [main-only delivery harness](../../coding-harness/README.md#mandatory-delivery-path): task → native model/effort handoff → checks → verification → scoped commit → exact-commit result. Native agents edit; parallelize read-only investigation/review, not shared writes. Closed experiments remain optional. Honor explicit user review holds before any continuation.
3. Each commit runs affected tests/builds; shared contracts, dependencies,
   security, unknown impact or failed selection escalate to integrated gates.
4. Meaningful public-feature integration boundaries run the full locked workspace
   plus relevant live/security/cache/federation checks, not every micro-commit.
5. The immutable release candidate runs all ADR-0055 gates and receives
   independent native Codex and Claude exact-delta review.

Ruflo records coordination and evidence identity; deterministic Rust tests and
release checks remain product authority. Native subscription transport only is
permitted; OpenRouter and provider API-key fallback remain prohibited. No subscription spend/token/request/invocation/quota ceiling is permitted.
Native subscription/model unavailability pauses execution with its exact error.
**Model correction (2026-09-10):** repeated Ultra follow-up is no longer the default. Ordinary tools handle Git/build/test/polling; Luna low/Haiku handle bounded mechanical tasks, Terra medium established patterns, and Sol medium/Sonnet normal implementation and review. Sol high is appropriate for the current bounded identity proof. Escalate to Astra high/Opus only for a named unresolved cross-component problem; max/Fable needs a bounded exceptional task, and Ultra a specific user request. Retain the selected main model and explicit requested effort. Once a hard question is answered, route subsequent tasks afresh rather than keeping its strongest reviewer. The native Ultra reviewer was stopped and Sol assigned; current evidence does not support a numerical speedup or cost claim. ADR-0055 records the policy and qualification boundary.

Historical harness receipts remain valid for their exact claims. MetaHarness V7,
Darwin/GEPA, AVO, generic score improvement and retrieval-policy evolution are
post-1.0. The flywheel remains off, and no bulk memory import may bypass owning
stores or synchronization.

## Non-goals unless the charter changes

- A-box materialization, ETL, persistent triple storage or CDC materialization;
- non-relational/file ingestion, RML/FNML/YARRRML, or external SPARQL `SERVICE`;
- admitting cloud/REST/ODBC/Oracle/HANA adapters from mocked happy paths;
- in-engine general result caching, a Kubernetes operator, or bespoke SDKs;
- ML/LLM query planning, GeoSPARQL, FTS, full OWL 2 QL, or other breadth without a
  named product need and differential/benchmark oracle;
- multi-node coordination inside the engine before replica-based deployment and
  the federated single-node coordinator demonstrate an actual scaling limit; and
- changing the semantic compiler merely to reduce file size. Decomposition
  follows characterization and preserves the proven algebra.

## Risk register

| Risk | Consequence | Control |
|---|---|---|
| Draft SPARQL 1.2 changes | Moving conformance target | Pin dated snapshot; publish delta; separate stable R2RML claims |
| Path/global-operator repair changes answers | New correctness regressions | Generated oracle, mutation and >256/cycle corpus before refactor |
| External scanner or raw cross-parse equality diverges from the pinned parser | False admission, rejection or cache miss classification before source I/O | No scanner authority; ADR-0055 parser-lifetime repair uses isolated parsing, bounded QueryV1 and focused release-profile differentials; full ELF/syscall attestation stays post-1.0 |
| Federation becomes a rewrite | Schedule and semantic drift | Preserve compiler; introduce SourceId/registry/physical plan behind ports |
| Spill substrate conflicts with ADR-0006 | Hidden architecture reversal | Separate design-lock ADR and benchmark both implementation choices |
| Backend behavior diverges | One green dialect masks another | Shared backend contract plus fail-closed live matrix |
| Policy taxonomy is unavailable | Platform coupling or stalled delivery | Provider-neutral SecurityContext/Policy port and reference fixtures |
| High gates become flaky | Teams bypass evidence | Controlled runners, variance classification, quarantine with owner/expiry |
| Supply-chain exceptions become permanent | Known reachable exposure | Reachability proof, owner, controls, expiry and release review |
| Harness optimizes its evaluator | False programme progress | Protected inputs, independent product gates, sealed holdouts, reward-hack scan |
| Documentation drifts again | Misleading claims and planning | Maintain G1–G6 from source-bound public acceptance and exact missing evidence; update only affected ADR decisions/status and required catalogue hashes |

## Definition of done

The v1 programme closes only when one immutable candidate satisfies ADR-0055:

- every applicable accepted product/runtime ADR is implemented with current
  executable evidence or explicitly superseded;
- every advertised query/profile/backend cell is exact, and every unsupported cell fails before a valid-looking response;
- bounded state, total budgets, cancellation and overload gates pass for every
  admitted operator and backend;
- identity/policy/provenance, snapshot lifecycle and operability gates pass;
- cross-source differential, boundedness, reduction, cancellation and failure semantics pass;
- the minimal immutable artifact, live smoke, SBOM, licence/advisory disposition,
  checksums, signature and provenance verify from one clean controlled build; and
- independent native Codex and Claude review the exact release delta without
  replacing any deterministic gate.

Post-1.0 research and advanced assurance remain explicitly incomplete backlog;
they do not change the v1 verdict. Anything short of the bullets above is not a
completed v1 application and must be reported with its exact failed gate.
