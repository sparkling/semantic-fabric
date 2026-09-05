---
status: accepted
date: 2026-06-26
updated: 2026-09-05
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

> **Implementation status (2026-09-05): partially implemented.** Commits
> `3e0f920`/`c9e6c53` add the closed pre-commit RFC 9457 problem vocabulary,
> opaque/redacted startup errors, bounded response-only correlation IDs,
> `no-store` and `nosniff`, plus hostile SQL/schema/credential leak tests;
> `6cd85eb` routes absolute-deadline expiry through it;
> `58cca82` adds the redacted `429 query-budget-exceeded` boundary, and `bac8240`
> rejects an unrepresentable timeout as redacted startup configuration before
> source, file, runtime, or network I/O. Commit `f1f4747` rechecks the deadline at
> response handoff and rejects zero-capacity ASK before backend acquisition.
> Commit `d391927` closes the current Axum-router path, method and body-extraction
> classes as RFC 9457: unknown paths, unsupported methods (with the derived
> `Allow` header), configured body-size rejection and body-buffer failure. At an
> expired representable handoff the public result is always `504`, while internal
> accounting retains any earlier sticky resource cause. Commit `1e85249` moves
> that deadline to Tower `Service::call`, after HTTP/request-target parsing but
> before Axum route/method dispatch. Commit `36488d5` adds strict media-specific
> admission: exactly one GET/form query or one raw query, per-request raw `n` and
> checked form `3n+16` wire limits with decoded `n`, rejection of optional
> version/dataset/extra parameters, and unsupported-media rejection without
> polling the application body. These redacted failures do not claim full
> Protocol conformance. Malformed HTTP request targets rejected before the outer
> Tower service remain outside this application boundary.
> Commit `484a4b4` adds a bounded redacted
> `SourceRef`: the CLI accepts exactly one
> credential-free inline source or environment reference, and typed PostgreSQL/
> MySQL parsing rejects inline passwords before runtime, file, or network I/O. Raw
> generated SQL, driver text, mapping details and source specifications cannot
> enter that public boundary. Current hardening also maps SELECT/CONSTRUCT
> executor and query-budget failures after committing `200` to the single stable
> body error
> `result stream failed`, so driver, mapping, schema and SQL text cannot escape
> through that channel. The failure still terminates the stream; it does not turn
> the response into RFC 9457 or prove an atomic no-prefix contract. Environment
> injection is not the layered
> TOML/config/secret-store model and remote PostgreSQL still uses `NoTls`.
> Correlation IDs currently reach the response
> only, not a log sink. The production crates still contain no tracing/metrics/OTLP
> stack or layered validated configuration model, and expose no metrics. The
> 2026-09-05 Rust runtime-snapshot foundation does expose a
> closed redacted readiness state: new requests acquire one immutable snapshot
> before request-body polling, not-ready state returns `503` with `Retry-After`,
> and in-flight response bodies retain their original snapshot through
> termination. Activation remains a crate-private, non-authorizing primitive;
> automatic drift observation, validated candidate construction, public reload,
> and telemetry remain M3/M5 work under ADR-0038 and ADR-0050. Commits `5694489`
> and `3abbdb4` add fixed `/livez` and `/readyz` endpoints plus bounded
> three-phase SIGTERM/Ctrl-C shutdown. `/livez` reports event-loop liveness only;
> `/readyz` reads the
> already-established runtime-snapshot state and never polls a source, request
> body, or application-work permit. Shutdown moves `Running` to `Draining`, marks
> the runtime administratively not ready, rejects newly minted budgets and stops
> ingress while existing admitted request identities may finish normally. At the
> validated positive bound (30 seconds by default), `Forced` broadcasts
> cooperative cancellation to remaining identities before the serving future is
> dropped. Unit and listener tests prove an in-flight `200` during drain, exact
> forced expiry, new-versus-existing budget behaviour, listener closure and
> capacity release. Follow-up commit `1e2de3e` sends real SIGTERM to the
> shipped CLI child, observes clean exit inside three seconds, and verifies the
> listener is closed. These slices do not implement telemetry, layered
> configuration, verified TLS, source polling/failure
> policy, SLOs, or the complete ADR-0011 control plane.

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

### Health, readiness, and bounded shutdown

`GET /livez` is a fixed process/event-loop liveness response. `GET /readyz` is a
fixed projection of the immutable runtime readiness state: ready is `200`,
not-ready is `503` plus `Retry-After: 1`, and poisoned state is a redacted `500`.
Both are `application/json`, `no-store`, and `nosniff`; neither reads request
content, acquires application capacity, or probes a database. SIGTERM and Ctrl-C
move `Running` to `Draining`, mark readiness administrative not-ready, reject new
budgets, and stop new ingress while already-admitted requests retain their
identities and may complete. At the configured positive monotonic-clock bound,
`Forced` broadcasts cancellation to remaining identities before the server and
connections are dropped. This is the implemented M3 probe/shutdown slice, not
source-health polling, reload, telemetry, SLOs, or a cross-backend cleanup
qualification.

### Redaction discipline
Credentials, result data, PII and bound-parameter values are never logged at any
level. `DEBUG` may record only a parameterized SQL template/AST with placeholders
and bounded structural metadata. SQL text from any adapter that interpolates
values is never loggable.

### Consequences
* Good, because observable + OTel-ready from day one; governance is visible (trace + metric); per-stage latency attributable.
* Bad, because instrumentation has a small runtime cost (keep hot-path spans cheap) and metric **cardinality must be bounded** (no per-query labels like raw text).
* Neutral, because the config surface grows with features (governance, store, modes).

### Confirmation
* A query produces a span tree + the metric set; governance actions appear as **both** a trace event and a counter.
* An invalid config **fails fast** at startup.
* **No secret, PII, result value or bound parameter appears in any log at any
  level** (redaction test + lint); only parameterized SQL templates may appear at
  `DEBUG`.

## More Information
* **Governance events / secret handling:** ADR-0010. **Exec model the hooks instrument:** ADR-0006. **Intensional graphs:** ADR-0004. **Architecture:** ADR-0003.

## Rules
* **R1** — one `tracing` span tree per request; every pipeline stage is a span.
* **R2** — metrics via the `metrics` facade only; **bounded cardinality** (no unbounded labels).
* **R3** — secrets via injection only, never inline or logged; bound values and
  PII are never logged; only placeholder-bearing parameterized SQL templates may
  appear at `DEBUG`.
* **R4** — every ADR-0010 governance action emits both a trace event and a metric.
