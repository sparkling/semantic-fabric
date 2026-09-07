---
status: accepted
date: 2026-06-27
updated: 2026-09-07
tags: [security, authorization, row-level-security, abac, multi-tenancy, sensitivity, data-sensitivity]
supersedes: []
depends-on:
  - ADR-0010
  - ADR-0011
implements:
  - ADR-0001
---

# Security edge — authorization, RLS, ABAC, sensitivity

> **Implementation status (2026-09-07): accepted, partially implemented.**
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
> This permits its authenticated principal to read **all mapped data**, not
> individual end-user/tenant rows. PostgreSQL `SET LOCAL` RLS, portable ABAC,
> sensitivity enforcement, policy-aware hot reload and paired access-decision
> metrics remain open; this ADR remains incomplete.

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
under that fixed read-all policy, not independently rotate it. General atomic
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
