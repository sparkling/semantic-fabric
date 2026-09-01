---
status: proposed
date: 2026-08-28
updated: 2026-09-01
tags: [federation, physical-plan, bounded-memory, spill, external-memory, sparql, consistency, cancellation]
supersedes: []
depends-on:
  - ADR-0006
  - ADR-0010
  - ADR-0012
  - ADR-0038
  - ADR-0048
implements:
  - ADR-0038
---

# Bounded federated global operators and spill

## Status boundary

This ADR is **proposed**. It records a concrete recommended architecture, but it
does not accept a spill substrate, claim federation exists, or weaken the
exact-or-reject rule. Acceptance waits for the comparison evidence and explicit
maintainer decisions listed below. Its `implements` relationship identifies the
ADR-0038 design lock, not implementation completion.
`sf-core::SourceId`/`SourceMapping` and the current immutable single-source runtime
binding now join one backend, dialect, T-box, constraint-quarantined compiler
schema, explicit `ConstraintAuthority::Unverified` and plan cache, and reject a
detached plan before I/O. Its cache scope includes that authority. They still
provide no digest-addressed runtime snapshot, structural/type drift/reload
lifecycle, verified-constraint lease, federation or `ConsistencyVector`.
Accepting this ADR would explicitly amend ADR-0006's cross-source rule: bounded
semi-join reduction and streaming merge alone cannot implement every exact N:M
join/operator listed here. The source-pushdown and no-general-OLAP decisions
remain; the proposed amendment admits only the irreducible, quota-bounded
external operators below.

## Context and problem statement

The semantic compiler currently emits a per-source `sf_sparql::Plan`: a bag of
SQL branches plus result form, DISTINCT, slice, ordering, and optional Rust-level
grouping. The public runtime owns one backend. Some multi-branch ORDER, GROUP,
solution/triple dedup paths retain source-sized `Vec` or `HashSet` state, and the
existing semi-join cost model has no production federation caller.

ADR-0006 correctly keeps relational scan/join/set work in a source database and
rejects a general in-process OLAP mediator. ADR-0038 nevertheless retains the
cross-RDBMS charter. No one source can perform a join, global order, aggregate,
or dedup over rows owned by several independent sources. The coordinator needs
typed global semantics and bounded external-memory algorithms without becoming
a second semantic compiler or treating a Bloom filter as an answer authority.

The revision-pinned canonical gold in `semantic-builder`—machine bundle,
14-category Turtle tree and manifest—and the separately revision-pinned source
snapshot plus mutable live development PostgreSQL instance are the initial
prototype corpus. Each run seals the gold manifest/source revision and records live version/schema
observations separately; no source-to-container provenance link is implied. Semantic
Fabric's in-charter qualified development R2RML KAT covers one of 112 tables/two columns. The
gold has 4,501 mapping quads total; its Source Mapping facet declares 134 generic
RML TriplesMaps/492 predicate-object maps outside Semantic Fabric's charter.
Wider federation evidence requires explicit maps or independent generated
fixtures, never inferred mappings.

## Considered options

- **Source-sized Rust collections.** Rejected. They violate ADR-0006/0010 and
  fail under skew or large N:M joins.
- **Route every source through a general embedded analytical engine.** Not
  selected. It would reverse ADR-0006's relational-path decision and enlarge the
  production closure before correctness, cancellation, and spill controls are
  proven.
- **Purpose-built bounded external-memory operators.** Recommended current
  default. This is the narrow proposed amendment to ADR-0006's semi-join-only
  cross-source clause and owns only the irreducible cross-source work.
- **Tightly scoped embedded analytical coordinator.** Retained only as a measured
  alternative. A pinned candidate such as DuckDB may receive reduced source
  fragments, but may not replace the compiler, connect to systems of record, or
  become the default without an explicit amendment to ADR-0006.

## Proposed decision

### 1. Preserve the semantic compiler and per-source `Plan`

Keep parsing, T-box saturation, mapping compilation, IQ normalization/cascade,
dialect emission, parameter binding, and RDF reconstruction in their existing
boundaries. Add source affinity without cloning those stages:

- `SourceId` is an opaque, non-secret identifier unique within one immutable
  `RuntimeSnapshot`; it never contains a DSN or credential.
- Every logical table/mapping and backend capability record is bound to one
  `SourceId` before query admission.
- A `SourceFragment` contains `SourceId`, the existing per-source `Plan`, typed
  output variables, explicit constraint authority, and immutable
  capability/schema/mapping/policy digests. It contains no transaction or
  snapshot handle.
- A cacheable `FederatedPlan` contains source fragments, one typed
  global-operator tree, failure policy, required reservation shape, and the
  runtime/configuration epoch plus immutable ontology, mapping, schema, and
  capability/constraint-policy digests. It contains no request budget, acquired
  token, timing, execution provenance, or database handle.
- A request-scoped `FederatedExecution` owns the `QueryBudget`, cancellation
  state, admitted reservations, acquired source sessions/transactions, timing,
  and execution provenance/receipt.
- A `ConsistencyVector` is captured only after acquisition and records each
  source snapshot/epoch token for receipt and provenance; it is never cached as
  part of the plan.

The federation lowering step partitions an already characterized semantic plan
at source boundaries. It may push a semantically equivalent subtree or
composite SQL into one source, but it may not reparse SPARQL or invent weaker
semantics. Single-source queries continue to execute the existing `Plan`
directly.

The current serving policy contributes only structural/type facts and unverified
constraint authority; constraint-driven rewrites remain disabled in every
fragment. A future verified mode must hold a source-specific, unforgeable lease
from coherent constraint revalidation through the complete streamed fragment
execution and bind that authority into the snapshot/cache namespace. A
fingerprint checked before execution is insufficient. Direct Mapping is also a
mapping lifecycle, not a compiler shortcut: because its PK/FK-derived mapping can
change across DDL, any future live Direct-Mapping fragment must be generated from
and execute under the same verified source generation. Current `sf-serve`
accepts authored R2RML and performs no such generation.

### 2. Exact global operator algebra

The only initially admissible coordinator nodes are:

| Node | Required behavior |
|---|---|
| `Fragment` | Stream one admitted per-source `Plan` under the execution's acquired snapshot and budget. |
| `UnionAll` | Concatenate input multisets; preserve every multiplicity. |
| `InnerJoin` | Emit every compatible merged solution, including full N:M multiplicity. |
| `LeftJoin` | SPARQL OPTIONAL: emit all compatible extensions or exactly one unchanged left solution when none matches. |
| `Minus` | Remove a left solution only for a compatible right solution with a non-empty shared domain; disjoint domains are a no-op. |
| `Filter` / `Extend` | Evaluate characterized SPARQL expressions over combined mappings, preserving error and UNBOUND behavior. |
| `Project` | Restrict variables without silently deduplicating the bag. |
| `OrderBy` | Apply the existing SPARQL term/value comparator globally, with deterministic spill-run tie handling. |
| `GroupAggregate` | Group and compute the currently admitted `COUNT`, `SUM`, `AVG`, `MIN`, and `MAX`, including aggregate `DISTINCT`, errors, and empty groups. |
| `DistinctSolutions` | External distinct over complete projected solution mappings; `REDUCED` lowers here for deterministic correctness. |
| `Slice` | Apply OFFSET/LIMIT after the preceding global modifiers. |
| `Construct` | Instantiate templates and externally deduplicate graph/quads where graph-set semantics require it. |

`ASK` is a result mode over this tree, not a second operator. Cross-source
recursive closure, external SPARQL `SERVICE`, window functions, and any aggregate
outside the admitted set remain pre-execution unsupported until a later design
adds an exact bounded node and evidence. There is no generic “evaluate in Rust”
fallback.

### 3. Semantic keys, bags, UNBOUND, and RDF terms

Spill keys are a versioned binary semantic encoding, never display strings,
source-native collation bytes, or SQL NULL equality. The encoding contains the
variable schema, a bound-domain bit mask, and typed RDF terms: term kind,
lexical form, datatype or normalized language tag, graph-scoped blank-node
identity, and recursively encoded RDF 1.2 triple terms.

Operator equality and order remain operator-specific:

- solution compatibility compares RDF terms only on variables bound in both
  domains; an UNBOUND slot is absence, not a value equal to another UNBOUND;
- DISTINCT compares complete projected solution mappings, including their bound
  domains and RDF-term identity;
- joins preserve bag multiplication, while only explicit DISTINCT or graph-set
  construction removes duplicates;
- grouping, aggregate expression evaluation, and ORDER use the same SPARQL
  value/error/comparator law as the characterized single-source path, not a byte
  sort of the spill encoding; and
- an R2RML-generated blank-node identity is derived from `(result-dataset or
  request scope, target graph identity, natural RDF lexical form through a
  versioned bijection)`. Equal generated identifiers in one graph share even
  across rows or term maps, while the same identifier in different graphs is a
  distinct node (R2RML §9.1/§11.2). `SourceId`, triples-map identity and
  term-map identity are therefore not identity components. SPARQL CONSTRUCT
  template blank nodes follow the separate fresh-per-solution rule.

Hashing is permitted only after the semantic key is constructed. Hash equality
always requires a full semantic equality check.

#### Implemented precursor: single-source blank-node identity

The 2026-09-01 Rust graph-scope slice implements this blank-node identity law
inside the existing single-source compiler and executor. A dedicated IQ term
definition carries the generated identifier recipe and effective graph recipe
through unfolding, unification, cascade rewrites, subplan projection,
reconstruction, query execution, quad dumping, class atoms, direct and
reference objects, and fixed-graph path endpoints. Runtime labels are a
versioned injective encoding of default/named graph plus natural identifier;
row-derived `rr:defaultGraph` normalizes to the default graph. CONSTRUCT
template nodes use a separate versioned fresh-per-solution domain.

Required SQLite differentials pin same-graph co-reference across maps,
cross-graph separation, constant/dynamic graph equivalence, direct dump
identity, child-graph reference-object scope, class/path propagation and
DISTINCT subplan remapping. The implementation remains deliberately partial:
differently shaped recipes that happen to render the same identifier can still
return `501`, as can dynamic-graph paths and row-dependent rendered-width
pooling. Those are sound completeness limits, not alternative identity laws.
No PostgreSQL/MySQL named-graph matrix, cross-source request scope, spill key,
federation runtime or bounded global graph dedup is claimed. This precursor
therefore does not accept this ADR.

### 4. Purpose-built bounded external-memory substrate

The recommended implementation has one fixed memory arena and one spill quota
charged to `QueryBudget`:

- `InnerJoin`, `LeftJoin`, and `Minus` use partitioned hash processing over
  bound-mask/term keys. Oversized partitions are recursively repartitioned;
  skewed hot partitions fall back to external sort-merge rather than entering an
  unbounded hash table.
- N:M hot keys are processed as bounded nested streams over spill runs. Every
  pair is emitted, subject to the declared result/work quota; multiplicity is
  never capped or sampled.
- `OrderBy` uses sorted runs plus bounded k-way merge. Comparator keys and stable
  input ordinals travel with the run; source collation is not authoritative.
- `GroupAggregate` uses partitioned aggregation with external-sort fallback.
  Aggregate-DISTINCT state spills through the same distinct primitive.
- `DistinctSolutions` and `Construct` graph dedup use partitioned hash distinct
  with full-key collision checks and external-sort fallback for hot partitions.
- `UnionAll`, `Filter`, `Extend`, `Project`, and eligible `Slice` paths remain
  streaming through bounded channels.

No operator may allocate proportionally to input cardinality in memory. Disk use
is also bounded: exhausting the reserved quota is a query failure, not authority
to fall back to RAM, discard rows, or return a prefix as success.

### 5. Semi-join reducers are non-authoritative

The existing cost model may select a bounded `IN` batch, source-local temporary
table, Bloom filter, or skip-if-unselective path. A reducer may reduce bytes; it
may not decide semantic membership:

- exact reducers contain only keys observed on the build side;
- a Bloom filter may have false positives but must have no false negatives;
- every surviving row is checked by the authoritative global join/operator; and
- stale/thin statistics may choose a slower plan but cannot change results.

Reducer creation, upload, source-local temporary objects, and cleanup are charged
to the shared budget and cancellation tree.

### 6. Admission, backpressure, cancellation, and failure

Before opening source work, each execution admission reserves bounded memory, spill bytes,
files/descriptors, operator tasks, source connections, and serializer/result
allowance. A request that cannot reserve its minimum is rejected. Global and
per-query high-water marks prevent disk exhaustion; a low-free-space watermark
stops new admission.

Sources, operators, and serialization connect through bounded, demand-driven
channels so a slow client propagates backpressure to every source cursor. One
cancellation token spans all fragments, reducers, spill tasks, and output. A
disconnect, deadline, source failure, integrity failure, or quota breach cancels
all participants and releases capacity within the ADR-0038/M2 bound.

Federation has no partial-success mode. A source is never replaced with an empty
bag and remaining fragments never continue as a successful answer. Before HTTP
headers, failure returns the normal safe problem response. After streaming has
begun, failure aborts the transport and serializer without a valid terminal
document and records a failed receipt; it never emits a normal completion or
success metric for the observed prefix.

### 7. Secure spill and crash cleanup

Spill uses a configured absolute directory opened without symlink traversal.
The process creates a private `0700` run directory and exclusive `0600` files
with random names. Each file has a versioned header, query/run identifier,
operator/schema digest, length bounds, and authenticated or checksummed blocks;
malformed, truncated, cross-query, or replayed blocks fail closed. Credentials,
DSNs, raw query text, and telemetry labels are never written to spill metadata.

Normal completion and every cancellation path close and unlink reducers, runs,
and manifests. A lease/owner manifest permits a startup janitor to remove only
this application's provably orphaned run directories after validating ownership,
format, age, and configured root. It never recursively removes an unvalidated
path. Deletion is not represented as secure erasure on SSDs.

The confidentiality policy is an explicit open maintainer decision below. The
recommended implementation is per-query AEAD encryption with an ephemeral
in-memory key plus block authentication, even when the volume is encrypted.

### 8. Consistency: a snapshot vector, not a distributed transaction

Every source fragment runs in one backend-specific read-only repeatable-read
equivalent snapshot for its complete lifetime: PostgreSQL repeatable-read,
MySQL consistent-snapshot semantics, and a stable SQLite read transaction. The
backend contract proves acquisition, retention, timeout, cancellation, and
cleanup before that backend is admitted to federation.

The request-scoped `ConsistencyVector` records `(SourceId, backend
snapshot/epoch token, acquired-at, schema/mapping digest)` only after all
required handles are acquired. This gives repeatability per source. It is
**not** a globally atomic point-in-time snapshot, two-phase commit, or serializable
transaction across systems. The response capability/provenance record states
that limitation. If a required per-source snapshot cannot be obtained or held,
the query fails rather than silently dropping to read-committed behavior.

## Substrate comparison required before acceptance

Implement comparable, non-production prototypes of:

1. the recommended purpose-built external-memory substrate; and
2. at least one pinned embedded analytical candidate, initially DuckDB, limited
   to coordinator-owned temporary data with a hard memory/disk quota, no source
   connectors, no semantic planning, and complete cancellation/cleanup hooks.

Run identical semantic-key adapters, query corpus, quotas, three-backend source
matrix, and failure schedules. Compare correctness, peak heap/RSS, spill bytes
and amplification, first-result and completion latency, skew/N:M behavior,
backpressure, cancellation latency, crash residue, binary/dependency closure,
advisories, and implementation/maintenance surface. A fast happy-path benchmark
alone is not decision evidence.

The recommendation remains the purpose-built substrate because it is the
narrowest explicit amendment of ADR-0006's cross-source clause. An embedded
candidate can win only after the maintainer separately authorizes a further
ADR-0006 amendment and all hard gates below pass.

## Open maintainer decisions

1. **Spill encryption policy.** Recommended: AEAD encryption and authentication
   are mandatory for every production spill, using a per-query ephemeral key.
   The maintainer must decide whether an explicitly attested encrypted ephemeral
   volume may satisfy confidentiality without application-layer encryption, and
   define the threat model and key-erasure evidence for either policy.
2. **Cross-source amendment to ADR-0006.** Recommended: explicitly replace only
   ADR-0006's semi-join/streaming-merge-only clause with the purpose-built,
   quota-bounded operator substrate in this ADR. If evidence instead favors an
   embedded coordinator, the maintainer must separately amend ADR-0006's
   no-OLAP-intermediary clause to allow that tightly scoped role. Silence or an
   optional Cargo feature is not consent.

Until both decisions and the substrate comparison are recorded, this ADR stays
`proposed` and no coordinator is production-admitted.

## Exact acceptance gates

Acceptance requires one immutable evidence bundle satisfying every gate:

1. The typed `SourceId`/`SourceFragment`/`FederatedPlan` model and exact node set
   serialize canonically, while request-scoped `FederatedExecution` and
   `ConsistencyVector` cannot enter the plan cache; constraint authority and its
   policy digest must enter it; unknown versions/nodes are rejected and the
   existing single-source `Plan` path remains unchanged.
2. Two-source and three-source differential tests equal a trusted materialized
   reference for every admitted node and composition across SQLite, PostgreSQL,
   and MySQL, including duplicates, UNBOUND masks, incompatible RDF terms,
   OPTIONAL matched/unmatched/N:M, MINUS shared/disjoint domains, aggregate
   DISTINCT/empty/error cases, global order/slice, and CONSTRUCT dedup.
3. Generated/property tests permute fragment order, partitions, spill thresholds,
   source row order, and hash seeds without changing the result multiset/order
   contract; shrunk counterexamples persist under ADR-0012.
4. Hot-key and N:M fixtures at 1×/10×/100× preserve all multiplicities. Peak
   coordinator heap and RSS from 10× to 100× remain within 10% after measured
   noise; only reserved spill/result work may scale.
5. Selective reducers cut transferred bytes by at least 80%. An unselective
   fixture selects skip-reduction and adds no more than 10% transfer or wall-time
   overhead versus the recorded no-reducer baseline. Bloom false-positive and
   forced-collision tests never change answers.
6. Memory, spill, file-count, descriptor, work, row, byte, and deadline quota
   mutations fail closed with no RAM fallback, valid-looking partial success, or
   residue. Low-disk admission rejects before source execution.
7. Failure injection at every source read, reducer upload, partition write/read,
   merge, serialization, and client-disconnect boundary cancels all sources and
   removes owned temporary state; capacity is returned within one second.
8. A kill-and-restart test proves the janitor removes only validated orphaned run
   directories and never follows a symlink or touches a sibling/unowned file.
   Permissions, block integrity, confidentiality, redaction, and malformed-spill
   tests enforce the maintainer-approved encryption policy.
9. Concurrent source mutation proves each fragment sees one repeatable snapshot.
   Receipts expose the snapshot vector and tests explicitly demonstrate that it
   is not falsely described as one globally atomic timestamp.
10. The purpose-built and scoped embedded prototypes run the identical benchmark
    and adversarial corpus; results, closure digests, and independent review are
    attached to the recorded maintainer substrate decision.
11. Cross-source recursive closure and every unknown/unproved operator fail
    before source execution through the capability profile.

No aggregate readiness score can offset a failed gate.

## Consequences

- Good: the existing compiler, dialects, and per-source execution stay intact.
- Good: every irreducible global operation has named semantics, a bounded
  algorithm, and an exact-or-reject admission path.
- Good: source statistics and reducers affect performance only, never answers.
- Cost: a correct external-memory operator layer, semantic spill encoding, skew
  handling, and cleanup are substantial maintenance surfaces.
- Cost: per-source repeatability is weaker than global atomicity and must remain
  visible to callers.
- Neutral: a scoped embedded coordinator remains evidence, not architecture,
  unless the maintainer explicitly changes ADR-0006.

## Rules

- **R1** — preserve the semantic compiler and existing per-source `Plan`; add
  source identity and federation only at explicit physical boundaries.
- **R2** — global RDF/SPARQL semantics use typed semantic keys and preserve bags,
  UNBOUND, OPTIONAL, MINUS, skew, and N:M multiplicity exactly.
- **R3** — every blocking operator is partitioned/spilled under reserved quotas;
  no source-sized in-memory or generic fallback exists.
- **R4** — reducers are optimizations, never answer authority.
- **R5** — one budget/cancellation tree covers sources, coordinator, spill, and
  serialization; no failure is reported as partial success.
- **R6** — consistency is a recorded vector of per-source repeatable snapshots,
  never a false global-atomicity claim.
- **R7** — the ADR remains proposed until the comparison evidence and both
  maintainer decisions are explicit.

## More information

- Execution and bounded-memory architecture: ADR-0006.
- Resource governance, streaming, backpressure, and cancellation: ADR-0010.
- Differential, property, fuzz, and CI evidence: ADR-0012.
- Parent programme and M1/M2/M6 gates: ADR-0038.
