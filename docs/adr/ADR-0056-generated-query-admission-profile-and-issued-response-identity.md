---
status: proposed
date: 2026-09-19
updated: 2026-09-19
tags: [sparql, admission, query-shape, generated-query, response-identity, fail-closed, security]
supersedes: []
depends-on: [ADR-0002, ADR-0007, ADR-0010, ADR-0018, ADR-0052, ADR-0053]
---

# Generated-query admission profile and issued response identity

## Status boundary

Proposed, not implemented. Nothing here is built, calibrated or activated, and
no existing behaviour changes until an implementation slice lands and is
recorded under a dated amendment below.

This ADR does not claim that the current serving boundary is unsafe. The
controls it asks for are additive to ADR-0010's reachability floor and
ADR-0018's subject-axis admission, both of which remain in force and are not
re-decided here. It claims only that a specific caller — a machine that
generates SPARQL it did not author and must record what it was permitted to
run — has no profile it can name and no identity it can bind to.

This ADR does not propose an authorization model, per-identity row filtering,
tenancy, or any change to who may read which rows. That is ADR-0018's axis and
stays there. It does not propose federation, SERVICE support, or any widening
of the ADR-0002 charter.

It is filed from a consumer's perspective by the semantic-query project, which
needs this boundary and cannot build it. Whether Fabric wants the capability at
all, and on what schedule, is the Fabric team's decision, not the filer's.

## Context

A downstream consumer composes SPARQL from a model's proposal rather than from
a human-authored template, then persists what was accepted so a later refresh
can replay the identical query. That consumer cannot admit its own generated
query, for the reason ADR-0010 already gives about gateways: the check has to
happen where the query is understood, which is inside the engine.

Its blocked call site refuses unconditionally today rather than route an
unadmitted generated query to a live source. An earlier attempt there scanned
keywords before parsing and was removed as wrong in both directions — it
rejected safe queries and admitted unsafe ones, because a keyword scan is not a
parse. The correct boundary is Fabric's, after its own parse and before any
source I/O.

Four things that consumer needs, and where each stands today. None of these is
an omission the corpus overlooked; three are deliberate decisions recorded in
source or in an existing ADR, which is why this is a new decision rather than a
bug report.

**Query form.** All three plan forms are served: `crates/sf-serve/src/http.rs`
dispatches `PlanForm::Select`, `PlanForm::Ask` and `PlanForm::Construct` to
execution, and DESCRIBE lowers into `PlanForm::Construct` in
`crates/sf-sparql/src/lib.rs`, so it is served too. ADR-0018 states the posture
plainly — "SELECT/ASK/CONSTRUCT and both supported UNION". A generated-query
caller that wants answers, never graph construction, cannot express that.
UPDATE is already excluded, but incidentally: `crates/sf-sparql/src/lib.rs`
only ever calls `spargebra`'s `parse_query`, and `parse_update` is never called
anywhere in the tree, so UPDATE dies as a parse error rather than as a policy
decision a caller can rely on.

**SERVICE.** Already rejected, and this ADR must not re-decide it. ADR-0002
excludes SERVICE federation from charter; ADR-0007 records that it returns
`501`; `crates/sf-sparql/src/unfold.rs` carries the catch-all arm commented
"Deferred → 501 (documented, never silent): LATERAL, SERVICE". The rejection is
sound. It is reached as an unsupported-feature deferral at translate time
rather than as a named rule at admission, which matters only in that a caller
cannot cite a rule for it.

**The dataset clause.** `FROM` and `FROM NAMED` have no allowlist and no
decided meaning at the serving boundary. The `dataset` field is checked in the
bounded-federation path (`crates/sf-sparql/src/federation.rs` refuses when
`dataset.is_some()`) and is silently discarded on the ordinary single-source
path, where every arm destructures as `{ pattern, .. }` and
`star/top_level.rs` clones it through the RDF-star rewrite. Such queries parse
and flow; the corpus's own tests exercise them. A generated query naming an
arbitrary graph is neither honoured nor refused — it is ignored.

**Issued identity.** Fabric has a rich internal digest vocabulary and
deliberately withholds it from callers. `crates/sf-sparql/src/runtime_identity.rs`
says of `SemanticIdentity` that it "partitions compiler/cache scope; it is
deliberately not an admission token by itself" — an explicit non-decision, not
a gap. The per-runtime binding identity is
`RuntimeBindingIdentity(Arc<()>)` compared by `Arc::ptr_eq`, a process-local
pointer that cannot be serialized to a client at all. No response header in
`sf-serve` carries identity; the only per-request value that reaches a caller
is an opaque, randomly salted correlation id attached to problem responses. The
three "profile id" hits elsewhere in the corpus are a different concept:
ADR-0050 and ADR-0051 are source-schema activation profiles that never accept
caller-supplied ids, and ADR-0052's compile-profile identity partitions cache.

The substantive failure is in coverage, and it is a silent one.
`crates/sf-serve/src/semantic_admission.rs` runs a real digest-backed gate —
projects the mapping to RDF, joins the ontology graph, runs SHACL, refuses on
violation — but at runtime construction, over the mapping, not per-query over
the query's own constant IRIs. Per-query, an unmapped IRI yields zero branches
and collapses to `Unify::Empty` in `crates/sf-sparql/src/unfold.rs`. A
generated query naming an out-of-ontology IRI therefore returns an empty `200`,
which a machine consumer cannot distinguish from "this question has no
answers". For a caller replaying a persisted query against a mapping that has
since moved, silent narrowing is the dangerous outcome: it reads as data, and
the recorded answer quietly stops meaning what it meant.

## Considered options

**Leave it to the consumer.** Rejected for ADR-0010's own reason: the check
requires the parsed algebra and the mapping, neither of which the consumer has.
A pre-parse scan is not a parse, and the one attempt was wrong in both
directions.

**Extend ADR-0018's `QueryAdmission` with a fifth variant.** Structurally the
closest fit — `QueryAdmission` is immutable, service-lifetime and deny-by-default,
which is the right shape. Rejected as the whole answer because its axis is
wrong: `Deny` / `UnrestrictedDevelopment` / `Bearer` / `ProvisionedBearers`
answers "who is asking, and which rows", and `admit()` takes only a
`&HeaderMap` — it never sees form, dataset clause or constant IRIs. Query shape
is a second axis, evaluated after authentication and before compilation. It
should compose with ADR-0018 rather than overload it.

**Treat unmapped IRIs as a global error.** Rejected: silent narrowing is
correct for ordinary human-authored queries, where an unmatched pattern
legitimately means no rows. Changing it globally would break the existing
contract. The strictness must be opt-in, selected by profile.

**A named query-shape profile that also issues an identity.** Preferred, below.

## Proposed decision

### 1. Add one named query-shape admission axis, evaluated after authentication and before compilation

A closed profile enum, deny-by-default and immutable for the service lifetime,
selected by the embedding exactly as `QueryAdmission` is. The existing default
must remain the current permissive behaviour, so no deployment changes posture
by upgrading. The generated-query profile is opt-in.

It is a distinct axis, not a `QueryAdmission` variant. A request satisfies
subject admission first and query-shape admission second; neither substitutes
for the other. `crates/sf-serve/src/request_compile.rs` already owns the
preflight/compile ordering ahead of source I/O and is the natural hook.

### 2. Under the generated-query profile, restrict the admitted form to SELECT and ASK

CONSTRUCT and DESCRIBE must be refused with a typed, redacted problem response,
before compilation. This narrows only within the profile; the served set for
every other profile is unchanged and ADR-0018's posture stands.

UPDATE must be refused by an explicit rule rather than by relying on
`parse_update` never being called. A caller must be able to cite the rule, and
the guarantee must not rest on the continued absence of a call site.

### 3. Refuse SERVICE and any dataset clause outside a pinned allowlist, by name

SERVICE is already rejected and this clause changes no behaviour — it requires
only that the refusal be reachable as a named admission rule under this
profile, at the same point as the other checks, so the guarantee is citable and
does not depend on a translate-time catch-all remaining a catch-all.

`FROM` / `FROM NAMED` must be decided rather than discarded. Under this
profile, a dataset clause is admitted only if every named graph it references
appears in a pinned allowlist supplied by the embedding, and refused otherwise.
Silently ignoring it must not remain an option here: a caller that names a
graph and is neither honoured nor refused cannot know which dataset answered.

### 4. Validate every constant IRI and named graph against the pinned mapping, and refuse rather than narrow

Under this profile, the existing mapping/ontology admission receipt must be
consulted per-query, before source I/O, for coverage of the query's own
constant IRIs and named graphs. An uncovered constant is a typed refusal, never
an empty result.

This is the clause that carries the real weight. It must not change the
default: outside the profile, silent narrowing stays exactly as it is.

### 5. Return a server-issued response-profile identity the caller can bind to

A successful admission under this profile must yield a stable, serializable
identity to the caller — reversing, for this one profile, the deliberate
decision that `SemanticIdentity` is not an admission token. That decision was
right for cache partitioning and remains right there; what a generated-query
caller needs is a different value with a different purpose.

The identity must be server-issued and never caller-supplied, consistent with
ADR-0050 and ADR-0051's refusal of caller-supplied profile ids. It must change
whenever anything it attests changes — profile constants, the pinned mapping or
ontology digest, the allowlist — so that a stale identity cannot silently
continue to look valid. It must be an attestation of what was admitted, not a
bearer credential: possessing it must confer no authority on a later request.

`PolicySnapshotId` in `crates/sf-serve/src/query_security.rs` is the nearest
existing analogue and is consumed internally only; whether the new value
extends it or stands beside it is an implementation question.

## Implementation gates

Every check in clauses 2 through 4 must run before any source I/O, on the
parsed algebra, and must fail closed. A check that cannot run is a refusal, not
a pass.

Refusals must be typed and redacted, consistent with ADR-0010's existing
`429` handling: a refusal must not disclose mapping shape, ontology contents or
allowlist membership to an unadmitted caller.

The profile must compose with ADR-0053's process-isolated parser boundary
rather than introduce a second parse. Admission decisions consume the algebra
that boundary already produced.

The default profile must be byte-for-byte behaviour-preserving. A regression
proving the unchanged path is unchanged is a gate, not a nicety.

## Evidence and acceptance

Refused: CONSTRUCT, DESCRIBE, UPDATE, SERVICE, a dataset clause naming a graph
outside the allowlist, and a query carrying a constant IRI absent from the
pinned mapping — each before source I/O, each typed, each redacted.

Admitted: SELECT and ASK whose every constant IRI and named graph is covered,
returning a response identity that a caller can persist and later compare.

The distinguishing test, which is the point of clause 4: a query naming an
out-of-ontology IRI must be refused under this profile and must still return an
empty `200` under the default profile, in the same build.

Identity stability: unchanged inputs yield an unchanged identity across
restarts; changing the mapping digest, ontology digest, allowlist or profile
constants changes it. A stale identity must be detectably stale rather than
silently accepted.

## Consequences

Fabric gains a caller it does not have today: a machine that generates queries
and can record, verifiably, what it was permitted to run. The
`AdmissionBlocked` refusal in the downstream consumer becomes resolvable
without that consumer guessing at safety it cannot establish.

The cost is a second admission axis to reason about, and a genuine reversal of
one recorded decision — that identity is deliberately not issued outward. That
reversal is scoped to one opt-in profile precisely so the existing rationale
keeps holding everywhere else.

Silent narrowing remains the default, and remains correct there. This ADR adds
a profile in which it is not correct; it does not argue it was ever wrong.

## Nonclaims

No authorization, tenancy, row-level or per-identity filtering: ADR-0018's axis
is untouched. No federation or SERVICE support: ADR-0002's charter is
unchanged. No claim that current serving is unsafe, and no claim about
performance, since nothing is built or measured. No schedule: whether this is
worth building, and when, is the Fabric team's call.

## More information

- [ADR-0002](ADR-0002-implementation-scope-rdbms-both-modes.md) — SERVICE excluded from charter.
- [ADR-0007](ADR-0007-sparql-to-sql-rewriting-strategy.md) — residual shapes return `501`.
- [ADR-0010](ADR-0010-security-and-resource-governance.md) — reachability floor; "the mapping is the allow-list"; pre-source-I/O ordering.
- [ADR-0018](ADR-0018-security-edge.md) — subject-axis admission profiles; `QueryAdmission`.
- [ADR-0052](ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md) — compile-profile identity and cache partitioning.
- [ADR-0053](ADR-0053-grammar-coupled-sparql-parser-governance-and-process-isolation-fallback.md) — process-isolated parser boundary.

Filed by the semantic-query project, whose ADR-010 E3/P3 work is blocked on
this boundary. Source references were read at semantic-fabric `e16a52b8`.
