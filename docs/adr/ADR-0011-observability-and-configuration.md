---
status: accepted
date: 2026-06-26
updated: 2026-09-08
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

> **Implementation status (2026-09-07): partially implemented.** Commits
> `01d0a67` and `0fcad17` implement the current trace/JSON slice; `64bae33`
> records this boundary and `fc29acc` proves the exact production filter through
> a real request. After private
> parser-worker dispatch and public argument parsing, only `serve` installs the
> JSON subscriber. Its only operator control is the closed
> `--log-level off|error|warn|info` value (default `info`); an exact product-target
> allowlist and real maximum-level hint reject every foreign target and all
> higher levels. `RequestDeadlineService::call` mints the sole opaque bounded
> correlation ID before any early return, ignores inbound correlation text, and
> carries that identity through the request root, RFC 9457 header/body,
> governance winner, response-body terminal, and producer terminal events.
> Request, body-lifetime, and actual compiler stages are spans with a closed
> payload-free vocabulary; the flat compiler reports rewrite/saturate/unfold/
> cascade operations, the tree compiler reports build/resolve/normalize/lower/
> cascade operations, and source-affine UNION parsing reports parse. Cascade
> recursion remains inside one stage span so trace volume does not scale with
> plan branches. Protocol no-body responses are not reported as dropped bodies.
>
> Commits `8e7e6e3` and `ae0606b` add the first bounded metrics slice.
> The default installs no recorder and exposes no `/metrics` route. Explicit
> `--metrics` installs one fail-closed product recorder and exposes three of the
> thirteen catalogue families on the existing listener:
> `sf_query_total{status,body}`, `sf_query_duration_seconds{status}` with
> fixed buckets, and `sf_governance_rejections_total{reason}`. Their labels and
> values are closed, foreign targets/names/labels/values become no-ops, fixed
> control/discovery traffic is excluded, and query/body and sticky governance
> terminals record exactly once. A real `sf-cli` child proves both the disabled
> default and enabled endpoint.
>
> ADR-0018's public bearer query-admission profile now emits the existing
> payload-free `security.access_decision` allow/deny trace once per attempt;
> credential material never enters its fields. Source-RLS profile rejection now
> adds an actual `deny`; an earlier credential `allow` is not a row authorization
> result. Real-request and source-denial capture tests verify these boundaries.
> Mask enforcement and paired access-decision metrics remain
> open; the three-family Prometheus contract is unchanged.
>
> The current public `serve` boundary now implements typed startup layering:
> validated defaults `<` a bounded deny-unknown TOML document `<`
> `SEMANTIC_FABRIC_*` environment values `<` explicit CLI arguments. Source,
> mapping and secondary-source alternatives replace as whole selector groups.
> Token-reference rotation retains lower row claims; a registry cannot become a
> lone unrestricted bearer. Explicit mode changes reject incompatible settings,
> and false anonymous permission never conflicts with authentication. Values
> merge before effective scalar validation and render as single option/value
> arguments. Non-regular files reject without waiting for a FIFO writer; TOML,
> environment, CLI and effective settings are each capped at 1 MiB. Static help
> bypasses configuration; argument terminators cannot bypass it during execution.
> TOML contains only environment references
> for source credentials, bearer material, RLS claims and portable row values;
> existing startup boundaries resolve those values once and keep all failures
> redacted. Required unit and real-child tests prove precedence, row-policy
> preservation, opaque errors and an authenticated exact two-source HTTP query.
> These security regressions correct the earlier whole-security-group merge;
> initial precedence tests alone did not establish this boundary. This closes
> the startup-layering slice, not hot
> reload or a direct external secret-store protocol. A subsequent Rust source-TLS
> slice verifies both certificate chains and host identities for remote PostgreSQL
> and MySQL. PostgreSQL query/control pools carry their connector into dirty-session
> cancellation; MySQL's socket fallback and verification bypasses are disabled.
> Bundled public roots or exclusive environment-injected private PEM roots are
> supported, with 64 KiB/64-certificate bounds and once-only resolution. Private
> roots force TLS even on loopback. Network startup is capped at 30 seconds;
> PostgreSQL create/recycle operations use the positive pool-wait bound too.
> Required real-TLS loopback tests prove trusted queries, certificate/name failures,
> PostgreSQL cancellation, no plaintext downgrade and stalled-handshake rejection.
> A fresh MySQL process proves explicit crypto-provider initialization. CLI tests
> prove unsafe settings and trust-reference errors reject before file/network I/O.
> Required native CLI qualification now covers digest-pinned PostgreSQL 16.15
> and MySQL 8.4.11: authenticated exact single-source queries and mixed UNION,
> server-observed encrypted sessions, separate private CAs, wrong CA/name rejection
> and a swapped second-source CA failure. Fixtures own their local Docker IDs,
> random credentials and temporary data; no external database endpoint is accepted.
> This exposed a bare PostgreSQL availability diagnostic, now routed through the
> product JSON subscriber at INFO with its unchanged closed, non-authorizing text.
> Exact-release-artifact qualification remains open; this does not add inbound
> HTTP TLS or a direct external secret-store protocol.
>
> **R1 is partial:** the root and current request/compiler boundaries are traced,
> but there is no distinct `emit_sql` span and no adapter-internal span propagation
> into blocking `sf-sql` bridges. **R4 is partial:** the exactly-once sticky
> governance winner emits both a bounded trace event and the closed counter, but
> the complete ADR-0010 action set is not yet covered. The remaining ten metric
> families, OTLP, separate control-listener/authentication policy, release-artifact TLS qualification,
> and measured instrumentation-overhead evidence remain
> pending.
>
> Earlier boundary work: commits
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
> the response into RFC 9457 or prove an atomic no-prefix contract. That earlier
> environment-only input was later incorporated into the bounded typed startup
> layering described above. The later source-TLS slice replaces serving `NoTls`
> with verified transport, including the cancellation connection.
> The 2026-09-05 Rust runtime-snapshot foundation exposes a
> closed redacted readiness state: new requests acquire one immutable snapshot
> before request-body polling, not-ready state returns `503` with `Retry-After`,
> and in-flight response bodies retain their original snapshot through
> termination. Activation remains a crate-private, non-authorizing primitive;
> automatic drift observation, validated candidate construction, public reload,
> and automated drift telemetry remain M3/M5 work under ADR-0038 and ADR-0050.
> Commits `5694489`
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
> real `sf-cli` child, observes clean exit inside three seconds, and verifies the
> listener is closed. Commit `bec1cf7` adds fixed, redacted query-less
> `GET`/`HEAD /sparql` Service Description discovery as control metadata: it
> consumes no request body, runtime lease, deadline, or application-work permit,
> including while saturated, not ready, or draining. The complete ADR-0011
> control plane still requires the remaining ten metric families, OTLP,
> release-artifact qualification, source polling/failure policy, and SLO
> qualification named above.

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
Structured events **and** spans. The query pipeline is instrumented as a span tree — `serve_request → parse_sparql → unfold → optimize_cascade` (one bounded stage span around the whole cascade, not a child per branch/pass) `→ emit_sql → execute → serialize`. `tracing-subscriber` uses a closed typed level ceiling and an exact product-target allowlist (JSON in production, pretty output may be added for development); `tracing-opentelemetry` remains the accepted route for future OTLP export to a collector.

### Metrics — `metrics` facade → `metrics-exporter-prometheus` (OTel-compatible)
Concrete catalogue:
* **Virtualisation:** `sf_query_duration_seconds` (histogram → p50/p95/p99), `sf_query_total{status}`, `sf_sql_emitted_total`, `sf_recursion_depth` (histogram — the `P+` governance signal, ADR-0010), `sf_result_rows` (histogram), `sf_governance_rejections_total{reason}`.
* **Streaming / memory:** `sf_peak_memory_bytes` (the bounded-memory invariant, ADR-0006), `sf_stream_rows_total`, `sf_first_result_seconds`.
* **Resource:** `sf_pool_connections{state}` (gauge), `sf_db_roundtrip_seconds`, `sf_cache_hits_total` / `sf_cache_misses_total`.

### Governance events (ADR-0010)
Limit-hit / timeout / rejection / injection-attempt emit **both** a `tracing` warn-event **and** a `metrics` counter — one trace, one alertable metric.

### Configuration model
Layered precedence: **defaults < config file (TOML) < env vars < CLI**, validated at startup and fail-fast. Secret references are resolved once after settings merge, not as another settings tier. The implemented boundary uses a bounded `serde`/TOML model and validates effective values through the typed Clap contract; a general `figment`/`config` dependency is not required. Sections: `[source]` (connections — ADR-0006), `[mappings]` (location/format), `[graphs]` (the in-memory T/M paths — ADR-0004), `[governance]` (the ADR-0010 limits), `[observability]` (log level and metrics enablement), `[serve]` (endpoint config), and `[security]` (environment references and explicit anonymous permission). OTLP endpoint/metrics-port settings are not implemented. **Secrets** are referenced, never inline (e.g. `auth_token_env = "SF_QUERY_BEARER"`); direct external secret-store transport remains separate work.

### Health, readiness, and bounded shutdown

`GET /livez` is a fixed process/event-loop liveness response. `GET /readyz` is a
fixed projection of the immutable runtime readiness state: ready is `200`,
not-ready is `503` plus `Retry-After: 1`, and poisoned state is a redacted `500`.
Both are `application/json`, `no-store`, and `nosniff`; neither reads request
content, acquires application capacity, or probes a database. SIGTERM and Ctrl-C
move `Running` to `Draining`, mark readiness administrative not-ready, reject new
budgets, and stop new ingress while already-admitted requests retain their
identities and may complete. HTTP drain and detached work retaining request
capacity share the original positive monotonic-clock deadline. At that bound,
`Forced` broadcasts cancellation; a separate three-second allowance keeps the
runtime alive for owned native stop/discard. Non-quiescence returns `TimedOut`,
never a clean shutdown. This corrects the earlier drop-serving-future assumption:
HTTP completion alone does not prove native cleanup completed. Paused-clock
tests cover grace, retained ownership and bounded failure; owned PostgreSQL
16.15/MySQL 8.4.11 TLS CLI tests observe native stop, clean exit and closed ingress
after forced SIGTERM during each of ASK/SELECT/CONSTRUCT and mixed UNION/joins.
Server-side lock/session witnesses distinguish real stop from future drop;
separate same-credential CLI siblings remain unaffected. Wider backend/proxy
qualification, source-health policy, remaining metrics/OTLP and release admission
remain open.

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
