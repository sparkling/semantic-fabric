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

## Context and Problem Statement

The production-readiness audit (2026-06-26) surfaced dimensions beyond the engine architecture. Those that are *architectural / urgent* became **ADR-0010** (security + governance), **ADR-0011** (observability), **ADR-0012** (test strategy). The remainder — operational, deployment-edge, and large-scale — are recorded here as explicitly deferred.

## Considered Options

* **Acknowledged-deferred backlog (chosen)** — track the remaining operational/deployment-edge/large-scale dimensions here; do not decide now, do not forget; each graduates to its own ADR when actionable.
* **Promote every dimension to its own ADR now** — done only for the architectural/urgent dimensions (ADR-0010 security core, ADR-0011 observability, ADR-0012 testing, ADR-0017 provenance, ADR-0018 security edge); rejected for the rest because they are not actionable until the virtualiser runs and a concrete deployment target exists.
* **Drop the remaining dimensions** — rejected; they are real and must remain tracked, not forgotten.

## Decision Outcome

**Acknowledged-deferred.** Not decided now; **not forgotten.** Revisit each when the engine reaches the maturity where it becomes actionable — typically once the virtualiser runs and a concrete deployment target exists.

### Deferred dimensions (each → its own ADR when actionable)

| Area | Items |
|---|---|
| **Reliability / resilience** | source-DB-down handling, retries, circuit-breakers, graceful degradation, failover (statement timeouts already in ADR-0010) |
| **Security edge** | **AuthZ / RLS / multi-tenancy / sensitivity promoted to ADR-0018.** Remaining here: TLS, secrets management, rate-limiting, audit-log transport |
| **Operability** | deployment/packaging (single binary → container / systemd / k8s), health / readiness / liveness probes, runbooks |
| **Horizontal scalability** | multi-node, read replicas, sharding, load balancing |
| **Lifecycle / change mgmt** | mapping hot-reload + versioning, ontology (T) version propagation, source-schema drift detection, migration / upgrade |
| **Data management** | result + T-mapping caching (**provenance / lineage emission promoted to ADR-0017**) |

### Consequences

* Neutral, because the remaining operational, deployment-edge, and large-scale dimensions are tracked rather than forgotten, and none of them gate the engine build.
* Good, because each deferred dimension graduates to its own ADR when it becomes actionable — typically once the virtualiser runs and a concrete deployment target exists.

### Confirmation

No verification gates apply while these dimensions remain deferred; conformance is checked at graduation, when each dimension becomes its own ADR and is verified under the ADR-0012 test strategy.

## More Information
* **Promoted-out (now their own ADRs):** ADR-0010 (security core), ADR-0011 (observability), ADR-0012 (testing), ADR-0017 (provenance), ADR-0018 (security edge).
