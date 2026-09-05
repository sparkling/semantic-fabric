---
status: accepted
date: 2026-09-05
updated: 2026-09-05
tags: [sparql, order-by, bounded-memory, query-budget, stable-sort, admission]
supersedes: []
depends-on: [ADR-0006, ADR-0010, ADR-0012, ADR-0024, ADR-0038, ADR-0040, ADR-0052]
implements: [ADR-0038, ADR-0040]
---

# Bounded stable root ORDER windows

## Status boundary

This ADR is **accepted** and its single-source execution and admission rules are
implemented. It does not accept ADR-0040's federation/spill proposal or claim
that ORDER has passed the complete ADR-0038 M1 qualification gate.

The required 1×/10×/100× requested-heap and fresh-process RSS gates exist; both
10×→100× median and p95 RSS growth are capped at 10%. The RSS workers exclude
source generation, assert the exact fixed result, and use an explicit 64 KiB
SQLite page cache as a measurement-source control, not a production tuning
claim. A required independent materialized-oracle differential covers the
admitted value domains, directions, UNBOUND, multiple keys and slice boundaries.
It uses `spareval` only where relative order is defined; separate bag,
repeatability and source-stable-tie assertions do not turn undefined
cross-domain, NaN, partial-calendar or partial-duration order into a semantic
claim. The optional live Product Mock KAT admits the exact `LIMIT 10001` query
through `sf-serve`, returns all 500 current typed rows in direct-SQL order, and
rejects `LIMIT 10002` before opening a poison pool. This completes the finite
root window's qualification gates, but is cross-session development evidence,
not PostgreSQL production admission or completion of the wider M1 global-
operator profile. The generated exact-bounded query profile therefore remains
planned. The CLI's retained-byte setting accounts exact textual binding payload;
it is not described as a total peak-heap ceiling.

## Context

The executor applies SPARQL ORDER after reconstructing RDF terms so source SQL
collation and affinity cannot become semantic authority. The old implementation
therefore buffered every ordered solution. Serving correctly rejected that
source-sized state, but it also rejected common finite top-K queries such as the
Product Mock `LIMIT 10001` demonstration.

A finite `OFFSET o LIMIT n` result depends only on the first `o + n` solutions
of a stable full sort. That permits exact bounded retention without pushing
SPARQL value ordering into a source or adding the external spill substrate
proposed by ADR-0040.

## Decision

### 1. Admit only a finite root variable-key window

The retained window is `K = offset.checked_add(limit)`.

- `LIMIT 0` has `K = 0` and returns before metadata, backend acquisition, cursor
  opening, row reconstruction, or retained-payload charging.
- Addition overflow, absent LIMIT, and every ordered nested subplan remain
  source-sized and reject before source I/O.
- Serving has an independent inclusive `max_order_rows` ceiling. The default is
  100,000; zero admits only `LIMIT 0`.
- Top-level ASK keeps its existing no-sort-buffer rule because ordering cannot
  change existence after the requested slice.

Only plain variable keys are admitted. The existing generic expression
evaluator is a raw/development surface and is not SPARQL-error exact. A finite
expression ORDER therefore has its own typed admission reason and returns 501
before source I/O. Raw expression keys use a NUL-prefixed internal name that the
SPARQL `VARNAME` grammar cannot express, preventing a user binding from being
overwritten or substituted when evaluation fails.

### 2. Compact by stable full-order prefixes

After each fixed reconstruction batch, append surviving solutions to the
retained buffer, perform a stable sort with the engine comparator, and discard
everything after `K`. The retained prefix is already in arrival-stable order;
new equal-key rows append after it. Repeating this operation is therefore equal
to taking the first `K` rows of one stable sort over the entire input.

The final pass sorts the retained buffer and then applies OFFSET/LIMIT. No SQL
ORDER or source collation participates. The fixed reconstruction batch remains
part of the ADR-0006 constant execution budget, so live row count is
`O(K + fixed_batch)`, never `O(source_cardinality)`.

The plain execution path keeps term reconstruction sequential whenever ORDER is
present. Repeated parallel batches distributed short-lived term allocations
across system-allocator arenas and failed the fresh-process RSS gate even though
requested live heap was bounded. Non-ORDER streaming and aggregate inner
collection retain their existing measured parallel path.

### 3. Use one deterministic total comparator

Rust sorting requires a total, transitive comparator. Literals use fixed,
non-overlapping value-domain ranks rather than a mixture of pairwise lexical and
value comparisons:

1. boolean;
2. numeric (`-INF`, finite arbitrary-precision decimal key, `+INF`, `NaN`);
3. dateTime/date/time normalized to a UTC representative;
4. duration through a deterministic linear extension at an XSD reference date;
5. lexical fallback by value, datatype, and language.

Terms retain the existing blank-node, IRI, literal, triple-term rank. UNBOUND is
first for ascending and last for descending. Multiple keys apply direction per
key. Stable arrival order resolves comparator ties.

The domain ranks deliberately extend SPARQL cases whose relative order is
undefined. They must never reverse a characterized defined comparison. Oracle
sequence equality is evidence only for defined comparisons; undefined
extensions require result-bag preservation and deterministic behavior without
borrowing the independent evaluator's implementation-defined sequence.

### 4. Separate row overhead from retained textual payload

`max_order_rows` bounds per-row/container overhead by plan shape. A fifth
`QueryCharge::RetainedBytes` dimension charges positive growth in the exact
textual binding-payload high-water mark: variable names and recursive RDF term
lexical/datatype/language/predicate bytes. Compaction may lower live payload but
does not refund the request's charged high-water mark.

Allocator metadata, vector capacity, parsed sort keys, indices, temporary
compaction slots, and the fixed reconstruction batch are excluded from that byte
counter. They are bounded by the row window and fixed batch and are measured by
heap/RSS evidence. A row can be allocated before its payload is charged, so the
setting is a fail-closed request budget, not a pre-allocation heap reservation.

## Rejected alternatives

- **Push ORDER/LIMIT into SQL:** source collation, affinity, null placement, and
  datatype coercion are not SPARQL value-order authority.
- **Unstable selection/heap:** equal-key arrival order at the `K` boundary must
  match the existing stable full-sort behavior.
- **One combined row/byte knob:** tiny numerous rows and one large lexical
  payload are independent risks.
- **Call payload bytes total memory:** allocator, container, parsed-key, and
  fixed-batch overhead are excluded.
- **Admit expression ORDER opportunistically:** one exact SPARQL evaluator and
  materialized oracle must cover its error and datatype semantics first.

## Evidence and acceptance

Required CI evidence covers checked window arithmetic, stable tie boundaries,
all reconstruction chunk sizes, mixed-domain comparator laws, exact row/payload
caps, typed expression rejection, internal-name hygiene, LIMIT-0 poison-source
behavior, file-backed 1×/10×/100× requested heap, and 50 fresh Linux workers per
scale for process-lifetime RSS. The controller creates exact 1,000/10,000/100,000
row SQLite sources before any worker starts. Each worker includes startup,
mapping/schema setup, execution and teardown in `VmHWM`, verifies all 80 ordered
rows, and runs with a measurement-only 64 KiB SQLite page cache. Exact doubled
median and nearest-rank p95 independently apply the checked 10% gate.

For this ORDER slice, ADR-0038 M1 qualification additionally required:

1. an independent materialized-oracle differential for every admitted value
   domain, direction, UNBOUND, multiple-key and slice boundary; and
2. an admitted live Product Mock serve-path result, not only a raw executor run.

Both items now pass their stated gates. Item 1 is required CI evidence; item 2
is a live-optional development KAT whose query is admitted by `sf-serve` but
which grants no backend admission. This closes the finite root ORDER slice, not
the complete M1 gate, whose other global operators remain open.

No readiness score or unrelated passing test offsets a failed item.

## Consequences

- Common finite variable-key ORDER queries can execute without source-sized
  engine memory.
- Unbounded, overflowed, nested, and expression ORDER continue to fail closed.
- The change is a single-source bounded precursor, not the federated external
  sort/merge implementation proposed by ADR-0040.
- The extra stable sort per reconstruction batch favors proof simplicity and
  deterministic behavior over asymptotically optimal selection machinery.

## More information

- Runtime and performance model: ADR-0006.
- Request governance: ADR-0010 and ADR-0052.
- Completion and M1 evidence gates: ADR-0038.
- Federated external operators: ADR-0040.
