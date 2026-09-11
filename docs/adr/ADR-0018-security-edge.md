---
status: accepted
date: 2026-06-27
updated: 2026-09-11
tags: [security, authorization, row-level-security, abac, multi-tenancy, sensitivity, data-sensitivity]
supersedes: []
depends-on:
  - ADR-0010
  - ADR-0011
implements:
  - ADR-0001
---

# Security edge — authorization, RLS, ABAC, sensitivity

> **Implementation status (2026-09-11): accepted, partially implemented.** OPTIONAL helper update (2026-09-11): scan, match/anti decomposition and pure-SubPlan OPTIONAL now prepay nullable/derived-alias inventories, shared-name comparisons, depth-checked borrowed term/graph/segment scans, NULL-safe condition vectors/column copies and generated-condition/deferred-binding appends. They no longer clone column inventories or shift NULL-safe disjunctions. Required exact/N-1 and every-charge stop tests retain raw semantics, graph-scoped blank-node absence, aggregate nullability and sound-501 boundaries. Two mapped ordinary/authenticated chained OPTIONAL paths, including DISTINCT SubPlan, reject at the observed nullable-alias helper before held source admission, then recover exact duplicate/UNBOUND-coalesced results, right-only bindings and compiler/source capacity on funded cold/key-only warm requests. OPTIONAL unification update (2026-09-11): all four fast/match/anti-match/pure-SubPlan callers now reuse `CompileContext::unify_terms`: separately paid exact-input measurement precedes the existing checked 512 + 64-times-input logical allowance. Funded Sat/Empty/Unsupported order and messages, early guards and raw behavior are unchanged. Exact/N-1 and every-charge cancellation/deadline tests cover all verdicts; mapped ordinary/authenticated fast, UNION decomposition and DISTINCT-SubPlan HTTP tests observe every actual call and refuse at the final unifier before held source admission, then recover exact bags/UNBOUND and compiler/source capacity on funded cold and key-only warm requests. OPTIONAL FILTER update (2026-09-11): fast and match/anti-match paths measure the expression, prospectively pay all key comparisons per variable occurrence and measure only referenced TermDefs before checked 1024 + 128*(expression+1)*(lookup-footprint+1) construction admission. Source validation now pays actual recursive source/projection visits, comparisons and logical output growth, preserving derived-plan fanout, DISTINCT order, aggregate/SQLite AVG positions, first-match/error order and the existing path-decoder refusal. FILTER output appends and anti-FILTER entry searches are paid without moving R5 conditions. Exact/N-1, arithmetic, UTF-8/layout, depth and every-charge cancellation/deadline tests preserve raw conditions/refusals; mapped ordinary/bearer fast and UNION HTTP tests observe both actual phases, reject each before held source admission, then recover exact duplicate/UNBOUND bags and capacity on cold/key-only warm requests. OPTIONAL shape/R2 update (2026-09-11): initial opts and constant-binding scans now pay visits and UTF-8 comparisons, retaining first refusal, Const-variant decisions, RefAtom decomposition and pure-SubPlan priority. R2 right-only insertions and nullable COALESCE construction pay current-map comparisons, logical entry carriers and boxes; matched WHERE/core/SubPlan vectors pay growth and relocation independently of allocator slack. Fast/matched left payloads move only after FILTER lowering; pure-SubPlan scalar copies retain exact-source admission. Hand-counted schedules, exact/N-1 and every-charge cancellation/deadline checks preserve decisions, copy boundaries and left-first values. Mapped ordinary/bearer shape and chained scan/DISTINCT-SubPlan tests observe the actual phases, reject before held source admission and recover exact duplicate/UNBOUND-coalesced bags and capacity on funded cold/key-only warm requests. Ordinary IQ FILTER update (2026-09-11): iterative group peeling, per-branch/group visits, boolean condition-tree depth, vector/box construction, exact borrowed SQL copies and visible WHERE growth now share request control. Expr construction reuses the measured-input allowance and then the actual source validator; Sql remains pass-through, earlier branches borrow and the final branch moves. EXISTS/NOT EXISTS/MINUS keep their existing delegation and independent clone-refusal boundaries; their inner work and combined recursion are not newly qualified. Exact hand-counted ownership/allocation schedules and every-charge cancellation/deadline tests preserve raw dialect decisions. Mapped ordinary/bearer plain, UNION and OPTIONAL-plus-FILTER HTTP paths refuse at actual construction, validation and append phases before held source admission, then recover exact cold/key-only warm bags and permits. The unchanged default-admission typed-literal OPTIONAL/outer-FILTER regression also passes: irrelevant binding payload is excluded, not actual lookup/copy work, and neither production limits nor the conservative multiplier is relaxed. Remaining compiler/source/backend/release guarantees stay open; no in-call raw-helper preemption, physical-heap/drop, GovernedV1 or completion claim. BIND/substitution update (2026-09-11): ordered dependency passes and pending reference vectors, resolved/existing definition copies, binding searches/edits and nullable/unification/WHERE output now pay request work. Supported BIND constants, variables and recursive CONCAT use internal depth, copy, lookup and output checks; unsupported Debug errors use measured pre-admission without changing text or retry order. Borrowed left-SubPlan scans retain actual-column semantics without allocating column inventories. Post-modifier folds now discard proven-disjoint branches; the regression covers DISTINCT, Slice, OrderBy and SQL aggregation. Exact/N-1, every-charge cancellation/deadline and default-admitted ordinary/bearer plain/UNION/OPTIONAL BIND requests prove pre-source refusal, funded cold/key-only warm exact bags and permit recovery. Construction projection/aggregate helpers and other compiler/source/backend/release controls remain separate; no whole-compiler, physical allocator/drop or completion claim.
> `6d91fa6` adds fixed-width, provider-neutral policy/subject/request-attribute
> identities with explicit construction, redacted diagnostics and no default or
> anonymous context. `a2c25ff` adds a separate plan-cache seam requiring
> both that context and an expected policy snapshot; mismatch rejects before
> parsing or cache access, and exact key equality prevents cross-policy,
> cross-subject and cross-attribute reuse even under hash collision. `e206cab`
> adds the closed, payload-free `allow|deny|mask` event vocabulary on ADR-0011's
> exact tracing target. Public Rust/CLI serving now wires an explicit bearer
> service-principal profile: deny by default, bounded credential reference,
> query admission before body/source work, context-bound execution, partitioned
> single-source caching and uncached protected UNION. Real allow/deny traces emit.
> The default bearer profile permits its principal to read **all mapped data**.
> An explicit PostgreSQL source-RLS profile now binds trusted custom settings
> inside the same transaction as each public query/UNION fragment, with live
> isolation and cleanup tests. A bounded provisioned-subject registry now selects
> each caller's identity and RLS settings atomically on one server/pool. A
> separate portable equality-row profile now injects operator-provisioned
> source/table/column predicates as bound parameters for the accepted simple
> authored-mapping shape, including both supported UNION fragments. This is a
> deliberately narrow ABAC subset. External identity issuers, ontology-backed
> resource attributes, sensitivity enforcement, policy-aware hot reload and
> paired access-decision metrics remain open; this ADR remains incomplete.

**Cache identity delta (2026-09-11):** ordinary and security-scoped raw/controlled caches use the same conservative cache-only normalization for internal aggregate and isolated constant-DESCRIBE binders (ADR-0007). Independent parses now reuse the same `Arc<Plan>` within a partition; full canonical equality and scope/profile/policy/subject/attribute isolation remain mandatory. Policy mismatch still precedes parsing/cache access, and warm hits still pay bounded key construction. Required fixed-AST exact/N-1 and no-publication tests, independent-parse secured reuse/forced-collision tests and mapped bearer COUNT HTTP refusal/recovery tests cover this change. The public warm witness observes no post-parse compilation stages; it does not assume parser-random name lengths have identical numeric cost. No new identity provider, authorization breadth or release authority is introduced.

### Implemented reference admission profile (2026-09-07)

`serve --auth-token-env SF_QUERY_BEARER` resolves a 32–1024-byte random bearer
credential before source or file I/O. Only a SHA-256 digest is retained by the
profile, compared in constant time. `Authorization: Bearer …` is the only
credential transport; duplicate/malformed/oversized headers reject with a
redacted `401` and `WWW-Authenticate: Bearer`. No mode selected means `403`;
`--allow-unauthenticated` explicitly selects unrestricted development access.
Public embeddings must select `QueryAdmission`; they also default to deny.

The policy is immutable for a server's lifetime and applies to all its mapped
sources. Rotation requires a new server. The existing `RequestBudget` retains
the context across workers, generation leases and streams; execution checks
that the compiled plan has the same identity. Source snapshots can change only
under the selected fixed admission policy, not independently rotate it. General atomic
policy/snapshot reload remains required work, not a capability of this profile.
Fixed health, service description and explicitly enabled bounded metrics remain
public control metadata. Use loopback behind a trusted TLS edge; never expose
this plaintext listener directly with real credentials. Bearer transport follows
the [RFC 6750 header pattern and transport warning](https://www.rfc-editor.org/rfc/rfc6750#section-2.1), not an OAuth issuance/introspection implementation.

Evidence: `sf-serve` public `query_security` tests, internal security-context/cache
and protected-federation tests, and `sf-cli` real-child bearer/federation and
secret-redaction tests. Astra and native-subscription Claude Sonnet performed
independent read-only code reviews; tests, not their agreement, establish behaviour.

## Context and Problem Statement

### Implemented PostgreSQL source-RLS profile (2026-09-07)

`--auth-token-env SF_QUERY_BEARER --pg-rls-context-env SF_PG_RLS_CONTEXT`
selects the public profile. The second environment variable is a bounded JSON
object of trusted custom-setting names to string values. Embeddings use
`BearerQueryAdmission::with_postgres_rls(PostgresRlsClaims::new(settings)?)`.
Names have exactly two lowercase ASCII identifier segments, at most 128 bytes,
and no reserved `pg_` namespace; there are 1–16 settings, each value is nonempty,
at most 1024 bytes and contains no NUL. The environment JSON is at most 32768
bytes; duplicate setting keys reject rather than silently taking the last value.
Framed, sorted claims join the credential digest in policy/attribute
identity, so changing a claim changes the cache partition. Diagnostics redact
names and values. Settings are never taken from unverified headers.

The implemented source profile is authored R2RML over 1–256 ordinary `public`
PostgreSQL base tables with simple ASCII names of at most 63 bytes. Explicit
`rr:datatype` is required where the existing observation cannot prove a literal
type. Raw `rr:sqlQuery`, views, inheritance/partition trees, other backends and
verified Direct Mapping generations fail closed; their existing contracts are
not weakened. Every mapped table, including reference parents, must be RLS-active
for a non-superuser/non-BYPASSRLS, non-owner reader, with no effective owner-role
membership. Unqualified execution must resolve to the exact guarded public OID.

One dirty-owned connection begins a repeatable-read, read-only transaction,
sets bounded local timeouts and `row_security=on`, locks the mapped relations,
checks the role/relation profile, and binds both arguments to
`pg_catalog.set_config($1,$2,true)`. SELECT/ASK/CONSTRUCT and both supported UNION
fragments execute on their own exact held transactions. Both UNION sources are
admitted before HTTP success. Acknowledged bounded rollback plus unique ownership
permits reuse; abandoned/failed/timed-out cleanup discards the pooled session.
These follow PostgreSQL's [transaction-local setting semantics](https://www.postgresql.org/docs/16/functions-admin.html#FUNCTIONS-ADMIN-SET)
and [RLS role/owner rules](https://www.postgresql.org/docs/16/ddl-rowsecurity.html).

The operator owns the dedicated reader pools, source policies/functions and
complete claim-to-policy contract. Fabric verifies RLS is active, not that an
operator-authored policy expresses the intended business authorization. No
`SET ROLE`, policy installation, universal tenant column, end-user issuer or
cross-database atomic policy snapshot is claimed. Credential admission may emit
`allow` before a later source-profile `deny`; neither is a per-row disclosure log.

Required-live test `pg_generation::live_tests::rls::public_row_security_is_isolated_and_cleans_pool`
uses `SF_PG_RLS_TEST_URL` for an explicitly disposable administrator endpoint.
It provisions and cleans its own database/reader; tests public forms, alternating
and concurrent identities, both UNION fragments, errors/deadlines/body drop,
normal reuse, abandoned/timeout discard, owner/BYPASSRLS/disabled-RLS rejection
and catalog-shadow name rejection. CI runs it explicitly; ordinary tests do not
silently connect to Product Mock or substitute an unavailable database.

On 2026-09-08 the same required public-router fixture also qualifies actual opt-in
lineage under RLS: A/B/A constant and overlapping-map SELECT/CONSTRUCT, two-source
UNION and bounded join, exact contributing map/source identities and clean cap-one
PID reuse after each complete response. It checks denied/empty results, spoofed
headers, concurrent constant SELECT, invalid credentials and disabled-RLS rejection.
Constant SELECT/CONSTRUCT lineage also exercises body-drop, slow-policy deadline and
policy-error cleanup followed by an isolated caller's successful query. ADR-0017
records the exact result/provenance and transport scope; this is not every failure
permutation, a new policy engine, TLS evidence or exact-artifact release admission.

### Implemented provisioned-subject registry (2026-09-07)

`--auth-subjects-env SF_QUERY_SUBJECTS` selects one immutable registry instead of
`--auth-token-env`, `--pg-rls-context-env` or `--allow-unauthenticated`. Its JSON
has `schemaVersion: 1` and 1–256 `subjects`, each with `subjectRef`,
`credentialEnv` and `postgresRlsContextEnv`. Schema version 2 instead permits
each subject to select exactly one of `postgresRlsContextEnv` or `portableRows`.
The whole document is at most 128 KiB;
environment references are 1–128 ASCII identifier bytes. Subject references are
opaque operator identifiers of 1–128 ASCII alphanumeric/`_.:-` bytes, not a
new tenant or sensitivity taxonomy. Unknown/duplicate fields, duplicate subjects,
shared credentials, bad references and malformed claims fail at startup, before
source/file I/O. Only digests and validated claims survive construction;
diagnostics expose none of the raw values. Rust embeddings construct
`ProvisionedBearerSubject::postgres_rls` and `ProvisionedBearerAdmission::new`.

Subjects are canonically ordered by their stable reference digest. One
domain-separated policy digest binds the full registry's count, subject digests,
credential digests and canonical RLS-attribute digests. Every caller shares that
policy identity, with separate subject/attribute identities. Credential rotation
preserves the stable subject identity but changes the registry policy; all old
contexts/cache partitions invalidate. Claim changes also change the policy and
attributes. The server scans every registry entry with constant-time credential
digest comparisons, then retains the selected context **and** settings in one
one-shot request-budget operation before cloning. No caller header supplies or
overwrites a subject or claim. Compilation and execution require that exact
registered context/settings pair. Protected UNION remains uncached and both
source transactions receive the same selected settings.

The required-live RLS test additionally exercises alternating and concurrent
credentials on the **same server**, clean reuse on a one-member pool,
SELECT/ASK/CONSTRUCT, spoofed identity headers and both UNION sources. Unit tests
prove canonical order, duplicate rejection, rotation, atomic handoff, mixed-bundle
rejection and cache/execution isolation between actual registered subjects. CLI
child tests prove mutual exclusions, startup ordering and redaction. This delivers
explicitly provisioned per-caller source authorization, not token issuance,
OIDC/introspection, portable ABAC, sensitivity enforcement or policy hot reload.

### Implemented portable equality-row profile (2026-09-07)

Schema-version-2 registry subjects may provide 1–1024 `portableRows`, each with
snapshot-local `sourceIndex`, exact mapped `table` and `column`, and a
`valueEnv` reference. Values are resolved once at startup, bounded to 16 KiB,
retained only in redacted policy objects, and emitted only as ordinary SQL bound
parameters. Table and column names are bounded trusted configuration matched
exactly to mapping-derived plan identifiers; duplicate source/table/column rules
reject. The complete canonical rule set contributes to the policy and request-
attribute identity, so subjects with different values cannot share a protected
cache or execution context.

The profile authorizes direct base-table scans and the compiler's validated
single-table projection view. Every base table reached by a plan must have a
matching rule for that source. The AST projection admits only one aliased table,
column-to-same-name projections, and no join, filter, grouping, ordering, limit,
CTE, table-function or other authored SQL feature. Authored simple views retain
policy-column exposure before the outer bound predicate. For a typed compiler D1
projection, bound native-equality guards now filter the original table before
deduplication; policy-only columns never widen its RDF key. Guarded projections
lose table-restore authority, and computed/nested shapes gain no policy authority.
Recursive property-path sources and every unproved source-query shape return a
redacted `403` before pool acquisition. Verified Direct Mapping generation is
also excluded from this authored-mapping profile.

Required Rust evidence covers SQLite public SELECT/ASK/CONSTRUCT isolation for
two callers, two-source SQLite UNION isolation, fail-before-source-I/O denial,
and SQLite/PostgreSQL/MySQL dialect emission with bound values. Required owned
PostgreSQL16.15/MySQL8.4.11 CLI checks now cover ordinary text/CHAR policy-filtered
set identity, denied-row controls and projection bags. Newly enabled dedup needs
live decoder proof for every key; mixed/natural identity remains open. These
specific native checks do not qualify every portable-policy shape or general Boolean ABAC,
ontology/sensitivity attributes, masking, policy installation, external identity
issuance or policy-aware reload. The broader three-layer ADR remains incomplete.

### External policy authority and remaining integration

`semantic-modelling` owns Category 11 under ODR-0071k: `hm:dataSensitivity`
uses Public/Internal/Confidential/Restricted; DPV concepts are reference values,
not an independently invented Fabric taxonomy. Semantic Builder's gold is
classification evidence, not executable authorization authority. Product Mock
has no canonical PostgreSQL RLS/GUC convention or universal tenant column.
`app.tenant_id` and `app.identity` below remain examples, not defaults. Before
Product Mock policy integration, its owner must specify the table/column
boundaries, whether `businessScope` is tenancy or another attribute, exact GUC
encodings and clearance/purpose decisions. This does not block the explicit
operator-configured source-RLS transport implemented here.

ADR-0010 establishes injection-safety + resource governance and "the mapping is the allow-list." That allow-list is **schema-reachability control, not an authorization model**: anyone who can query reads all mapped rows; there is no per-identity / tenant / value differentiation; and because the fabric connects with a single service account, **source RLS is inert unless identity is propagated**. AuthN/TLS belong at the edge (ADR-0010 R5), but **row-level access, tenancy, and sensitivity cannot be delegated to a reverse proxy** — it never sees the generated SQL or the source rows (the same argument ADR-0010 makes for why injection/DoS must be in-engine). These need an in-engine or source-delegated layer.

## Considered Options

* Reverse-proxy / edge-only authorization — rejected for row-level access: the proxy never sees the generated SQL or the source rows, so row-level access, tenancy, and sensitivity cannot be delegated to it (AuthN/TLS still stay at the edge per ADR-0010 R5).
* Mapping-as-allow-list alone (the ADR-0010 floor) — insufficient as an authorization model: it is schema-reachability control with no per-identity / tenant / value differentiation, and source RLS is inert under a single service account.
* Source-RLS via identity propagation (Layer 1) — adopted: `SET LOCAL` of a GUC / `SET ROLE` so PostgreSQL filters rows server-side; requires source-side RLS policies/roles.
* Rewriter-enforced ABAC (Layer 2) — adopted: portable across every backend by compiling policy predicates into the AST as bound parameters; covers non-RLS backends (DuckDB/SQLite).
* Data-sensitivity propagation (Layer 3) — adopted: consume the platform data-sensitivity taxonomy to deny/mask labeled columns per clearance.
* Reasoning-aware enforcement — adopted: apply masking/denial after T-saturation (ADR-0008) so masked facts are not re-derived.

## Decision Outcome

Chosen: three layers + reasoning-aware enforcement.

1. **Source-RLS via identity propagation.** Before running the generated query, in the same transaction, set a source session context — `SET LOCAL` of a GUC (`app.tenant_id` / `app.identity`) or `SET ROLE` — so PostgreSQL RLS filters rows **server-side**, independent of the rewriter's correctness. **`SET LOCAL`, never `SET`** (a leaked `SET` bleeds tenant context across pooled connections — the ADR-0010 stream-lane pool discipline). Requires source-side RLS policies/roles; DuckDB/SQLite have no RLS and fall back to Layer 2.
2. **Rewriter-enforced ABAC (portable, every backend).** The engine is the policy-enforcement point: compile tenant/policy predicates into the `sqlparser` AST as **bound parameters** — the same injection-safe value-binding ADR-0010 R1 already mandates, inheriting its safety proof. Policy = **ABAC** (subject attributes from the authenticated identity × resource attributes from the mapping/ontology/sensitivity tags). This is the published policy-protected-VKG / Stardog / GraphDB rewrite approach.
3. **Data-sensitivity propagation.** The mapping IR carries a sensitivity label per column/predicate, **sourced from the platform's data-sensitivity taxonomy** (consume it; don't invent a parallel scheme). The rewriter denies or masks labeled columns per the caller's clearance at query time (composes with the ADR-0017 query-time provenance tags).
4. **Reasoning-aware enforcement.** Apply masking/denial **after** T-saturation (ADR-0008), or exclude sensitive facts from the saturated rewrite — otherwise a masked fact is re-derived from permitted ones.

AuthN stays at the endpoint; access decisions (allow/deny/mask) emit **audit events** to the observability layer (ADR-0011) alongside the existing governance events.

### Consequences

* Good, because a real multi-tenant authZ story layered on ADR-0010's floor; the portable rewriter layer reuses the existing injection-safe AST machinery and works for non-RLS backends; sensitivity consumes the platform taxonomy.
* Bad, because full per-triple sensitivity is hard to secure without collateral (documented masking-leak caveats: zero-length paths, full-text, edges); multi-tenancy + RLS need source-side policy/role setup.

### Confirmation

Access decisions (allow/deny/mask) emit audit events to the observability layer (ADR-0011) alongside the existing governance events, providing the audit trail for enforcement. The injection-safety of the rewriter-enforced ABAC layer inherits the ADR-0010 R1 bound-parameter safety proof; further verified via the ADR-0010 conformance/governance gates.

## More Information
* **Injection-safety / governance floor:** ADR-0010. **Provenance / restricted graphs:** ADR-0017. **Reasoning interaction:** ADR-0008. **Audit:** ADR-0011. **Deployment-edge backlog:** ADR-0014.
* **Cross-project:** the platform's access-control / data-sensitivity taxonomy. **Research:** `docs/research/provenance-security`.
