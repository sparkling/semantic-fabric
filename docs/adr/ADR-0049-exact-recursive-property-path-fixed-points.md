---
status: accepted
date: 2026-09-01
updated: 2026-09-01
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
compiler. It does not complete `QueryBudget`, cost admission, source-native
timeouts, cancellation, result limits, cross-source closure, production backend
admission, or live MySQL qualification.

## Context

The previous recursive CTE keyed rows by `(sf_s, sf_o, sf_d)`, where `sf_d` was
the walk depth. `UNION` could not collapse a cyclic revisit at a different depth,
so recursion stopped only at `sf_d < 256`. A path longer than 256 edges returned
a normal but incomplete result.

A numeric depth limit cannot simultaneously guarantee exact SPARQL reachability
and authorize successful completion. Once the limit is reached, the engine must
either prove that the fixed point was already complete or fail the whole query.
Returning the accumulated prefix is never valid.

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
limits only if exceeding one fails the entire query. They may never convert the
current prefix into success. Production admission remains blocked until one
absolute ingress-to-serialization budget, source-native statement controls,
cancellation, and backend-specific load evidence govern this CTE.

### 4. Keep the evidence adversarial

Required regression evidence includes:

- chains at, below, and beyond the former 256-edge boundary;
- cyclic and diamond graphs with exact pair counts and no duplicates;
- `P+` and `P*` comparison against an independent semantic oracle;
- exhaustive typed rejection over every unproved dialect; and
- live execution for each backend before that backend is production-admitted.

Commit `5c379f6` implements the pair fixed point and rejection boundary. SQLite
executes the hostile suite, and a local PostgreSQL run produced all 33,411 pairs
for a 258-edge chain plus all nine pairs for a three-node cycle. MySQL syntax is
supported by its normative engine documentation, but live MySQL execution is
still open and no admission follows from SQL-string inspection.

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
- **R3** — every resource-limit breach fails the whole query; no prefix succeeds.
- **R4** — unproved recursive dialects reject before SQL emission or source I/O.
- **R5** — compiler support never implies backend production admission.
- **R6** — live MySQL and total resource governance remain explicit open gates.

## Links

[ADR-0007](ADR-0007-sparql-to-sql-rewriting-strategy.md),
[ADR-0008](ADR-0008-reasoning-strategy.md),
[ADR-0010](ADR-0010-security-and-resource-governance.md), and
[ADR-0038](ADR-0038-sota-application-completion-programme.md); see also the
[MySQL 8.4 recursive CTE specification](https://dev.mysql.com/doc/refman/8.4/en/with.html).
