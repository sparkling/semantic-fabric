# Provenance/lineage + OBDA security edge

**Date:** 2026-06-27 · **Decisions:** ADR-0017 (provenance), ADR-0018 (security). **Evidence:** [A]/[B]/[C].

## Framing
The **materialize/virtualize split is load-bearing.** Materialize persists triples → provenance is data, access control applies to stored quads. Virtualize stores nothing → provenance is query-time, access control is in the rewrite or delegated to the source.

## Part A — Provenance / lineage
- **Named graphs** carry "which mapping/source produced this" as a **group** property → cost **O(mappings × sources × runs), not O(triples)** — essentially free per triple, native to Oxigraph; serves source mapping (lineage/coverage/impact). [A]
- **PROV-O sidecar** describes each materialization run (`prov:Activity` used/generated + timestamps + counts) → serves a run-tracking data concern; RMLMapper ships PROV-O as prior art. [A]
- **RDF-star** only for true triple-level metadata (RDF 1.2 still WD; claim-object for multi-source; immature with SHACL). **Avoid** reification / singleton-property / singleton-named-graph (all dominated). [A]
- **Virtualize:** query-time where/how-provenance via the same rewrite engine (the SQL already knows the triples-map + source columns); Calvanese IJCAI 2019; never store. [A]

## Part B — Security edge
- **"Mapping = allow-list" (ADR-0010 R2) is the schema-reachability *floor*, not authorization** — single service account makes PostgreSQL RLS inert unless identity is propagated; no row/tenant/value control. [B/C]
- **Layer 1 — source RLS via identity propagation:** `SET LOCAL` GUC / `SET ROLE` in the same txn → server-side row filtering (**`SET LOCAL`, never `SET`** — the PgBouncer transaction-pooling hazard); DuckDB/SQLite have no RLS → fall to Layer 2. [A]
- **Layer 2 — rewriter-enforced ABAC** as bound AST predicates (reuses ADR-0010 R1 injection-safety); portable to every backend; the published policy-protected-VKG / Stardog / GraphDB rewrite approach. [A/C]
- **Layer 3 — data-sensitivity label propagation + masking** (Stardog model + leak caveats: zero-length paths, full-text, edges). [B/C]
- **Reasoning-aware:** mask/deny **after** the OWL-RL closure (ADR-0008), else a masked fact is re-derived. [A/C]

## Decisions
ADR-0017: named-graph-per-(mapping×source×run) + PROV-O sidecar (materialize); query-time provenance (virtualize); RDF-star opt-in. ADR-0018: three layers (source RLS / rewriter ABAC / data-sensitivity masking), reasoning-aware, audit to ADR-0011.

## Sources
- PROV-O: https://www.w3.org/TR/prov-o/ · Named Graphs/Provenance (WWW 2005): https://dl.acm.org/doi/10.1145/1060745.1060835 · metadata-representation benchmark (reification/singleton/RDF-star): https://fabriziorlandi.net/pdf/2021/ICSC2021_REF-Benchmark.pdf · RMLMapper PROV-O: https://github.com/RMLio/rmlmapper-java
- OBDA provenance (Calvanese IJCAI 2019): https://www.ijcai.org/proceedings/2019/0224.pdf · provenance polynomials via rewriting (2025): https://arxiv.org/abs/2508.14608
- Policy-protected VKG (Springer 2025): https://link.springer.com/chapter/10.1007/978-981-95-5009-8_14 · Stardog FGS: https://docs.stardog.com/operating-stardog/security/fine-grained-security · GraphDB FGAC: https://graphdb.ontotext.com/documentation/11.3/fine-grained-access-control.html
- PostgreSQL RLS: https://www.postgresql.org/docs/current/ddl-rowsecurity.html · multi-tenant RLS (Crunchy): https://www.crunchydata.com/blog/row-level-security-for-tenants-in-postgres · Starburst ABAC: https://www.starburst.io/blog/abac-fine-grained-governance-cloud-data/
