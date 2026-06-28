---
status: accepted
date: 2026-06-26
tags: [observability, logging, metrics, tracing, configuration, opentelemetry, production]
supersedes: []
depends-on:
  - ADR-0003
  - ADR-0006
  - ADR-0010
implements:
  - ADR-0001
---

# Observability & configuration

## Context and Problem Statement

A production fabric needs structured **logging**, **metrics**, **tracing**, and a **configuration model** — none of which the design carried (the observability gap from the production-readiness audit). The query pipeline is multi-stage (`SPARQL → IQ → SQL → rows`), so flat logs are insufficient; and ADR-0010's governance actions (limit-hit, timeout, rejection) need a sink. These hooks are cheap to design in and painful to retrofit, so they are fixed now, before the first engine increment.

## Decision Drivers

* The query path is a *pipeline* → needs **span** tracing, not just log lines, to attribute latency per stage.
* ADR-0010 governance events must be both **traceable** and **alertable** (metric).
* Secrets (DB credentials) and PII (result data, bound-param values in SQL) are present → redaction is a first-class concern, not an afterthought.
* Retrofitting instrumentation across a built engine is expensive; wire it from increment 1.

## Considered Options

* **A (chosen)** — `tracing` (logs + spans) + `metrics` (Prometheus/OTel) + a layered config model, designed in from the first increment.
* **B** — `log`-crate lines now, metrics later. Rejected: no span attribution for the pipeline; costly retrofit.
* **C** — full OpenTelemetry-everything from day one. Rejected as heavier than needed — but `tracing`/`metrics` are OTel-compatible, so A is a clean subset/upgrade path.

## Decision Outcome

### Logging + tracing — one tool: `tracing`
Structured events **and** spans. The query pipeline is instrumented as a span tree — `serve_request → parse_sparql → unfold → optimize_cascade` (a child span per cascade pass, ADR-0007) `→ emit_sql → execute → serialize`. `tracing-subscriber` (env-filter; JSON in prod, pretty in dev) + `tracing-opentelemetry` for OTLP export to a collector.

### Metrics — `metrics` facade → `metrics-exporter-prometheus` (OTel-compatible)
Concrete catalogue:
* **Virtualisation:** `sf_query_duration_seconds` (histogram → p50/p95/p99), `sf_query_total{status}`, `sf_sql_emitted_total`, `sf_recursion_depth` (histogram — the `P+` governance signal, ADR-0010), `sf_result_rows` (histogram), `sf_governance_rejections_total{reason}`.
* **Streaming / memory:** `sf_peak_memory_bytes` (the bounded-memory invariant, ADR-0006), `sf_stream_rows_total`, `sf_first_result_seconds`.
* **Resource:** `sf_pool_connections{state}` (gauge), `sf_db_roundtrip_seconds`, `sf_cache_hits_total` / `sf_cache_misses_total`.

### Governance events (ADR-0010)
Limit-hit / timeout / rejection / injection-attempt emit **both** a `tracing` warn-event **and** a `metrics` counter — one trace, one alertable metric.

### Configuration model
Layered precedence: **defaults < config file (TOML) < env vars < secret injection** (via `figment`/`config` + `serde`, validated at startup, fail-fast). Sections: `[source]` (connections, dialect — ADR-0006), `[mappings]` (location/format), `[graphs]` (the in-memory T/M paths — ADR-0004), `[governance]` (the ADR-0010 limits), `[observability]` (log level, OTLP endpoint, metrics port), `[serve]` (endpoint config). **Secrets** are referenced, never inline (e.g. `password_env = "PG_PASSWORD"`).

### Redaction discipline
Credentials never logged at any level; result-data/PII never logged; **generated SQL only at `DEBUG`** (it carries bound-param values).

### Consequences
* Good — observable + OTel-ready from day one; governance is visible (trace + metric); per-stage latency attributable.
* Bad — instrumentation has a small runtime cost (keep hot-path spans cheap) and metric **cardinality must be bounded** (no per-query labels like raw text).
* Neutral — the config surface grows with features (governance, store, modes).

### Confirmation
* A query produces a span tree + the metric set; governance actions appear as **both** a trace event and a counter.
* An invalid config **fails fast** at startup.
* **No secret appears in any log at any level** (redaction test + lint); generated SQL appears only at `DEBUG`.

## Rules
* **R1** — one `tracing` span tree per request; every pipeline stage is a span.
* **R2** — metrics via the `metrics` facade only; **bounded cardinality** (no unbounded labels).
* **R3** — secrets via injection only, never inline, never logged; generated SQL at `DEBUG` only.
* **R4** — every ADR-0010 governance action emits both a trace event and a metric.

## More Information
* **Governance events / secret handling:** ADR-0010. **Exec model the hooks instrument:** ADR-0006. **Intensional graphs:** ADR-0004. **Architecture:** ADR-0003.
