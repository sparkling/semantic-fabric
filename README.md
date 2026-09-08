# semantic-fabric

**A uniform query layer over systems of record—the live data foundation for agents, applications, analytics, and compliance.**

[![CI](https://github.com/sparkling/semantic-fabric/actions/workflows/ci.yml/badge.svg)](https://github.com/sparkling/semantic-fabric/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#contributing-and-license)
[![W3C RDB2RDF](https://img.shields.io/badge/W3C%20RDB2RDF-81%2F87%20SQLite%20%C2%B7%2080%2F87%20PostgreSQL%20%C2%B7%2074%2F87%20MySQL-success.svg)](#correctness-and-verification)
[![Rust 1.96](https://img.shields.io/badge/rust-1.96.0-orange.svg)](rust-toolchain.toml)

semantic-fabric is a Rust-native, virtualisation-only OBDA engine. It rewrites a tested read-only SPARQL 1.2 query subset to SQL over live relational databases through R2RML mappings. It has no JVM and keeps no instance-data copy: only ontology `T` and mappings `M` live in the engine; source rows stream and vanish.

<!-- capability-matrix:start -->
## Evidence-scoped capability status

As of 2026-09-08, the generated catalog records:

- **Qualified:** A finite server-wide fail-fast gate now bounds requests admitted into application work. Saturation returns a distinct redacted 503 with Retry-After before Router/body polling; the request identity retains capacity through active producers and queued/running compiler or SQLite workers, while completed buffered bytes do not hold it. This is not a connection/socket/response bound, rate limit, fairness guarantee, native-work pre-emption, total QueryBudget, production admission or atomic post-200 delivery; a shed unpolled POST body can affect HTTP/1 reuse/reset behaviour.
- **Qualified:** Opt-in authored reload validates and atomically replaces complete application generations across SQLite/PostgreSQL/MySQL single-source and the bounded two-source UNION/join. Detected file/schema drift immediately fences new requests while old leases remain pinned; invalid candidates recover only after valid rebuilding. One worker is retained through timeout; panic and shutdown cannot heal. Required CLI tests include encrypted remote single/mixed reload. Endpoints, resolved credentials/TLS roots, caller policies and limits stay fixed; other protected backend DDL leases and configuration/policy hot reload remain open; the closed PostgreSQL Direct profile is separately qualified. Required owned TLS lineage reload now verifies changed/restored actual mapping documents and exact results for constant/multi-map SELECT/CONSTRUCT, mixed UNION and nonempty both-order joins. Native-held multi-map SELECT on both providers plus mixed UNION/forward join on PostgreSQL completes with pre-invalid results while readiness and new queries fail closed; repair restores readiness. These are authored-generation checks, not policy/configuration reload or every held graph/operator/order.
- **Current:** SIGTERM/Ctrl-C moves Running to Draining, marks readiness administrative not-ready, rejects new budgets and closes ingress while admitted identities may finish with 200. HTTP drain and detached work retaining request capacity share the original positive drain deadline (30 seconds by default). At expiry, Forced cancels survivors and allows three further seconds for owned cleanup; retained capacity then returns TimedOut rather than a clean shutdown. Required paused-clock/listener tests cover grace, retained ownership, exact forced expiry and bounded cleanup failure. Owned PostgreSQL 16.15/MySQL 8.4.11 TLS CLI tests observe native stop, clean process exit and closed ingress after forced ASK/SELECT/CONSTRUCT SIGTERM. Wider backend/profile qualification, source-health policy, remaining metrics/OTLP, SLO and release admission remain open. Required pinned TLS mixed PostgreSQL/MySQL CLI tests observe each target stop under deadline/disconnect/forced SIGTERM while its native table-lock witness remains granted, preserve a distinct blocked same-credential query in a separate CLI process, and recover the full exact federated bag through both cap-one pools. Join failures remain pre-200; streamed UNION failure has no complete chunked success.
- **Qualified:** Explicit lineage media type admits positive BGP/JOIN/UNION with a root projection/DISTINCT/REDUCED/slice spine, constant non-rdf:type query predicates and constant mapping predicate/graph maps. At most 64 distinct map IDs, 256 prospective branches, 128 algebra/triple visits and 1024 witnesses per intermediate relation are admitted. Native RDF equality, not SQL collation, matches constants/repeated variables and joins. Atom duplicates combine actual origin bits; explicit UNION and projected bags remain distinct, and DISTINCT combines origins before LIMIT. Borrowed exact keys and pre-clone retained-byte/work/output charges bound request-local witness evaluation; overflow fails rather than truncating. Only actual final solutions carry PROV-O used mapping/source entries; mappingCatalog is a dictionary, not contributing provenance. CONSTRUCT uses native reification outside the unchanged product graph. Required SQLite HTTP tests cover unused/overlapping origins, NULLs, nested bags, collation, late witnesses, whole-response portable-policy isolation and witness/byte failure. Required owned pinned PostgreSQL16.15/MySQL8.4.11 serving-only TLS CLI tests passed on 2026-09-08 for SELECT/UNION/JOIN/CONSTRUCT, parsed returned origins, allowed/empty policy callers and unsupported FILTER under held locks. Shared terminal tests require successful cleanup and clean transport EOF. Ordinary query optimization remains separate; this is not total parser/heap governance, a database-wide data snapshot, row-key authority, federation, wider operators/native lifecycle/source-RLS lineage, backend admission or exact-release qualification. Twelve required pinned native multi-map single-source SELECT/CONSTRUCT cases additionally observe the exact encrypted target stop under deadline/disconnect/forced SIGTERM while native table locks remain granted, preserve a distinct same-credential blocked CLI sibling, recover exact lineage through the cap-one target pool after deadline/disconnect, and require bounded clean forced exit plus closed ingress. Failed HTTP responses cannot end successfully or contain parsed completion records, including records crossing chunk boundaries. Those single-source cases do not qualify federated lineage JOIN, portable/source-RLS cancellation, reload or all operator combinations. A separate bounded two-source SELECT UNION lineage profile now carries actual source-keyed origins under the same request owners, with required HTTP and pinned native TLS deadline/disconnect/forced-shutdown qualification; reversed arms, caller isolation, source-scoped blank nodes and shared caps are covered. A separate bounded two-source join lineage profile now seals the actual map/source pair per mandatory arm, follows cost-side swapping, and emits both contributors only for each final matched bag occurrence. It reuses the 128-build/4096-probe executor and capped pre-200 serializer with explicit map-to-source links and no hidden keys. Required public checks cover exact/projected bags, policy, activation and limits; the pinned TLS CLI aggregate adds twelve both-order join-lineage deadline/disconnect/forced-shutdown cases with exact encrypted target, held-lock, sibling and cap-one recovery witnesses. Federated lineage CONSTRUCT remains outside the declared profile; exact-release qualification remains required; see ADR-0017. Required owned TLS lineage reload now verifies changed/restored actual mapping documents and exact results for constant/multi-map SELECT/CONSTRUCT, mixed UNION and nonempty both-order joins. Native-held multi-map SELECT on both providers plus mixed UNION/forward join on PostgreSQL completes with pre-invalid results while readiness and new queries fail closed; repair restores readiness. These are authored-generation checks, not policy/configuration reload or every held graph/operator/order. Required owned PostgreSQL16.15 public-router source-RLS evidence now parses actual constant/overlapping-map SELECT/CONSTRUCT and two-source UNION/join lineage for A/B/A callers: exact authorized products/bags, actual source/map origins, no denied values or raw identities, empty results, concurrent constant SELECT and clean cap-one PID reuse after each complete response. Constant-lineage SELECT/CONSTRUCT body-drop, policy-error and deadline cases fail terminally and recover with isolated caller state. This is the recorded RLS profile, not every failure permutation, remote TLS, policy installation/configuration reload or exact-artifact admission. Under ADR-0055 the declared lineage-profile gate is now qualified and l-lineage is non-blocking. Full historical ADR-0017 remains incomplete; separate runtime-budget, backend-admission and exact-release gates stay blocking.
- **Qualified:** The disabled default has no /metrics route or recorder. Explicit --metrics exposes exactly three fixed-cardinality Prometheus families on the existing listener for terminal query attempts, query duration and sticky governance rejections. Discovery/control traffic is excluded, terminal counts are exactly once and a fail-closed recorder discards foreign targets, names, labels and values. This is three of thirteen ADR-0011 metric families and has no separate listener/authentication boundary; it is not OTLP, layered configuration, TLS, SLO/overhead qualification, full observability or production admission.
- **Qualified:** The independent Rust sf-capture-supervisor crate implements an exact bounded transaction/state-store kernel for one immutable request, lease and attempt, overlap fencing, terminal closure, post-commit replay and pending-outbox state. Required deterministic tests pass; developer-local isolated PostgreSQL 16.15 differential/contention tests passed without touching Product Mock, but have no tracked replay receipt. The kernel is non-advertisable and non-production: every authenticated transport, principal, signer, runner, database-hardening/restart, publication/witness, controlled-performance, admission and release gate remains open, and ADR-0042 stays proposed. Node remains development/evidence-only.
- **Current:** Shared flat/tree lowering has required compiler evidence for exactly one DESCRIBE target expression: a constant IRI or one in-scope variable. It reserves authored variables, collapses repeated target solutions, and proves or fail-closes the one-hop RDF-graph set union. This compiler claim establishes neither backend execution nor unrestricted DESCRIBE.
- **Qualified:** Explicit Accept application/vnd.semantic-fabric.lineage+json-seq on existing authenticated GET/form/raw POST. A compiler proof admits exactly one authored TriplesMap, no referencing object maps and nonempty SELECT/CONSTRUCT BGPs with JOIN/UNION/projection/DISTINCT/REDUCED/slicing only. Every witness has the same source/map: the final sink preserves bags, dedup and slicing without guessed origins or source re-query. The header binds mapping/source, opaque snapshot-instance, logical compiler-input and policy identities. SELECT pairs standard JSON bindings with PROV-O JSON-LD. CONSTRUCT frames one response-wide native RDF 1.2 N-Quads dataset: unchanged product triples in the default graph, PROV-O bundles and rdf:reifies triple terms only in named graphs. Parse concatenated fragments with one blank-node scope; shared mapped nodes and fresh template nodes retain identity. Reification repeats only authorized emitted terms, not hidden columns or row keys. Empty/invalid outputs create no activity; repeated triples retain RDF set semantics with explicit occurrence counts and no new result-sized dedup state. All escaped bytes and completion are charged; success requires execution/cleanup plus clean transport EOF. Required SQLite tests cover native graph equality/reification, nested/directional terms, blank nodes, escaping, policy/cache/reload isolation, eligibility and byte bounds, and terminal failure. The 256-visit cap includes template triples, not total parsing/compiler work. Dynamic origins, row keys, federation, wider native-profile/exact-release qualification and full ADR-0017 remain open. Required owned pinned PostgreSQL16.15/MySQL8.4.11 serving-only CLI evidence additionally verifies exact SELECT/CONSTRUCT lineage, portable-row allowed/empty whole-response isolation, cache identities, encrypted sessions and unsupported requests under a held native table lock. These native cases do not qualify every operator/lifecycle combination or source-RLS lineage. Required owned TLS lineage reload now verifies changed/restored actual mapping documents and exact results for constant/multi-map SELECT/CONSTRUCT, mixed UNION and nonempty both-order joins. Native-held multi-map SELECT on both providers plus mixed UNION/forward join on PostgreSQL completes with pre-invalid results while readiness and new queries fail closed; repair restores readiness. These are authored-generation checks, not policy/configuration reload or every held graph/operator/order. Required owned PostgreSQL16.15 public-router source-RLS evidence now parses actual constant/overlapping-map SELECT/CONSTRUCT and two-source UNION/join lineage for A/B/A callers: exact authorized products/bags, actual source/map origins, no denied values or raw identities, empty results, concurrent constant SELECT and clean cap-one PID reuse after each complete response. Constant-lineage SELECT/CONSTRUCT body-drop, policy-error and deadline cases fail terminally and recover with isolated caller state. This is the recorded RLS profile, not every failure permutation, remote TLS, policy installation/configuration reload or exact-artifact admission. Under ADR-0055 the declared lineage-profile gate is now qualified and l-lineage is non-blocking. Full historical ADR-0017 remains incomplete; separate runtime-budget, backend-admission and exact-release gates stay blocking.
- **Qualified:** Only a top-level SELECT over two source-affine default-graph triple patterns is admitted: distinct subject/object variables per arm, one constant predicate/direct-object emitter, one authored table/scan and a shared reversible single-slot IRI or explicit literal key. A fixed 128-triple driving batch supplies parameter-bound conservative probe reduction; up to 4096 distinct complete probe triples and independent retained-payload/work/result/serialization/deadline controls bound exact RDF set normalization and bag merging. Database collation never establishes RDF identity. Both sources are acquired under one application snapshot/security/budget before work; source-local statement views are independent, not a distributed snapshot or DDL lease. The complete capped body is staged before 200; timeout/overflow/source failure cannot return partial success. Public oracle/boundary/policy tests and SQLite plus encrypted PostgreSQL 16.15/MySQL 8.4.11 CLI tests are required. Wider shapes reject pre-I/O; wider backend/profile cancellation qualification, backend-generation protection and exact-release admission remain open. General DISTINCT/spill and proposed ADR-0040 are not accepted. Required pinned TLS mixed PostgreSQL/MySQL CLI tests observe each target stop under deadline/disconnect/forced SIGTERM while its native table-lock witness remains granted, preserve a distinct blocked same-credential query in a separate CLI process, and recover the full exact federated bag through both cap-one pools. Join failures remain pre-200; streamed UNION failure has no complete chunked success.
- **Current:** The accepted ADR-0006 UNION vertical serves a top-level SELECT UNION containing exactly two one-triple arms, each statically affine to one distinct active source. Both sources are acquired before 200; source-local plans stream sequentially as bag-preserving UnionAll under one snapshot lease, RequestBudget and serializer. Required compiler/runtime and file-backed SQLite CLI evidence covers duplicates, UNBOUND, budgets, fail-closed admission, failures and generation pinning. Required native CLI evidence additionally proves exact duplicate-preserving PostgreSQL 16.15/MySQL 8.4.11 UNION over independent verified TLS. This is not broad M6 and does not accept proposed ADR-0040. Required pinned TLS mixed PostgreSQL/MySQL CLI tests observe each target stop under deadline/disconnect/forced SIGTERM while its native table-lock witness remains granted, preserve a distinct blocked same-credential query in a separate CLI process, and recover the full exact federated bag through both cap-one pools. Join failures remain pre-200; streamed UNION failure has no complete chunked success.
- **Limitation:** Full SPARQL 1.2 Query or Protocol conformance is not claimed.
- **Qualified:** The required per-PR generated QE train is exactly 5,000 fixed-seed SQLite SELECT cases: 50 generated schema/data/R2RML fixtures times 100 queries, compared across four compiler paths and a separately materialized spareval oracle with ordinal replay. It meets only that numeric M4 sub-gate; nightly 100,000, other backends/forms, NoREC/MR1, fuzz/shrink, coverage/mutation, live-service and load/soak gates remain open.
- **Current:** GET /livez reports fixed event-loop liveness and GET /readyz projects only immutable runtime plus administrative readiness. Their fixed JSON responses do not poll sources or bodies or consume application-work capacity. Automatic protected source-generation guarantees and the remaining metrics/OTLP catalogue, TLS, SLOs and the complete ADR-0011 control plane remain open.
- **Current:** Serve merges bounded typed TOML, environment and final CLI settings over defaults, then validates effective values with redacted errors. Token rotation preserves row policies; a registry cannot become a lone unrestricted bearer. Values cannot inject flags or bypass layers. Required real-child tests prove authenticated exact two-source HTTP execution, bounded input and policy resolution before source I/O. Configuration/policy hot reload, exact-release-artifact TLS qualification and direct external secret-store transport remain open.
- **Qualified:** PostgreSQL and MySQL have live query and endpoint evidence, but those suites can still skip and do not establish production admission.
- **Qualified:** Sealed required-live MySQL RDB2RDF execution records 62 passes and one documented R2RMLTC0002f deviation across 63 R2RML cases, plus 12 passes and 12 exact typed unsupported outcomes across 24 Direct Mapping cases under RequirePrimaryKey. Its mysql-w3c-sql-2008-v1 type profile is conformance-only; native product MySQL conservatively treats ambiguous TINYINT(1)/BOOL as integer unless explicit rr:datatype supplies authority. The v5 receipt leaves provider image/toolchain provenance unbound. This is mapping evidence only, not Query/Protocol conformance or production admission.
- **Qualified:** Before runtime binding, mandatory ontology and effective authored-R2RML mapping form one bounded source-local M-join-T graph. The sealed contract is exactly four rules: three SHACL Core shapes run through rudof Native and one exact parsed, digest-pinned datatype sh:select runs once globally against the same checked store. Violations reject, warnings remain advisory counts, and detailed reports are discarded. Policy v2 binds exact shapes/query, topology, evaluator/parser identities and features, limits, preflight revision and blank-focus policy. The static Product Mock replay keeps ontology categories 01-12 and 14 in T and independently supplies category 13 as M. ADR-0050 is accepted as the lifecycle design, while its wider runtime phases remain incomplete. Public closed PostgreSQL Direct Mapping consumes the same sealed semantic validation and is separately required-live; generalized SHACL, other backend generations and production admission remain open.
- **Qualified:** Sealed required-live PostgreSQL RDB2RDF execution records 57 passes, one documented deviation and five exact skips across 63 R2RML cases, plus 23 passes and one exact skip across 24 Direct Mapping cases; this is mapping evidence only and does not establish production admission.
- **Qualified:** Sealed SQLite RDB2RDF execution records 62/63 R2RML cases passing with one documented deviation and 19/24 Direct Mapping cases passing with five exact skips; this is mapping evidence only.
- **Qualified:** Public PostgreSQL/MySQL SELECT/ASK/CONSTRUCT and source fragments own dirty connections through bounded native stop/discard and retain request capacity; success requires acknowledged drain/reset before reuse. PostgreSQL uses the immutable TLS policy with a one-second cancel allowance. MySQL uses a separate same-credential/TLS control connection, random target/control session locks, nullable fail-closed identity checks and a two-second allowance; no KILL retry. Public data/control constructors exclude unowned setup-query drains. Required owned TLS CLI tests observe stopped native work, cap-one recovery, stricter MySQL source timeout/504 and forced ASK/SELECT/CONSTRUCT SIGTERM with native stop and clean process exit. This is session-affine endpoint evidence, not arbitrary proxy or atomic external-ID-reuse protection, termination acknowledgement, raw embedding-pool construction qualification, wider backend/profile qualification, total source-work governance or release admission. Required pinned TLS mixed PostgreSQL/MySQL CLI tests observe each target stop under deadline/disconnect/forced SIGTERM while its native table-lock witness remains granted, preserve a distinct blocked same-credential query in a separate CLI process, and recover the full exact federated bag through both cap-one pools. Join failures remain pre-200; streamed UNION failure has no complete chunked success.
- **Limitation:** The public closed PostgreSQL Direct profile now has a coherent request generation lease, mandatory control lifecycle and exact 16.9/16.15 owned-TLS CLI qualification. Typed no-PK identity, other backend generations, general multi-profile reload, total-resource-qualified recursion, wider bounded operators, charter-complete federation, total governance, security/identity, remaining telemetry/configuration/TLS/SLO closure and an exact production artifact remain planned. Public authored reload, the bounded two-source UNION/join, layered configuration/TLS, the serving/developer split, partial traces, probes and shutdown close only their evidenced slices, not those broad profiles.
- **Qualified:** Public `serve --direct-mapping-base` enables exactly one PostgreSQL 16.9/16.15 ADR-0051 source with permanent public PK tables, fixed ontology/base/configuration, no row-policy/raw SQL/federation/additional source and independently constructed request/control TLS pools. Mandatory observation defaults to five seconds; nonzero reload interval overrides. Startup acceptance is 60 seconds and control work 30 seconds, independent of requests. One skip-tick coordinator fences completed failures, retries serialized builds and activates only sealed full-state CAS candidates; requests never fence. Candidate/native cleanup retains cap-one ownership through timeout and bounded shutdown, without overlapping retry or late publication. Required owned-TLS public CLI evidence qualifies exact authenticated SELECT/ASK/CONSTRUCT/lineage, drift/rebuild, fixed ontology, NOWAIT lock-conflict/recovery, bounded shutdown and wrong-CA/no-PK startup rejection on both exact patches. This is not hard CPU preemption, broader backend qualification or production admission.
- **Qualified:** The optional Product Mock serve KAT feeds the current sealed Style R2RML through sf-serve's HTTP admission and controlled-execution path. The historical ORDER BY window with LIMIT 10001 returns HTTP 200 and requires every typed binding, in order, to equal direct SQL; the 2026-09-05 run compared 500 rows. A separate offline structural test identifies LIMIT 10002 before opening its poison PostgreSQL pool, but issues no HTTP request and reads no live database. This cross-session mutable-development observation is not production admission or a coherent shared snapshot.
- **Limitation:** The optional mutable product-mock PostgreSQL Style differential is distinct from static gold/source verification; its 11-database/112-table/598-column inventory currently fails closed on observed drift and grants no qualification.
- **Limitation:** None of SQLite, PostgreSQL or MySQL is production-admitted under ADR-0055 v1.
- **Qualified:** SQLite, PostgreSQL and MySQL compilers use exact finite-pair fixed points for recursive P+/P*; SQLite/PostgreSQL compiler-only evidence pins the tested no-primary-key physical-row path SQL shapes. PostgreSQL still uses a collision-prone synthetic `rowid` sentinel and snapshot-local `ctid`. Hostile required execution is SQLite-only; live PostgreSQL/MySQL qualification and total resource governance remain open.
- **Current:** Current Axum-router path, method, strict request-admission/body and mapped pipeline failures plus startup CLI surfaces are redacted. Unsupported media is rejected without polling the application body. Malformed request targets rejected before the outer Tower service remain outside this application boundary. One generated correlation ID spans response, RFC 9457 problem, governance, body and stream trace surfaces. SELECT/CONSTRUCT adapter failures drain only the already-produced bounded prefix, then expose one stable body error and fused EOF without an exact Content-Length; post-200 failures cannot become atomic RFC 9457 responses.
- **Current:** Active application work carries one control identity from Tower Service::call after HTTP/request-target parsing and before Axum route/method dispatch through strict request admission, handler handoff and serialization, with one absolute deadline and inclusive ceilings for observable metadata probes, branch opens, pull attempts, semantic result items, serializer writes and ORDER retained textual-payload growth. Fixed health and query-less discovery metadata intentionally bypass query-work accounting. An independent admitted row window bounds fixed ORDER row overhead; the payload ceiling is not a total heap cap. Charges and terminal transitions are linearizable and internal accounting retains its sticky first cause. At an expired representable handoff the public classification is always deadline/504; before it, non-deadline SELECT/CONSTRUCT failures remain stable post-200 body errors. ASK rejects zero result capacity before backend match/acquisition, stops after its first final solution without truncating grouped input, and uses no global sort buffer for top-level ordering. Compiler CPU, database rows scanned, recursive SQL/source cost, raw/conformance callers, full native cancellation/admission qualification and atomic/no-prefix post-200 responses are not claimed.
- **Qualified:** SQLite, PostgreSQL and MySQL source selectors feed immutable source/backend/compiler/cache bindings inside a source-keyed runtime snapshot; reachability is not admission. Normal public execution is single-source authored mapping or the closed PostgreSQL Direct profile, with separately qualified two-source one-triple UNION and fixed-cap two-pattern join exceptions. Ready requests pin one application generation through response termination, and private process-local binding identities reject stale plans even across content-equal replacement snapshots; the checked publication primitive swaps whole snapshots without mixing generations but remains crate-private and non-authorizing. The public closed PostgreSQL Direct Mapping route now derives one exact observed generation under a non-owner, NOINHERIT login with no memberships or elevated/mutation authority. Within its closed database/public-schema/mapped-base-table surface it requires CONNECT/USAGE/SELECT plus the exact collation probe. It retains a compiler permit across lease acquisition, executes SELECT/ASK/CONSTRUCT on the binding-matched protected transaction and rechecks before bounded rollback. Exact 16.9/16.15 observation-profile receipts pass without granting runtime authority. PostgreSQL introspection and unqualified base-table resolution are `public`-scoped and fail on schema/catalogue identity ambiguity. Explicit-R2RML serving still quarantines unverified integrity constraints and prevents mutable startup types from authorizing cross-column PostgreSQL pooling. Every recursively reachable base Table/Query source is probed and its referenced columns fail closed before cursor I/O, including base-source references inside nested emission. Offline/synthetic alias folding and translate-time immediate wrappers retain a bounded non-SQL-token-aware heuristic, never live metadata authority. Shared term-dedup captures the complete BGP key, including an active graph variable, before projection, crosses only physically key-preserving pure unary wrappers, is revalidated before metadata I/O, and overlays only a private execution clone; unsafe scopes return `501`. Fully ground overlap uses an exact SQL unit-relation pool, and serving rejects the remaining source-sized fallback pre-I/O. Trusted raw `rr:sqlQuery` is not schema-confined. Typed row identity, protected backend generations, configuration/policy hot reload and production admission remain open. Public Direct Mapping is separately qualified only for the closed PostgreSQL profile.
- **Qualified:** The Rust serving path owns one source-keyed immutable runtime snapshot and pins one ready generation plus its state witness per request through response EOF, error, cancellation or drop. Private process-local binding identities reject stale plans even across content-equal replacement snapshots. Generation-only activation IDs and an opaque checked state revision reject stale candidates/reporters and same-cause repeated-not-ready races; shutdown closes transitions before fencing the winning state, exhaustion terminalizes readiness, and old resources release only after their last lease. The public closed PostgreSQL Direct Mapping path adds a binding-matched request-owned source-generation lease. Construction primitives remain crate-private; other protected backend generations, configuration/policy hot reload and production admission remain open; the separately tested authored coordinator now publishes application generations and fences observed drift.
- **Qualified:** Public Rust/CLI queries default to deny. Explicit bearer profiles provide single-principal access or a bounded registry whose callers atomically select PostgreSQL RLS settings or a portable equality-row policy. One immutable registry policy separates subjects and attributes across cache and execution; protected UNION is uncached. Read-only RLS transactions require guarded non-owner PostgreSQL tables and cleanup. The portable profile requires complete exact source/table coverage, validates compiler-generated single-table projections, binds all values, and rejects uncovered/complex shapes before source I/O. Required tests cover live PostgreSQL RLS plus SQLite portable SELECT/ASK/CONSTRUCT and both UNION fragments. General ABAC/sensitivity, live cross-backend portable qualification, external issuers and policy-aware reload remain incomplete. Required owned PostgreSQL16.15 public-router source-RLS evidence now parses actual constant/overlapping-map SELECT/CONSTRUCT and two-source UNION/join lineage for A/B/A callers: exact authorized products/bags, actual source/map origins, no denied values or raw identities, empty results, concurrent constant SELECT and clean cap-one PID reuse after each complete response. Constant-lineage SELECT/CONSTRUCT body-drop, policy-error and deadline cases fail terminally and recover with isolated caller state. This is the recorded RLS profile, not every failure permutation, remote TLS, policy installation/configuration reload or exact-artifact admission.
- **Qualified:** The in-repo required tests lock a verifier for 246 canonical semantic-builder gold artifacts and 171 files from exact semantic-product-mock revision 7c45292fccb8b88afe263e18de6806667ae18573. Ontology categories 01-12 and 14 form T; independently sealed category 13 supplies M as 148 TriplesMaps/721 predicate-object maps over 112 tables/598 columns. The external replay joins 47,463 ontology and 3,064 projection triples into a 50,527-triple closure and requires zero violations and warnings. That external KAT is diagnostic because CI supplies neither root. Generated .metaharness copies are not verifier inputs, and this static contract proves neither mutable PostgreSQL conformity, image/source provenance nor production admission.
- **Current:** Exact query-less GET/HEAD /sparql now exposes a deterministic, redacted Turtle SPARQL Service Description. Single-source mode advertises only the exact versioned DESCRIBE feature urn:semantic-fabric:service-description:describe-one-target-one-hop-query-v1; two-source mode does not. Two-source discovery names only versioned source-affine UNION and bounded two-pattern join features. The document otherwise advertises only custom bounded language/feature resources and reachable result formats, deliberately omitting full SPARQL language, BasicFederatedQuery, external SERVICE and production-admission claims. Fixed discovery metadata bypasses query admission, runtime leases and body polling, while query-bearing requests retain the strict Protocol boundary.
- **Qualified:** External SERVICE and named non-enabled source forms are rejected before query execution or connector construction.
- **Current:** Credential-bearing PostgreSQL/MySQL source specifications can be supplied through a bounded environment reference in CLI, TOML or SEMANTIC_FABRIC_* configuration; parsed inline passwords fail before runtime, file or network I/O, and the bounded trace and Prometheus slices reject seeded source/foreign secrets. Exact-release-artifact TLS qualification, OTLP and a direct external secret-store protocol remain open.
- **Current:** Serving rejects known source-sized Rust fallback plans before backend selection or source I/O. It admits only finite root variable-key ORDER windows within independent row and retained textual-payload ceilings; LIMIT 0 performs no source I/O, while expression, overflow, unbounded and nested ORDER fail closed. Controlled exact 1x/10x/100x heap and fresh-process RSS sweeps pass the 10% growth gate; the RSS fixture's 64 KiB SQLite page cache is measurement-only, not production tuning. Required independent materialized-oracle evidence passes for defined ORDER surfaces; undefined extensions retain only deterministic bag and stable-tie evidence. The live Product Mock KAT admits LIMIT 10001 through sf-serve; the LIMIT 10002 poison-pool boundary is a separate offline structural test with no HTTP request or live database read. These gates complete the finite root ORDER slice without admitting PostgreSQL or completing full M1; GROUP, dedup and wider bounded global operators remain open. Top-level ordered ASK needs no global sort buffer.
- **Qualified:** The owned serving SQLite path has required evidence for query-local interruption of active VM work after mutex acquisition, exact local-cause preservation, handler cleanup and connection reuse. Its separate serving admission gate does not make raw standard-mutex wait or submitted/running blocking work cancellable; busy/UDF/VFS/I/O, compiler, raw/conformance, PostgreSQL/MySQL, database/recursive work, total M2, production admission and atomic post-200 delivery remain outside this narrow profile.
- **Qualified:** Owned serving SQLite has required proof of one permanent cap-one admission identity per physical pool member. SELECT/ASK/CONSTRUCT wait under the same RequestBudget before blocking-worker submission; admission timeout/cancellation leaves the wait, timeout is pre-200 504 without SQLite entry, and private lease state is retained/cloned into metadata/row workers through exit. The public lease is non-Clone and consumed once. Active leases, per-waiter lifetime and aggregate admitted application work are bounded. Raw pick/foreign holders bypass the gate; the standard mutex and submitted/running work remain non-cancellable; busy/UDF/VFS/I/O, availability-aware selection, other backends, compiler/raw/conformance/database-row/recursive work, total M2, post-200 atomicity and production admission remain open.
- **Current:** SQLite has required endpoint evidence for the versioned one-target-expression, one-hop DESCRIBE profile. One constant IRI or in-scope variable may identify several resources; repeated values collapse into the exact outgoing RDF-graph set union. Blank-node objects are not recursively followed, and the complete lowered plan remains subject to normal single-source admission. Multiple projected targets, unbound target variables and currently unqualified modifier shapes reject before source I/O. PostgreSQL/MySQL execution and wider DESCRIBE remain unqualified.
- **Current:** SQLite has required evidence for a strict read-only endpoint subset: exactly one query in GET/form POST or one raw POST body; raw per-request cap n, checked form wire cap 3n+16 and decoded cap n; optional version/dataset/extra parameters rejected instead of ignored; unsupported media rejected without application-body polling; ASK JSON, SELECT JSON/XML/CSV/TSV, CONSTRUCT Turtle/N-Triples/JSON-LD, and the versioned one-target-expression, one-hop DESCRIBE Turtle profile. Full Protocol remains open.
- **Current:** The simple SQLite streaming-CONSTRUCT profile has a required growing-source constant-memory benchmark; this does not cover global operators.
- **Qualified:** Only serve installs the production JSON subscriber. Its exact product target and closed off/error/warn/info ceiling filter every event and span. One generated opaque correlation ID crosses untrusted ingress, response/problem, governance, body and stream surfaces. Request and compiler stages use a closed payload-free vocabulary, with one cascade span independent of branch count; seeded-secret and target-filter mutation evidence pass. The separately qualified opt-in Prometheus slice consumes the same exact terminals. This partial ADR-0011 slice remains short of OTLP, SLO/overhead results, TLS, adapter-internal sf-sql spans, full observability or production admission.
- **Qualified:** The public closed PostgreSQL Direct Mapping path has required CI evidence for exact candidate observation and mapping derivation plus every internal request's binding-matched generation lease. The runtime role is non-owner and NOINHERIT with no memberships or elevation/mutation authority; within the closed profile's database/public-schema/mapped-base-table surface it requires CONNECT/USAGE/SELECT plus the exact collation probe. Isolated live mutations fail closed and actual SET ROLE, DDL and DML are denied. The path charges 34 metadata source-work units before pool I/O, retains one compiler permit without requeue, executes mapped SELECT/ASK/CONSTRUCT on the sole protected repeatable-read transaction, rechecks the same facts and rolls back within a cleanup bound; a 33-unit source-work budget, drift, cancellation, retained clients and unsafe drops fail closed. Exact PostgreSQL 16.9/16.15 observation-profile receipts pass two fresh runs per patch and independent clean replay. Required owned-TLS CLI evidence on both patches now qualifies authenticated SELECT/ASK/CONSTRUCT and lineage, mandatory traffic-independent drift/rebuild, fixed ontology, DDL-conflict rejection/recovery, bounded shutdown and wrong-CA/no-PK startup rejection. This is not production admission or authored-R2RML/other-backend generation qualification.
- **Qualified:** The public source boundary requires certificate- and hostname-verified TLS for PostgreSQL/MySQL DNS and non-loopback targets. Local literal-loopback plaintext remains a development profile; private trust or explicit TLS selection forces verification there too. Exclusive environment PEM roots are resolved once and bounded to 64 KiB/64 certificates. PostgreSQL query/control pools and dirty-session cancellation share immutable trust. MySQL disables socket fallback and certificate/name bypasses. Network startup is capped at 30 seconds and PostgreSQL create/recycle use the pool-wait bound. Required real-TLS protocol peers prove trusted queries, name/CA rejection, no plaintext downgrade, fresh MySQL crypto setup and same-policy cancellation; real CLI negatives reject before file/network I/O. Required native CLI evidence against digest-pinned PostgreSQL 16.15 and MySQL 8.4.11 proves authenticated exact single-source and mixed two-source UNION queries, actual encrypted sessions, wrong CA/name rejection and independent second-source trust. Exact-release-artifact qualification and backend admission remain open.

See the generated [capability/backend/standards matrix](docs/capability-matrix.md)
for per-cell evidence grades, exact limitations, and dated standards reference metadata.

<!-- capability-matrix:end -->

## Why this exists

Operational data is split across systems whose table names and schemas do not share meaning. Warehouses and ETL provide a common view by copying data, trading freshness, storage, and operational simplicity for convenience.

semantic-fabric leaves the data in place and exposes it as one virtual, ontology-shaped RDF graph. Consumers query stable domain concepts rather than per-system schemas; the source database still performs the set work.

| | Warehouse / ETL | semantic-fabric |
|---|---|---|
| Setup | Pipeline plus duplicate store | Database plus an R2RML mapping |
| Freshness | Last completed load | Live at query time |
| Instance storage | Full second copy | None |
| Query surface | SQL over the copy | SPARQL over the live source |

The architecture is governed by [ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md),
[ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md), and [ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md).

## How it works

```text
SPARQL 1.2
    │ parse with Oxigraph/spargebra
    ▼
algebra ── unfold against mappings M + tier-1 T-box saturation
    │ ISWC-2018 base translation and operator-tree normalization
    ▼
relational plan ── dialect SQL + parameters
    │ SQLite / PostgreSQL / MySQL
    ▼
live source ── set work and native spilling
    │ bounded RowStream batches
    ▼
RDF terms or SPARQL results; the A-box is never retained
```

Key properties:

- **Virtualisation only:** no persistent triple store or ETL mode
  ([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md)).
- **One shared executor core:** dialect SQL plus thin native backend adapters
  ([ADR-0024](docs/adr/ADR-0024-executor-backend-abstraction.md)).
- **Bounded simple streaming profile:** `O(|T| + |M| + batch)` for the measured
  simple streaming path; some global operators remain release blockers
  ([ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md)).
- **Correctness before coverage:** unsupported shapes return an explicit
  `501`/`Error::Unsupported`, never a guessed answer
  ([ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)).
- **RDF 1.2 / RDF-star:** native triple terms and reification over live SQL,
  with the basic encoding confined below the visible query surface
  ([ADR-0029](docs/adr/ADR-0029-rdf-star-mapping-extension-rml-star-vocabulary-basic-encoding.md)–[ADR-0032](docs/adr/ADR-0032-rdf-12-soundness-completeness-native-reification.md)).
- **Named graphs:** `GRAPH <g>` and `GRAPH ?g`, including normalized
  subject-map/POM graph unions and exclusion of `rr:defaultGraph` from named
  graph bindings
  ([ADR-0035](docs/adr/ADR-0035-variable-graph-querying.md)).

## Quick start

Prerequisite: the pinned Rust toolchain in `rust-toolchain.toml`.

```bash
cargo build --locked --release -p sf-cli --no-default-features
target/release/semantic-fabric --help
```

This serving-only build retains SQLite/PostgreSQL/MySQL but excludes developer commands and dependencies. Build only `sf-cli`: a workspace build can re-enable development backend features. `bash scripts/check-serving-profile.sh` checks the dependency boundary; CI also runs serving regressions and live source TLS with defaults disabled. This is not yet a qualified release artifact.

For development, `cargo run --locked -p sf-cli -- --help` keeps all three commands through the default `development-tools` feature:

| Command | Purpose |
|---|---|
| `serve` | Read-only SPARQL query endpoint over a live relational source |
| `conformance` | W3C RDB2RDF suite over SQLite with EARL reporting |
| `bench` | GTFS-Madrid OBDA workload over SQLite |

Start the endpoint with an R2RML mapping. Inject a random 32–1024-byte `SF_QUERY_BEARER` through your secret store/process manager first; do not put the credential in arguments or URLs. Its holder can read all mapped data. Clients send `Authorization: Bearer <credential>`. Without an access mode, queries return `403`; `--allow-unauthenticated` explicitly opts into unrestricted development access. Keep real credentials behind a trusted TLS edge; the listener itself is plaintext. This is query admission, not tenant/row-level authorization; see [ADR-0018](docs/adr/ADR-0018-security-edge.md).

For an authored PostgreSQL mapping backed by existing RLS policies, additionally use `--pg-rls-context-env SF_PG_RLS_CONTEXT`. Inject that variable as a JSON object of trusted custom settings, for example `{"app.tenant_id":"tenant-a"}`; the name is an operator-selected example, not a Product Mock convention. The profile requires an RLS-enforced non-owner reader and simple `public` base-table mappings. It binds identity within each read-only query transaction and rolls back or discards the connection before reuse. It rejects unsupported sources and never falls back to unfiltered execution. See ADR-0018 for exact bounds and exclusions; the separate portable equality-row profile below covers a narrow authored-mapping subset. General ABAC, sensitivity, external identity issuers and policy reload remain unfinished.

For multiple callers on **one server and pool**, use `--auth-subjects-env SF_QUERY_SUBJECTS` instead of the single-principal flags. Inject a versioned registry such as:

```json
{"schemaVersion":1,"subjects":[
  {"subjectRef":"opaque-a","credentialEnv":"SF_CALLER_A","postgresRlsContextEnv":"SF_CLAIMS_A"},
  {"subjectRef":"opaque-b","credentialEnv":"SF_CALLER_B","postgresRlsContextEnv":"SF_CLAIMS_B"}
]}
```

Resolve each credential and claims variable through your process manager/secret store. Every registry member requires explicit RLS settings; there is no read-all
fallback. Clients still send only the bearer credential. Claimed subject/tenant
headers cannot override the provisioned identity or settings. The registry is
loaded once, rejects duplicate identities/credentials/settings, and supports
1–256 subjects. Changes require a new server; this is not OIDC or hot reload.

For portable per-caller row isolation, use registry schema version 2 and replace
each subject's PostgreSQL settings with a non-empty `portableRows` array:

```json
{"schemaVersion":2,"subjects":[
  {"subjectRef":"opaque-a","credentialEnv":"SF_CALLER_A","portableRows":[
    {"sourceIndex":0,"table":"people","column":"tenant_id","valueEnv":"SF_TENANT_A"}
  ]},
  {"subjectRef":"opaque-b","credentialEnv":"SF_CALLER_B","portableRows":[
    {"sourceIndex":0,"table":"people","column":"tenant_id","valueEnv":"SF_TENANT_B"}
  ]}
]}
```

The operator supplies exact mapped table/column names and injects each value
through its named environment reference. Values are resolved once at startup,
included in the immutable policy identity, redacted from diagnostics, and sent
to SQLite/PostgreSQL/MySQL SQL only as bound parameters. Every table reached by
the query needs a matching rule for its snapshot-local source index. The profile
admits direct tables and the compiler's validated simple single-table projection;
joins inside authored SQL, CTEs, recursive paths, generated Direct Mapping and
other unproved shapes fail with `403` before source acquisition. Required public
execution evidence currently covers SQLite SELECT/ASK/CONSTRUCT and the exact
two-source SQLite UNION profile; dialect tests cover bound SQL emission for all
three product dialects. This is equality-row authorization, not general ABAC,
sensitivity masking, policy installation, or live PostgreSQL/MySQL qualification.

`serve` can load the same typed settings from a bounded TOML file. Precedence is
validated defaults, then TOML, then `SEMANTIC_FABRIC_*` environment variables,
then explicit CLI arguments. Higher source, mapping and secondary-source selectors
replace their mutually exclusive group. Rotating a bearer reference preserves its
row policy; replacing a subject registry with a lone unrestricted token is rejected.
Explicit anonymous mode must not conflict with credentials or row policies, and
`allow_unauthenticated = false` never enables it. Values merge before effective
scalar validation and cannot inject CLI flags. Unknown TOML fields, non-regular
files and inputs over 1 MiB fail safely; environment and effective settings have
the same aggregate cap. `serve --help` needs no working configuration. Store only
environment-variable names for credentials and policy values; their contents are
resolved once by the existing redacted startup boundary.

Remote PostgreSQL and MySQL sources use certificate- and hostname-verified TLS.
PostgreSQL `sslmode=prefer` is upgraded to `require` for DNS/non-loopback hosts;
remote `sslmode=disable` is rejected. MySQL verification bypasses and explicit
sockets are rejected, and automatic socket fallback is disabled. Literal loopback
addresses retain local plaintext development support; use PostgreSQL
`sslmode=require` or MySQL `require_ssl=true` to require TLS there too.

Public CA roots are bundled. For an exclusive private CA bundle, inject PEM certificates through `--source-tls-roots-env SF_SOURCE_CA` (second source:
`--source-tls-roots-env-2`). These are also `[source]` TOML fields using underscores
and `SEMANTIC_FABRIC_*` settings. Bundles are resolved once, limited to 64 KiB/64
certificates and never logged. They force TLS even on loopback. PostgreSQL queries
and cancellation share the same immutable trust settings. Network startup has a
30-second ceiling; PostgreSQL creation/recycling additionally uses the pool-wait
bound. Required peers test TLS, downgrade rejection and cancellation. A required
native CLI test also serves authenticated exact queries against disposable,
digest-pinned PostgreSQL 16.15/MySQL 8.4.11 TLS servers, including their mixed
two-source UNION and independent-CA/hostname failures. Release-artifact
qualification and backend admission remain pending.

```toml
# semantic-fabric.toml
[source]
source_env = "SF_DATABASE_SOURCE"

[mappings]
mapping = "/srv/semantic-fabric/mapping.ttl"

[graphs]
ontology = "/srv/semantic-fabric/ontology.ttl"

[governance]
timeout_secs = 30
max_concurrent_requests = 64
max_result_items = 100000

[observability]
log_level = "info"
metrics = true

[serve]
bind = "127.0.0.1:7878"
shutdown_timeout_secs = 30

[security]
auth_subjects_env = "SF_QUERY_SUBJECTS"
```

Run it with `semantic-fabric serve --config semantic-fabric.toml`. Every field also has an uppercase `SEMANTIC_FABRIC_` override matching its CLI spelling, for example `SEMANTIC_FABRIC_TIMEOUT_SECS` and `SEMANTIC_FABRIC_LOG_LEVEL`.
Opt in to authored ontology/mapping/schema observation with `--reload-interval-secs 30`, `[serve] reload_interval_secs = 30`, or `SEMANTIC_FABRIC_RELOAD_INTERVAL_SECS=30`. Zero (default) disables polling; enabled values are 1–86400 seconds. One off-path worker validates all sources and atomically replaces the application generation. Detected drift or a failed build makes readiness/new queries return `503` until a valid rebuild; old request leases remain pinned. The 60-second attempt deadline fences readiness and retains unfinished worker ownership. Source endpoints, resolved credentials, TLS roots, caller policies and service limits stay fixed. This works for authored SQLite/PostgreSQL/MySQL single-source and the bounded two-source UNION; it is not a protected backend DDL lease or configuration/policy hot reload. Separately, `--direct-mapping-base https://example.org/data/` selects one PostgreSQL 16.9/16.15 source with permanent public PK tables under ADR-0050's closed lease profile. Its non-owner NOINHERIT read-only role needs CONNECT/USAGE/SELECT and collation-probe execution; row-policy profiles, companions, other backends and no-PK tables reject. The ontology/base/configuration stay fixed. Direct observation is mandatory: zero selects five seconds, nonzero overrides; control attempts are bounded to 30 seconds and startup acceptance to 60 seconds. Request/control cleanup shares the documented shutdown allowance.

```bash
# SQLite
cargo run --locked -p sf-cli -- serve \
  --auth-token-env SF_QUERY_BEARER \
  --source sqlite:/path/to/app.db \
  --mapping /path/to/mapping.ttl \
  --ontology /path/to/ontology.ttl
# PostgreSQL
cargo run --locked -p sf-cli -- serve \
  --auth-token-env SF_QUERY_BEARER \
  --source 'pg:host=localhost dbname=app' \
  --mapping /path/to/mapping.ttl \
  --ontology /path/to/ontology.ttl
# MySQL; SF_MYSQL_SOURCE is injected by the process manager or secret store
cargo run --locked -p sf-cli -- serve \
  --auth-token-env SF_QUERY_BEARER \
  --source-env SF_MYSQL_SOURCE \
  --mapping /path/to/mapping.ttl \
  --ontology /path/to/ontology.ttl
```

Choose exactly one primary selector. `--source` accepts only credential-free values; credential-bearing PostgreSQL/MySQL values must use bounded `--source-env` resolution, and parsed inline passwords fail before runtime, file, or network I/O. An optional `--source-2`/`--source-env-2` plus `--mapping-2` pair enables the sealed two-source `SELECT UNION` and bounded two-pattern inner join described in [ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md). The join requires distinct subject/object variables per arm, constant source-affine predicates, base-table mappings and a reversible shared key; it caps complete driving/probe triples at 128/4,096 and stages the capacity-checked response before 200. Wider shapes reject; this is not general federation.
Every `serve` invocation requires an explicit `--ontology` Turtle document. Authored mappings receive a schema-independent preflight before connector I/O; after a bounded source observation, the Rust product projects the executable mapping IR with effective datatypes and joins it with that ontology. The sealed gate runs three Core shapes through rudof Native and the exact parsed datatype `sh:select` once globally; blank POM focus fails closed and only violation/warning counts survive. Validation policy v2 plus the warning policy, mapping origin, ontology and projection partition admission/compile/cache identity before any binding. A reserved projection namespace prevents ontology laundering and every configured source must pass before publication. This receipt is not a physical-schema generation lease and does not admit live Direct Mapping or any backend to production. Optional flags include `--bind`, `--timeout-secs`, `--shutdown-timeout-secs`, `--max-query-len`, `--max-concurrent-requests`, `--max-source-work`, `--max-result-items`, `--max-order-rows`, `--max-order-bytes`, `--max-serialized-bytes`, and PostgreSQL/SQLite pool sizing.
The shared request-admission ceiling defaults to 64—a conservative finite governance value, not a throughput result. The default endpoint is `http://127.0.0.1:7878/sparql`; an exact query-less `GET`/`HEAD` returns its fixed, redacted Turtle Service Description.

```bash
curl -s 'http://127.0.0.1:7878/sparql' \
  -H 'Accept: application/sparql-results+json' \
  --data-urlencode 'query=PREFIX gtfs: <http://vocab.gtfs.org/terms#>
    SELECT ?route ?agency WHERE { ?route a gtfs:Route ; gtfs:agency ?agency . }'
```

`GET` and `POST /sparql` implement a strict read-only subset of draft SPARQL 1.2 Protocol: exactly one GET/form query or one raw query body. Raw bodies have a per-request cap `n`; forms have a checked `3n+16` wire cap and decoded cap `n`. Version, dataset, duplicate and extra parameters reject; unsupported media rejects without polling the application body.
SELECT/ASK returns SPARQL Results JSON/XML/CSV/TSV; CONSTRUCT returns Turtle/N-Triples/JSON-LD. SQLite also has required Turtle endpoint evidence for a versioned one-target-expression, one-hop DESCRIBE profile: the target is one constant IRI or in-scope variable, may resolve to several resources, and repeated resources collapse into one RDF-graph set union. The complete lowered plan still passes normal single-source admission. Blank-node objects are emitted but not recursively followed, so this is not Concise Bounded Description or unrestricted DESCRIBE. For active application work, Tower `Service::call` mints one deadline after HTTP/request-target parsing and before Axum route/method dispatch. Fixed health and Service Description metadata bypass query-work accounting. The service then fail-fast admits active work before Router or body polling: saturation creates no internal queue and returns stable `503 service-overloaded` plus `Retry-After: 1`; an expired deadline wins as `504`, and a closed gate is an internal `500`. Values outside `1..=Semaphore::MAX_PERMITS` fail startup before source, file, runtime, or network I/O.
The admission permit follows the shared request budget into active handlers, producers, compiler closures, and backend workers, but not into the later draining of already-completed bounded body bytes. Each public compilation entry (lineage preparation, preflight, authoritative compile, including cache hits) precharges decoded UTF-8 query bytes against the same compiler-work allowance after authentication/policy validation; insufficient input capacity returns redacted 429. This input floor now shares its counter with measured normalization/lowering/nested-cascade clone work and checked tree inner-join candidate products/left-branch copies plus atom-resolution candidates/logical-source copies and path mapping-search/complement work with exact term/source copies and mapping-wide graph inventory/reflexive checks on compilation misses and preflight; shared cache hits do not replay avoided work. Public parsing now runs in a fresh bounded Rust worker with request-control-aware waits, exact reap and bounded QueryV1 transfer; the parent never reparses query text. The CLI verifies its held executable before readiness; Rust embeddings must install `sf_sparql::ParserRuntime::prepare` using the explicit `semantic-fabric-parser` host (build with `cargo build --locked -p sf-serve --bin semantic-fabric-parser`). Missing runtime fails closed. Full-file SHA is an evidence-only diagnostic, not a serving startup scan. Required ordinary/lineage CLI tests cover nested, Unicode-created, additive and malformed inputs, server survival and cap-one exact recovery. These are bounded parser-lifetime and operation-local compiler controls, not total compiler-work governance or complete syscall/release qualification. Streaming terminal state is out-of-band: a buffered prefix can be followed by exactly one stable `result stream failed` error and fused EOF, with no `Content-Length`. Owned serving SQLite separately acquires its selected physical connection's cap-one lease before worker submission and interrupts active VM work after mutex acquisition.
This is zero-queue shedding, not fairness, a socket/rate limit, or recursive-work accounting. Owned PostgreSQL/MySQL serving queries now attempt native stop on timeout/disconnect and discard dirty connections, retaining request capacity during cleanup (PostgreSQL one second; MySQL two). Success must acknowledge draining/reset before reuse. MySQL uses a separate same-credential/TLS session with random ownership witnesses; a missing/NULL/mismatched witness refuses KILL. Public MySQL construction avoids unowned setup queries: default client packet/idle limits are 4 MiB/30 seconds, preserving explicit settings; these are not server timeouts. Required owned live tests observe stopped database work, cap-one recovery, a stricter MySQL timeout returning 504, and forced SIGTERM during ASK/SELECT/CONSTRUCT on both backends. The pinned mixed UNION/join matrix additionally proves native stop under timeout/disconnect/SIGTERM with granted-lock witnesses, separate same-credential CLI sibling safety and exact full-bag recovery through both cap-one pools. Arbitrary proxies, raw embedding-pool construction, wider backend/profile combinations, compiler/SQLite blocking work and post-`200` atomicity still need qualification under [ADR-0010](docs/adr/ADR-0010-security-and-resource-governance.md). `GET /livez` reports event-loop liveness; `/readyz` projects snapshot readiness without source polling. SIGTERM/Ctrl-C enters `Draining`, rejects new budgets and closes ingress while admitted work may finish; at the original `--shutdown-timeout-secs` deadline, `Forced` cancels survivors and allows three further seconds for owned cleanup. If cleanup remains active, shutdown fails rather than reporting a clean exit.

## What works today

The generated matrix above is authoritative. Required SQLite evidence covers the frozen core query, strict read-only HTTP subset and bounded one-target-expression, one-hop DESCRIBE profile. PostgreSQL now has a required-CI public closed Direct Mapping generation lifecycle with exact 34-unit metadata source-work accounting, binding-matched request leases and mapped SELECT/ASK/CONSTRUCT. Exact 16.9/16.15 observation-profile receipts pass two fresh runs per patch plus independent clean replay, and the separate owned-TLS CLI lifecycle check qualifies public startup, query forms and drift/recovery on both patches; no backend is production-admitted. The narrow two-source `SELECT UNION` and bounded inner join profiles (including verified-TLS PostgreSQL/MySQL), opt-in atomic authored reload with fail-closed observed drift, typed layered startup configuration, fixed Service Description and health/readiness endpoints, three-phase bounded shutdown, a closed Serve-only structured request/compiler trace slice, and exactly 5,000 deterministic per-PR SQLite SELECT differential cases are implemented. Broader federation and DESCRIBE, metrics/OTLP and the rest of observability/TLS, configuration/policy hot reload and protected backend generations, the 100,000-case nightly train, other generated backends/forms, total resource governance and production admission remain open. Property paths, RDF-star, named graphs, R2RML, Direct Mapping and simple streaming retain the exact deviations recorded per matrix cell.

## Correctness and verification

The load-bearing authority is direct repository evidence, not a model or harness score:

- W3C RDB2RDF passes across the exact 87-case inventory: **81/87 SQLite**, **80/87 PostgreSQL**, **74/87 MySQL**. The one shared deviation is `R2RMLTC0002f`. SQLite has five and PostgreSQL six typed fixture-load skips; MySQL instead has 12 deliberate `DirectMappingUnsupported` product outcomes under `RequirePrimaryKey` (all 12 pass on PostgreSQL), as documented in [ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md) and [ADR-0015](docs/adr/ADR-0015-datatype-dialect-correctness.md).
- The canonical mapping-input inventory seals 1 suite manifest, 26 scenarios, 87 case identities (63 R2RML and 24 Direct Mapping), 189 case-tree files, every SHA-256 digest, and the per-backend allowed-outcome policy. It is mapping
  fixture evidence, not a SPARQL query/protocol conformance claim.
- SQLite, PostgreSQL, and MySQL runners consume that inventory in canonical order;
  per-ID policy rejects count-neutral outcome drift, missing sealed input is fatal, and PostgreSQL/MySQL provider absence fails required-live replay. Backend-aware v5 receipts bind the execution type profile and all 87 ordered identities, kinds, statuses, and typed causes: SQLite records 81 pass / 1 deviation / 5 fixture-load skips, PostgreSQL 80 / 1 / 6 fixture-load skips, and MySQL 74 / 1 / 12 typed `DirectMappingUnsupported` outcomes. MySQL's `mysql-w3c-sql-2008-v1` profile is conformance-only; native product MySQL treats ambiguous `TINYINT(1)`/`BOOL` as integer unless explicit `rr:datatype` supplies authority. Receipt provider image/toolchain provenance is explicitly unbound; pinned exact-image CI/live runs are mapping evidence, not SPARQL Query/Protocol
  conformance, production admission, or provider provenance.
- Per-test expected SQLite query and Protocol regression baselines are now
  receipt-bound. They are product regression oracles, not evidence of W3C SPARQL
  Query/Protocol conformance, runtime provenance, or backend admission.
- The living `tests/rust-dependency-closure-current.tsv` receipt covers the default **developer** profile's locked packages/features/edges; `tests/rust-dependency-closure.tsv` remains byte-frozen historical evidence. Neither attests
  binary bytes, build-script output, linker or system provenance, an SBOM,
  reproducibility, or production admission.
- Differential suites compare flat and operator-tree planners with native
  materialized RDF and spareval across ordinary queries, paths, graphs, and
  RDF-star.
- The ignored exact static Product Mock gate parses sealed T and fixed-source M,
  projects 3,064 triples, joins them with 47,463 ontology triples, and validates
  the 50,527-triple closure at 0/0. Uncontrolled validation-only observations of
  about 1.3–1.9 s are diagnostic, not a benchmark, SLO, or backend-admission claim.
- The 2026-09-04 discovered engineering-harness inventory contained 980 cases across 126 files. Its hermetic checkpoint passed 971 cases across 125 files with two intentional skips; the separate seven-case mutable ambient Ruflo collector passed three fail-closed controls and rejected four positive cases on untrusted installed/runtime files, granting no authority.

Reproduce the primary gates:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace --all-targets
cargo test --locked --workspace
cargo run --locked -p sf-cli -- conformance
```

## Application-completion programme

The issue-independent [completion programme](docs/plans/sota-application-completion-programme.md) is governed by accepted [ADR-0055](docs/adr/ADR-0055-v1-product-completion-and-release-profile.md).
It preserves the virtualisation-only, Rust-native, cross-RDBMS charter.
[ADR-0038](docs/adr/ADR-0038-sota-application-completion-programme.md) is superseded
as completion authority; its dated evidence and post-1.0 research backlog remain
visible, not relabelled complete.

The application is **not complete**. Public bearer/row-policy admission, authored reload/drift,
configuration/TLS and a bounded cross-source join are implemented. General policy/lifecycle,
total controls/native cancellation qualification, complete observability, backend admission and exact-artifact
qualification remain required; only public behavior and required negative/live evidence close them.
The generated capability table above is the current evidence-scoped status.

Release also requires ADR-0055's clean-build smoke, admitted backend matrix,
SBOM, licence/advisory disposition, checksums, signature and provenance.
Advanced capture/witness quorums, exhaustive runtime closure, two-builder
agreement, controlled research benchmarks and harness evolution remain
post-1.0. They are unfinished work, not mandatory additions to the v1 gate.

The canonical Product Mock gold is in `semantic-builder`:
`expected-ontology.json`, the Turtle `categories/` tree and
`candidate-manifest.json` under
`docs/reviews/semantic-product-mock-gold-candidate-v0.1.0/artifacts/`.
Generated `.metaharness` copies are not canonical inputs. Rust KATs seal the
gold and source manifests, select the in-charter relational R2RML subset and
compare public HTTP results with direct SQL. Generic RML is outside this
application's charter. The dated live results, drift failures and precise
non-admission boundaries are retained in the programme and capability catalog.

## Open-issue remediation closeout

[ADR-0036](docs/adr/ADR-0036-correctness-first-open-issue-remediation.md) and
the [execution plan](docs/plans/open-issues-ruflo-metaharness-implementation-plan.md)
record the complete decisions and evidence.

| Issue | Disposition | Evidence |
|---|---|---|
| [#8](https://github.com/sparkling/semantic-fabric/issues/8) incompatible binding pruning | Closed: every incompatible subject/predicate/object/class/graph bind prunes its branch | `10dedd4`; flat/tree/materialized-oracle regressions; green CI `8b66428` |
| [#9](https://github.com/sparkling/semantic-fabric/issues/9) graph-union wrong results | Closed: normalized subject/POM graph union and default-graph handling across BGP, paths, and RDF-star | `5218874`; W3C and differential regressions; green CI `8b66428` |
| [#10](https://github.com/sparkling/semantic-fabric/issues/10) `rusqlite` link conflict | Closed: one workspace `rusqlite 0.40.2`, preserving `bundled` and `column_decltype`; the MySQL dependency chain is also upgraded | `5b8415c`; `mysql_async 0.37.0`; one `libsqlite3-sys` link target; green CI `8b66428` |
| [#7](https://github.com/sparkling/semantic-fabric/issues/7) cloud backends | Open and deliberately deferred. SQLite, PostgreSQL, and MySQL have runtime source paths; none is production-admitted under ADR-0055 | `9d709dd`; provider-specific protocol/security gates remain |
| [#6](https://github.com/sparkling/semantic-fabric/issues/6) Nova collaboration | Closed: federation/materialization pilots work without exposing raw plans; the optional fallible early-exit sink remains consumer-driven | No speculative public API added; green CI and Pages `8b66428` |

The `mysql_async 0.37.0` upgrade resolves the `lru 0.16.4` unsoundness warning,
and a fresh resolution selects fixed `h2`. `cargo audit` is a blocking CI gate
and currently passes its configured policy. That policy has six documented
exceptions: two `quick-xml` denial-of-service advisories in the pinned RDF/XML
stack, three `rustls-webpki` advisories in the SQL Server-only `tiberius` path,
and `RUSTSEC-2026-0235` in an unused optional `rust_decimal` archive feature.
They are accepted exposure, not closure, and ADR-0055 requires owner/expiry/
reachability evidence and removal when upstream constraints permit. Cloud
adapters are not relabelled production-ready merely because mocked happy paths
exist.

## Engineering MetaHarness status

Native Codex/ChatGPT and Claude Code subscription agents perform the normal
edit/test/inspect loop. One integration owner writes and commits on `main`;
read-only investigation/review and compatible tests may run concurrently.
Ruflo is accessed through structured MCP for useful coordination and individual
memory updates; unavailable or malformed recall is not a delivery gate.

[ADR-0055](docs/adr/ADR-0055-v1-product-completion-and-release-profile.md) sets
task-based model allocation and proportional verification. The adapter forwards
explicit Codex effort unchanged, including Astra `max` and `ultra`; omission
uses the native default. No provider API keys, OpenRouter or subscription
spend/token/request/invocation/quota ceilings are permitted. Native subscription
or requested-model failure is reported, not silently routed around.

The private [coding harness](coding-harness/README.md), including its
`supervisor-service/` oracle, is optional, non-deployable Node evidence
infrastructure under [ADR-0037](docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md)
and [ADR-0048](docs/adr/ADR-0048-rust-production-and-node-evidence-runtime-boundary.md).
Its historical closed transactions, isolation, protected inputs, native adapters,
QE/SAST and digest-bound replay remain valid for their exact claims.
Legacy worktree-creating launchers must not run under the main-only rule;
a closed experiment is not a per-commit builder or application-completion test.
The harness has no commit, push, publication, deployment or promotion authority.

The [six-hour review prompt](docs/plans/programme-six-hour-review-prompt.md)
compares promised public outcomes with `main` evidence, checks whether the
previous correction worked, and changes execution when delivery stalls.
The installed user-systemd timer queues that prompt into the pinned conversation
through [the native queue launcher](scripts/queue-programme-review.sh), not a
second `codex exec resume` writer. Queue failure propagates without fallback.

Darwin/GEPA, AVO and retrieval-policy tuning remain off for this programme.
The frozen relevance benchmark and historical receipts are retained; replay
verifies recorded integrity and gate decisions, not a fresh benchmark execution.
No bulk import or direct managed-memory access is permitted. Harness details,
dated test checkpoints and explicit nonclaims live in ADR-0037 and the programme.

## Benchmarks

The reproducible methodology and full caveats live in
[BENCHMARKS.md](BENCHMARKS.md) and [COMPARISON.md](COMPARISON.md).

The strongest measured invariant is constant engine heap during a streamed
CONSTRUCT dump over a file-backed SQLite source:

| Scale | Triples | Peak engine heap | Bytes/triple |
|---:|---:|---:|---:|
| 1× | 5,200 | **129,358 B** | 24.88 |
| 10× | 51,880 | **129,358 B** | 2.49 |
| 100× | 518,680 | **129,358 B** | 0.249 |

The peak is byte-identical across 100× source/result growth. The published
Ontop 5.5.0 comparison uses the same PostgreSQL source and warm HTTP endpoints;
semantic-fabric wins the measured Q1–Q7 cells except one tie within noise, while
the report preserves the pre-optimization Q5 loss and avoids a blanket speed
claim. It is a small localhost workload, not a production sizing result.

## Current status and open work

| Area | Honest status |
|---|---|
| Serving | Working read-only GET/POST query subset of the draft SPARQL 1.2 Protocol over SQLite, PostgreSQL, and MySQL; full Protocol conformance is not claimed |
| RDB2RDF mapping | Receipt v5 seals all 87 outcomes per backend: SQLite 81 pass / 1 deviation / 5 fixture-load skips, required-live PostgreSQL 80 / 1 / 6 fixture-load skips, and required-live MySQL 74 / 1 / 12 typed `DirectMappingUnsupported` outcomes. MySQL's SQL-2008 type profile is conformance-only; provider provenance and production admission remain unbound |
| Cloud/REST adapters | Prototype/library-only; Databricks, AWS Athena, Snowflake, BigQuery, Trino/Presto and other adapters are not admitted to `serve` |
| Property paths | Broad support; explicit `501` residuals remain for bound-endpoint, nested-closure, shape-mismatched, and some reflexive composite forms |
| Named graphs | `GRAPH <g>` and `GRAPH ?g` work in the evidenced SQLite profile. Generated R2RML blank nodes now use `(effective target graph, generated identifier)` identity across default, constant and row-derived graph maps, including direct/reference objects, class atoms, fixed-graph paths and subplan remapping. Dynamic-graph paths and row-dependent rendered-width pooling remain explicit `501` boundaries; PostgreSQL/MySQL still lack direct named-graph matrices |
| Federation | One accepted ADR-0006 vertical serves a top-level `SELECT UNION` with exactly two one-triple arms, each statically affine to a distinct source, as sequential bag-preserving `UnionAll` under one snapshot/budget/serializer. Required tests and a real `sf-cli` child over two file-backed SQLite sources cover duplicates, UNBOUND, admission, failure recovery and generation pinning. A second bounded profile now joins two source-affine triples using a 128-triple driving batch, conservative parameter-bound reducer and at most 4,096 probe triples, exact RDF set/bag semantics and capped pre-200 serialization; independent graph-oracle, failure/boundary and SQLite/encrypted PostgreSQL/MySQL CLI checks pass. Each source has its own statement view, not a distributed snapshot. Wider operators, the full native cancellation/admission matrix and release qualification remain open; general DISTINCT/spill and proposed ADR-0040 are not adopted. External SPARQL `SERVICE` remains excluded |
| Materialization | Not a product mode. A one-off streamed dump uses the query/execution path; Nova owns its downstream bulk-load adapter |
| Exactness and boundedness | Recursive closures use exact finite-pair fixed points on evidenced dialects and reject unproved dialects; known source-sized ORDER/GROUP/DISTINCT/CONSTRUCT fallbacks reject before I/O. Serving has fail-fast aggregate active-work admission (default 64), finite observable probe/open/pull, semantic-result and serializer-byte limits, SQLite cap-one connection admission, and active-VM interruption. Total admitted-path compiler/database/recursive/source-cost governance, raw mutex and submitted-work cancellation, busy/UDF/VFS/I/O and full native cancellation/admission qualification remain product gaps under ADR-0055. Wider excluded global operators and atomic/no-prefix post-`200` delivery are not automatic v1 prerequisites; rejection and fail-terminal stream checks remain required |
| Maintainability | The normalizer is characterized and decomposed, but legacy product/test files still exceed the 500-line rule. Decompose affected code when it enables a concrete product change; do not create a separate whole-repository cosmetic programme or change semantics merely to shorten files |
| Production hardening | Fixed probes, bounded signal shutdown, typed startup configuration, verified source TLS and partial structured tracing/metrics are implemented; required native CLI evidence covers encrypted PostgreSQL/MySQL single-source and mixed UNION execution. Broader security, metrics/OTLP, source health, SLO, cleanup and exact-artifact packaging remain under ADR-0055 |
| Accepted designs not fully wired | ADR-0017 now exposes an opt-in constant-mapping/source SELECT and CONSTRUCT profiles: send exactly `Accept: application/vnd.semantic-fabric.lineage+json-seq` to `/sparql` for per-solution PROV-O, unchanged bindings or product triples and snapshot/logical-plan/policy identifiers. CONSTRUCT frames one response-wide native RDF 1.2 dataset: product triples in the default graph, PROV-O and reification in named bundles; preserve one blank-node scope across all fragments. Require its `complete` record and clean transport EOF. The additional bounded multi-map profile propagates actual origins with finite RDF witness joins and late duplicate merging; its exclusions and caps are documented in ADR-0017. The proved one-map BGP/JOIN/UNION/projection/dedup/slice subset rejects other provenance shapes before I/O; row keys, wider multi-origin operators/federation and wider native-profile/exact-release qualification remain open. Required owned TLS CLI evidence now covers PostgreSQL 16.15/MySQL 8.4.11 exact lineage, empty/allowed portable-policy isolation and locked-source rejection; it does not qualify every lineage operator/lifecycle combination. Twelve multi-map SELECT/CONSTRUCT native stop cases additionally verify exact-target TLS, held-lock stop, unaffected sibling, cap-one recovery and bounded forced exit; no coverage of other lineage paths is inferred from those single-source cases. The separate bounded two-source UNION lineage profile now carries actual source-keyed mapping origins through the public path, preserving bags and blank-node scope with one snapshot/security/budget. Required HTTP checks cover reversed/unbound arms, entailed affinity, portable callers, pinned activation, witness and exact byte/result limits, and cap-one failure recovery. Six additional pinned native TLS UNION cases cover exact-target deadline/disconnect/forced-shutdown stop, unaffected siblings and recovery. Federated lineage CONSTRUCT and wider qualification remain open. The separate bounded federated join lineage profile now seals the actual map/source pair per mandatory arm, follows cost-side swapping, and emits both contributors only for each final matched bag occurrence. It reuses the 128-build/4096-probe executor and capped pre-200 serializer, with explicit map-to-source links and no hidden keys. Required public checks cover exact/projected bags, policy, activation and limits; the pinned TLS CLI aggregate adds twelve both-order join-lineage deadline/disconnect/forced-shutdown cases with exact encrypted target, held-lock, sibling and cap-one recovery witnesses. Native lineage reload, portable/source-RLS cancellation and exact-release qualification remain open. See [ADR-0017](docs/adr/ADR-0017-provenance-lineage.md) for the wire contract. ADR-0018 security and full ADR-0011 observability remain incomplete; ADR-0055 controls v1 and separately deferred research |
| Dependency security | The root `Cargo.lock` is tracked, CI dependency-resolving Cargo commands use `--locked`, and the default `sf-cli` package resolution/feature/edge closure is receipt-bound. A private external observation binds one current binary and observed final-link inputs; the sealed-source smoke round-trips an in-memory `authority=none` record, checks the closed ELF policy identity, and statically parses the exact held bwrap bytes as `RootPie`. A separate `authority=none` counterfactual inventory now binds bounded loader stdout and replayed bwrap-host names/paths under held identity/policy fences. Digest checks detect source drift; private native tests prove the static preflight and narrow late cBPF enforcement. The inventory does not execute bwrap, and its interpreter, DSOs, and path target are unheld/undigested. Receipt V1 remains byte-compatible, does not attest the preflight, inventory, or live late-filter proof, and has no final-FD inventory. None establishes authenticated execution or complete build/tool/system/runtime closure—including actual bwrap-host byte consumption, time-of-use, cache/hwcaps/preload/LSM semantics—opaque ELF semantics, SBOM, reproducibility, minimality, admission, or release. Six advisory exceptions, three unmaintained-crate warnings, hosted-runner/apt-transitive closure, and release SBOM/provenance remain debt |
| Advanced performance/capture evidence | ADR-0041–0047 retain implemented non-authorizing Rust kernels and Node oracles. Authenticated capture, witnesses, controlled research performance, exhaustive runtime closure and two-builder agreement remain open post-1.0; they do not block ADR-0055’s minimum release bundle. No Node component enters the product runtime |

Unsupported shapes are designed to fail explicitly. Exact path fixed points and pre-I/O fallback admission now enforce that invariant for their evidenced scope.

## Workspace

| Crate | Role |
|---|---|
| `sf-core` | Shared mapping/source-affinity IR, neutral relational schema DTOs, RDF terms, graph-map semantics, datatypes |
| `sf-sql` | Dialects, native source adapters, typed binding, schema introspection; re-exports core schema DTOs for compatibility |
| `sf-mapping` | R2RML and Direct-Mapping parsing into the core IR |
| `sf-sparql` | SPARQL algebra unfolding, normalization, SQL emission, execution |
| `sf-conformance` | W3C, differential, mutation, and EARL evidence |
| `sf-bench` | GTFS-Madrid and constant-memory benchmarks |
| `sf-serve` | Read-only SPARQL query HTTP endpoint with finite active-work/deadline/source/result/byte controls |
| `sf-cli` | `serve`, `conformance`, and `bench` binary |
| `sf-validation` | Bounded ontology/mapping semantic admission |
| `sf-capture-supervisor` | Rust capture-state kernel; not an operational or admitted production service |

## Architecture decisions

As of 2026-09-07, the canonical [ADR corpus](docs/adr/) contains 52 records:
38 accepted, 12 proposed and two superseded. [ADR-0030](docs/adr/ADR-0030-metaharness-darwin-mode-dev-process-adoption.md)
is superseded by [ADR-0037](docs/adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md);
[ADR-0038](docs/adr/ADR-0038-sota-application-completion-programme.md) is superseded
by the active [ADR-0055](docs/adr/ADR-0055-v1-product-completion-and-release-profile.md).
The minimal-artifact and wider federation designs (ADR-0039/0040) remain proposed.
ADRs are living plans: `accepted` means the decision is adopted, not that every
public runtime path is implemented. The dated status and direct evidence
determine completion; update affected records in the same verified slice.

| Area | Records |
|---|---|
| Charter, substrate, conformance, execution, rewriting, reasoning | ADR-0001–0008 |
| Governance, tests, datatype correctness, provenance, security, readiness | ADR-0010–0019 |
| Optimisation, Ontop parity, operator-tree IR, backend abstraction, QE | ADR-0020–0028 |
| RDF-star mapping/query, path joins, set/graph semantics | ADR-0029, ADR-0031–0035 |
| Remediation, engineering control plane, application completion and design locks | [ADR-0036](docs/adr/ADR-0036-correctness-first-open-issue-remediation.md)–[ADR-0055](docs/adr/ADR-0055-v1-product-completion-and-release-profile.md) |

Research grounding and prior-art reviews are under [`docs/research/`](docs/research/). RDF-star has a normative [specification](docs/rdf-star/specification.html) and practical [guide](docs/rdf-star/guide.html).

## Contributing and license

Before opening a pull request, run [Correctness and verification](#correctness-and-verification). Architectural changes must update or add an ADR in the same commit.

semantic-fabric is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
