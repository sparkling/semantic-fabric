---
status: accepted
date: 2026-06-27
tags: [security, authorization, row-level-security, abac, multi-tenancy, sensitivity, data-sensitivity]
supersedes: []
depends-on:
  - ADR-0010
implements:
  - ADR-0001
---

# Security edge — authorization, RLS, ABAC, sensitivity

## Context and Problem Statement

ADR-0010 establishes injection-safety + resource governance and "the mapping is the allow-list." That allow-list is **schema-reachability control, not an authorization model**: anyone who can query reads all mapped rows; there is no per-identity / tenant / value differentiation; and because the fabric connects with a single service account, **source RLS is inert unless identity is propagated**. AuthN/TLS belong at the edge (ADR-0010 R5), but **row-level access, tenancy, and sensitivity cannot be delegated to a reverse proxy** — it never sees the generated SQL or the source rows (the same argument ADR-0010 makes for why injection/DoS must be in-engine). These need an in-engine or source-delegated layer.

## Decision Outcome — three layers + reasoning-aware enforcement

1. **Source-RLS via identity propagation.** Before running the generated query, in the same transaction, set a source session context — `SET LOCAL` of a GUC (`app.tenant_id` / `app.identity`) or `SET ROLE` — so PostgreSQL RLS filters rows **server-side**, independent of the rewriter's correctness. **`SET LOCAL`, never `SET`** (a leaked `SET` bleeds tenant context across pooled connections — the ADR-0010 stream-lane pool discipline). Requires source-side RLS policies/roles; DuckDB/SQLite have no RLS and fall back to Layer 2.
2. **Rewriter-enforced ABAC (portable, every backend).** The engine is the policy-enforcement point: compile tenant/policy predicates into the `sqlparser` AST as **bound parameters** — the same injection-safe value-binding ADR-0010 R1 already mandates, inheriting its safety proof. Policy = **ABAC** (subject attributes from the authenticated identity × resource attributes from the mapping/ontology/sensitivity tags). This is the published policy-protected-VKG / Stardog / GraphDB rewrite approach.
3. **Data-sensitivity propagation.** The mapping IR carries a sensitivity label per column/predicate, **sourced from the platform's data-sensitivity taxonomy** (consume it; don't invent a parallel scheme). The rewriter denies or masks labeled columns per the caller's clearance at query time (composes with the ADR-0017 query-time provenance tags).
4. **Reasoning-aware enforcement.** Apply masking/denial **after** T-saturation (ADR-0008), or exclude sensitive facts from the saturated rewrite — otherwise a masked fact is re-derived from permitted ones.

AuthN stays at the endpoint; access decisions (allow/deny/mask) emit **audit events** to the observability layer (ADR-0011) alongside the existing governance events.

## Consequences
* Good — a real multi-tenant authZ story layered on ADR-0010's floor; the portable rewriter layer reuses the existing injection-safe AST machinery and works for non-RLS backends; sensitivity consumes the platform taxonomy.
* Bad — full per-triple sensitivity is hard to secure without collateral (documented masking-leak caveats: zero-length paths, full-text, edges); multi-tenancy + RLS need source-side policy/role setup.

## More Information
* **Injection-safety / governance floor:** ADR-0010. **Provenance / restricted graphs:** ADR-0017. **Reasoning interaction:** ADR-0008. **Audit:** ADR-0011. **Deployment-edge backlog:** ADR-0014.
* **Cross-project:** the platform's access-control / data-sensitivity taxonomy. **Research:** `docs/research/provenance-security`.
