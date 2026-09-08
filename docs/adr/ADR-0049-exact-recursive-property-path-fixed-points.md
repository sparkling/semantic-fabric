---
status: accepted
date: 2026-09-01
updated: 2026-09-08
tags: [sparql, property-paths, correctness, recursive-cte, resource-governance]
supersedes: []
depends-on: [ADR-0007, ADR-0010, ADR-0038]
implements: [ADR-0038]
---

# Exact recursive property-path fixed points

## Status boundary

This ADR is **accepted** by explicit maintainer direction on 2026-09-01. It
corrects ADR-0010 R3 where a numeric recursion-depth limit was allowed to end a
successful `P+` or `P*` query. It does not accept a partial answer at any depth.

This decision closes the known semantic truncation in the single-source
compiler. The serving lane now carries one absolute deadline plus finite limits
for observable probes, opens, pulls, semantic results, and serializer writes.
Raw/conformance APIs remain explicitly uncontrolled; total compiler/source work,
general decoded-key equality, cross-source closure and backend admission remain open.
Required native serving cleanup is recorded in ADR-0055, not superseded here.

**2026-09-08 exact query slice:** required owned-TLS CLI checks execute one-hop
DESCRIBE and `P+`/`P*` on PostgreSQL 16.15/MySQL 8.4.11: a duplicate-edge
cycle/diamond returns all 16 pairs; a 258-edge chain returns all 33,411/33,670
pairs, not a depth-limited prefix. Joined paths retain typed recipes until live
emission (ADR-0033); folded native identifiers resolve before cursor work.

**Text-comparison repair (2026-09-08):** SQLite NOCASE, PostgreSQL
nondeterministic ICU and MySQL case-insensitive PAD SPACE fixtures now preserve
case-distinct and trailing-space-distinct RDF nodes for `+`, `*`, `?` and
alternatives. SQLite comparison uses storage-preserving `COLLATE BINARY`;
native prepare-time varying-text facts authorize PostgreSQL `COLLATE "C"` or
MySQL text `CONVERT(... USING utf8mb4) COLLATE utf8mb4_0900_bin`. MySQL's
[NO PAD text collation](https://dev.mysql.com/doc/refman/8.4/en/charset-mysql.html)
preserves trailing spaces without changing the result decoder to binary/hex.
Numeric, date and unknown native types are not blanket-cast; CHAR is handled below.

The rejected blanket SQLite prototype lost CHAR/DATE metadata. The integrated
compiler instead emits a prepare-only, same-IR metadata twin omitting only its
own comparison decorations; projection and parameter vectors must match.
Authored SQL is untouched, including implicit expression names. Borrowing and
owned SQLite retain DATE types, CHAR padding, original bound parameters and
the existing admission/cancellation owner; only actual SQL executes. The earlier
guarded authored-collation metadata recovery remains separate and unchanged.

External key comparisons decorate both sides, retaining NULL/unbound guards.
Authenticated SQLite JOIN/OPTIONAL/EXISTS/NOT EXISTS/MINUS tests reject false
case-insensitive correlations. Native metadata propagates through standalone and
joined paths and SubPlans; aggregate SQL and metadata share one actual column
layout, including SQLite AVG's metadata operand. Twenty nested two-column plans
require 21 metadata visits, not width-exponential recursion. These are specific
tested profiles, not every native outer correlation or a total compiler-work proof.

**CHARACTER identity repair (2026-09-08):** required duplicate and transitive
connectivity regressions now pass in the default suite. SQLite normalizes each
leaf with the same Rust lexical/padding function used by row decoding. UTF-8
scalar counts include embedded NUL, overlong values are never truncated, and
the request's source-work allowance is charged before allocation. An explicit
compiler flag, not SQL substring matching, requests a query-local function;
case/arity collisions fail without replacing callbacks. Cursor cleanup removes
it; failed removal leaves only inert state with no retained request control.
The metadata twin retains normalization, preventing first-arm CHAR(4) metadata
from re-padding CHAR(2) outputs. Native BPCHAR uses
[its padding-preserving send value](https://raw.githubusercontent.com/postgres/postgres/REL_16_STABLE/src/backend/utils/adt/varchar.c)
through `convert_from(bpcharsend(...), 'UTF8')`; extra parentheses preserve the
qualified function through the SQL parser before COLLATE. MySQL nonbinary STRING
uses the existing session-sensitive native value with UTF8/NO PAD comparison,
without inferred width or added trimming/padding. Required owned TLS tests cover
4/2, 2/4 and 4/4 CHAR widths for +/*/?/alternative on PostgreSQL16.15/MySQL8.4.11.
SQLite also covers CHAR/VARCHAR, Unicode/NUL and authenticated outer correlations.

**Remaining exactness gate:** general mixed/natural-type key identity, raw
mixed-decoder SubPlan UNION, ordinary early-wrapper native folding and ordinary
source-collation behavior remain open. ADR-0015 now closes the ordinary NULL-term
absence defect with atom-local guards and public/native evidence. Native CHAR path expectations
use an independent decoded graph, not a case-collapsing ordinary MySQL DISTINCT
result. Total source work, synthetic row identity and exact-release qualification
remain blockers. No broad semantic-exact, bounded or admission flag is promoted.

## Context

**Compiler-side update (2026-09-08):** predicate-hop mapping searches and negated
complement expansion now use request-owned prospective work and exact term/source
copy reservations. Public rejection precedes source admission and sufficient work
preserves exact NPS multiplicity; fixed-point/oracle regression checks pass.
Graph inventory/reflexive eligibility now shares those prospective controls.
Source-side recursive iterations and other compiler phases remain open. ADR-0055 controls current v1 release scope;
the historical ADR-0038 links below do not add deferred research prerequisites.

The previous recursive CTE keyed rows by `(sf_s, sf_o, sf_d)`, where `sf_d` was
the walk depth. `UNION` could not collapse a cyclic revisit at a different depth,
so recursion stopped only at `sf_d < 256`. A path longer than 256 edges returned
a normal but incomplete result.

A numeric depth limit cannot simultaneously guarantee exact SPARQL reachability
and authorize successful completion. Once the limit is reached, the engine must
either prove that the fixed point was already complete or abort semantic
completion. An observable prefix in a failed post-`200` transport is never a
valid successful answer.

For a finite relational hop relation, the reachable `(subject, object)` pair set
is finite. Its transitive closure can therefore terminate by eliminating pairs
already present in the recursive relation. This preserves the existing
source-native execution strategy and avoids an in-process graph fallback.

## Decision

### 1. Compute a finite-pair fixed point

For an evidenced recursive-path compiler target, `P+` and `P*` use a recursive
CTE whose identity is exactly `(sf_s, sf_o)`:

- the anchor is the distinct one-hop relation;
- `P*` also anchors the soundly enumerable reflexive pairs;
- the recursive member joins the current object to the next hop subject; and
- `UNION`, never `UNION ALL`, eliminates pairs already reached.

Recursion terminates when an iteration adds no new pair. Walk depth is not a CTE
column and is not part of row identity. The outer answer retains SPARQL
property-path set semantics.

### 2. Reject unproved dialects before SQL emission

The compiler may emit this form only for SQLite, PostgreSQL, and MySQL, whose
recursive `UNION DISTINCT` form is evidenced. This is a compiler capability
boundary, not production backend admission.

Every other dialect returns typed `Unsupported` during translation for `P+` and
`P*`. Non-recursive path forms keep their existing per-shape support. A dialect
joins the recursive set only after syntax, cycle, long-chain, differential,
resource, and live-provider evidence passes.

### 3. Separate semantic completion from resource completion

Pair collapse is cycle detection and bounds distinct recursive state by the
finite reachable pair set, at worst quadratic in the reachable node count. It is
not a constant-memory or total-resource proof.

Resource controls may impose cost, work, time, row, byte, spill, or admission
limits only if exceeding one aborts semantic completion. The engine may never
label the accumulated path prefix complete. Current HTTP streaming is not
atomic: after `200`, already-sent bytes can remain observable when the body
terminates. That is a failed transport, not a successful semantic answer;
atomic no-prefix delivery remains open under ADR-0010/0011. Production admission
also requires extending the serving budget to compiler CPU, database rows,
recursive iterations and source cost, plus source-native statement controls,
cancellation, and backend-specific load evidence.

### 4. Keep the evidence adversarial

Required regression evidence includes:

- chains at, below, and beyond the former 256-edge boundary;
- cyclic and diamond graphs with exact pair counts and no duplicates;
- `P+` and `P*` comparison against an independent semantic oracle;
- exhaustive typed rejection over every unproved dialect; and
- live execution for each backend before that backend is production-admitted.

Commit `5c379f6` implements the pair fixed point and rejection boundary. SQLite
executes the hostile suite, and a local PostgreSQL run produced all 33,411 pairs
for a 258-edge chain plus all nine pairs for a three-node cycle. The required
native CLI command now executes the pinned PostgreSQL/MySQL profile:
`cargo test --locked -p sf-cli --no-default-features --test source_tls_live query_profile::native_describe_and_recursive_paths_are_exact -- --ignored --exact --nocapture`.
This qualifies the described native text profile, not all typed-key equality or admission.

## Consequences

- **Positive:** no supported recursive property path can return the former
  successful 256-hop prefix.
- **Positive:** cycles terminate through semantic pair identity rather than an
  arbitrary walk counter.
- **Positive:** unproved dialects fail before touching a source.
- **Cost:** the source engine may materialize up to a quadratic reachable-pair
  relation and therefore still requires cost, timeout, and cancellation gates.
- **Cost:** dialects previously receiving generic but unproved SQL now receive a
  correct 501 until independently qualified.

## Alternatives rejected

- **Keep the depth cap and document truncation** — contradicts SPARQL semantics
  and ADR-0038's exact-or-explicit rule.
- **Return a partial result with a warning** — response metadata cannot turn an
  incomplete graph into a successful query answer.
- **Remove recursion limits without pair collapse** — cycles may not terminate.
- **Evaluate the graph in Rust** — creates source-sized in-process state and
  bypasses the relational execution boundary.
- **Assume one recursive syntax for every dialect** — emits invalid or unproved
  SQL and hides capability differences.

## Rules

- **R1** — successful `P+`/`P*` results are exact finite-pair fixed points.
- **R2** — walk depth never participates in recursive row identity.
- **R3** — a resource-limit breach never marks a path prefix semantically
  complete; post-`200` transport atomicity remains an explicit nonclaim.
- **R4** — unproved recursive dialects reject before SQL emission or source I/O.
- **R5** — compiler support never implies backend production admission.
- **R6** — native key equality, total resource governance and exact-release qualification remain explicit open gates.

## Links

[ADR-0007](ADR-0007-sparql-to-sql-rewriting-strategy.md),
[ADR-0008](ADR-0008-reasoning-strategy.md),
[ADR-0010](ADR-0010-security-and-resource-governance.md), and
[ADR-0038](ADR-0038-sota-application-completion-programme.md); see also the
[MySQL 8.4 recursive CTE specification](https://dev.mysql.com/doc/refman/8.4/en/with.html).
