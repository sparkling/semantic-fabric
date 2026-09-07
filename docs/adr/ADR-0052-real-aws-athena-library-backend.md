---
status: proposed
date: 2026-09-07
updated: 2026-09-07
tags: [athena, aws, sql-backend, sigv4, cloud-backends]
supersedes: []
depends-on: [ADR-0010, ADR-0024, ADR-0036]
implements: []
---

# Real AWS Athena library backend

## Status boundary

This ADR proposes and records a real AWS Athena `SqlBackend` library
implementation for the first bounded provider slice of
`sparkling/semantic-fabric#7`. It does not admit Athena through `sf-serve`,
establish production support, or close that issue. No live AWS credentials are
available for this work, so its provider evidence is limited to deterministic
signing tests and mocked AWS protocol contracts.

## Context

ADR-0036 found that the type named `AthenaBackend` spoke the generic
Presto/Trino `/v1/statement` protocol rather than the AWS Athena API. Athena
instead uses AWS JSON 1.1 operations authenticated with SigV4:
`StartQueryExecution`, polling through `GetQueryExecution`, paged
`GetQueryResults`, and `StopQueryExecution`.

The accepted ADR-0036 is a historical remediation record tied to its own commit
and CI evidence. This later provider decision therefore depends on it rather
than modifying it.

## Decision

### Provider and dependency boundary

Athena is enabled only by the dedicated `athena-backend` Cargo feature. The
existing `rest-backends` feature continues to enable Snowflake, BigQuery,
Databricks, and Trino/PrestoDB without bringing in the AWS dependency closure.

The implementation uses the official low-level `aws-sigv4` signer and
`aws-credential-types` credential container. It does not use the full
`aws-sdk-athena` or AWS profile/IMDS credential chain. Configuration reads only
the dedicated `SF_ATHENA_*` environment namespace:

- `SF_ATHENA_REGION`
- `SF_ATHENA_ACCESS_KEY_ID`
- `SF_ATHENA_SECRET_ACCESS_KEY`
- optional `SF_ATHENA_SESSION_TOKEN`
- optional endpoint, catalog, database, workgroup, and output-location values

Injected configuration and credentials remain available for embedding and
tests. Secret-bearing values are redacted from debug output and local or remote
diagnostics.

### Protocol behavior

One logical query:

1. sends a SigV4-signed `StartQueryExecution` request with a fresh idempotency
   token;
2. polls `GetQueryExecution` within finite request and total deadlines;
3. handles `SUCCEEDED`, `FAILED`, and `CANCELLED` explicitly;
4. issues `StopQueryExecution` when abandoning a known in-flight query; and
5. fetches `GetQueryResults` lazily, one bounded page at a time.

Retries are bounded and limited to transport failures, server failures, and
AWS throttling shapes. Redirects are disabled because signatures are
host-bound and request headers can contain a session token. Result tokens are
treated as opaque, length-bounded values sent only to the fixed validated
endpoint. Constant-memory cycle detection rejects repeated token chains.

First-page `ColumnInfo` establishes the result schema. Later metadata must
match it exactly or the response is rejected. Rows preserve nulls and are
normalized to that established width. Athena's repeated header row is removed
only from the first page when every value exactly matches the column names.

### Parameter boundary

The current `SqlBackend` interface carries `Vec<String>` without SQL or XSD
type metadata. Values are therefore sent through Athena
`ExecutionParameters` as escaped SQL string literals, not interpolated into
the query text. Non-string comparisons require explicit casts in generated
SQL. This is not claimed as typed parameter transport.

## Evidence

Mocked contracts use the documented AWS JSON 1.1 request and response shapes
and cover:

- signed start, poll, stop, and paged-results requests;
- a deterministic SigV4 known-answer value;
- queued, running, succeeded, failed, and deadline-cancellation paths;
- lazy paging, null and short-row decoding, repeated tokens, and schema drift;
- AWS error-shape-aware retries and non-retryable validation failures; and
- credential redaction from hostile service diagnostics.

The feature matrix verifies the package with neither provider feature,
`rest-backends` alone, `athena-backend` alone, and both together. Local
workspace format, strict Clippy, all-target build, tests, dependency audit,
generated capability checks, and W3C conformance remain required before the
branch is presented. Required CI has a dedicated `athena-backend` Clippy and
test lane because workspace feature unification does not otherwise enable it.
This evidence is local until that lane passes at the exact PR SHA and does not
substitute for a live AWS canary.

## Consequences and remaining admission gates

The former Presto behavior remains available under `TrinoBackend` and
`PrestoDbBackend`. `AthenaBackend` is now an intentionally different
constructor and protocol surface.

Athena remains unavailable through `sf-serve`. Admission still requires:

- a credentialed live-AWS canary;
- end-to-end R2RML execution through a supported serving surface;
- live datatype and differential evidence;
- serving-level cancellation and stream-drop behavior;
- bounded-memory evidence through that admitted surface;
- resolution or explicit acceptance of the typed-parameter limitation; and
- exact-SHA remote CI and deployment evidence.

`GetQueryResults` also requires S3 `GetObject` permission for the selected
result location. Running `column_names` executes a billed Athena query.
Dropping a stream after query success does not undo that execution.

## References

- [AWS Athena StartQueryExecution]
- [AWS Athena GetQueryExecution]
- [AWS Athena GetQueryResults]
- [AWS Athena StopQueryExecution]
- [AWS Signature Version 4]

[AWS Athena StartQueryExecution]: https://docs.aws.amazon.com/athena/latest/APIReference/API_StartQueryExecution.html
[AWS Athena GetQueryExecution]: https://docs.aws.amazon.com/athena/latest/APIReference/API_GetQueryExecution.html
[AWS Athena GetQueryResults]: https://docs.aws.amazon.com/athena/latest/APIReference/API_GetQueryResults.html
[AWS Athena StopQueryExecution]: https://docs.aws.amazon.com/athena/latest/APIReference/API_StopQueryExecution.html
[AWS Signature Version 4]: https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv.html
