# Application-completion programme

- **Status:** In progress — ADR-0055 v1 completion profile active
- **Date:** 2026-08-26
- **Updated:** 2026-09-09
- **Controlling decision:** [ADR-0055](../adr/ADR-0055-v1-product-completion-and-release-profile.md) (accepted v1 profile)
- **Historical programme:** [ADR-0038](../adr/ADR-0038-sota-application-completion-programme.md) (superseded; post-1.0 SOTA backlog retained)
- **Supporting decisions:** [ADR-0037](../adr/ADR-0037-dual-host-ruflo-engineering-metaharness.md), [ADR-0039](../adr/ADR-0039-minimal-production-serving-artifact.md), [ADR-0040](../adr/ADR-0040-bounded-federated-global-operators-and-spill.md), [ADR-0048](../adr/ADR-0048-rust-production-and-node-evidence-runtime-boundary.md), [ADR-0049](../adr/ADR-0049-exact-recursive-property-path-fixed-points.md), [ADR-0050](../adr/ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md), [ADR-0051](../adr/ADR-0051-postgresql-16-public-observed-schema-profile.md), [ADR-0052](../adr/ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md), [ADR-0053](../adr/ADR-0053-grammar-coupled-sparql-parser-governance-and-process-isolation-fallback.md), and [ADR-0054](../adr/ADR-0054-bounded-stable-root-order-windows.md)

**Scope:** Repository source, tests, accepted ADRs, CI, and measured benchmark evidence; GitHub issues and pull requests are deliberately not programme inputs.

**Status note:** This programme now separates v1 product completion from the post-1.0 SOTA and advanced-assurance backlog. Reclassification is explicit; nothing moved to post-1.0 is relabelled implemented, supported, or complete.

## Immediate delivery queue (2026-09-09)

**MySQL exact numeric continuation (2026-09-09):** the public `?o = 1.00` failure returned one row instead of four equivalent spellings. The new full-digit lane covers decimal/all 13 integer-family types over qualified TEXT/VARCHAR/CHAR and native Integer/NEWDECIMAL, without changing raw RDF output. It preserves native scale, signedness, XSD grammar/facets, source validation, policy isolation and anonymous binding order. Read-only Astra Ultra review found missing profile-sensitive Integer provenance; negative tests now reject missing/revoked facts and preserve W3C Boolean validation. The first expanded run reached a YEAR fixture error: string `'0'` seeds 2000, so native fixtures now use numeric literals. Strong public checks separately verify invalid source integers against malformed/nonnumeric constants and runtime-NULL operands in both directions, bounded cap-one recovery and denied-row EXISTS/OPTIONAL. A known UNDEF comparison lowers to a constant expression error before typed operand handling; it is not a runtime-NULL materialization witness. The existing required native query-profile aggregate includes the new matrix; no acceptance command, backend or release-blocking flag changes. Verification: four focused compiler tests and 760 affected-library tests passed (four existing ignored); all other workspace targets passed, with only the unchanged, previously verified 5,000-case SQLite component omitted. An existing rejection assertion required preserving the compatible-decoder error wording; test-only Clippy style findings were corrected. Strict workspace Clippy, locked workspace build and serving-only build passed. The first required aggregate completed its semantic assertions but failed at an incorrectly post-teardown TLS observation after 392.44 seconds; each TLS witness now runs while its corresponding application client remains live. The short source-validation/TLS check passed in 6.87 seconds before the corrected required PostgreSQL16.15/MySQL8.4.11 semantic aggregate passed in 360.04 seconds. This is combined integration evidence, not exact-release qualification. Wider native floating/arithmetic, lifecycle/control and exact-release gates remain open.

**Evidence-selector correction (2026-09-09):** `authenticated_public_queries_require_verified_source_tls` tests transport/authentication, lineage, reload and cancellation; it does **not** invoke the numeric/query-profile fixtures. Earlier passing runs of that selector must not be read as full semantic-profile evidence. The required `cmd-native-query-profile-live` command selects `query_profile::native_describe_and_recursive_paths_are_exact`, which calls the natural NUMERIC, FLOAT and numeric-text matrices on owned providers. Keep focused evidence distinct, and derive the final selector from the catalog before running it.

**Explicit datatype closure:** R2RML's matching-natural-datatype rule is wired through Rust construction, source-qualified identity and raw-preserving set-work. Qualified PostgreSQL/MySQL decimal and MySQL temporal CLI cases, SQLite public DISTINCT/GROUP BY/mixed-IRI cases and bounded literal federation reduction are covered by existing required checks (ADR-0015/0034). Review caught and corrected dynamic datatype loss, raw grouping and comparison/projection authority confusion. The natural/explicit column BGP slice now passes authenticated SQLite and required owned PostgreSQL/MySQL decimal/integer joins, EXISTS/OPTIONAL, type mismatches, lexical overrides, NULLs and denied-invalid-row isolation (ADR-0015/0034). Review closed MySQL Integer/Boolean UNION metadata promotion and legacy no-datatype fallback hazards; matching explicit types cannot borrow unqualified raw recipes. The native aggregate passed in 95.79 seconds; the 5,000-case generated SQLite component passed in 703.28 seconds. After catalog-hash refresh and the separately tested legacy guard (which does not affect that generated component), all remaining locked workspace targets passed. This is combined integration evidence, not one clean first-pass release qualification. Broader native/floating/temporal and coercing-UNION identity, arithmetic, total execution/lifecycle controls and exact-release qualification remain open; no release flags, acceptance commands or backend scope changed. The following PostgreSQL FLOAT4/FLOAT8 identity slice now passes the public edge/cross-width matrix in 4.39 seconds and the required owned PostgreSQL/MySQL TLS aggregate in 128.26 seconds. Exact raw/scientific keys preserve Rust spelling; D1/DISTINCT keep signed zero and normalize NaN payloads without replacing native output. Read-only review corrected a hidden-column pooling over-rejection; safe same-width pools and independent unlike-width UNIONs remain supported. Consumed coercing pools, floating value arithmetic and wider native identity remain required follow-up (ADR-0015/0034). Integration checks passed all other workspace targets with the unchanged, previously verified 5,000-case SQLite component omitted. One existing parser closed-pipe test exposed a 100ms/1s scheduling race; `0b8fefa` separates closed-pipe and expired-deadline assertions without changing production controls. The full affected library then passed 746 tests; strict workspace Clippy, locked build, format and 113-cell catalog checks passed. This remains combined integration evidence, not exact-release qualification.

**Double value-comparison closure (2026-09-09):** the public REAL 1.1 wrong-answer regression is repaired with a separate RDF-promoted predicate. All six operators now preserve signed-zero bags, NaN/NULL and malformed-literal negation across qualified FLOAT4/FLOAT8 and native integer/full-range NUMERIC operands. Required owned TLS tests cover exact rounding boundaries, facet-checked integer-derived constants, invalid authorized NUMERIC/string overrides, policy-first EXISTS/OPTIONAL and cap-one recovery. Independent review found the override-validation and subtype-classification gaps; both were corrected before integration. Float-only promotion (including a reproduced decimal/float comparison failure), broader typed-column parsing/arithmetic, native/lifecycle/control and exact-release gates remain open. No backend, acceptance command or release-blocking flag is removed. Verification: the focused public matrix passed in 17.24 seconds; locked workspace tests passed with only the unchanged, previously verified 5,000-case SQLite component omitted, followed by strict workspace Clippy and locked build. The first native aggregate failed an existing lineage recovery assertion while a concurrent build could replace the held CLI image. Executable metadata is intentionally revalidated at parser launch, so these checks are not compatible parallel work. With all builds finished, the complete owned PostgreSQL16.15/MySQL8.4.11 TLS aggregate passed in 125.33 seconds. Keep shared-artifact builds and live CLI suites serial; the overlap is a likely cause, not a captured original error diagnosis. This is combined integration evidence, not exact-release qualification.

**Float-only continuation (2026-09-09):** qualified native INTEGER/NUMERIC and numeric constants now round directly to f32/REAL, with exact overflow/underflow guards and no Double intermediate. Natural PostgreSQL REAL retains RDF Double semantics. Authored xsd:float columns over native INTEGER/NUMERIC/FLOAT4/FLOAT8 have separate raw-lexical parsing authority; FLOAT8 shortest text can differ from binary narrowing at a Float midpoint. The owned public matrix passes all six operators/NOT, 32-bit boundaries, decimal/BIGINT double-rounding counterexamples, invalid raw infinity lexicals, source NUMERIC failure/recovery and denied-row EXISTS/OPTIONAL isolation. No output identity, pool authority, release flag or backend scope changes. General text/other typed-column parsing, wider native arithmetic, lifecycle/control and exact-release gates remain open. Shared-artifact builds and live CLI suites remain serial. Verification: 131 core tests and 752 SPARQL tests passed (four existing ignored); strict workspace Clippy and the locked workspace build passed. The expanded focused public matrix passed in 38.69 seconds, and the complete owned PostgreSQL16.15/MySQL8.4.11 TLS aggregate passed in 124.44 seconds after all builds finished. Independent Astra Ultra review found no remaining blocker in the decoder/rounding scope. No shared-executable overlap or rerun was needed. This is combined integration evidence with the prior workspace boundary, not exact-release qualification.

**Authored Double continuation (2026-09-09):** explicit xsd:double over native PostgreSQL INTEGER/NUMERIC now supports exact value comparisons using retained wire proof. The public failure was reproduced before repair. The expanded all-six-op/NOT matrix passed in 47.42 seconds, including Float/Double constants, full-range rounding, authorized invalid NUMERIC failure/recovery and denied-row EXISTS/OPTIONAL. Read-only review identified the explicit-map negative coverage gap; it is now covered. Missing/foreign scalar proof and revoked natural FLOAT4/FLOAT8 facts still reject. The affected library passed 752 tests (four existing ignored); affected strict Clippy, the locked serving-only CLI build and 113-cell catalog checks passed. This narrow follow-up reuses the preceding broad integration evidence rather than rerunning unchanged suites; it is not exact-release qualification. General typed-column parsing/arithmetic and the remaining native, lifecycle, total-control and release gates stay open.

**Text numeric continuation (2026-09-09):** PostgreSQL TEXT/CHAR numeric mappings now support exact floating-promoted comparisons for all 16 XSD numeric datatypes. ADR-0015 records the 1100-digit-plus-sticky rounding proof, exponent/mantissa cancellation bounds, lexical grammar and integer facets. Authenticated tests cover long midpoint tails, extreme exponents, invalid/NULL/NOT, nondeterministic source collation, unchanged raw RDF spelling and policy-isolated EXISTS/OPTIONAL. The initial standalone fixture lacked its own identifier; this was corrected before the actual public failure was reproduced. SQL-shape tests then caught a qualified SUBSTR keyword-rendering incompatibility; quoting the builtin fixed it without bypassing AST emission. The final public matrix passed in 24.20 seconds. Workspace tests passed with only the unchanged, previously verified 5,000-case SQLite component omitted; the affected library passed 753 tests (four existing ignored), followed by strict workspace Clippy and locked build. The full owned PostgreSQL/MySQL TLS aggregate passed serially in 137.40 seconds. This is combined integration evidence, not exact-release qualification. Non-floating integer/decimal comparisons, other native typed overrides, wider arithmetic, lifecycle/total-control and release gates remain required.

**Exact integer/decimal continuation (2026-09-09):** a public negated comparison reproduced invalid decimal text being admitted as rows. The PostgreSQL repair compares full normalized digit sequences, not SQL strings, native floating values or truncated prefixes. All 14 non-floating XSD numeric types have grammar/facet checks over qualified TEXT/CHAR and native INTEGER/NUMERIC decoders. NUMERIC scale remains visible to integer overrides; missing/foreign proofs reject. The initial expanded TEXT/CHAR/NUMERIC matrix passed in 94.18 seconds. Independent review caught an offline-alias/live-catalog distinction; a branch-level red regression and strict live-unknown negative now pass. Invalid NUL constants also reproduced a transport failure; the guarded expression-error fix passed its public check in 2.64 seconds while retaining invalid native-source failure. The correct required PostgreSQL16.15/MySQL8.4.11 query-profile aggregate passed in 235.00 seconds, including BIGINT, all-six-op/NOT full-tail/facet matrices, raw output and source/policy isolation. Its first run exposed a previously unexercised aggregate-only PK/default fixture conflict; fixture-owned unique IDs preserve the PK constraint. The general TLS suite separately passed in 126.34 seconds and is not numeric-profile evidence. The workspace run passed all other targets but had two existing intermittent SPARQL telemetry/deadline-test failures; the affected library subsequently passed 756 tests (four existing ignored), and the final focused compiler tests, strict Clippy and locked workspace/serving builds passed. This is combined integration evidence, not a clean exact-release qualification or a fix for those intermittent tests. Only the unchanged, previously verified 5,000-case SQLite component was omitted; the shared core/SQLite numeric path, acceptance commands and release flags are unchanged. Other native typed overrides, arithmetic, lifecycle/total-control and exact-release gates remain open.

**Earlier explicit-datatype integration:** the locked workspace boundary passed its existing 5,000-case generated component in 630.36 seconds, then exposed stale optimizer assertions about raw PK/literal uniqueness. Those fixtures now distinguish injective IRI keys from canonical literal collisions, including an executable SQLite BOOLEAN-PK counterexample. All remaining locked workspace tests now pass with only that unchanged, already-passed generated component excluded from the follow-up run. Runtime safeguards remain unchanged; this is combined integration evidence, not a fresh single-command release-candidate qualification.

**PostgreSQL decoder prerequisite (2026-09-09):** corrected the signed digit-count interpretation so a valid 131,072-digit NUMERIC with `.00` scale reconstructs its exact IRI over authenticated owned PostgreSQL TLS. The wire boundary/truncation test and public CLI check pass; the existing required native aggregate includes the same assertion. The following static NUMERIC IRI slice now preserves display scale through listed/fixed/equality/sameTerm lookup, D1/DISTINCT/COUNT, mixed natural consumers, order/slice, native Ref joins and authored SQL-expression results. Hidden non-finite terms fail terminally; denied invalid rows stay excluded and cap-one requests recover. Native payloads remain unchanged. Existing source-sized multi-arm DISTINCT rejection stays tested and closed; compiler UNION-key repair does not promote its serving admission. Natural decimal construction now also passes full-range lexical normalization and authenticated owned TLS SELECT, DISTINCT, hidden COUNT/ASK, mixed raw-IRI and output-cap recovery checks. Natural decimal fixed/sameTerm and separate integer/decimal Eq/Ne now have owned TLS CLI evidence on both providers, including malformed-literal negation, ZEROFILL, NULL/OPTIONAL and PostgreSQL denied-invalid-row policy isolation. Raw payload/provenance remains separate; coercing UNIONs cannot inherit identity proof. General natural BGP/mixed identity, ordered/floating arithmetic and other families remain open. This closes exactness slices, not whole-family or release qualification (ADR-0015/0024/0034). The full-workspace GTFS Q5 regression was repaired first (`ae4146f`): verified-key self-OPTIONAL elimination retains NULL-tolerant decoder checks on the same row; serving constraint authority and required decoder failures are unchanged.

**Current query slice:** required owned PostgreSQL 16.15/MySQL 8.4.11 CLI checks cover one-hop DESCRIBE, exact cycle/258-edge closures and joined-path native folding. The text-collation repair passes SQLite NOCASE/native case-and-trailing-space fixtures plus authenticated outer correlations, retaining native decoding via live text facts and a SQLite prepare-only metadata twin. Aggregate/SubPlan metadata follows actual SQL projection and visits nested plans once per metadata walk. CHARACTER duplicate/connectivity, mixed-width/Unicode/NUL, authenticated correlation and pinned native CHAR tests now pass; explicit compiler callback admission and lifecycle/budget tests protect the SQLite normalization. Atom-local NULL guards now prevent absent subject/predicate/object/ref-parent terms from becoming solutions, including class shortcuts, projection/ASK/COUNT and correlation. Required compiler and authenticated SQLite checks plus pinned native subject/object/class and non-NULL recovery evidence cover this slice. Dynamic predicate maps remain rejected at serving admission; raw fixed-predicate matching is not added to the v1 critical path. Ordinary early-wrapper native folding now passes pinned public SELECT: D1/D2 retain typed recipes until original-source metadata is resolved; stable output names, narrow row-policy/federation proofs and parameter isolation remain. Ordinary non-native-consumer text/CHAR SELECT, DISTINCT, ASK/COUNT and correlations now pass SQLite/native checks using exact decoded keys with preserved raw D1 outputs. Native Ref/policy conditions are explicitly separate and do not license RDF-key substitution. Typed Ref atoms now join and policy-filter both original sources before decoder-key dedup with live text/CHAR proof for every key, retaining raw descriptors, projection bags and graph-scoped blank nodes. Unknown families retain prior per-source D1; distinct signed-zero IRIs survive. The formerly ignored witness regression is active; SQLite HTTP and owned PostgreSQL/MySQL tests pass. OPTIONAL keeps its established decomposition; parameter ordering and original-source preflight have focused checks. Required native CLI TLS/query-profile checks also qualify both source selectors; stale missing/optional selector-test metadata is reconciled without promoting production admission. Ordinary text/CHAR row-policy predicates now run inside D1 before dedup, preserving projected bags and original decoder values. Required HTTP/native checks cover denied rows, native case equality, CHAR duplicates and correlations; a mixed-key signed-zero test rejects partial-key normalization. Ordinary SQLite all-IRI-template mixed D1 now shares live decoder facts with row reconstruction: signed-zero IRIs survive while integer/REAL and CHAR lexical duplicates collapse before projection. Public count/join/UNION/BLOB/date and numeric-filter checks pass, with callback cleanup/source-charge and native mixed integer/collated-text compatibility evidence. Explicit SQLite column literals now preserve lexical/datatype/language identity through SELECT/COUNT, constant matching, BGP joins and sameTerm; a separate typed numeric FILTER path preserves value promotion, precision and NaN behavior without borrowing identity authority. IRI/literal dual-use, NULL errors and callback lifecycle/work-limit checks pass. Numeric VALUES compare in Rust across dialects while existing string-pair VALUES equality remains supported; plain-column literal/IRI mismatches preserve errors under NOT. Owned native HTTP checks additionally cover numeric VALUES, UNDEF and byte-exact datatype-IRI identity under case-insensitive collations. Natural declared SQLite constant matching and native natural integer compatibility have bounded checks; general natural/native/mixed-descriptor identity remains open. SQLite column-IRI identity now resolves decoded operands for constant/BGP/FILTER matching and D1/Ref-atom dedup, preserving raw values, native joins, unique-key soundness and callback bounds. Cross-map fallback retains hidden graph identity and remains source-sized/rejected by serving. Independent processor bases now flow through parser options and both serving sources with precedence, identity and reload tests; column generation correctly prefixes relative values without URL normalization. The template encoder prerequisite is now verified: shared RFC3987 ranges, correct UTF-8 escaping on all three dialects, SQLite malformed-text rejection and public dedup/count, and required owned native different-shape template equality. Late-base core generation and the bounded SQLite IRI-template/constant atom now pass public SELECT/COUNT, fixed matching, mixed static/late joins, hidden graph/object bags, invalid expansion, zero-slot callback and policy-before-dedup checks. Typed operands retain real per-part aliases; no source-key proof replaces finalized IRI identity. OPTIONAL constants preserve absence through existing match/no-match decomposition. Native late-template SQL, mixed natural/literal-column atoms, reference/path/proposition identities and wider pooling remain unqualified and reject. Next: native resolved-IRI execution, broader DISTINCT/GROUP/SubPlan and wider template qualification, general mixed/natural keys, remaining source controls and exact-release qualification. The rejected universal singleton Ref-SubPlan experiment is not part of this implementation. These are correctness gates, not new research; no whole-backend admission or arbitrary deadline follows.

Recovery integration `458faf1` on `main` passed full locked Rust tests/build, formatting, the harness build and focused contracts; it closed fragmented integration, not the remaining public requirements. The 2026-09-09 static single-slot IRI-constant correction now passes authenticated SQLite and owned TLS PostgreSQL/MySQL text/CHAR/integer regressions plus the surrounding native profile. It replaces raw encoded-key inversion with decoder-owned forward identity; the next required native aggregate also passes PostgreSQL boolean/BYTEA and MySQL binary string/blob/NEWDECIMAL lookup, equality/sameTerm, COUNT/OPTIONAL, NULL/negation and exact spelling checks. Typed raw-projection metadata retains separate decoder authority; coercing decimal UNIONs do not inherit it. MySQL BIT/TIME/TIMESTAMP now also pass byte-width, fractional/signed/zero and non-UTC public checks; exact scalar alphabets no longer expand through the generic SQL byte encoder. MySQL DATE/DATETIME lexical-only D1 now preserves original text before both window output and partition copies; required CLI tests cover partial/invalid/zero dates, fractional times, lookup and duplicate/explicit-date-literal bags. Natural/native consumers retain their veto and unprotected temporal facts are revoked across wrappers. PostgreSQL numeric/temporal, native floating and wider template identities remain required follow-up (ADR-0007/0034), not removed features or a closed release gate.

**Natural temporal identity delta (2026-09-09):** MySQL natural DATE/DATETIME now retains native payload and datatype while decoder-qualified identity keys match canonical Rust output; query constants remain verbatim. Required owned TLS CLI checks cover canonical/noncanonical fixed and sameTerm matches, DATE/leap/year-zero/extrema and DATETIME fractions, duplicate bags, nested projection, mixed literal/IRI joins, NULL/negation, invalid hidden SELECT/COUNT/ASK terms, cap-one recovery and denied-invalid-row policy/existential/OPTIONAL isolation. Coercing mixed temporal SubPlans retain a rejection marker instead of falling back to raw equality. Wider natural/native identity and native-consumer copies remain open. ADR-0015/0024 describe the decoder and authorization boundaries. This closes the qualified natural temporal slice, not general native identity or release admission. Recovery clarification (2026-09-09): the 387 saved delivery commits through `fb7b684` remain pushed on `programme/application-completion-20260909`. After the current main-only instruction, the clean checkout was fast-forward integrated into local `main` with one writer; GitHub `main` remains at `8a91042`, and the delivery branch is preserved. No code was lost during the subscription interruption. Read-only Astra Ultra/Fable support and the existing Cargo harness continue; no new branch/worktree or push is authorized by this recovery. README rewriting is cancelled. Finish the next public-path closure before opening another implementation lane. The serving/developer split builds `sf-cli` alone with defaults disabled and all three database drivers. The 2026-09-09 `0.1.0-dev.1` package now has a pinned Rust/Debian controlled build, immutable image/archive/source identifiers and a non-root/read-only reference deployment (ADR-0039's implemented ADR-0055 subsection). Exact-image Cargo smoke covers authenticated SQLite/PostgreSQL 16.15/MySQL 8.4.11, mixed TLS UNION, wrong credentials/CA, health and SIGTERM. It exposed and repaired the parser's invalid rejection of a legitimate PID-1 parent without changing its containment policies. This closes that concrete deployment defect and adds packaged-path evidence; complete exact-artifact qualification, advisory disposition, SBOM, signing/provenance and other product gates remain open. The development version is not release admission, and proposed ADR-0039 research is not adopted.

| Order | Required outcome | Observable acceptance |
|---|---|---|
| 1 | Public identity and policy enforcement | Provisioned callers now share one server/policy/pool with distinct trusted RLS settings; public forms/UNION, cache isolation and cleanup pass. Finish portable ABAC, external sensitivity, external identity issuers and policy-aware reload |
| 2 | Coherent lifecycle and total request controls | Opt-in authored reload now has immediate drift fencing, atomic generation replacement, fixed policy preservation, timeout/worker ownership and real CLI recovery/shutdown evidence across SQLite and encrypted PostgreSQL/MySQL single/mixed UNION. Native PostgreSQL/MySQL stop/discard has required live timeout/disconnect/cap-one recovery and forced ASK/SELECT/CONSTRUCT SIGTERM evidence; the pinned mixed UNION/join matrix now observes each native session stop with its table lock still granted, preserves separate same-credential CLI siblings and recovers full cap-one results. Shutdown retains the runtime through owned cleanup under the original drain deadline plus a finite three-second forced allowance; exhaustion fails closed. The public closed PostgreSQL 16.9/16.15 PK-backed Direct CLI profile now has required owned-TLS exact query/lineage, traffic-independent drift/rebuild, DDL-conflict/recovery, bounded shutdown and wrong-CA/no-PK rejection evidence. Fixed ontology/configuration, dedicated control pooling and retained candidate/cleanup ownership preserve the accepted boundary. Protected backend generations, policy/config hot reload and total controls remain open |
| 3 | Secure configuration and remaining observability | Public configuration/TLS validation, redaction, bounded metrics/traces and required operational tests pass |
| 4 | Useful bounded cross-source join | Implemented: 128-driving/4,096-probe triple caps, exact RDF merge, public materialized-oracle and encrypted PostgreSQL/MySQL CLI proof, pre-200 overflow/source failure; pinned PostgreSQL/MySQL timeout/disconnect/SIGTERM and full cap-one recovery now pass; finish wider backend/profile and exact-release admission |
| 5 | Minimal Rust serving artifact and release | Clean-build live smoke, backend/profile matrix and every ADR-0055 minimum release check verify one immutable artifact |

No required product guarantee is removed; advanced assurance remains open post-1.0. Forecasts require measured remaining work/dependencies, not commit counts or a replacement four-hour/multi-day promise. The [six-hour correction prompt](programme-six-hour-review-prompt.md) must change execution after a missed outcome and test whether that change helped.

## Execution status and dated evidence

| Slice | Status | Verified evidence |
|---|---|---|
| H0a — frozen replay-policy foundation | Complete | `b40dbc6`; schema-v4 surfaces unchanged; schema-v5 policy fingerprint `11c17544e97c1509456f6efb88081a55bd56c93ac306a9b05c2da7102e5f755b`; 381 tests passed and 2 expected skips |
| H0b — schema-v5 evaluator, scorer and envelope | Complete | `7a1fa24`; accepted golden policy/assessment/envelope `0d5505e4…61bb` / `4f4fe45c…a977` / `fdab0843…65e7`; hardened build; 430 tests passed and 2 expected skips; independent Codex and Claude COMMIT verdicts |
| H0c — trusted-launcher activation | Complete | V6 run `programme_v6_h0c_20260828_02` passed the candidate transaction and every hard gate at 100/100, with seven native-evidence digests, two final native reviews, no retry or repair, a sealed schema-V6 envelope, and provider-free verified replay. V4/V5 remain frozen |
| M0A — Rust application foundation | Active; fail-closed single-source authority boundary implemented | ADR-0048 fixes a Rust-only product closure. `9d228dd` moves neutral schema DTOs to `sf-core`; `67a779a` centralizes dialect capabilities; `faee07a` binds source/backend/compiler/cache; `9d0da85` binds PostgreSQL `public` catalogue identity; `24a0e20` quarantines PK, UNIQUE, FK, FD and NOT-NULL facts. The 2026-09-02 slice additionally quarantines cross-column type authority; probes recursively reachable base Table/Query columns before cursor I/O; captures the full BGP key, including an active graph variable, before projection and remaps it only through physically key-preserving pure unary wrappers onto a private execution clone; revalidates mutable plans before metadata I/O; restores exact zero-variable pooling for fully ground BGPs; propagates conformance metadata failures; redacts streamed executor text; and pins compiler-only SQLite `rowid`/PostgreSQL `ctid` SQL shapes for the synthetic no-PK path sentinel. Offline/synthetic aliases and translate-time immediate wrappers retain a non-authoritative lexical heuristic, never live metadata authority. Real-`rowid` collision safety, CTID stability and live PostgreSQL path execution remain unproved. Raw `rr:sqlQuery` is not schema-confined. Coherent schema generations, typed row identity, atomic reload, Direct Mapping regeneration, federation and production admission remain open. Gold/source/live KATs remain development evidence; ADR-0039 remains proposed |
| M0E — advanced evidence authority | Deferred post-1.0; implemented kernel retained | The Rust transaction/store kernel and Node reference oracle remain valid non-production evidence. Authenticated capture, transparency/witness quorums, two-builder agreement and exhaustive runtime closure stay open and do not block the ADR-0055 minimum release bundle |
| M0 — ADR-0047 final-`WHERE` mutation slice | Complete; additive V3 hosted replay green (non-authorizing) | A protected source-specific quartet freezes all 19 final-`WHERE` deletion spans against the 6,859-byte projection. Two ten/nine-mutant serializable rollback-only batches preserve the unchanged raw-derived control bag, prove 15 hidden seed families and four zero-candidate guards independently, and produce exactly 19 executed, 15/15 non-equivalent killed, four guard-equivalent and zero unresolved. Its historical service verification remains 611/611. Additive V3 retains the V1/V2 receipts and historical replay implementations byte-exact without invoking those runners, then defines `baseline-v1`, `baseline-v2`, `branch` and `final-where`, each over two distinct fresh networkless anonymous-volume containers. PID 1 must be exactly `postgres` and `pg_isready` must succeed. Hosted run [`33636424967`](https://github.com/sparkling/semantic-fabric/actions/runs/33636424967) at exact `d0cc5fb938a1ff8b70859c19882934461fe23c5a` passes both exact-Node lanes. CASE/JOIN/value/nullability/order/duplicate/array/element and `UNION ALL` replacements, live observations and admission remain open |
| M1/M2/M5 — boundedness, governance, snapshots | Active; public identity/source-RLS and portable equality-row subset integrated; lifecycle incomplete | Exact recursive paths, finite ORDER windows, shared request controls, fail-fast admission, SQLite ownership and immutable source-keyed snapshots remain integrated. `c701352` adds the private closed PostgreSQL-16 Direct lifecycle. `26664e5` exposes default-deny bearer admission, exact security-context cache partitions and real access traces; `4279d3d` adds public transaction-local PostgreSQL RLS with required live isolation/cleanup. The provisioned registry selects distinct caller/policy bundles on one server/pool, preserves stable subjects across credential rotation, and rejects cross-subject cache/execution use. Its schema-version-2 portable profile adds bounded exact source/table/column equality predicates as bound parameters, with public SQLite SELECT/ASK/CONSTRUCT, two-source UNION isolation, three-dialect emission and fail-before-source-I/O evidence. General ABAC/sensitivity, live cross-backend portable-policy qualification, external issuers, policy-aware/general reload, total compiler/database/recursive governance, wider operators, access metrics, full lineage and production admission remain open; ADR-0017 now has the bounded public constant-origin SELECT/CONSTRUCT slice detailed in M5 |
| M3 — secure, observable, operable runtime | Typed layered startup configuration, probes, bounded shutdown, partial tracing and default-off bounded metrics integrated | Configuration follow-up corrects token-rotation row-policy loss, argument bypass/injection, unredacted values and blocking FIFO input from the initial slice. Unit/real-child tests now cover these failures, bounded deny-unknown TOML, environment/CLI precedence and an authenticated exact two-source HTTP query. Public CLI/HTTP tests cover explicit `--metrics` on the existing listener and three closed families: query total, query duration and governance rejections. Default operation has no recorder or route. Verified remote TLS, private CA references, PostgreSQL cancellation trust and bounded setup have required peer/CLI tests. Required live CLI evidence covers authenticated exact PostgreSQL 16.15/MySQL 8.4.11 queries, mixed UNION, database-observed encryption and independent-CA/name rejection. OTLP, exact-release-artifact qualification, SLO/overhead qualification, the full metric catalogue, source-health/failure policy, adapter-internal spans and complete cross-backend cleanup remain open |
| M4 — v1 standards profile | Release-surface gates active; research train deferred post-1.0 | Three-backend RDB2RDF receipts, Service Description, bounded SQLite DESCRIBE, static Product Mock `M ⋈ T`, and the exact 5,000-case train remain evidence. V1 still needs its dated advertised-surface/live matrix; the nightly 100,000 train, broad NoREC/MR1, long fuzzing, global coverage/mutation ratchets and soak expansion remain open post-1.0 |
| M6 — cross-source federation | UNION and bounded join implemented; charter-complete M6 active | `43399ce` serves only a top-level SELECT UNION with exactly two one-triple arms that each resolve to one distinct source. One snapshot lease, budget and serializer span sequential `UnionAll`; required compiler/runtime tests and a real CLI child over two file-backed SQLite sources prove bag/UNBOUND semantics, fail-closed admission, failure recovery and generation pinning. A bounded two-pattern join now preserves complete RDF triple-set identity before exact bag merging, retains 128 driving/4,096 probe triples under independent byte/work/result caps, and stages the full capped response before 200. Required oracle/negative, SQLite CLI reload and encrypted PostgreSQL/MySQL join tests pass. Source-local statement views are not a distributed snapshot; native guards are integrated and single-source native stop/recovery is proven, while the full federated cancellation matrix, backend generations/admission and exact-release qualification remain open. Wider algebra/spill and research performance stay outside this slice; proposed ADR-0040 remains proposed |
| M7 — v1 release | Gated | Requires the ADR-0055 product guarantees and minimum exact-artifact evidence. Two-builder, transparency, exhaustive runtime-closure and research benchmark proof remain open post-1.0 |
The current QueryV1 evidence closeout also includes closed malformed-directive proof (`38e9c7a`), one-fingerprint/one-held-descriptor matrix execution with fresh children and an 87% focused runtime reduction (`5a9919b`), and a separately generated default-only Rust closure refresh (`949cf11`). None qualifies the parser dependency/syscall profile or changes product authority.
Runs `_03`, `_04`, V5 `_05`, and V6 `_01` remain immutable honest failures; `_05` was rejected at 85/100 by frozen prior-attempt law, and `_01` failed closed on native-origin policy. The additive V6 contract never reinterprets V4/V5 evidence.
Fresh V6 run `programme_v6_h0c_20260828_02` passed every gate at 100/100 with no retry or repair. Its policy, candidate, receipt, envelope, execution-claim, and provider-free replay digests are `e71107e5…ae34`, `a1dc3071…ac7f`, `d9d244ef…0216`, `02c30ed3…9a06`, `578799ef…9c86`, and `f1bcf0fe…bf02`. H0c is complete; ADR-0055 now requires one integration writer on `main` while M0E expansion stays post-1.0.

The SPARQL regression receipts bind per-test expected SQLite query and Protocol outcomes. They are regression baselines only: they do not attest W3C SPARQL Query/Protocol conformance, runtime provenance, or backend admission. Backend-aware v5 mapping receipts bind the execution type profile and all 87 sealed ordered outcomes: SQLite records 81 pass, one deviation and five skips; required-live PostgreSQL records 80 pass, one deviation and six skips; required-live MySQL records 74 pass, the documented `R2RMLTC0002f` deviation and 12 exact typed Direct Mapping unsupported outcomes under `RequirePrimaryKey`. The MySQL SQL-2008 profile is conformance-only; native product MySQL treats ambiguous `TINYINT(1)`/`BOOL` as integer unless explicit `rr:datatype` supplies authority. Provider image/toolchain provenance remains explicitly unbound; pinned exact-image CI/live runs are mapping evidence only, not Query/Protocol conformance or production admission. The default `sf-cli` dependency receipt closes locked package resolution, enabled features, and normal/build dependency edges only. It does not attest binary bytes, build-script output, linker or system provenance, an SBOM, reproducibility, or production admission.

The current tranche adds a fail-closed **host-observed non-closure observation** for one freshly built current `sf-cli` executable. The first private `0600` external receipt, from clean `5a06eac`, replayed with 363 raw inputs, 357 canonical terminals and three one-hop HostSystem aliases; portable/host/receipt digests are `72ce37b4…9b9a`, `024fbbbd…3ad8` and `173d0698…51ca`. It is uncommitted, unpublished and noncanonical. CI tests only the parser/integration contract on mutable `ubuntu-24.04` and neither captures nor publishes. Linker-only alias authority binds alias topology and terminal bytes while generic authority remains symlink-rejecting; structured GNU-note parsing binds build-ID owner, type, size and digest. The producer still requires an exclusive, quiescent root/effective-UID builder. Same-principal/root ABA, linker time-of-use and path-resolution race resistance are explicitly unattested. The additive runtime-linkage contract canonicalizes strict bounded glibc `ld.so --list` output. Commit `863a058` adds the private descriptor-rooted holder; `c8305c3` adds a private one-shot executor that independently authorizes exact bubblewrap path/digest/length/policy, holds its root-owned inode, revalidates exact sealed-source transfer duplicates, and invokes only that inode via `execveat` with an empty environment, fail-closed FD allowlist, process limits, pidfd/process-group cleanup and bounded cancellable output. Bubblewrap creates a fresh networkless/user-isolated read-only tmpfs containing only sealed-source copies, then runs the copied loader; the strict view must equal prior discovery. Commit `805f413` converts a completed observation to a private canonical record with fixed `authority=none`, 34 `not-attested` fields, domain-separated record/receipt digests, exact tool and binding identities, and bounded raw stdout. Commit `9282e60` checks a caller-supplied closed runtime-ELF tag/search/flag policy ID plus exact five-source byte digest `cd23f2d8…b0a` before construction and during the immediate pre-run validation phase; the native diagnostic maintains a separate literal, but the API authenticates no reviewer. Commit `73e9864` binds separately sealed policy `x86_64-prepared-loader-late-cbpf-default-kill-v1` (`0092c69f…e80a`, 55 cBPF instructions/440 bytes) to both bubblewrap's namespace PID 1/reaper and the copied loader child, requires its single exact FD/argv placement, and proves a same-layout `fstat` control against a `socket`/`SIGSYS` canary. Commit `50adc0a` hashes and parses the same exact held bubblewrap bytes as `RootPie`; `b34b6d7` then creates a separate canonical private `authority=none` inventory under `counterfactual-controlled-name-resolution-not-actual-exec`, deriving interpreter/direct names from the held view and binding environment-cleared, cache-inhibited, empty-hwcaps bounded loader stdout plus replayed names/paths under pre/post bwrap identity/policy fences. `4b14635` brings all five new authority files into the harness protected set. The interpreter, reported DSOs and path-passed target remain unheld/undigested, bwrap is not executed, and replay proves only record self-consistency. Receipt V1's schema and canonical serialization remain unchanged; both `runtime-elf-policy-replay` and `target-seccomp-or-syscall-trace` remain `not-attested`. The digests detect source drift, not the change class, approval, compiled bytes, configuration, dependencies or toolchain. The exact-host workflow keeps both formats in memory, with no writer/importer, signature, witness, product caller or authenticated execution/output provenance. Discovery remains prior and unauthorized; the artifact is not executed; opaque GNU-property/hash/symbol/relocation/version/TLS/cross-table payloads, initialization, `dlopen`/NSS, VDSO and complete runtime closure are unproven; counterfactual names/paths do not bind actual bwrap-host byte consumption, time-of-use, default cache/hwcaps, preload or LSM state; no final-FD inventory, syscall trace or aggregate cgroup containment exists; kernel/bubblewrap/glibc/copy/mount and an exclusive principal remain trusted; and there is no SBOM, reproducibility, minimality, admission, performance or release authority.

Performance machinery exists, and `f2cc800` removes issue-8 lock hardcoding from future V5/V6 patch tasks. Proposed ADR-0041 keeps the first clean-release measurement outside that patch transaction; no controlled runner profile, baseline, candidate, capture receipt, or measured numbers exist. The capture controller now parses a conservative byte-canonical subset of the product runner profile and observes two fixed, read-only `/proc`/`sysfs` snapshots; an AST-walked exact-import/open-call mutation sentinel rejects known command, provider, socket, loader, write-flag and ambient escape forms in the protected current source, but is not an OS capability sandbox. Captured module-private operations distinguish fixed from injected collection locally; durable records prove canonical self-consistency, not independently witnessed collector provenance, which remains required before positive capture. Its result domain is deliberately only `ineligible | unproven`; either result snapshots immutable profile bytes through typed-array intrinsics, rederives the classification, binds exact input-attested and returned terminal states, and leaves the attempt count at zero. This module cannot emit `pass-host-preflight` or authorize capture; future positive replay must authenticate evidence kind before the generic state transition. Raw source hashes remain recorded, stability uses normalized controls, and CPU lists are parsed independently so malformed evidence cannot hide relations among the valid lists or other disqualifiers. A live diagnostic on this development host returned `ineligible`: allowed CPUs `0-31` are not isolated, governors are `powersave`, turbo control is unavailable, and swap is `33,519,612 KiB`. Because no canonical tracked profile or capture task exists, this is diagnostic non-admission evidence only, not an authoritative programme run or measurement.

The capture control plane now has a local single-use run claim and its first pre-admission consumer. The claim slot is keyed only by independently supplied project authority and run ID; its immutable body binds controller, task, input attestation, runner profile and expected runner identity. The consumer reopens that rooted claim, re-attests an exact primary or bare controller store, rejects include/filter/config and attribute authority plus foreign-owned or cross-UID-writable Git control/object nodes, bounds the object-authority walk, preflights path-counted blob sizes before checkout, materializes only the claimed commit through a private Git index, seals the source tree, and returns a digest-bound opaque local view. Full path/index/tree inventories reject unsupported modes, symlinks, gitlinks, hard links, `.git`, extras, replacements, output injection and ambient branch/worktree changes; pristine-only cleanup preserves poisoned trees. Host admission remains unevaluated and all lease, attempt, build, execution and capture authority stays false. The owner-only claim and source roots remain same-UID cooperative controls: they prove neither external append-only durability, rollback resistance nor path-ABA resistance. There is no lease, launch, TTL, reclaim or retry API, and the source view is not persistence proof or a receipt. Tests use temporary synthetic primary/bare stores and roots; no real project claim, source tree, profile, build, receipt or measurement was created on this host.

Commits `99fa2e1` and `92f5376` freeze the non-authorizing signed registration seam; `7139b05`, `1d33638`, `3ee0ed6`, and `d54518f` add RFC 9162 proof verification, shared Ed25519 verification, signed checkpoint parsing, and registration inclusion/consistency replay. Commit `f1a3a48` adds a sealed, private, nonoperational decision kernel; `f604d0f` adds its dormant transaction coordinator, and `3e0ceab` adds ADR-0043's bounded whole-transaction retry prerequisite. The coordinator keeps write and exact-recovery roots disjoint, makes checkout acquisition allocation-free, captures checkout-local cleanup before the sole `open()`, freezes adapter results, binds staged event and joined-row provenance to the candidate and transaction snapshot, quarantines ambiguous terminal outcomes, and releases response bytes only after literal commit. Valid unrelated global interleaving commits while forged original-registration provenance rolls back; exact recovery cannot read head, run or staging authority. A known internal abort can trigger at most three fresh attempts after successful rollback/destruction; roots are snapshotted, the peer is consumed once, all decisions and staging are recomputed, and ambiguous commit or cleanup failure is never retried. The protected private materializer exact-key checks roots before traversal, snapshots bounded trap-free graphs, exposes copied signing bytes on a one-use prepared identity, consumes before signature parsing, verifies the pinned Ed25519 SPKI/signature, and emits complete DB-shaped 201/409 rows for genesis/non-genesis and adjacent/interleaved histories. Commits `28addbc`/`c586973` add the exact catalogue/parser/deparse oracle; ADR-0047 adds the independently replayed 4,059-record PostgreSQL 16.15 PUBLIC candidate and mutation evidence. Additive V3 preserves the historical V1/V2 receipts/runners byte-exact and invokes only its four evidence profiles, twice each in fresh networkless anonymous-volume containers after PID-1 `postgres` plus `pg_isready` readiness. Hosted run [`33636424967`](https://github.com/sparkling/semantic-fabric/actions/runs/33636424967) at exact `d0cc5fb938a1ff8b70859c19882934461fe23c5a` passes exact Node 20.0.0/24.14.1 V3 lanes and the complete required workflow; no hosted run receipt is tracked. Commits `1e2d88d`/`7d5af51` freeze lifecycle and command metadata; `0d5d09e`/`e37cce7` add evidence-only INSERT value/result contracts and structural DDL coupling. Fresh local exact Node 20.0.0 and 24.14.1 clean installs each pass all 715 oracle-package tests. Artifact `422c6854…3219` keeps the public bundle at 49,106 bytes/`90e21e7c…f7c3a`, runtime dependencies empty and all authority flags false; the direct Linux-native Rollup pin is development-only. This Node lane is an executable development oracle only. Supervisor PostgreSQL catalogue-observation SELECT and wire-transport/tag vectors plus the separate Rust supervisor's authenticated transport, signer, runner and live admission remain Rust work; its narrower transaction state store is now implemented. ADR-0047's remaining mutations stay open post-1.0. Both npm audits are clean. Generic MCP/threat-model tools report clean/info but cannot inspect the actual tracked launcher surfaces and remain `INCONCLUSIVE`; the deep generic scanner reports only reviewed fixture/static-string heuristics. The project-owned gate requires exact-empty `.mcp.json`, an exact `.agents/config.toml` digest, and a pinned networkless Ruflo reader over private copies of two status files. A fixed-seed 2×1 Darwin Shield diagnostic passed only 9/12 gates and grants no promotion authority. An ADR-plugin `--help` probe on 2026-09-01 unexpectedly entered the live project importer and may have partially upserted ADR rows before exit; its exact scope is untrusted. No database copy, reconciliation, rollback, or later memory-backed ADR verification was attempted; Git remains canonical. The ignored, untracked local two-task MetaHarness diagnostic suite remains ineligible for Darwin/GEPA evolution, and the retrieval-policy flywheel remains off. ADR-0042 through ADR-0047 remain proposed. Their implemented Rust kernel and Node oracle retain their exact non-authorizing claims; authenticated capture, witnesses, controlled performance, external administration, two-builder agreement and real capture remain open post-1.0. None enters the Rust product runtime or substitutes for ADR-0055's minimum release evidence.

On 2026-08-28, exact commit `ad94cdb` was cloned twice without local hard links under the hardened-builder `umask 0022`. Each checkout rebuilt the controller, passed all 91 harness files (627 tests passed and 8 environment-intentional skips), replayed the RDB2RDF, query, Protocol, dependency-closure, performance-scenario, and capability authorities, remained Git-clean, and produced byte-identical authority and controller digests. The harness correctly rejected an earlier pair created under `umask 0002` because tracked inputs were group-writable; no trust check was relaxed. This closes current-tranche checkout repeatability, not binary reproducibility or final agreement.

M0A's standalone serving dependency boundary is implemented; admission and ADR-0055's SBOM, signature, provenance and clean-build smoke remain open. Complete binary/runtime closure, controlled performance and two-builder byte identity remain post-1.0.

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

M1–M5 product work proceeds through the two-writer limit once M0A supplies typed
seams. M6 follows the operator and source-identity dependencies. M7 waits for
all ADR-0055 product and minimum-evidence gates, not the post-1.0 branch.

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
- retain focused boundedness evidence while keeping diagnostic records distinct
  from artifact and release authority; and
- write subordinate ADRs for the production artifact and the federated global-
  operator/spill choice.

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
- lower global ORDER, GROUP/aggregate, DISTINCT and graph/CONSTRUCT dedup into one
  composite relational plan where sound;
- define the bounded external operator contract for shapes that cannot be pushed
  to one source, with no accidental in-memory fallback;
- close the PostgreSQL delimited-identifier deviation or retain it as an explicit
  release-profile exclusion; and
- keep the characterized normalizer split below 500 lines and continue mechanical, characterization-preserving decomposition of the remaining compiler/executor hotspots.

QA gate:

- chains and cycles at 1, 255, 256, 257 and >1,000 hops are exact or explicitly
  rejected, never silently partial;
- every advertised blocking shape respects its configured retained-state caps
  and has targeted source-growth evidence; formal comparative RSS qualification
  remains post-1.0;
- flat/tree/unoptimized/optimized/materialized-oracle results agree;
- unsupported-shape tests assert the exact pre-execution failure class.

### M2 — Total request governance and cancellation ([ADR-0052](../adr/ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md))

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
- admit post-parse algebra before bounded canonical rendering, then reserve every mapping/product/normalization/lowering/cascade/plan-build operation and unavoidable recursive copy through one shared `CompileContext` before work;
- physically isolate governed cache capacity, propagate `Arc<Plan>` without deep hit/insert copies, and prospectively contain eviction and recursive destruction. Source/configuration/scope may locate an initial candidate, but exact validated AST/wire decides equality; stable cross-parse hits require a versioned scope-aware alpha canonicalizer;
- add cooperative cancellation/work bounds to the current cap-four compiler admission without activating `GovernedV1` until parser, owned-phase, cache and calibration gates all pass;
- retain per-physical-connection cap-one admission and active-VM cancellation for owned SQLite; then cover raw mutex/submitted-work/busy/UDF/VFS/I/O gaps and retain the implemented native PostgreSQL/MySQL guards and the pinned mixed-source timeout/disconnect/SIGTERM matrix; finish wider backend/profile qualification without claiming fairness or bounded waiting;
- propagate disconnect/cancellation to all tasks, streams and connections; and
- retain the implemented generated RFC 9457/trace correlation identity, closed payload-free vocabulary and seeded-secret rejection; add adapter-internal spans without exposing driver/schema/credential strings.

QA gate:

- for active application work, one deadline covers Tower `Service::call` after request-target parsing but before Axum route/method dispatch, then admission, parse, compile, acquire, execute and serialize; fixed health and query-less discovery metadata remain available without entering that work path;
- timeout/disconnect releases worker and connection capacity within the declared
  bound for every advertised path; focused parser cancellation/reap is required
  defect-repair evidence, not the deferred full containment-attestation programme;
- exact `0`, `N` and `N+1` parser/algebra/build/work/cache tests plus focused
  release-profile differential and malformed-input proofs pass; long corpus
  fuzzing and cross-process alpha-equivalence expansion remain post-1.0;
- aggregate overload sheds immediately without Router/body polling or an internal queue; provider pools retain their separately configured bounds;
- exact result and byte caps work for every result format; total recursive/source work and atomic no-prefix failure remain required before admission.

### M3 — Secure, observable, operable runtime

Outcomes:

- retain the implemented bounded typed startup layers and environment-only
  secret injection; direct external secret-store transport remains separate work;
- retain implemented verified remote-source TLS and credential-free argv; retain required live PostgreSQL/MySQL TLS/UNION checks and rerun on the exact release artifact;
- retain ADR-0011's implemented request/compiler spans, Serve-only JSON logs and
  governance events and integrated three-family default-off Prometheus profile
  without broadening its existing-listener claim; add missing adapter spans and
  pinned OpenTelemetry export;
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
- retain the now-wired public query-admission context and isolated cache; retain the implemented PostgreSQL transactional source-RLS and portable equality-row profiles, and add general ABAC/sensitivity, end-user identity and atomic policy-aware snapshot reload;
- extend emitted query-admission allow/deny traces to row/sensitivity decisions and add a paired bounded access-decision metric; and
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
2. Native agents use normal edit/test loops; parallelize read-only investigation,
   review or compatible tests, not shared writes. Closed experiments are optional.
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
ADR-0055 defines task-based model allocation, unchanged Astra max/ultra and
native-default effort when omitted; the main model is never downgraded.

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
| Documentation drifts again | Misleading claims and planning | Generate capability/status tables from receipts; update ADR with each slice |

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
