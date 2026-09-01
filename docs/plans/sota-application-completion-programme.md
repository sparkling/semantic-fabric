# SOTA application-completion programme

- **Status:** In progress
- **Date:** 2026-08-26
- **Updated:** 2026-09-01
- **Decision records:** [ADR-0038](../adr/ADR-0038-sota-application-completion-programme.md) (accepted), [ADR-0039](../adr/ADR-0039-minimal-production-serving-artifact.md) (proposed artifact design), [ADR-0040](../adr/ADR-0040-bounded-federated-global-operators-and-spill.md) (proposed bounded-federation design), [ADR-0041](../adr/ADR-0041-manifest-bound-controlled-observational-evidence-capture.md) (proposed capture design), [ADR-0042](../adr/ADR-0042-witnessed-single-use-capture-supervisor-protocol.md) (proposed witnessed supervisor protocol), [ADR-0043](../adr/ADR-0043-postgresql-supervisor-registration-state-and-dormant-adapter.md) (proposed PostgreSQL state/adapter design), [ADR-0044](../adr/ADR-0044-postgresql-supervisor-catalogue-contract.md) (proposed exact catalogue design), [ADR-0045](../adr/ADR-0045-canonical-postgresql-supervisor-catalogue-oracle-representation.md) (proposed canonical oracle representation), [ADR-0046](../adr/ADR-0046-sealed-postgresql-supervisor-migration-authority-bundle.md) (proposed sealed migration authority), and [ADR-0047](../adr/ADR-0047-canonical-postgresql-16-15-public-acl-baseline-projection.md) (proposed pinned PostgreSQL baseline projection)

**Scope:** Repository source, tests, accepted ADRs, CI, and measured benchmark evidence; GitHub issues and pull requests are deliberately not programme inputs.

## Execution status

| Slice | Status | Verified evidence |
|---|---|---|
| H0a — frozen replay-policy foundation | Complete | `b40dbc6`; schema-v4 surfaces unchanged; schema-v5 policy fingerprint `11c17544e97c1509456f6efb88081a55bd56c93ac306a9b05c2da7102e5f755b`; 381 tests passed and 2 expected skips |
| H0b — schema-v5 evaluator, scorer and envelope | Complete | `7a1fa24`; accepted golden policy/assessment/envelope `0d5505e4…61bb` / `4f4fe45c…a977` / `fdab0843…65e7`; hardened build; 430 tests passed and 2 expected skips; independent Codex and Claude COMMIT verdicts |
| H0c — trusted-launcher activation | Complete | V6 run `programme_v6_h0c_20260828_02` passed the candidate transaction and every hard gate at 100/100, with seven native-evidence digests, two final native reviews, no retry or repair, a sealed schema-V6 envelope, and provider-free verified replay. V4/V5 remain frozen |
| M0 — architectural truth and deterministic foundation | In progress | Backend-aware mapping/query/protocol receipts, exact-commit capture inputs, negative-only host admission, sealed one-shot observation, ELF/seccomp/bubblewrap checks, RFC 9162 checkpoint proofs, the private registration kernel/transaction/materializer, exact PostgreSQL catalogue/deparse oracle, and ADR-0047's independently replayed 4,059-record PUBLIC ACL candidate are implemented as non-authorizing slices. ADR-0046 now also has quoted-callable closure, a Linux-only held-descriptor reader, opaque fixed-root Plan, descriptor-first receipt replay, unbranded descriptor-only capability capture, seven exact terminal singletons, and commit `1e2d88d`'s sealed 36-name lifecycle/control/deadline representation. Node 24 passes 685 private-service tests; exact Node 20 passes TypeScript, 11 focused lifecycle/seal tests, artifact replay, the parent build and 39 focused parent gates. Artifact `e426c495…fdc1` preserves dependency-free public bundle `90e21e7c…f7c3a`. ADR-0042 through ADR-0047 remain proposed. Remaining M0 work begins with exact insert and observation statement/result candidates, cardinality/byte-budget semantics, aliases/serializer/transport bounds and script tag-vector evidence, then a separately attested composition-owned `pg` bridge, adapter-owned store/Promise/protocol brands, runner/deadline execution/live verifier, PostgreSQL credentials, ADR-0047 mutation/hosted evidence, leases/fencing/outbox, independent supervision, actual-consumption closure, final-FD/syscall/cgroup evidence, SBOM/reproducibility, production admission and controlled performance |
| M0 — ADR-0047 final-`WHERE` mutation slice | Complete locally; hosted gate pending | A protected source-specific quartet freezes all 19 final-`WHERE` deletion spans against the 6,859-byte projection. Two ten/nine-mutant serializable rollback-only batches preserve the unchanged raw-derived control bag, prove 15 hidden seed families and four zero-candidate guards independently, and produce exactly 19 executed, 15/15 non-equivalent killed, four guard-equivalent and zero unresolved. Two fresh networkless runs reproduce 11,963,849-byte (`2df643f9…c50f`) and 11,608,234-byte (`688a5ed3…522`) transcripts with cleanup. Its historical service verification is 611/611; the later ADR-0046 Plan slice leaves the public bundle unchanged. Exact Node 20/24 CI runs eight isolated replays under 150 minutes. CASE/JOIN/value/nullability/order/duplicate/array/element and `UNION ALL` replacements, hosted evidence, a versioned run receipt, live observations and admission remain open |
| M1–M7 — application completion | Gated | Existing product evidence remains valid, but no later milestone starts merely because an M0 slice lands; each milestone still requires its own executable QA gate |
Runs `_03`, `_04`, V5 `_05`, and V6 `_01` remain immutable honest failures; `_05`
was rejected at 85/100 by frozen prior-attempt law, and `_01` failed closed on
native-origin policy. The additive V6 contract never reinterprets V4/V5 evidence.
Fresh V6 run `programme_v6_h0c_20260828_02` passed every gate at 100/100 with no
retry or repair. Its policy, candidate, receipt, envelope, execution-claim, and
provider-free replay digests are `e71107e5…ae34`, `a1dc3071…ac7f`,
`d9d244ef…0216`, `02c30ed3…9a06`, `578799ef…9c86`, and `f1bcf0fe…bf02`.
H0c is complete; M0 continues through verified slices and M1–M7 remain gated.

The SPARQL regression receipts bind per-test expected SQLite query and Protocol outcomes. They are regression
baselines only: they do not attest W3C SPARQL Query/Protocol conformance, runtime provenance, or backend admission.
Commit `a84aa05` adds backend-aware v3 mapping receipts: SQLite records 81 pass, one deviation and five skips;
required-live PostgreSQL records 80 pass, one deviation and six skips. The receipts bind sealed inputs and ordered
typed outcomes only—not runner/toolchain/host/provider provenance, Query/Protocol conformance or production admission.
The default `sf-cli` dependency receipt closes locked package resolution, enabled features, and normal/build
dependency edges only. It does not attest binary bytes, build-script output, linker or system provenance, an SBOM,
reproducibility, or production admission.

The current tranche adds a fail-closed **host-observed non-closure observation** for one freshly built current `sf-cli` executable. The first private `0600` external receipt, from clean `5a06eac`, replayed with 363 raw inputs, 357 canonical terminals and three one-hop HostSystem aliases; portable/host/receipt digests are `72ce37b4…9b9a`, `024fbbbd…3ad8` and `173d0698…51ca`. It is uncommitted, unpublished and noncanonical. CI tests only the parser/integration contract on mutable `ubuntu-24.04` and neither captures nor publishes. Linker-only alias authority binds alias topology and terminal bytes while generic authority remains symlink-rejecting; structured GNU-note parsing binds build-ID owner, type, size and digest. The producer still requires an exclusive, quiescent root/effective-UID builder. Same-principal/root ABA, linker time-of-use and path-resolution race resistance are explicitly unattested. The additive runtime-linkage contract canonicalizes strict bounded glibc `ld.so --list` output. Commit `863a058` adds the private descriptor-rooted holder; `c8305c3` adds a private one-shot executor that independently authorizes exact bubblewrap path/digest/length/policy, holds its root-owned inode, revalidates exact sealed-source transfer duplicates, and invokes only that inode via `execveat` with an empty environment, fail-closed FD allowlist, process limits, pidfd/process-group cleanup and bounded cancellable output. Bubblewrap creates a fresh networkless/user-isolated read-only tmpfs containing only sealed-source copies, then runs the copied loader; the strict view must equal prior discovery. Commit `805f413` converts a completed observation to a private canonical record with fixed `authority=none`, 34 `not-attested` fields, domain-separated record/receipt digests, exact tool and binding identities, and bounded raw stdout. Commit `9282e60` checks a caller-supplied closed runtime-ELF tag/search/flag policy ID plus exact five-source byte digest `cd23f2d8…b0a` before construction and during the immediate pre-run validation phase; the native diagnostic maintains a separate literal, but the API authenticates no reviewer. Commit `73e9864` binds separately sealed policy `x86_64-prepared-loader-late-cbpf-default-kill-v1` (`0092c69f…e80a`, 55 cBPF instructions/440 bytes) to both bubblewrap's namespace PID 1/reaper and the copied loader child, requires its single exact FD/argv placement, and proves a same-layout `fstat` control against a `socket`/`SIGSYS` canary. Commit `50adc0a` hashes and parses the same exact held bubblewrap bytes as `RootPie`; `b34b6d7` then creates a separate canonical private `authority=none` inventory under `counterfactual-controlled-name-resolution-not-actual-exec`, deriving interpreter/direct names from the held view and binding environment-cleared, cache-inhibited, empty-hwcaps bounded loader stdout plus replayed names/paths under pre/post bwrap identity/policy fences. `4b14635` brings all five new authority files into the harness protected set. The interpreter, reported DSOs and path-passed target remain unheld/undigested, bwrap is not executed, and replay proves only record self-consistency. Receipt V1's schema and canonical serialization remain unchanged; both `runtime-elf-policy-replay` and `target-seccomp-or-syscall-trace` remain `not-attested`. The digests detect source drift, not the change class, approval, compiled bytes, configuration, dependencies or toolchain. The exact-host workflow keeps both formats in memory, with no writer/importer, signature, witness, product caller or authenticated execution/output provenance. Discovery remains prior and unauthorized; the artifact is not executed; opaque GNU-property/hash/symbol/relocation/version/TLS/cross-table payloads, initialization, `dlopen`/NSS, VDSO and complete runtime closure are unproven; counterfactual names/paths do not bind actual bwrap-host byte consumption, time-of-use, default cache/hwcaps, preload or LSM state; no final-FD inventory, syscall trace or aggregate cgroup containment exists; kernel/bubblewrap/glibc/copy/mount and an exclusive principal remain trusted; and there is no SBOM, reproducibility, minimality, admission, performance or release authority.

Performance machinery exists, and `f2cc800` removes issue-8 lock hardcoding from future V5/V6 patch tasks. Proposed ADR-0041 keeps the first clean-release measurement outside that patch transaction; no controlled runner profile, baseline, candidate, capture receipt, or measured numbers exist. The capture controller now parses a conservative byte-canonical subset of the product runner profile and observes two fixed, read-only `/proc`/`sysfs` snapshots; an AST-walked exact-import/open-call mutation sentinel rejects known command, provider, socket, loader, write-flag and ambient escape forms in the protected current source, but is not an OS capability sandbox. Captured module-private operations distinguish fixed from injected collection locally; durable records prove canonical self-consistency, not independently witnessed collector provenance, which remains required before positive capture.
Its result domain is deliberately only `ineligible | unproven`; either result snapshots immutable profile bytes through typed-array intrinsics, rederives the classification, binds exact input-attested and returned terminal states, and leaves the attempt count at zero. This module cannot emit `pass-host-preflight` or authorize capture; future positive replay must authenticate evidence kind before the generic state transition. Raw source hashes remain recorded, stability uses normalized controls, and CPU lists are parsed independently so malformed evidence cannot hide relations among the valid lists or other disqualifiers. A live diagnostic on this development host returned `ineligible`: allowed CPUs `0-31` are not isolated, governors are `powersave`, turbo control is unavailable, and swap is `33,519,612 KiB`.
Because no canonical tracked profile or capture task exists, this is diagnostic non-admission evidence only, not an authoritative programme run or measurement.

The capture control plane now has a local single-use run claim and its first pre-admission consumer. The claim slot is keyed only by independently supplied project authority and run ID; its immutable body binds controller, task, input attestation, runner profile and expected runner identity. The consumer reopens that rooted claim, re-attests an exact primary or bare controller store, rejects include/filter/config and attribute authority plus foreign-owned or cross-UID-writable Git control/object nodes, bounds the object-authority walk, preflights path-counted blob sizes before checkout, materializes only the claimed commit through a private Git index, seals the source tree, and returns a digest-bound opaque local view. Full path/index/tree inventories reject unsupported modes, symlinks, gitlinks, hard links, `.git`, extras, replacements, output injection and ambient branch/worktree changes; pristine-only cleanup preserves poisoned trees. Host admission remains unevaluated and all lease, attempt, build, execution and capture authority stays false.
The owner-only claim and source roots remain same-UID cooperative controls: they prove neither external append-only durability, rollback resistance nor path-ABA resistance. There is no lease, launch, TTL, reclaim or retry API, and the source view is not persistence proof or a receipt. Tests use temporary synthetic primary/bare stores and roots; no real project claim, source tree, profile, build, receipt or measurement was created on this host.

Commits `99fa2e1` and `92f5376` freeze the non-authorizing signed registration seam; `7139b05`, `1d33638`, `3ee0ed6`, and `d54518f` add RFC 9162 proof verification, shared Ed25519 verification, signed checkpoint parsing, and registration inclusion/consistency replay. Commit `f1a3a48` adds a sealed, private, nonoperational decision kernel; `f604d0f` adds its dormant transaction coordinator, and `3e0ceab` adds ADR-0043's bounded whole-transaction retry prerequisite.
The coordinator keeps write and exact-recovery roots disjoint, makes checkout acquisition allocation-free, captures checkout-local cleanup before the sole `open()`, freezes adapter results, binds staged event and joined-row provenance to the candidate and transaction snapshot, quarantines ambiguous terminal outcomes, and releases response bytes only after literal commit. Valid unrelated global interleaving commits while forged original-registration provenance rolls back; exact recovery cannot read head, run or staging authority.
A known internal abort can trigger at most three fresh attempts after successful rollback/destruction; roots are snapshotted, the peer is consumed once, all decisions and staging are recomputed, and ambiguous commit or cleanup failure is never retried. The protected private materializer exact-key checks roots before traversal, snapshots bounded trap-free graphs, exposes copied signing bytes on a one-use prepared identity, consumes before signature parsing, verifies the pinned Ed25519 SPKI/signature, and emits complete DB-shaped 201/409 rows for genesis/non-genesis and adjacent/interleaved histories.
Commits `28addbc`/`c586973` add the 232,822-byte exact catalogue (`e7ce3572…e69`; 9,125 nodes, 963 records), bounded pre-scan/one-parse contract verifier, caller-pinned digest binder, 123 KATs, and 85 PostgreSQL 16.15 deparse facts with zero fixture mismatch. Commit `f192a8b` makes shared capability admission reject hidden symbol keys before invocation. ADR-0047's exact-platform-manifest-pinned PostgreSQL 16.15 fixture has 4,059 records and 860,988 bytes (`a108e05f…5be`). Its immutable V1 receipt and owned networkless replay bind two fresh anonymous-volume `template0` runs, full profiles/results, independent raw-SQL transcripts and ownership-checked cleanup. Additive V2 receipt `48d54b63…98e8` pins V1, an OID/attribute-aware candidate matrix, and 13,603 effective-privilege checks by a fresh no-membership role across six populated classes; FDW/server remain explicit zero classes. V1's raw oracle independently expands the direct ACL atoms, and V2 corroborates all 4,059 atoms. A protected test-only scanner/reader and opaque non-authorizing fixture brand pass 97 focused tests. Protected pure mutators and fail-closed replay support drive two fresh networkless rollback-only runs for both the 12 branch/record-set mutants and 19 final-`WHERE` deletions. Historical unpublished exact Node 20/24 runs report V1/V2 plus both mutation replays, 611/611 service tests and 905 parent tests with two intentional skips; these run results have no version-bound repository receipt, and passing GitHub-hosted evidence remains open. The private Linux-only reader and opaque Plan add eleven focused KATs; two quoted-callable KATs and seven descriptor-first receipt KATs close their dormant boundaries. Commits `1ef4e64`/`cb60b1f` correct the lifecycle to 36 names with a separate ledger observation and 24-step exact-no-op path, require adapter-branded intrinsic Promises and reviewed statement/result pins, and seal unbranded receiver-free capability captures plus seven frozen non-authorizing terminal identities. Commit `1e2d88d` implements the private non-executable lifecycle contract: all 36 names, 31 execute operations, exact 32/24 schedules, ten control literals and script-count-derived deadlines with 360-second margins. Node 24 passes 685 service tests; exact Node 20 passes TypeScript, 11 focused lifecycle/seal gates, artifact replay, the parent build and 39 focused parent gates. Artifact `e426c495…fdc1` preserves dependency-free public bundle `90e21e7c…f7c3a`. Exact inserts, SELECT aliases/serializer/transport bounds, 52/73 script tag-vector evidence, adapter brands, separately attested composition-owned `pg` bridge, deadline execution/runner, live observations and admission remain open, as do ADR-0047's CASE/JOIN/value/nullability/order/duplicate/array/element/`UNION ALL` and hosted evidence gates.
Both npm audits are clean. Generic MCP/threat-model tools report clean/info but cannot inspect the actual tracked launcher surfaces and remain `INCONCLUSIVE`; the deep generic scanner reports only reviewed fixture/static-string heuristics. The project-owned gate requires exact-empty `.mcp.json`, an exact `.agents/config.toml` digest, and a pinned networkless Ruflo reader over private copies of two status files. A fixed-seed 2×1 Darwin Shield diagnostic passed only 9/12 gates and grants no promotion authority. An ADR-plugin `--help` probe on 2026-09-01 unexpectedly entered the live project importer and may have partially upserted ADR rows before exit; its exact scope is untrusted. No database copy, reconciliation, rollback, or later memory-backed ADR verification was attempted; Git remains canonical. The ignored, untracked local two-task MetaHarness diagnostic suite remains ineligible for Darwin/GEPA evolution, and the retrieval-policy flywheel remains off.
ADR-0042 through ADR-0047 remain proposed. ADR-0046's quoted-callable policy, fixed reader, opaque Plan, descriptor-first receipt, unbranded capability captures, exact terminal singletons and lifecycle/control/deadline contract are dormant non-authorizing representations. ADR-0046 now separates accepted cardinalities, max-plus-one probes and hard row caps, plus raw/encoded/inner/outer byte budgets; dual-native review rejected a value-free code singleton or erasable brand as unfalsifiable scaffolding. Exact per-operation insert/SELECT bytes, aliases, serializer, transport bounds and tag-vector evidence are the next gate and must introduce the runtime representation with actual KAT-backed values. The runner, protocol brands, live verifier, PostgreSQL adapter/writer connection, deployed signer, network, external administrator, lease/fence/outbox publisher, attempt/capture authority and real supervisor events remain absent. ADR-0047 still requires its remaining mutation and hosted evidence gates before activation.

On 2026-08-28, exact commit `ad94cdb` was cloned twice without local hard links under the hardened-builder
`umask 0022`. Each checkout rebuilt the controller, passed all 91 harness files (627 tests passed and 8 environment-
intentional skips), replayed the RDB2RDF, query, Protocol, dependency-closure, performance-scenario, and capability
authorities, remained Git-clean, and produced byte-identical authority and controller digests. The harness correctly
rejected an earlier pair created under `umask 0002` because tracked inputs were group-writable; no trust check was
relaxed. This closes current-tranche checkout repeatability, not binary reproducibility or final agreement.

M0 remains open for complete binary artifact closure, SBOM/reproducibility and production minimality/admission,
plus controlled performance evidence.
Its two-builder gate must pre-register distinct roles/trust roots and run IDs,
commit both complete results before reveal, bind identical inputs, and require
byte-identical artifacts. Missing, failed, mismatched, selected, tiebroken, or
retried results fail the pair; the existing directional comparator is not
agreement. This extends the evidence plane without changing application goals.

The current M0 critical path is:

1. use ADR-0046's frozen cardinality/byte-budget semantics to seal the first complete operation values, then exact insert/SELECT bytes, aliases, serializer/transport bounds and 52/73 tag-vector evidence; implement the pinned `pg` bridge as a separate composition-owned attested artifact before adapter store/Promise/protocol brands, bounded deadline execution/runner and PostgreSQL 16.15 live gates, while closing ADR-0047's remaining mutation/hosted evidence in parallel and only then advancing to leases, resource fencing, outbox and independent deployment;
2. provision and admit the canonical controlled-runner profile;
3. produce one two-builder-agreed exact artifact under ADR-0039; and
4. run one fresh-ID attempt, seal/replay it, then admit only the verified artifact bytes.

M1–M7 remain gated until the full M0 gate passes.

## Outcome

Complete semantic-fabric as a state-of-the-art, virtualisation-only knowledge graph over live relational systems of
record. Completion means exact-or-fail semantics, bounded execution, real cross-source query execution, production
security and operability, and independently verifiable release evidence.

The semantic compiler does **not** need a wholesale rewrite. Two substantial, evolutionary architecture changes are required:

1. lower every advertised global operator to a bounded physical execution path;
2. add source identity, a source registry, and a federated physical plan.

If the second item is removed, the application charter must change from systems
of record/cross-RDBMS federation to one source per deployment. This programme
retains the accepted charter.

## Audited baseline

This is a product-readiness assessment, not a code-quality score. The frozen
2026-08-26 planning baseline is **44/100** and has not been formally rescored;
subsequent M0 evidence is recorded above without silently changing its points.
Hard-gate failures make the repository not release-ready regardless of aggregate.

| Dimension | Weight | Baseline | Evidence-led finding |
|---|---:|---:|---|
| Correctness and standards | 25 | 17 | Strong fixed differential/W3C coverage and sealed mapping inputs/runners; path truncation, one R2RML deviation, incomplete backend receipts, and no generative/fuzz layer |
| Security and governance | 20 | 8 | Bound parameters and partial timeout/pooling exist; no total budget, result/cost cap, TLS, identity, or policy enforcement |
| Federation and architecture | 15 | 3 | Reusable backend abstraction and semi-join cost model; runtime and mapping IR are single-source |
| Performance and boundedness | 15 | 10 | Strong measured simple-streaming and Ontop evidence; global sort/group/dedup retain source-sized state |
| Operability and reliability | 15 | 2 | No production config, telemetry, health/readiness, graceful shutdown, reload, or drift handling |
| Release and product evidence | 10 | 4 | At the frozen snapshot, CI/audit/harness existed but the app lockfile was ignored; broad binary closure, version 0.0.0, and product release proof were absent |
| **Total** | **100** | **44** | **Target ≥98 and every hard gate green** |

Material gaps found directly in the current tree:

| Priority | Gap | Current evidence | Required disposition |
|---|---|---|---|
| P0 | Silent path truncation | `path.rs` fixes `PATH_MAX_DEPTH = 256`; recursive rows retain depth | Exact cycle-safe closure or explicit limit failure, tested beyond 256 |
| P0 | Bounded-memory invariant is too broad | `exec_core.rs` buffers global order, Rust grouping, solution/triple dedup | Composite SQL or bounded spill/merge; otherwise capability-profile `501` |
| P0 | Cross-source charter is not delivered | One `ServeConfig.backend`; no `SourceId`; semi-join planner has no production caller | Source registry, source-bound mappings, federated plan and coordinator |
| P0 | Reproducibility closure is incomplete | `93ae3c2` tracks `Cargo.lock`; `374ca99` pins actions, images and selected tools; `31a1164` installs MetaHarness/Darwin from the npm lock; the package receipt closes resolution/features/edges; external observation `173d0698…51ca` binds one current binary plus observed build/link inputs; the private prepared one-shot holds exact sealed sources and bubblewrap inode and reproduces candidate loader resolution; `805f413` adds only a canonical in-memory non-admission record and semantic reparse; `9282e60` checks caller-supplied live ELF policy identity; `73e9864` adds exact late default-kill cBPF confinement and a comparative native canary; `50adc0a` adds same-byte static bwrap `RootPie` preflight; `b34b6d7` adds a separately replayable counterfactual name/path inventory, but Receipt V1 attests or replays none of those three diagnostics | Accept a production collector design before promotion; bind authenticated held/digested exact interpreter, DSO and target bytes actually consumed by bwrap plus complete dynamic host closure, final-FD inventory, syscall-trace evidence and aggregate cgroup/process containment; add durable authenticated witness/provenance and replay authority plus build-script, tool, linker and system closure; produce SBOM and reproducibility/admission evidence; close hosted-runner, apt-transitive and release-toolchain residuals; repeat the complete M0 gate in two clean builders |
| P0 | Standards evidence is not yet release-complete | `a84aa05` binds all 87 ordered SQLite and required-live PostgreSQL mapping outcomes in backend-aware v3 receipts; mapping-only scope and zero production admission remain explicit. Per-test SQLite query/protocol baselines detect regression without claiming W3C conformance | Add MySQL mapping coverage and the pinned supported-surface SPARQL/Protocol manifests; keep mapping/query/protocol and backend-admission evidence disjoint |
| P1 | Governance covers only part of a request | compile is unbounded; no result/byte/cost cap or common cancellation budget | One `QueryBudget` from ingress through serialization, every backend |
| P1 | Production secret/transport exposure | PostgreSQL `NoTls`; source secrets accepted in argv; parse errors echo conninfo | `SecretRef`, verified TLS, redacted errors and secret-corpus tests |
| P1 | Accepted runtime ADRs are not delivered | ADR-0011/0017/0018 have no production implementation | Implement or supersede with dated status/evidence |
| P1 | Test strategy is incomplete | no `proptest`, fuzz target, `insta`, durable coverage or mutation gate | Generated, fuzz, snapshot, mutation and LCOV trains |
| P1 | Serving artifact is too broad | `sf-cli` imports conformance/bench; conformance enables REST and SQL Server | Minimal serve artifact; opt-in evidence/developer features |
| P2 | Lifecycle and admission are absent | immutable-in-practice epoch; no reload/drift; `M ⋈ T` not a startup gate | Validated immutable snapshot, atomic swap, drift detection and readiness |
| P2 | Maintainability risk | compiler/executor files exceed the 500-line rule by several multiples | Characterize first, then decompose by stage without semantic rewrite |

`cargo audit` passes with six configured advisory exceptions and three
unmaintained crates: `paste 1.0.15` (`RUSTSEC-2024-0436`),
`proc-macro-error2 2.0.1` (`RUSTSEC-2026-0173`), and `rustls-pemfile 1.0.4`
(`RUSTSEC-2025-0134`). This is not clean supply-chain closure: every exception
still needs reachable-feature, owner, expiry, and compensating-control evidence.

## Domain model and target architecture

| Bounded context | Aggregate or port | Current home | Target responsibility |
|---|---|---|---|
| Semantic contract | `RuntimeSnapshot`, T-box, mapping IR, capability profile | `sf-core`, `sf-mapping` | Versioned T/M/schema/source identity and fail-closed validation |
| Query compiler | IQ, optimizer, dialect-neutral physical operators | `sf-sparql` | Exact supported-profile rewrite to single/federated physical plan |
| Source runtime | `SourceRegistry`, `SqlBackend`, backend capability contract | `sf-sql`, `sf-serve` | Source lifecycle, binding, streaming, cancellation, health and admission |
| Federation | `FederatedPlan`, fragment, reducer, global operator | new modules behind existing crates | Per-source fragments, bounded data movement, merge/spill and failure semantics |
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
   ├── RuntimeSnapshot { T, M, schemas, epochs, digests }
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

The dependency direction should move relational schema contracts out of
`sf-mapping -> sf-sql` into a neutral domain/port module. `sf-sparql` should be
decomposed by parse/build/normalize/lower/emit/execute only after the generated
characterization suite exists.

## Programme dependency graph

```text
M0 truth/evidence foundation
  └─► M1 exactness + bounded physical execution
        └─► M2 total governance
              ├─► M3 secure observable runtime
              ├─► M4 standards + generative QE
              └─► M5 snapshot lifecycle, policy + lineage
M1 + M2 + M4 ─► M6 cross-source federation
M3 + M4 + M5 + M6 ─► M7 minimal release + SOTA proof
```

M3 and M4 are intentionally parallel after the request-budget contracts settle.
M5 may start its snapshot model during M3, but policy/lineage promotion waits for
the relevant security and generated noninterference tests. M6 does not wait for
all UI/packaging work, but cannot start before exactness and backend contracts.

## Milestones and QA gates

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
- freeze controlled heap/RSS and latency receipts, while keeping private
  self-consistency records distinct from artifact and supply-chain authority; and
- write subordinate ADRs for the production artifact and the federated global-
  operator/spill choice.

QA gate:

- two clean checkouts resolve the same dependency graph and evidence digests;
- every public claim maps to a test/profile entry or is labelled planned;
- missing/malformed standards inputs fail; no new skip can hide behind a count;
- the programme backlog remains derived solely from the charter, source,
  decisions, standards and executable evidence.

### M1 — Exactness and bounded physical execution

Outcomes:

- replace silent 256-hop truncation with exact cycle-safe closure for each
  admitted dialect, or reject before returning a success response;
- lower global ORDER, GROUP/aggregate, DISTINCT and graph/CONSTRUCT dedup into one
  composite relational plan where sound;
- define the bounded external operator contract for shapes that cannot be pushed
  to one source, with no accidental in-memory fallback;
- close the PostgreSQL delimited-identifier deviation or retain it as an explicit
  release-profile exclusion; and
- characterize then split compiler/executor hotspots into files below 500 lines.

QA gate:

- chains and cycles at 1, 255, 256, 257 and >1,000 hops are exact or explicitly
  rejected, never silently partial;
- every advertised blocking shape has flat heap and RSS under 1×/10×/100× source
  growth; growth from 10× to 100× is within 10% after measurement noise;
- flat/tree/unoptimized/optimized/materialized-oracle results agree;
- unsupported-shape tests assert the exact pre-execution failure class.

### M2 — Total request governance and cancellation

Outcomes:

- introduce `QueryBudget`: absolute deadline, compile work, result rows/bytes,
  source cost, recursion, admission and stream-lifetime limits;
- bound blocking compiler work with admission and cooperative cancellation;
- apply source-native statement/transaction timeouts and capacity shedding to
  SQLite, PostgreSQL, and MySQL;
- propagate disconnect/cancellation to all tasks, streams and connections; and
- return safe RFC 9457-style problem details with correlation IDs, not internal
  driver/schema/credential strings.

QA gate:

- one deadline covers ingress, parse, compile, acquire, execute and serialize;
- timeout/disconnect releases worker and connection capacity within one second;
- overload sheds within the configured wait and never queues without a bound;
- row and byte caps work for every result format without producing valid-looking
  partial success.

### M3 — Secure, observable, operable runtime

Outcomes:

- implement typed layered configuration and redacted `SecretRef` resolution;
- add verified TLS for remote admitted sources and remove credentials from argv;
- implement ADR-0011 request/stage spans, bounded-cardinality metrics, structured
  logs and governance events using a pinned OpenTelemetry conventions version;
- add `/livez`, `/readyz`, service description, graceful shutdown and source
  failure policy; and
- define SLOs for latency, overload, availability, correctness and recovery.

QA gate:

- a seeded secret corpus appears nowhere in argv, logs, traces, metrics or errors;
- telemetry overhead stays below 5% on the controlled benchmark;
- no metric label contains query text, IRIs, source values or other unbounded data;
- readiness fails for an invalid snapshot or unavailable mandatory source;
- SIGTERM drains or cancels within the configured bound without connection leaks.

### M4 — Standards and generated QE train

Outcomes:

- make the exact W3C RDB2RDF inventory fail closed and run equivalent admitted-
  backend coverage for SQLite, PostgreSQL and MySQL;
- add a pinned supported-surface SPARQL manifest and full Protocol/content-
  negotiation tests;
- execute the real upstream-generated `M ⋈ T` closure, retaining synthetic units;
- add valid schema/data/R2RML/SPARQL generators, NoREC, MR1, cross-dialect and
  `spareval` properties with persisted shrunk counterexamples;
- add parser/rewriter/emitter/serializer fuzz corpora, SQL snapshots, mutation,
  workspace LCOV, Agentic-QE gap analysis, fault injection and backend contracts;
- make required live services fail closed in CI while retaining explicit local
  opt-in skips; and
- automate Criterion, concurrent ramp, slow client, disconnect, overload and
  one-hour soak tests on controlled runners.

QA gate:

- exact suite inventory, zero unexpected failures and zero new skips/deviations;
- at least 5,000 deterministic generated cases per PR and 100,000 nightly with no
  divergence, panic, unexpected unsupported result or nontermination;
- bounded fuzz smoke per PR, long persisted-corpus campaigns nightly, zero crash;
- ≥90% mutation score in critical translation/governance modules with no survivor
  that weakens binding, NULL/bag semantics, dedup, recursion or cancellation;
- ≥95% line/region coverage on critical boundaries and ≥90% workspace coverage,
  both ratcheted rather than reset;
- median regression ≤5%, p95 regression ≤10%, and no monotonic RSS growth during
  the soak. Variance above the harness threshold makes the run inconclusive.

### M5 — Snapshot lifecycle, identity, policy and lineage

Outcomes:

- build immutable `RuntimeSnapshot {T, M, schemas, sources, epochs, digests}`;
- validate `M ⋈ T` and source capabilities off-path, then atomically swap; existing
  queries retain their original snapshot;
- fingerprint source schemas, detect drift, invalidate affected plans, roll back
  invalid snapshots and expose readiness state;
- add provider-neutral `SecurityContext`, PostgreSQL transactional `SET LOCAL`
  RLS, portable ABAC/sensitivity enforcement and audit decisions; and
- implement opt-in query-time provenance using mapping/source/row-key and plan/
  policy hashes, never persisted instance data or source values in telemetry.

QA gate:

- invalid reloads never activate; valid reloads are zero-downtime and invalidate
  stale plans exactly once;
- drift is detected within the declared interval and blocks affected readiness;
- tenant noninterference and pooled-context cleanup pass across concurrency,
  cancellation and failure;
- provenance identifies expected mapping/source/row keys and remains bounded by
  returned results and the request budget.

### M6 — Charter-complete cross-source federation

Outcomes:

- add `SourceId` to mapping/source affinity and a capability-aware
  `SourceRegistry`;
- partition the physical plan into per-source SQL fragments and typed global
  operators;
- wire the existing semi-join cost model to IN/temp-table/Bloom reducers with
  skip-if-unselective behavior, streaming merge and bounded spill;
- define global bag/NULL/order/group/OPTIONAL/MINUS semantics, consistency model,
  partial-failure classification, shared cancellation and provenance vector; and
- admit cross-source shapes one by one through the capability profile.

QA gate:

- two-source and three-source differential results equal a trusted materialized
  reference for every admitted shape and failure schedule;
- coordinator heap/RSS is independent of source cardinality within the M1 bound;
- selective fixtures reduce transferred bytes by at least 80%; unselective
  fixtures correctly skip reduction and stay within declared overhead;
- cancellation reaches every source and spill artifact; no partial result is
  labeled successful.

### M7 — Minimal release and SOTA proof

Outcomes:

- split the production server from conformance, benchmark and experimental
  connector closures; give the application a non-zero semantic version;
- define supported platforms/backends, upgrade/rollback/runbooks and a non-root,
  read-only OCI reference deployment;
- gate licences, sources, duplicates, semver/API compatibility, Rust/npm audit,
  time-bounded advisory waivers, SBOM, checksums, signatures and SLSA provenance;
- reproduce the exact release from two clean builders and smoke the packed
  artifact against all admitted backends;
- re-run the fair Ontop 5.5-or-current benchmark, publish wins and losses, and add
  only profile-proven optimizer changes; and
- generalize `coding-harness/` from its single-task driver to reusable task contracts.
  Enable Darwin/GEPA only after 5+5 discriminating/sealed tasks and held-out gain.

QA gate:

- production dependency tree contains only admitted functionality;
- zero unwaived reachable critical/high vulnerability and every waiver has owner,
  dependency path, feature/target reachability, controls and expiry;
- signatures, SBOM and provenance verify independently; clean-machine smoke uses
  the exact packed digest;
- all hard gates pass, product readiness ≥98/100, and independent Codex and Claude
  verification agree. A harness diagnostic or model score contributes no points.

## Ruflo and MetaHarness execution model

For M0–M7 code changes, use the ADR-0037 control plane proportionally:

1. **Recall and route.** Search repository and user Ruflo memory; freeze task,
   standards, evaluator and route snapshots. Native subscription hosts only.
2. **Specify.** Researcher and domain architect write executable acceptance law.
   Security/performance specialists join only when their boundary is touched.
3. **Design duel.** Native Codex and Claude propose independently, cross-critique,
   synthesize once, and independently verify the design. Missing host fails the
   architecture gate rather than triggering an indirect-provider fallback.
4. **Parallel implementation.** One writer per isolated branch/worktree; tester
   owns protected evaluators; no writer may change sealed oracle inputs.
5. **Verify and repair.** Build the patched candidate, run applicable public,
   independent, live, mutation, security, performance and regression gates; allow
   bounded verifier-directed repair with full rerun, retry/breaker/cancellation.
6. **Integrate.** One integration owner applies accepted patches in dependency
   order and produces digest-chained evidence. Ruflo state proves coordination,
   not product correctness.
7. **Learn carefully.** Persist verified patterns/outcomes. Diagnostics remain
   read-only signals. This does not opt into the retrieval-policy flywheel. It
   stays off unless separately and explicitly activated after every gate passes;
   passing gates alone does not activate it. No evolution starts before the 5+5
   holdout and reward-hack controls pass.

Native Codex/ChatGPT and Claude subscription invocations have no project-imposed
provider-dollar spend ceiling. `subscriptionCostUsd: 0` records zero marginal
provider-API charge in the receipt/routing ledger; it is neither a budget or cap
nor a claim of unlimited subscription capacity. Task, turn, wall-clock, output,
concurrency, first-party rate-limit backoff, retry/repair, resource, and receipt
limits remain operational safety controls. Do not set or consume provider API
keys and do not route through OpenRouter. Native-provider exhaustion or
unavailability fails the affected gate closed; it never authorizes an indirect
fallback. References elsewhere in this programme to query “cost” or
`QueryBudget` govern source work and result resources, not model-provider spend.

Small documentation/status-only corrections use the same truth and review rules
without paying for an irrelevant full harness transaction. Product behavior,
security boundaries, evaluator law, or release controls always use the full lane.

The H0c V6 pass also exposed an engineering-efficiency target: its isolated Rust
verifier outputs peaked at about 35 GiB and the controller spent material time
rehashing them before sealing, although cleanup completed and all resource gates
held. A subsequent harness slice should benchmark and introduce immutable,
content-addressed Rust closure/build reuse plus per-lane digest manifests. It
must retain separate writable targets, verifier independence, exact input
bindings, fail-closed cache validation, and replayable receipts; speed or disk
savings cannot weaken those controls.

The 2026-08-31 generic MetaHarness diagnostic scores the repository root 75:
fit 75, compile 100, task coverage 65, tool safety 90, and memory usefulness 46.
Its six hard constraints pass, but the generic scorer remains distinct from the
ADR-0037 product and promotion gates. Generic OIA and launcher scans report
clean/info without reading the actual tracked `.mcp.json` and `.agents/config.toml`,
so those results are `INCONCLUSIVE`. The project-owned admission gate instead
requires exact-empty MCP JSON, an exact agent-config digest, and rejects TOML,
Unicode, duplicate-key and extra-key launcher indirection. The pinned local Ruflo
reader reconstructs only its already-attested schema-v2 CLI closure from a protected two-file
gzip overlay without opening ambient targets; an opaque snapshot and three exact Git-backed
packed-runtime resources change no identity, export, fetch/provider authority or application goal. Earlier 905-test evidence predates this overlay; the current run passes 124 files/941 tests with two intentional skips. It copies only exact status files and runs
under `--unshare-net`; network/type/link/mode/race KATs keep `0664` evidence non-authoritative.
The 2026-09-01 ADR-plugin `--help` incident may have partially upserted the live project ADR namespaces; no copy, reconciliation, rollback, or later reliance occurred, and Git remains canonical.
A fixed-seed Darwin Shield run passed only 9/12 gates; no valid champion or serving
epoch exists, product tests remain authoritative, and the retrieval flywheel is off.

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
| Federation becomes a rewrite | Schedule and semantic drift | Preserve compiler; introduce SourceId/registry/physical plan behind ports |
| Spill substrate conflicts with ADR-0006 | Hidden architecture reversal | Separate design-lock ADR and benchmark both implementation choices |
| Backend behavior diverges | One green dialect masks another | Shared backend contract plus fail-closed live matrix |
| Policy taxonomy is unavailable | Platform coupling or stalled delivery | Provider-neutral SecurityContext/Policy port and reference fixtures |
| High gates become flaky | Teams bypass evidence | Controlled runners, variance classification, quarantine with owner/expiry |
| Supply-chain exceptions become permanent | Known reachable exposure | Reachability proof, owner, controls, expiry and release review |
| Harness optimizes its evaluator | False programme progress | Protected inputs, independent product gates, sealed holdouts, reward-hack scan |
| Documentation drifts again | Misleading claims and planning | Generate capability/status tables from receipts; update ADR with each slice |

## Definition of done

The programme closes only when:

- every accepted ADR is implemented with current executable evidence or is explicitly superseded;
- every advertised query/profile/backend cell is exact, and every unsupported cell fails before a valid-looking response;
- bounded heap/RSS, total budgets, cancellation and overload gates pass for every admitted operator and backend;
- identity/policy/provenance, snapshot lifecycle and operability gates pass;
- cross-source differential, boundedness, reduction, cancellation and failure semantics pass;
- the minimal immutable artifact and its supply-chain/release evidence verify on clean machines; and
- the score is at least 98/100 with no hard-gate failure or inconclusive evidence.

Anything less may be a useful preview or interim single-source production
profile, but it is not the completed application described by the charter.
