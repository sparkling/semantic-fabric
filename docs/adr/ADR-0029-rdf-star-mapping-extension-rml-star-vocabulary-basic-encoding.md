---
status: accepted
date: 2026-07-16
updated: 2026-07-18
tags: [rdf-star, r2rml, rml-star, mapping-model, obda, virtualization]
supersedes: []
depends-on:
  - ADR-0002
  - ADR-0004
  - ADR-0007
  - ADR-0028
implements: []
---

# RDF-star mapping support: reuse RML-STAR vocabulary, compile it to the plain-RDF basic encoding

## Implementation status (2026-07-18)

**Accepted and implemented in `sf-mapping`.** The mapping-compiler half (§A/§B/§D)
ships: `rml:starMap` in **both subject and object position** desugars, parser-side,
into the existing R2RML IR (a synthetic-id `TermMap::Template` + the four
`rdf:PropositionForm` basic-encoding POMs) — no new `sf-core` IR variant, no
executor/SQL change (§B, decision 1). The synthetic id is a deterministic,
percent-encoding-injective `urn:sf-star:` template (R2; hashing deferred per the
Neutral clause). `rml:AssertedTriplesMap`/`NonAsserted` suppression, and the four
load-time rejections (R3 nested, R4 predicate-position, cross-source v1, non-single-spo)
are all implemented and tested. Code: `crates/sf-mapping/src/r2rml.rs` +
`crates/sf-mapping/src/r2rml/star.rs`; 12 new unit tests (`r2rml/tests.rs`),
`cargo test -p sf-mapping` green, `differential_tree` 166/0 (no parity regression).
**Still out of scope (unchanged):** the query-side SPARQL-star rewrite (§C, tracked
as the unwritten ADR-0031) — this ADR alone does not make RDF-star queries work
end-to-end.

## Superseded in part (2026-07-18, same day — ADR-0032)

`ADR-0032` (accepted, implemented) supersedes this ADR's **§B** compiled shape
and narrows two rules: the single synthetic identifier is replaced by
**role-split id families** (proposition `urn:sf-star:pf:` — still the pure
function of the quoted shape this ADR's R2 demands — plus per-declaration
reifier `urn:sf-star:r:`), subject-position star maps now additionally emit the
explicit `rdf:reifies` triple with the description on a standalone deduped map,
**object-side nesting is supported** (recursive template composition — no SQL
views needed, contrary to §D's assumption; subject-side stays rejected, now
citing RDF 1.2 Concepts §3.1), and **cross-source quoted maps are supported**
via `rr:joinCondition` on the star map. The vocabulary-reuse decision, the
basic-encoding target (as the internal wire format), R2's determinism-by-
construction invariant, and R4 all stand. Read `ADR-0032` D1 for the current
emission.

## Context and Problem Statement

`ADR-0028` is **accepted**. Its backlog item **#17** ("RDF-star via plain-RDF
encoding") is explicitly scoped there as *research-stage, not a scoped
feature*. This ADR is that item's first real design step — graduating it from
research into a concrete, reviewable proposal. **No code has been written for
this yet; status is `proposed`, not `accepted`.**

The problem: allow an R2RML/RML mapping to declare that a `TriplesMap`'s
subject or object represents a **quoted (RDF-star) triple**, so a SPARQL-star
query containing `<<?s ?p ?o>> ?p2 ?o2` can be answered against relational
data — without ever materializing (`ADR-0001`/`ADR-0002`'s core architectural
commitment) and without requiring Oxigraph-side native triple-term storage,
since `ADR-0028 §G` confirmed semantic-fabric's SQL rewrite has no atomic
triple-term value to scan against the way Oxigraph's own storage does.

This ADR covers **only the mapping-compiler half** of that problem: what a
mapping author writes, and what SQL-projectable shape it compiles to. The
query-side SPARQL-star algebra rewrite pass that reconstructs quoted triples
from that shape at query time is real, necessary, unscoped follow-on work,
tracked separately (see §C, Consequences).

## Decision Drivers

* **Reuse RML-STAR's actual vocabulary** (`rml:StarMap`, `rml:quotedTriplesMap`,
  `rml:AssertedTriplesMap`/`rml:NonAssertedTriplesMap`) rather than invent new
  terms — an explicit choice made this session, knowingly accepting that this
  repo's compiled semantics diverge from the spec's own target semantics (see
  Consequences).
* **Target the W3C RDF 1.2 Interoperability "basic encoding"** (a blank-node/IRI
  stand-in plus `rdf:PropositionForm`/`propositionFormSubject`/`Predicate`/
  `Object`) as the actual compiled/projected shape — per `ADR-0028 §E`, the
  only encoding confirmed reconstructable by a query-time rewrite without
  native triple-term storage.
* **No prior art anywhere** for this combination (`ADR-0028 §G` — checked 7
  OBDA/virtualization engines plus the 2023-2026 literature, found nothing).
  Scope conservatively: **single-level (non-nested) quoted triples only for
  v1**.
* **Determinism is non-negotiable.** Repeated query execution over the same
  underlying rows must yield the same synthetic identifier for the same
  quoted triple — Jena's documented one-reification-per-quoted-triple
  invariant (`ADR-0028 §E`). Leaving this to mapping-author discipline (the
  vanilla hand-templated sketch `ADR-0028 §E` originally floated) risks a
  whole class of silently-wrong-on-repeat-query bugs; this design closes that
  risk structurally (§B.1 below).
* **`sf-mapping` currently parses R2RML only** — confirmed directly
  (`crates/sf-mapping/src/lib.rs` exposes only `r2rml::parse_r2rml`; no `rml:`
  namespace term is recognized anywhere in the crate today). This is not
  "add one more R2RML feature" — it is the **first RML-namespace term** this
  processor will ever need to parse. Scope that cost explicitly (Consequences).

## Considered Options

* **(a) Invent semantic-fabric-specific vocabulary** (e.g. `sf:StarMap`) —
  rejected per explicit user direction this session: familiarity to anyone who
  already knows RML-STAR was preferred over avoiding the semantic-mismatch
  risk that reuse carries.
* **(b) Reuse RML-STAR's vocabulary, redefine its compiled target as the
  basic encoding rather than native RDF-star assertion — chosen.**
* **(c) Wait for RML-STAR to reach Working-Group/Recommendation status with
  settled reference semantics** — rejected: `ADR-0028` confirms RML-STAR is
  Draft-CG-status with **zero working implementations anywhere** in the RMLio
  ecosystem (`rmlmapper-java`: none; `MappingWeaver-java`: all 16 positive
  JUnit cases `@Disabled`). Blocking on it has no visible timeline, and even a
  finalized spec would still target native triple-term assertion, not the
  basic-encoding shape this repo's architecture actually needs.

## Decision Outcome

Adopt RML-STAR's mapping-author-facing vocabulary verbatim, but define
semantic-fabric's **own compiled semantics** for it: an `rml:StarMap` compiles
to the RDF 1.2 Interoperability basic encoding, projected as SQL rows — never
to a native RDF-star/RDF-1.2 triple term. **This is an explicit, documented
semantic divergence from the RML-STAR spec's own target semantics**, and must
be stated prominently in mapping-authoring docs so a reader who knows real
RML-STAR is not misled about what querying the result actually returns.

### A. Mapping-author-facing syntax (unchanged from RML-STAR)

```turtle
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<PersonAge>
    a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<PersonAgeAssertion>
    a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [
        rml:starMap [
            rml:quotedTriplesMap <PersonAge> ;
            rml:nonAssertedTriplesMap <PersonAge> ;
        ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:CensusRecord2026 ]
    ] .
```

This is exactly RML-STAR's real shape: a `StarMap` nested in a Subject/Object
map, referencing the `TriplesMap` whose (subject, predicate, object) becomes
the quoted triple. A mapping author familiar with real RML-STAR writes this
identically to how they would for `rmlmapper-java` or `MappingWeaver-java`
(were either to actually implement it).

### B. Compiler-side expansion (semantic-fabric-specific — the actual new work)

When `sf-mapping`'s R2RML/RML processor encounters an `rml:StarMap` at a
Subject/ObjectMap position, it expands it to:

1. **Synthetic identifier — compiler-derived, never mapping-author-supplied.**
   A blank node/IRI built by concatenating, in a fixed documented order, the
   `quotedTriplesMap`'s own subject-template, predicate value, and
   object-template/column expressions, then applying a stable string hash
   (e.g. FNV-1a over the concatenated column-reference expressions — reuse
   whatever hashing this crate already uses for blank-node identity if one
   exists; introduce one fixed function if not) as the sole variable binding
   of a `rr:template`-shaped identifier. There is no user-supplied template to
   get wrong here — the one-reification-per-quoted-triple invariant is
   **structurally guaranteed by construction**, not documented-and-hoped-for.
2. **Four basic-encoding triples**, sourced from `quotedTriplesMap`'s own
   subject/predicate/object expressions, each with the synthetic identifier
   as *their* subject:
   * `(id rdf:type rdf:PropositionForm)`
   * `(id rdf:propositionFormSubject <quotedTriplesMap's subject>)`
   * `(id rdf:propositionFormPredicate <quotedTriplesMap's predicate>)`
   * `(id rdf:propositionFormObject <quotedTriplesMap's object>)`
3. **`rml:AssertedTriplesMap` vs `rml:NonAssertedTriplesMap`** maps directly
   onto whether `quotedTriplesMap` is *also* compiled as an ordinary,
   independent `TriplesMap` (its own subject/predicate/object emitted as
   plain triples, matchable by `?s ?p ?o` outside any `<<>>`) —
   Asserted = yes, NonAsserted = no. A straightforward compile-time branch,
   no new machinery.
4. The synthetic identifier from step 1 is what is actually substituted into
   the outer `TriplesMap`'s subject/object position — in the example above,
   `<PersonAgeAssertion>`'s subject, in the SQL projection, **is** the
   synthetic identifier's value.

### C. Query-side reconstruction — explicitly out of scope for this ADR

The SPARQL-star algebra rewrite pass that recognizes `<<?s ?p ?o>>` triple
patterns and rewrites them into a SQL join against the basic-encoding rows
this compiler emits is real, necessary follow-on work. `ADR-0028 §G` found no
Oxigraph-internal analog to reuse for it (Oxigraph's own evaluator relies on
an atomic stored triple-term a SQL backend doesn't have) and no prior art
anywhere for this combination. **Tracked as a separate design item, not
resolved here.**

### D. Scope boundary (v1)

* **Single-level (non-nested) `StarMap` only.** A `quotedTriplesMap` whose own
  subject/object contains another `rml:StarMap` is **rejected at mapping-load
  time with a clear error**, never silently mishandled. Nesting needs SQL-view
  precomputation or RML-FNML functions (`ADR-0028 §E`) and is explicitly
  deferred, matching Morph-KGC^star's recursive algorithm being the one place
  this is solved today — for materialization, not virtualization
  (`ADR-0028 §F`).
* **No `rml:StarMap` in predicate position.** RML-STAR itself restricts this —
  it was one of `MappingWeaver-java`'s own negative/should-error test cases
  (`ADR-0028 §E`) — semantic-fabric rejects it identically, for
  spec-consistency, at mapping-load time.

## Consequences

### Good

* Zero changes needed to Oxigraph, `sf-sparql`'s existing algebra
  representation for ordinary (non-star) triples, or the executor's SQL
  generation core for anything *other than* `StarMap`-bearing `TriplesMap`s —
  additive, isolated to the mapping compiler (`sf-mapping`).
* Determinism is compiler-guaranteed, closing an entire bug class (silently
  wrong results on repeated identical queries) the original hand-templated
  sketch in `ADR-0028 §E` would have left to mapping-author discipline.
* Familiar syntax for anyone who already knows RML-STAR — the explicit
  trade-off made this session.

### Bad / risk

* **Real semantic divergence from the RML-STAR spec — must be documented
  loudly, everywhere this vocabulary is mentioned.** A mapping author (or a
  mapping-portability tool) who assumes `rml:StarMap` means "assert a native
  RDF-star/RDF-1.2 triple term, portable to any RML-STAR-conformant
  processor" will be wrong: semantic-fabric's compiled output is basic-encoding
  plain RDF, not a native triple term, and is **not** portable without
  translation to, say, a future working `MappingWeaver-java`.
* **This is the first RML-namespace term `sf-mapping` will ever parse.**
  Today it recognizes R2RML (`rr:`) exclusively — introducing `rml:` support
  at all, even scoped to just this one construct, is new parser surface, not
  a drop-in extension of existing R2RML term-map handling.
* The query-side rewrite pass (§C) is real, unscoped, follow-on work with no
  prior art anywhere (`ADR-0028 §G`) — **this ADR alone does not make
  RDF-star queries work end-to-end.** Accepting this design is not the same
  as shipping working RDF-star query support.
* Nested quoted triples remain entirely unsupported; a real use case needing
  them is not served by this design without further, separate work.

### Neutral

* The hashing function chosen for synthetic-identifier derivation (§B.1) is an
  implementation detail this ADR deliberately leaves unpinned to a specific
  algorithm — any stable, collision-resistant-enough string hash satisfies the
  invariant; picking one is an implementation-time decision, not an
  architectural one.

## More Information

* Depends on and extends: `ADR-0028 §E`/`§F`/`§G` — all the research this
  design is built from (RML-STAR's real mechanism, the W3C basic-encoding
  spec, the no-prior-art findings across 7 OBDA engines, and Oxigraph's
  internals confirming no reusable execution-side analog exists).
* Real spec sources: RML-STAR Draft CG Report (`kg-construct/rml-star`,
  release `v0.1.0`, 2023-05-10); W3C RDF 1.2 Interoperability ("basic
  encoding"/"basic decoding"); W3C RDF 1.2 Concepts §1.5 ("Triple Terms and
  Reification").
* Explicitly deferred, tracked separately: the SPARQL-star query-rewrite pass
  (§C above); nested `StarMap` support (§D); pinning a specific hash function
  (Consequences, Neutral).
* `ADR-0028`'s backlog item #17 should be updated to point here once this
  design is reviewed — see the amendment made to `ADR-0028` alongside this
  ADR's creation.
* **Status: `proposed`.** No code has been written against this design. It
  requires review/acceptance before `sf-mapping` work begins.

## Rules

* **R1** — `rml:StarMap` compiles to the RDF 1.2 Interoperability basic
  encoding (blank-node/IRI stand-in + 4 linking triples), never to a native
  RDF-star/RDF-1.2 triple term.
* **R2** — the synthetic identifier is always compiler-derived from
  `quotedTriplesMap`'s own subject/predicate/object expressions, never
  mapping-author-supplied, so the one-reification-per-quoted-triple invariant
  is structurally guaranteed.
* **R3** — a nested `rml:StarMap` (a `quotedTriplesMap` whose own
  subject/object itself contains another `StarMap`) is rejected at
  mapping-load time with a clear error, never silently mishandled.
* **R4** — `rml:StarMap` in predicate position is rejected at mapping-load
  time, matching RML-STAR's own restriction.
