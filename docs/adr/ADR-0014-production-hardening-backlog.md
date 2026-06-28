---
status: proposed
date: 2026-06-26
tags: [production-hardening, backlog, reliability, operability, security-edge, horizontal-scale, lifecycle, deferred, stub]
supersedes: []
depends-on:
  - ADR-0008
  - ADR-0010
implements:
  - ADR-0001
---

# Production-hardening backlog (acknowledged-deferred)

> **STUB / TRACKING RECORD** (2026-06-26) — these production dimensions are real but (a) do not gate the engine build and (b) are mostly deployment/operational, actionable only once the engine runs. Recorded here so they are **tracked, not forgotten**; each graduates to its own ADR when actionable.

## Context

The production-readiness audit (2026-06-26) surfaced dimensions beyond the engine architecture. Those that are *architectural / urgent* became **ADR-0010** (security + governance), **ADR-0011** (observability), **ADR-0012** (test strategy). The remainder — operational, deployment-edge, and large-scale — are recorded here as explicitly deferred.

## Deferred dimensions (each → its own ADR when actionable)

| Area | Items |
|---|---|
| **Reliability / resilience** | source-DB-down handling, retries, circuit-breakers, graceful degradation, failover (statement timeouts already in ADR-0010) |
| **Security edge** | **AuthZ / RLS / multi-tenancy / sensitivity promoted to ADR-0018.** Remaining here: TLS, secrets management, rate-limiting, audit-log transport |
| **Operability** | deployment/packaging (single binary → container / systemd / k8s), health / readiness / liveness probes, runbooks |
| **Horizontal scalability** | multi-node, read replicas, sharding, load balancing |
| **Lifecycle / change mgmt** | mapping hot-reload + versioning, ontology (T) version propagation, source-schema drift detection, migration / upgrade |
| **Data management** | result + T-mapping caching (**provenance / lineage emission promoted to ADR-0017**) |

## Decision

**Acknowledged-deferred.** Not decided now; **not forgotten.** Revisit each when the engine reaches the maturity where it becomes actionable — typically once the virtualiser runs and a concrete deployment target exists.

## More Information
* **Promoted-out (now their own ADRs):** ADR-0010 (security core), ADR-0011 (observability), ADR-0012 (testing), ADR-0017 (provenance), ADR-0018 (security edge).
