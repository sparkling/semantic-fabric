---
status: accepted
date: 2026-09-07
updated: 2026-09-12
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

**Optimizer proof boundary (2026-09-12):** controlled compilation applies prospective logical-work admission and sticky cancellation to the complete optimizer cascade: candidate search, dependency inference, column/template rewrites and nested branches. Refusal discards the candidate, never falling back to raw execution. Raw compatibility remains uncontrolled but shares corrected proofs: FD elimination covers every dropped-alias reference, including filters/later OPTIONAL conditions; FK optional promotion requires catalog-proven extra predicates; composite FK elimination requires one-to-one key membership and simultaneous, non-cascading substitution. Malformed raw FK/key vectors cannot establish match/uniqueness. These are correctness restrictions, not feature exclusions. Serving still quarantines unverified constraints; public fixtures do not claim quarantined rewrites execute. The programme ledger records slice verification; cache/source obligations and G1–G6 remain open, without physical allocator/preemption or release-completion claims.
**Cache logical-work boundary (2026-09-12):** ordinary and security caches prepay nonblocking lookup probes and bounded insertion/clock/rehash work against construction-time reserved geometry. Exact canonical comparison pays in 256-byte chunks; sticky refusal cannot become a cache miss. Reservation prevents growth, not tombstone rehash; only residents rehash full fixed keys, while ghosts reuse stored hashes. Scope/profile/security identity and raw/shared reuse remain unchanged. Fixed identity accounting is 512 logical units on supported at-most-64-bit targets; oversized construction retains the existing infallible allocation/panic boundary. Default-corpus checks retain the one-million-unit serving allowance. Earlier phase tests pay only the added lookup prerequisite and use the actual 64-entry geometry; public ordinary/bearer tests cover unpaid publication, warm equality, held-source refusal and exact recovery. This does not bound physical allocator/last-Arc destruction or close G1; the delivery task binds final verification and integration. Combined compiler/cache coverage is now established by the integrated phase chain and independent entry-path audit; the programme ledger binds combined acceptance. This closes G1b logical compiler/cache coverage only. G1c post-compile resource admission now pays a depth-checked, single-pass classifier and root ORDER inspection inside the existing compiler worker, including resident plans and every federated fragment. A fixed five-state accumulator avoids temporary alias inventories on the controlled path; raw classification stays compatible. Semantic rejection retains 501 while terminal control errors retain their own mapping. Public ordinary/bearer and two-fragment refusal/recovery tests and exact/N-1/every-stop classifier checks are bound by controlled-resource-admission-20260912. Source/recursive execution, terminal cleanup ownership, G1d and release completion remain open.

**RESOLVE schema authority (2026-09-11, verified in `308d75b`):** the tree RESOLVE D1/D2 path constructs one interruptible, uniquely named schema map before duplicate-elimination mutation and shares it across pooling groups.
Duplicate table names now return an explicit ambiguous-schema error, including
identical duplicates: an arbitrary unstable-sort winner must not grant key or
column-type authority. This tightens tree compilation validation; it is not a
claim that historical raw optimizer helpers already rejected ambiguous input.
Unique-name profiles, PK/UNIQUE/nullability rules and physical-source authority
remain unchanged. The programme ledger records verification and remaining work;
this decision neither closes G1 nor activates `GovernedV1`.

**DESCRIBE accounting correction (2026-09-11):** isolated constant-target binders receive deterministic fresh names before work accounting and QueryV1 transfer.
Direct parsing uses the same helper; the governed parent does not walk again. This is alpha-renaming, not authored-spelling provenance: indistinguishable authored isolated constant BINDs may also be renamed; observable/ambiguous binders remain intact.
Containment/post-parse envelopes remain required. Raw parsing retains oversized ASTs on validation refusal; governed compilation/workers reject. Separate parser/cache names preserve parsed/string reuse.
Forced 1–32-character names and worker/direct cold/warm exact-budget tests cover this correction, not parser CPU/heap governance. Defaults, DESCRIBE scope and remaining G1 obligations are unchanged.

**Aggregate parser correction (2026-09-11, integrated/verified at `87f48a7`):** the same parser-boundary normalization now covers uniquely defined, non-observable internal aggregate binders using the existing cache role analysis. Authored bindings, grouping/project/template escapes, ambiguous definitions and the SELECT-star visibility guard retain their names. Fresh parser names cannot capture authored names. This prevents random upstream aggregate names from changing controlled compilation work between identical requests. Direct and isolated parsing share the helper before accounting/wire transfer; raw oversized-AST compatibility is unchanged. Repeated aggregate aliases retain one aggregate result and copy its exact RDF term to each alias; unique bare aliases retain direct renaming. These corrections do not widen supported aggregate arithmetic or close G1/G2. All ten declared delivery checks and independent native Sol review passed; the harness verified the exact integrated commit.

**Updated 2026-09-07:** implement the delivery review's main-only integration, native-builder/model-effort, proportional-check and queued course-correction rules. Public portable equality-row authorization, safe layered configuration and verified remote-source TLS close narrow product boundaries, not general ABAC or operability. A standalone serving-only Cargo build excludes conformance/benchmark crates and extra backend features while preserving SQLite/PostgreSQL/MySQL and all serving controls. Opt-in authored reload now validates and publishes complete generations off-path, immediately fences detected drift, preserves request leases and fixed caller policies, and cannot heal shutdown/worker panic. Required real-CLI evidence covers SQLite and encrypted PostgreSQL/MySQL single/mixed-source reload and invalid-input recovery. Backend DDL leases, policy/configuration hot reload, product completion and the exact release bundle remain open.

This decision is **accepted**. It replaces ADR-0038 as the controlling definition of product completion and release work for v1. ADR-0038 remains an auditable record of the broader SOTA programme; its research and advanced-assurance work becomes a labelled post-1.0 backlog.

**2026-09-08 public Direct lifecycle:** the closed PostgreSQL 16.9/16.15 PK-backed profile is now wired through `serve --direct-mapping-base`, with independent review and required owned-TLS CLI evidence for exact authenticated SELECT/ASK/CONSTRUCT/lineage, traffic-independent drift and successor activation, fixed startup ontology, incompatible DDL rejection/recovery, bounded shutdown and wrong-CA/no-PK rejection. Independent request/control pools retain resolved trust; mandatory five-second observation (nonzero reload interval overrides), 30-second control and 60-second startup bounds are fixed independently of requests. Native candidate and cleanup ownership prevents overlapping retries and late publication; shutdown counts both lanes under the existing drain plus three-second forced allowance. Row-policy profiles, additional sources and other backends remain excluded from this Direct profile, not from authored serving. Other backend-generation guarantees, total compiler/source controls and exact-release qualification remain open; no hard CPU-preemption or production-admission claim is made.

**2026-09-10 protected authored PostgreSQL:** `--require-verified-generation` plus nonzero reload adds the ADR-0050 lease to one authored PostgreSQL 16.9/16.15 public-base-table/read-all profile, without imposing Direct Mapping's PK requirement. No source row policies, raw SQL or companion source are admitted to this explicit profile; existing modes remain unchanged. Legacy compiler tables and rich lease facts come from the same locked snapshot. Required owned-TLS public checks cover exact authenticated forms, DDL conflict, schema/file drift and repair, complete old responses across successor activation, disconnect/deadline/forced-shutdown cleanup, unrelated public-schema coupling and invalid-profile/budget refusal. The common lease, independent control pool, exact origin/source/mapping binding and serialized shutdown-aware reload are reused. Other backend/policy generation guarantees and exact-release qualification remain open; this is not whole-application completion or expanded v1 scope.

**Lineage qualification (2026-09-08, consolidated 2026-09-11):** authoritative
[ADR-0017 profiles and required commands](ADR-0017-provenance-lineage.md) cover opt-in
constant/multi-map SELECT/CONSTRUCT and bounded two-source UNION/join actual-origin
lineage, with pinned snapshot/plan/policy identifiers, finite fail-terminal output,
separate graph metadata and response-wide blank-node identity. Mapping candidates,
hidden keys, filtered rows and empty joins do not acquire invented provenance.
Required SQLite HTTP and pinned PostgreSQL 16.15/MySQL 8.4.11 TLS CLI evidence covers
the declared bags, policy, limits, source affinity, stop/sibling isolation, recovery
and authored reload profiles; held-query coverage is not every graph/operator/order.
[ADR-0018](ADR-0018-security-edge.md) retains source-RLS A/B/A caller isolation,
cap-one reuse and constant-lineage body-drop/policy-error/deadline evidence.
These records replace the incremental lineage diary here, not its tests or scope.
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
products and exact left-branch copies, then conservatively reserves the same right
branch before direct field copies, without a shadow clone. Authenticated HTTP tests
prove pre-source rejection, exact VALUES bags, recovery and completed-cache reuse;
inclusive bounds, cancellation and path-guard precedence are test-locked. Unifier
allocations, nullable sets, extra left-internal copies and other compiler work remain open. Atom resolution now
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
Ordinary/security cold and warm cache paths now prepay canonical UTF-8 output, logical geometric capacity growth/relocation and one hash. The existing iterative AST walk also prepays visits/collections/payload and stack allocation/relocation; its actual Extend/project/variable counts conservatively prepay hidden formatter searches and logical projection payload before recursive Display (2026-09-10, ADR-0010).
Exact/N-1, expanded UTF-8, raw/controlled shared reuse, preparation/hash cancellation and held-source HTTP tests protect identity, policy precedence and permit recovery. Independent preparation tests cover empty cells, duplicate projects, EXISTS/root reentry and overflow; later-phase tests pay prerequisite key work. Warm hits still avoid compilation work.
Controlled ordinary/security cache lookups and insertions now try each lock once without waiting (2026-09-10). Contention causes an optimization miss/skipped write, not a retry or contention error; authoritative compilation retains request control and exact scope/security identity. Real held-shard tests prove completion before release, unchanged resident identity and later shared reuse, plus budget/cancellation/deadline and policy-first errors. Raw APIs remain blocking/interoperable.
Structural BUILD now uses the same control for every supported AST-to-IQ arm, including GRAPH/EXISTS, visits, source-bound copies, collection slots/logical growth/relocation and stable scope comparison/copy work (2026-09-10). An uncached-entry depth envelope, exact/N-1 raw-tree comparisons and per-charge cancellation/deadline checks protect the boundary. Required public tests reject unpaid BUILD before a held source, recover compiler capacity and retain ordinary/security warm hits without rebuilding. Later-phase tests pay BUILD independently to preserve their original targets. Infallible Clone/Box/string/map allocation, physical overgrant, destruction and other compiler/source/release work remain open; this does not activate GovernedV1 or close l-query-budget. Clone measurement now also prepays iterative traversal and logical stack growth under the same control, checking every step including absent/UNDEF slots (2026-09-11). Its separate work is retained on copy refusal; raw clone metrics/envelopes remain unchanged. Hand-counted walker and caller/public regression checks cover this additional boundary. Constant-row NORMALIZE now also prepays UNION visits/folding/reordering, DISTINCT comparisons/removal, SLICE planning/movement and constant-expression probes/materialization (2026-09-11). Required exact/N-1, per-charge cancellation/deadline, default-stack, malformed-header and public pre-source/recovery/warm-hit checks preserve bags, UNDEF, projection guards, contiguous runs and residual offsets. This closes the stated row-rule boundary. Structural NORMALIZE additionally prepays node/condition/EXISTS traversal, stable scopes, construction composition, INNER/FILTER/left-only LEFT distribution, logical collection/box/map work and substitution merging (2026-09-11). Checked conservative logical unification admission follows paid exact-input measurement and preserves the current ordered Sat/Empty/Unsupported law; it is not physical allocation proof or in-call preemption. Required raw-equivalence, exact/N-1, default-stack node/condition depth and per-charge cancellation/deadline checks pass; ordinary/authenticated public checks prove unpaid normalization fails before a held source, capacity/result recovery and key-only warm reuse. Independent downstream prerequisites retain the original clone/product rejection boundaries. A mapped-table positive control now localizes charges through the existing NORMALIZE span and proves both cold and paid-key-only warm requests reach held source admission before release, exact duplicate-bag return and capacity recovery; source-free negative fixtures alone do not prove this boundary. Remaining logical compiler/source/backend controls and exact-release gates stay open. Exact allocator-byte/overgrant or bounded derived-drop proof is a non-claim, not an independently required research programme; finite admitted growth and cleanup stay required. No GovernedV1 activation or whole-application completion is claimed. OPTIONAL expansion now prepays fast L candidates or decomposed 2*L*R match/anti-match candidates plus L owned tails, logical result-vector growth and owned condition/join appends (2026-09-11). Same-source whole-branch reservations precede direct copied fields; SubPlan nullable-left scalar copies are separately paid. The original left branch moves into each no-match tail without a shadow clone. Required exact/N-1, prospective overflow, independent dispatcher-cost, raw-equivalence and every-charge cancellation/deadline tests cover fast, ordinary/forced decomposition, nested-right and DISTINCT-SubPlan paths. Mapped ordinary/authenticated HTTP checks observe the LOWER span and independently measure its cost, reject before held source admission, then prove exact duplicate/UNBOUND bags and compiler/source recovery on funded cold and input/key-only warm requests. Existing sliced-SubPlan rejection is unchanged. This closes candidate/result-vector/direct-copy admission only: generated ON conditions, nullable sets, FILTER/unifier temporaries, other structural LOWER/compiler/source work, backend guarantees and exact-release checks remain required. It does not activate GovernedV1, prove physical heap/drop behavior or establish application completion. LOWER entry/scope update (2026-09-11): the actual root and nested SubPlan alias walk now pays IQ/condition/SQL visits, including literal-only IRI-template parts, and checks combined recursive depth and the initial alias successor. Spine dispatch, stable output-scope construction, explicit projection copies and final ordered result-variable materialization share request control; implicit fallback remains lexically sorted/unique with paid comparisons and logical growth. Required exact/N-1, raw-equivalence, UTF-8/project-order/duplicate/slack, every-charge cancellation/deadline, overflow and default-stack depth tests cover this boundary. Existing clone/product tests independently pay the preparation prefix and keep their original failure points; successful budgets alone add final materialization. Three mapped ordinary/authenticated HTTP shapes prove unpaid LOWER scope never enters the held source, funded cold/key-only warm requests do enter, and exact bags, renamed fields, head order and permit recovery survive. Internal alias reservations now use checked prospective ranges on LOWER SubPlan/path/SQL-group and shared Unfolder/flat-pool paths (2026-09-11). One work unit per identifier precedes use; overflow and terminal refusal preserve counters, path batches and two-alias SQL pool arms. Resource errors propagate rather than falling back to Rust grouping. Exact/raw, N-1, overflow and per-charge cancellation/deadline checks cover this boundary; mapped ordinary/authenticated single/UNION GROUP BY requests prove pre-source refusal, exact funded cold/key-only warm results and capacity recovery. This is not whole-helper traversal accounting. The COUNT/constant-DESCRIBE identity defect is now repaired (2026-09-11): a prospectively paid cache-only whole-query copy normalizes unique non-exposed aggregate binders and isolated constant-IRI DESCRIBE targets. Fresh names, preserved authored bindings/outputs and exact canonical equality prevent capture and cross-query/scope/profile/security reuse; ambiguous roles retain safe misses. Raw/controlled/security independent-parse shared hits, exact/N-1/no-publication and every-charge interruption checks pass. Mapped ordinary/authenticated single/UNION COUNT HTTP checks prove paid-key refusal before source, cold/warm source admission, exact counts and capacity recovery; observed parse-only warm stages prove reuse without assuming random-name work totals are identical. This does not mutate the submitted AST/plan, activate GovernedV1 or establish general alpha-equivalence (ADR-0007). Other operator helpers, generated conditions/unification, source/backend controls and exact release remain open. No GovernedV1, whole-compiler, physical-heap/drop or application-completion claim. OPTIONAL helper update (2026-09-11): scan, match/anti decomposition and pure-SubPlan OPTIONAL now prepay nullable/derived-alias inventories, shared-name comparisons, depth-checked borrowed term/graph/segment scans, NULL-safe condition vectors/column copies and generated-condition/deferred-binding appends. They no longer clone column inventories or shift NULL-safe disjunctions. Required exact/N-1 and every-charge stop tests retain raw semantics, graph-scoped blank-node absence, aggregate nullability and sound-501 boundaries. Two mapped ordinary/authenticated chained OPTIONAL paths, including DISTINCT SubPlan, reject at the observed nullable-alias helper before held source admission, then recover exact duplicate/UNBOUND-coalesced results, right-only bindings and compiler/source capacity on funded cold/key-only warm requests. OPTIONAL unification update (2026-09-11): all four fast/match/anti-match/pure-SubPlan callers now reuse `CompileContext::unify_terms`: separately paid exact-input measurement precedes the existing checked 512 + 64-times-input logical allowance. Funded Sat/Empty/Unsupported order and messages, early guards and raw behavior are unchanged. Exact/N-1 and every-charge cancellation/deadline tests cover all verdicts; mapped ordinary/authenticated fast, UNION decomposition and DISTINCT-SubPlan HTTP tests observe every actual call and refuse at the final unifier before held source admission, then recover exact bags/UNBOUND and compiler/source capacity on funded cold and key-only warm requests. OPTIONAL FILTER update (2026-09-11): fast and match/anti-match paths measure the expression, prospectively pay all key comparisons per variable occurrence and measure only referenced TermDefs before checked 1024 + 128*(expression+1)*(lookup-footprint+1) construction admission. Source validation now pays actual recursive source/projection visits, comparisons and logical output growth, preserving derived-plan fanout, DISTINCT order, aggregate/SQLite AVG positions, first-match/error order and the existing path-decoder refusal. FILTER output appends and anti-FILTER entry searches are paid without moving R5 conditions. Exact/N-1, arithmetic, UTF-8/layout, depth and every-charge cancellation/deadline tests preserve raw conditions/refusals; mapped ordinary/bearer fast and UNION HTTP tests observe both actual phases, reject each before held source admission, then recover exact duplicate/UNBOUND bags and capacity on cold/key-only warm requests. OPTIONAL shape/R2 update (2026-09-11): initial opts and constant-binding scans now pay visits and UTF-8 comparisons, retaining first refusal, Const-variant decisions, RefAtom decomposition and pure-SubPlan priority. R2 right-only insertions and nullable COALESCE construction pay current-map comparisons, logical entry carriers and boxes; matched WHERE/core/SubPlan vectors pay growth and relocation independently of allocator slack. Fast/matched left payloads move only after FILTER lowering; pure-SubPlan scalar copies retain exact-source admission. Hand-counted schedules, exact/N-1 and every-charge cancellation/deadline checks preserve decisions, copy boundaries and left-first values. Mapped ordinary/bearer shape and chained scan/DISTINCT-SubPlan tests observe the actual phases, reject before held source admission and recover exact duplicate/UNBOUND-coalesced bags and capacity on funded cold/key-only warm requests. Ordinary IQ FILTER update (2026-09-11): iterative group peeling, per-branch/group visits, boolean condition-tree depth, vector/box construction, exact borrowed SQL copies and visible WHERE growth now share request control. Expr construction reuses the measured-input allowance and then the actual source validator; Sql remains pass-through, earlier branches borrow and the final branch moves. EXISTS/NOT EXISTS/MINUS keep their existing delegation and independent clone-refusal boundaries; their inner work and combined recursion are not newly qualified. Exact hand-counted ownership/allocation schedules and every-charge cancellation/deadline tests preserve raw dialect decisions. Mapped ordinary/bearer plain, UNION and OPTIONAL-plus-FILTER HTTP paths refuse at actual construction, validation and append phases before held source admission, then recover exact cold/key-only warm bags and permits. The unchanged default-admission typed-literal OPTIONAL/outer-FILTER regression also passes: irrelevant binding payload is excluded, not actual lookup/copy work, and neither production limits nor the conservative multiplier is relaxed. Remaining compiler/source/backend/release guarantees stay open; no in-call raw-helper preemption, physical-heap/drop, GovernedV1 or completion claim. BIND/substitution update (2026-09-11): ordered dependency passes and pending reference vectors, resolved/existing definition copies, binding searches/edits and nullable/unification/WHERE output now pay request work. Supported BIND constants, variables and recursive CONCAT use internal depth, copy, lookup and output checks; unsupported Debug errors use measured pre-admission without changing text or retry order. Borrowed left-SubPlan scans retain actual-column semantics without allocating column inventories. Post-modifier folds now discard proven-disjoint branches; the regression covers DISTINCT, Slice, OrderBy and SQL aggregation. Exact/N-1, every-charge cancellation/deadline and default-admitted ordinary/bearer plain/UNION/OPTIONAL BIND requests prove pre-source refusal, funded cold/key-only warm exact bags and permit recovery. Aggregate helpers and other compiler/source/backend/release controls remain separate; no whole-compiler, physical allocator/drop or completion claim. Construction/UNION update (2026-09-11): all three Construction projection paths prepay bool decisions, ordered project comparisons and extra_keep membership, then the bounded in-place retain pass; failure before that pass leaves bindings unchanged and retained term payload is not cloned. extra_keep iteration admits reported HashSet capacity and scans every element for order-invariant work, a logical admission convention rather than a physical bucket/probe proof. Construction output capacity is reserved at the original point; UNION children lower in order before paid per-branch moves and logical geometric growth/relocation. Exact/N-1 and every-charge cancellation/deadline tests cover UTF-8, duplicate projects, capacity slack, extra-only component retention and owned/borrowed/post-spine branches. Ordinary/bearer plain, UNION, OPTIONAL and DISTINCT HTTP checks observe actual retention/output phases, refuse before held source, recover exact cold/key-only warm bags and permits, and retain default admission. SubPlan/aggregate projection and other compiler/source/backend/release work remain open; no physical destruction or whole-compiler/completion claim.
Finite cumulative work bounds requested capacity, not allocator overgrant or upstream infallible projection allocation/cancellation inside prepaid scans. G1a base/VALUES work (2026-09-11) now prospectively admits singleton/scan carriers, row/cell/key/map materialization and INNER seed/child visits, preserving moved payload, duplicate/UNDEF bags and child/error order. Exact/N-1/every-stop and ordinary/bearer actual-phase public checks cover early refusal, funded cold/key-only warm results and recovery; the programme/catalogue hold the scoped evidence. G1b's one-target DESCRIBE form now uses a checked input-derived envelope for its complete copy/scope/inventory block, separately paid collision attempts and output copy. Scoped exact/N-1, every-stop, depth/overflow and ordinary/bearer HTTP tests retain hygiene, graph sets, early refusal, cold/key-only warm results and capacity recovery at unchanged defaults. Initial RDF-star rewriting now has separate per-invocation control: typed envelopes precede each rewrite, while inventories, fresh collisions, environment operations, amplified copies and VALUES/basic-encoding output growth are dynamically paid. Raw/exact/N-1, stop, depth/overflow and ordinary/bearer actual-stage tests cover default-funded cold/key-only warm exact bags and recovery. This handles nested FILTER/UNION/EXISTS amplification without assuming a global polynomial output bound; RDF-star realization now prospectively admits each recursive composition, template/name copy, binding operation and exact ordered nested-projection guard. Phase-specific ordinary/bearer SELECT/CONSTRUCT/DISTINCT tests cover unchanged-default cold/warm results and recovery. This is logical admission, not in-call raw projection preemption. Constant-DESCRIBE parser-generated hidden-name work nondeterminism remains an identified compiler/cache obligation. Remaining admitted compiler/cache/source/cleanup obligations are G1b–G1d, not an open-ended helper queue; no `GovernedV1`, physical allocator/drop or whole `l-query-budget` completion claim follows.

**Parser-lifetime correction (2026-09-08):** an authenticated, sub-ingress-limit
query reproduced a server-process abort. Public compilation now uses a prepared
Rust parser process on Linux x86_64 GNU, including security, lineage, preflight
and bounded federation. The held executable is verified before readiness;
embeddings explicitly supply `ParserRuntime`, or readiness/compilation fail closed. The 2026-09-09 debug-image regression scopes the 512 MiB full-file SHA ceiling to diagnostics; runtime retains bounded ELF/build-ID reads and all held-descriptor, metadata-drift and launch checks without scanning debug sections.
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
cancellation tests remain required; no capability or backend is removed. Profile-scope reconciliation (2026-09-11): l-authz, l-observability and l-protocol describe the broader unadmitted ABAC/external-issuer/policy-hot-reload, full telemetry/OTLP/SLO and full pinned-Protocol programmes, not additional v1 prerequisites. Their broad cells remain planned and non-advertisable; the flags are non-blocking only for that extra breadth. Existing admitted security-context/cache/row-policy, bounded logs/metrics, strict HTTP, native TLS/cleanup and rejection checks remain required. Required backend/snapshot/cancellation and exact-artifact qualification remain blocking in l-production-admission, l-snapshot, l-deadline-cancellation, l-transport-security and l-release-artifact; this does not remove any backend, advertised format or runtime guarantee.

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

This is an explicit priority and evidence-scope change, not an implementation claim.
Moving an item to post-1.0 does not make it complete, supported, or
production-admitted. ADR-0002's virtualisation and cross-RDBMS product charter,
ADR-0048's Rust runtime boundary, and the accepted security, governance, and
operability contracts remain in force. ADR-0037 remains the accepted engineering control-plane design; its full transaction is no longer a per-commit gate.

## Context

**PostgreSQL NUMERIC decoder delta (2026-09-09):** the native reader now correctly
interprets the unsigned digit count, preserving 131,072-digit IRI substitutions
and their display scale. Focused wire and authenticated owned PostgreSQL TLS CLI
evidence cover this boundary; the query-profile aggregate includes that check.
This removes a prerequisite decoder defect. The subsequent qualified static
NUMERIC IRI slice now passes listed/fixed/`=`/`sameTerm`, scale-sensitive D1 and
DISTINCT/COUNT, mixed natural consumers, ordering/slicing, native Ref keys and
authored SQL-expression result checks. Original decoder validation survives
hidden COUNT/ASK/OPTIONAL; portable policy excludes invalid denied values and
cap-one requests recover. Raw NUMERIC output and Rust construction remain intact.
Required owned PostgreSQL TLS aggregate evidence covers this public slice;
ADR-0015/0024/0034 record the precise authority. The existing source-sized
multi-arm DISTINCT serving gate is unchanged, not newly qualified by compiler
UNION tests. Natural decimal reconstruction now normalizes full-range lexical digits in Rust: owned TLS CLI checks cover 131,072 integral/16,383 fractional digits, SELECT/DISTINCT/COUNT/ASK, mixed raw-IRI identity and output-cap recovery (ADR-0015). PostgreSQL/MySQL natural decimal fixed/sameTerm keys now preserve exact spelling/datatype/NULL identity, with a separate full-range integer/decimal Eq/Ne value lane, invalid-lexical errors and PostgreSQL policy-before-validation checks. General natural BGP/mixed identity, ordered/floating arithmetic and other exactness/release gates remain open.
The workspace GTFS Q5 regression is repaired by retaining NULL-tolerant decoder checks on the same row during verified-key self-OPTIONAL elimination (ADR-0007); constraints do not erase validation or expand serving authority.
**Explicit datatype correction (2026-09-09):** matching the live natural datatype now uses natural construction and qualified identity, rather than always applying a raw lexical override. ADR-0015/0034 qualify PostgreSQL/MySQL decimal, MySQL DATE/DATETIME and SQLite Boolean/dateTime/dynamic-double public paths, canonical DISTINCT/GROUP BY, mixed-IRI payload preservation and bounded literal join reduction. Different-datatype/language cases retain their separate semantics; invalid matching dates fail terminally with policy isolation and admission recovery. The subsequent natural/explicit column BGP slice closes premature xsd:string pruning: SQLite HTTP and owned PostgreSQL/MySQL decimal/integer CLI checks cover exact BGP/EXISTS/OPTIONAL bags, lexical overrides, type mismatches, NULLs, integer error recovery and policy-before-validation. Datatype evidence is distinct from canonical-key authority; MySQL Integer/Boolean UNION promotions reject without a resulting-decoder proof. Broader native/floating/temporal and coercing-UNION identity, arithmetic and exact-release gates remain open; no release-blocking flag or accepted scope changes. PostgreSQL FLOAT4/FLOAT8 now additionally have decoder-owned exact raw/scientific identity keys and raw-preserving signed-zero/NaN dedup. Owned TLS CLI tests cover both-width edge values, natural/matching-explicit/textual BGP, EXISTS/OPTIONAL, COUNT/DISTINCT, raw string/IRI spelling, nested pass-through and cross-width equality under degraded PostgreSQL text precision. Same-width SQL pools and independent unlike-width UNIONs retain their descriptors; consumed coercing float pools fail closed, while hidden in-arm guards do not block safe pools. ADR-0015/0034 retain the exact boundary; floating value arithmetic, general coercing-pool preservation and other product/release gates remain open. The following PostgreSQL double-promotion slice now repairs all six value comparisons, including REAL-shortest-decimal versus binary widening, native integer/full-range NUMERIC rounding, NaN/NULL, facet-checked integer-subtype constants and authorized/denied invalid NUMERIC overrides. Required owned TLS CLI tests and focused compiler/core checks cover that public slice. The subsequent Float-only lane now directly rounds native INTEGER/NUMERIC and constants to REAL, while natural REAL remains RDF Double. Authored Float columns over exact native numeric descriptors parse their original lexical values before optional Double widening; required owned TLS tests cover 32-bit boundaries, double-rounding counterexamples, invalid raw infinities, NUMERIC failure recovery and policy isolation. Authored Double over native INTEGER/NUMERIC now also has exact wire-value comparison authority and required public rounding/error/policy coverage without restoring revoked natural float facts. PostgreSQL TEXT/CHAR numeric mappings now also support floating-promoted comparisons through bounded exact lexical normalization: required public tests cover 16 numeric datatypes, facets, long exponent/midpoint cases, nondeterministic collation, original RDF spelling and policy-isolated EXISTS/OPTIONAL. The separate PostgreSQL non-floating lane now compares every digit for decimal/all13integer-family types over TEXT/CHAR and retained INTEGER/NUMERIC decoders. Required public checks cover all-six-op/NOT, full tails, facets, native scale, source validation and policy isolation; missing/foreign decoder proofs reject. The corresponding MySQL non-floating lane now covers qualified TEXT/VARCHAR/CHAR and native Integer/NEWDECIMAL with full-digit/facet checks, binary ordering, preserved native scale/YEAR/ZEROFILL and profile-sensitive natural validation. Required public tests include invalid/nonnumeric/runtime-NULL counterparts, cap-one recovery and policy-isolated EXISTS/OPTIONAL; missing/revoked decoder facts reject. Other native floating overrides, wider arithmetic, lifecycle/control and exact-release gates remain required and open. MySQL floating promotion now additionally covers all 16 numeric mappings over qualified TEXT/VARCHAR/CHAR and native Integer/NEWDECIMAL, using full lexical/facet validation, exact Float midpoint correction, native numeric JSON tags and policy-first source checks (ADR-0015). The required native matrix covers subnormal/overflow tails, native scale, both-column comparisons and recovery; the wider outstanding guarantees remain required. Native MySQL DOUBLE now additionally has retained, value-only binary64 authority for natural/matching explicit Double. Focused owned TLS checks cover finite extremes, signed-zero canonical output, NULL/errors, all operators in both directions, native fixed-decimal/ZEROFILL pass-through and policy-isolated EXISTS/OPTIONAL. This grants no lexical/template/RDF dedup or multi-arm pooled-UNION authority. Native FLOAT natural/matching Double and authored Float over both native widths now additionally have decoder-qualified value paths (2026-09-10, ADR-0015). Exact bounded shortest-decimal selection respects Rust's native width, minimum-normal interval and decimal tie rule; DOUBLE-to-Float needs that selection only at binary32 midpoints. Public focused checks cover rounding witnesses, extremes/NULL/special constants, original RDF output and policy isolation. Remaining native identity, pooled normalization, wider arithmetic, lifecycle/control and exact-release gates stay required and open. Subsequent native MySQL natural/matching Double RDF identity now has canonical fixed/sameTerm/BGP comparison and raw-preserving signed-zero D1/DISTINCT keys (ADR-0015). The focused owned TLS matrix passes both native widths, noncanonical/special constants, NULL/negation and raw authored Float output. Independent Sol high review identified and closed an overbroad typed-Float dispatch and MySQL ORDER dialect defect. Consumed multi-arm float pools remain rejected; this is no general lexical/template authority or release-completion claim. The subsequent native MySQL FLOAT static-IRI slice now shares the proven shortest selector for exact fixed/equality/sameTerm lookup and differently shaped IRI comparisons. Public signed-zero join regression exposed two extra rows; same-width RDF keys now distinguish zero signs without changing native foreign-key semantics. Required owned TLS tests cover 81 boundary values, noncanonical constants, policy-isolated EXISTS/OPTIONAL, D1/DISTINCT/COUNT and fixed-decimal/ZEROFILL pass-through. Single-arm FLOAT template rendering is exact; compiler checks prevent rendered projections from concealing unqualified pooled native floats. ADR-0015/0034 retain these precise authorities. Coercing pools and all remaining arithmetic/lifecycle/control/release requirements stay open; no accepted scope, deadline, backend or release flag changes. Native MySQL DOUBLE static IRI/template lexical behavior is now additionally qualified through a shortest-JSON candidate plus bounded exact decimal-tie correction (ADR-0015). Required owned TLS public checks cover 248 finite binary64 values including exact ties and adjacent non-ties, extrema, signed zero/NULL, lookup/negation, same/different-shape joins, D1/DISTINCT/COUNT, policy isolation and fixed-decimal/ZEROFILL nested pass-through. Single-arm rendering uses the exact recipe, while consumed raw/rendered multi-arm float pools and missing/foreign decoder authority remain rejected. Numeric/literal identity keeps its separate proof. No general pooled or resolved column-IRI authority, new architecture, relaxed execution bound or release-completion claim follows. Mixed-width native MySQL FLOAT/DOUBLE static-template joins now additionally preserve each decoder's exact lexical identity, with required owned TLS both-direction BGP/filter/negation, nested projection, signed-zero/NULL/bag and policy EXISTS/OPTIONAL checks. Same-width fast paths, native FK semantics, pool rejection and all remaining required guarantees are unchanged (ADR-0015/0034). Independent review additionally caught typed literal-template value FILTERs borrowing raw identity keys: the unsafe fallback now rejects, while separate sameTerm identity and qualified column-value paths remain. Full typed-template value construction remains required; the guard is no scope reduction or completion claim.

**Static IRI-constant delta (2026-09-09):** ADR-0007/0034 replace inverse raw-slot comparison with decoded forward identity for static single-slot IRI constants. Authenticated SQLite and owned TLS PostgreSQL16.15/MySQL8.4.11 checks cover text/CHAR, native integer spelling, percent collisions and surrounding native query profiles; SQLite adds mixed-storage/signed-zero, NULL/negation and COUNT/OPTIONAL. Native scalar lexical proof is independent of text or constraint authority. The required native aggregate additionally passes PostgreSQL boolean/BYTEA and MySQL binary string/blob/NEWDECIMAL fixed/equality/sameTerm, COUNT/OPTIONAL, NULL/negation, hex-case and decimal scale/ZEROFILL checks; AST/metadata tests reject cross-provider recipes and decimal proof through coercing UNIONs. BIT byte widths and TIME/TIMESTAMP spelling are now additionally qualified through fixed/equality/COUNT/OPTIONAL checks, including signed durations, zero values and non-UTC timestamp sessions; exact scalar alphabets avoid the generic SQL encoder. MySQL DATE/DATETIME lexical-only projection now preserves partial/invalid/zero dates and fractional times before window copies; required CLI evidence covers constant lookup and duplicate IRI bags and fail-terminal matching-datatype calendar validation. Native/natural consumer vetoes remain; unprotected raw temporal facts confer no copied identity. PostgreSQL numeric/temporal, native floating and existing multi-slot/template-pair identities remain required, unclosed work; no release-blocking flag changes. Saved `fb7b684` delivery history is now fast-forward integrated into local `main`; GitHub `main` remains unchanged, and the saved delivery branch is retained.

**Natural temporal identity delta (2026-09-09):** MySQL natural DATE/DATETIME now retains native payload and datatype while decoder-qualified identity keys match canonical Rust output; query constants remain verbatim. Required owned TLS CLI checks cover canonical/noncanonical fixed and sameTerm matches, DATE/leap/year-zero/extrema and DATETIME fractions, duplicate bags, nested projection, mixed literal/IRI joins, NULL/negation, invalid hidden SELECT/COUNT/ASK terms, cap-one recovery and denied-invalid-row policy/existential/OPTIONAL isolation. Coercing mixed temporal SubPlans retain a rejection marker instead of falling back to raw equality. Wider natural/native identity and native-consumer copies remain open. ADR-0015/0024 describe the decoder and authorization boundaries. This closes the qualified natural temporal slice, not general native identity or release admission.

**Query-profile delta (2026-09-08):** required owned-TLS PostgreSQL 16.15/MySQL 8.4.11 CLI checks cover one-hop DESCRIBE, duplicate-edge cycles, complete 258-edge closures and joined-path folded identifiers. The text-comparison repair now preserves case/trailing-space-distinct nodes under SQLite NOCASE, PostgreSQL nondeterministic ICU and MySQL PAD SPACE collations. Live varying-text facts gate native decorations; SQLite uses a same-IR prepare-only metadata twin, retaining CHAR/DATE decoding and authored SQL. Authenticated JOIN/OPTIONAL/EXISTS/NOT EXISTS/MINUS regressions pass. SubPlan metadata shares actual aggregate projection order, retains standalone paths and avoids width-exponential recursion. Required CHARACTER duplicate/connectivity tests now pass: per-backend decoder recipes normalize path leaves before joins/UNION, keep normalized metadata as text and preserve each endpoint width. SQLite uses an explicitly requested, query-local Rust scalar with shared decoding, pre-allocation source charges and tested cleanup/collision safety; native 4/2, 2/4 and 4/4 CHAR fixtures pass. General typed/mixed-key identity, ordinary source-collation behavior, total source controls and exact release remain open; no path exactness/admission flag is promoted. ADR-0033/0049 record the precise boundary.

**NULL-term correction (2026-09-08):** ordinary atoms now require every generated subject/predicate/object, including referenced parent subjects, before binding or inverse swapping. Class shortcuts require their subject; selected graph filtering keeps valid alternatives. Required tree/flat/unoptimized regressions and authenticated SQLite HTTP cover projection, ASK/COUNT and OPTIONAL/existence/anti-join behavior; owned PostgreSQL16.15/MySQL8.4.11 CLI checks cover admitted subject/object/class absence and non-NULL recovery. Dynamic predicate maps still reject at serving startup. Their separate raw fixed-predicate matching defect is not a newly adopted v1 requirement. Ordinary source-collation correctness and remaining source controls/release qualification are still open.

**Native wrapper correction (2026-09-08):** ordinary D1 DISTINCT and D2 rendered projections now retain typed source/column/NULL recipes until live emission. Original sources alone supply metadata; generated output names stay stable. Owned TLS public SELECT now passes mapping SRC/DST over actual lowercase columns on PostgreSQL16.15/MySQL8.4.11, including the existing DESCRIBE/path/CHAR/NULL profile. Raw-column policy exposure and bounded-join restore keep their narrow structural proofs; computed wrappers gain no table authority. Native transparent templates retain decoder descriptors, and source-sized shared dedup remains rejected by serving. Ordinary non-native-consumer text/CHAR SELECT, ASK/COUNT and correlations now also pass public/native checks: D1 uses exact decoded window keys while returning raw values; DISTINCT/pooled outputs and template comparisons use matching decoder facts. Native Ref/policy comparisons retain separate markers; native equality cannot authorize RDF FK substitution. Typed Ref atoms now join/filter both original sources before decoded-key dedup with live text/CHAR proof for every key, preserving projection bags, individual descriptors and named blank-node scope. Unknown keys retain prior per-source D1; a signed-zero regression proves distinct IRIs survive. Their former OPTIONAL decomposition and parameter order remain intact; required HTTP and owned PostgreSQL/MySQL tests cover this template-based text/CHAR slice. Ordinary text/CHAR policy consumers now filter native rows inside D1 before dedup, preserving projection bags and raw descriptors with public/native evidence; unknown keys keep raw D1 and the signed-zero check detects unsafe partial-key normalization. Ordinary SQLite all-IRI-template mixed D1 now shares live row decoding, preserving signed-zero IRIs and deduplicating integer/REAL and CHAR lexical duplicates without changing numeric literal comparisons. Public count/join/UNION/BLOB/date checks and native mixed-key compatibility pass; callback lifetime and source-charge checks pass. Explicit SQLite column literals now preserve lexical/datatype/language identity through SELECT/COUNT, constant matching, BGP joins and sameTerm; a separate typed numeric FILTER path preserves value promotion, precision and NaN behavior without borrowing identity authority. IRI/literal dual-use, NULL errors and callback lifecycle/work-limit checks pass. Numeric VALUES compare in Rust across dialects while existing string-pair VALUES equality remains supported; plain-column literal/IRI mismatches preserve errors under NOT. Owned native HTTP checks additionally cover numeric VALUES, UNDEF and byte-exact datatype-IRI identity under case-insensitive collations. Natural declared SQLite constant matching and native natural integer compatibility have bounded checks; general natural/native/mixed-descriptor identity remains open. SQLite column-IRI identity now carries mapping bases through constant/BGP/FILTER matching and decoded D1/Ref-atom keys, sharing the Rust resolver and prospective work/callback lifecycle controls. Source uniqueness cannot elide base-resolution collisions; multiple consumers retain separate keys and unknown identities reject. Cross-map reconstructed-term fallback preserves hidden graph keys and remains source-sized/rejected by serving. Compiler layout inspection is separate from live SQL emission. R2RML processor bases now have explicit per-source CLI/TOML/environment configuration, independent Turtle syntax bases, bounded pre-I/O validation, digest separation and fixed reload retention; shared column generation uses R2RML verbatim prefixing rather than URL normalization. General mixed/natural keys, native resolved-IRI execution, broader DISTINCT/GROUP/SubPlan qualification, wider template identity, total controls and exact release remain blockers (ADR-0034). The shared R2RML template alphabet now correctly percent-encodes C1 controls, private-use and excluded noncharacters as UTF-8 on Rust/SQLite/PostgreSQL/MySQL paths, with malformed UTF-8 rejection, NULL/NUL controls and required authenticated native equality evidence (ADR-0015). Late processor-base classification and core generation now pass grammar/expansion tests; the bounded SQLite IRI-template/constant atom also passes public equality, constant matching, mixed static/late joins, hidden-key bags, policy-before-dedup and COUNT/error checks (ADR-0015/0034). Typed operands retain per-part aliases and finalizer ownership; already sealed atoms are not re-deduplicated. OPTIONAL constants now preserve absence via existing match/no-match decomposition. Native resolved-template SQL, mixed natural/literal-column atoms, reference/path/proposition identities and wider pooling remain unqualified and reject. The rejected universal Ref-SubPlan experiment is not integrated evidence.

The application programme mixed three different outcomes:

1. a usable, secure, bounded semantic-fabric product;
2. the minimum evidence needed to release that exact product responsibly; and
3. research-scale quality trains, advanced provenance infrastructure, and
   autonomous harness evolution.

Treating all three as one serial completion gate delayed product integration
without making incomplete runtime features safer. The code already contains
substantial public exact-query, authorization, snapshot/lifecycle and bounded
federation behavior. The finite [G1–G6 completion ledger](../plans/sota-application-completion-programme.md#remaining-release-gates-2026-09-11)
records remaining admitted-profile guarantees and candidate evidence; neither
historical foundation lists nor proposed ADRs independently expand that scope.

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
also runs with defaults disabled. The 2026-09-09 `0.1.0-dev.1` image/reference package is described in [ADR-0039](ADR-0039-minimal-production-serving-artifact.md#implemented-adr-0055-reference-package-2026-09-09): pinned controlled build, exact-image non-root/read-only SQLite/PostgreSQL/MySQL/UNION smoke and a corrected PID-1 parser-parent check. The version is explicitly developmental, not a stable release.
Dependency graphs and this smoke are not an SBOM, signed release, full exact-artifact matrix or backend admission; remaining minimum-release gates stay required.

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

**User correction 2026-09-10:** every build uses `coding-harness`'s main-only delivery mode: scoped task/adoption → native model/effort binding → source-bound implementation request → actual checks → feedback-directed repair if needed → independent read-only native review → scoped main commit → exact-commit completion. `advance`/`submit` persist task/stage/attempt/source/route/prerequisite bindings; the outer controller admits dependencies and the real MetaHarness kernel verifies each ready stage. It does not assume upstream's retry loop supplies feedback or receipts provide crash-resume. The existing Codex/Claude host executes requests, without a second build daemon; the integrator remains accountable for native dispatch, meaningful acceptance and commit. Missing/stale evidence fails closed, no-progress repairs and native unavailability pause, and resume needs fresh handoff/request identity. Ruflo MCP synchronization remains host-owned and must be read back; local records are not managed memory or provider attestation. This cooperative harness is not an OS sandbox or release proof. Model allocation below and subscription-only/no-quota rules remain intact. Manifests track `latest` with exact lockfile-resolved checks. ADR-0037's closed experiment remains optional, its worktree launchers prohibited, historical isolation/replay law intact and evolution post-1.0. User pauses override queued continuation. The user explicitly released this hold on 2026-09-10: application work resumes through the completed harness and a tracked swarm, with one main writer and independent native review. The first resumed slice is exact MySQL rendered static-IRI pools; its named public TLS acceptance and remaining boundaries are recorded in ADR-0015/0034. No publication or whole-application completion is implied.

### 7. Status and claim discipline

Status is evidence-scoped:

- `implemented` means the named behavior exists at the cited commit and its
  required executable evidence passes;
- `complete` means every gate of the explicitly named profile passes on one
  immutable candidate;
- `deferred-post-1.0` means work remains undone outside the v1 gate; and
- `unsupported` means an exact rejection is implemented, not that unfinished
  work was renamed.

Moving a task between those sets requires an ADR or a dated implementation note that names the changed authority. Capability tables, README projections, and
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

### Model and reasoning-effort allocation (2026-09-10)

Use the selected main model without asking for a downgrade. Choose supporting
native subscription agents by task, with no spend/token/request/invocation/quota
ceilings, API keys or OpenRouter. Default to faster workers; escalate for a named
unresolved defect, not accumulated context or reviewer habit. This is a routing policy, not a measured ranking:

| Task | Starting choice |
|---|---|
| Deterministic operations | Ordinary tools; no model delegation required |
| Bounded mechanical work | Luna low or Haiku native default; prefer deterministic edits/checks |
| Established-pattern implementation | Terra medium; no competing integration writer |
| Normal implementation, test repair and bounded review | Sol medium or Sonnet native default; Sol high for a specific correctness proof |
| Difficult cross-component reasoning/review | Astra high or Opus only with an identified uncertainty the normal tier cannot resolve |
| Hard unresolved semantics, concurrency or integration | Astra max or Fable for a bounded escalation with its own acceptance check |
| Ultra | Not a routine programme worker; use only when explicitly requested for a named exceptional problem |

Astra max/ultra remain available, not silently clamped. Select effort explicitly at assignment; omission uses the native default. An escalation records the failing behavior or unresolved proof, owner and focused check; stop it when answered and route the next task afresh. Keep prompts limited to the relevant delta and acceptance criteria. Do not run duplicate reviews or assign models to deterministic polling, Git or test execution.

Observed programme delta: the existing V5/V6 harness and gate-contract defaults already name Sol/Sonnet, but successive live native numeric assignments reused Astra Ultra; its difficult rounding proof was useful, but continued bounded follow-up did not justify retaining Ultra by default. On 2026-09-10 the user requested faster allocation: the Ultra reviewer was stopped and the remaining identity review assigned to native Sol high. The available history lacks comparable per-model integration timings; no speedup, cost saving or programme ETA is inferred.

This task-based effort change is consistent with [official model guidance](https://developers.openai.com/api/docs/guides/latest-model#whats-new); the table is project policy based on live executor capabilities, not an OpenAI/Anthropic comparative benchmark. Reassess using actual integrated outcomes, elapsed time, failed checks and rework; never create a model-benchmark programme to enforce routing.

Distinct model/effort configurations need distinct candidate IDs when used. Native errors are authoritative: pause and report exact client/model/error on subscription or requested-model unavailability; never substitute a provider or silently change a requested model. Independent native Codex and Claude review remains required for the release delta, not every edit.

### Six-hour course correction

The [scheduled prompt](../plans/programme-six-hour-review-prompt.md) compares
the finite gate ledger and previous correction with actual public acceptance on
`main`. **2026-09-11 correction:** group compatible work by whole phase/public
outcome, audit prerequisite tests before scope freeze, and update only affected
decisions/status plus required source-bound evidence. A correction has not worked
merely because another helper/receipt passed. Preserve checked amplification/copy/
growth bounds; a replacement phase envelope requires finite-input/full-work proof,
bounded checkpoints and unchanged default-corpus acceptance, not timeout alone.
Stalls require an execution change, not a new arbitrary ETA or harness programme.
React immediately to blockers. Native `codex queue` targets the pinned conversation
with no competing resume/fallback; Ruflo MCP recall remains optional, never a gate.

## Current implementation status

The v1 profile is accepted and **not complete** on 2026-09-11. Existing code has
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
sessions and independent-CA/hostname failures. These required real-CLI paths also
qualify both source selectors; old optional/missing-unit-test metadata adds no gate.
Exact-release-artifact evidence and backend admission remain required; M3/M6 remain incomplete.

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
