---
status: proposed
date: 2026-07-18
tags: [rdf-star, rdf-1.2, sparql-1.2, soundness, completeness, native-reification, query-rewrite, mapping-model]
supersedes: []
depends-on:
  - ADR-0007
  - ADR-0028
  - ADR-0029
  - ADR-0031
implements: []
---

# RDF 1.2 soundness and completeness: native reification at every visible surface, encoding only under SQL

## Context and Problem Statement

`ADR-0029`/`ADR-0031` (both accepted, implemented) shipped RDF-star v1: `rml:StarMap`
mappings and SPARQL-star patterns compiled onto a proposition-form encoding. v1
deliberately carried documented boundaries (nesting, cross-source, VALUES, the five
triple-term functions, CONSTRUCT templates, native triple terms in results) and one
loudly-documented divergence (clients see the encoding, never native triple terms).

The mandate for this ADR: **make the extension sound and complete for RDF 1.2 as
actually specified**, and dissolve the native-term divergence. Both halves were
grounded first — the current W3C texts fetched and quoted (spec-scout), and every
implementation seam verified against the real dependency sources (cap-scout).

### Specification status (weight the citations accordingly)

| Document | Status (fetched 2026-07-18) |
|---|---|
| RDF 1.2 Concepts | **Candidate Recommendation Snapshot** (2026-04-07) — normative core |
| RDF 1.2 Semantics | **Candidate Recommendation Snapshot** (2026-04-07) |
| SPARQL 1.2 Query | Working Draft (2026-06-25), Rec track |
| RDF 1.2 Turtle | Working Draft (2026-06-12), Rec track |
| RDF 1.2 Interoperability ("basic encoding") | **Group Note DRAFT, note track, non-normative** — its basic-*decode* algorithm is literally the stub "Issue 2: Write this algorithm" |

The substrate is already 1.2-native end to end: `oxrdf 0.3.3` (`rdf-12`) has
`Term::Triple`; `spargebra 0.4.6` (`sparql-12`) parses every 1.2 surface form;
`sparesults 0.3.3` serializes triple-term bindings in JSON/XML/CSV; `oxttl 0.2.3`
serializes nested triple terms; `spareval 0.2.6` in the conformance oracle is a
complete SPARQL-star evaluator (functions, CONSTRUCT templates, dataset matching —
verified in its source, not assumed). **The only place a triple term cannot exist
is a SQL row.**

### What the specs actually say (the findings that force corrections)

1. **Triple terms are object-position-only** (Concepts §3.1, normative): a triple's
   subject is an IRI or blank node; its predicate an IRI; only the *object* may be a
   triple term. Cycles are impossible by construction. §7's Symmetric/Generalized
   RDF (triple terms in other positions) is explicitly non-normative opt-in.
2. **Reifier ≠ proposition.** Concepts §1.5: a *reifying triple* is
   `reifier rdf:reifies <triple term>`; reification does **not** assert the inner
   triple; "There can be multiple, distinct reifiers related to the same abstract
   proposition," and one reifier may reify several propositions. The interop note's
   encoding keeps this split: the reifier and the minted `rdf:PropositionForm` node
   are distinct, linked by an explicit `rdf:reifies` triple ("The blank nodes
   generated to replace triple terms should not be confused with the reifiers").
3. **Co-identity is semantic, not just conventional.** Semantics §5 interprets a
   ground triple term via an **injective** mapping `IT(s,p,o)` — the same
   components always denote the same proposition. Triple terms are **transparent**
   (Concepts §1.5): components keep their ordinary denotation.
4. **Sugar assertion table** (Turtle 1.2 / SPARQL 1.2 shared grammar): bare
   `<< s p o >>` and explicit `<< s p o ~ r >>` do **not** assert `s p o` — they
   produce only the reifying triple; annotation syntax `s p o {| … |}` asserts
   **and** reifies. And crucially: `<< … >>` (a **ReifiedTriple**) evaluates to the
   *reifier*, while `<<( … )>>` (a **TripleTerm**) *is* the triple term — two
   different node kinds wherever they appear.
5. **Subject-position triple-term patterns never match** (SPARQL §18.1.3, verbatim):
   "A triple pattern that has another triple pattern in its subject position will
   fail to match on any RDF graph." Legal to write; guaranteed empty.
6. **Functions** (§17.4.6): `TRIPLE(s,p,o)` errors unless the components form a
   legal triple; `SUBJECT/PREDICATE/OBJECT` error on non-triple-terms; `isTRIPLE`
   never errors. **Equality**: `sameTerm` is component-wise syntactic; `=`
   (renamed `sameValue`) is *recursive value* equality that propagates component
   errors. **ORDER BY** (§15.1): triple terms are the highest category; ordering
   *between* two triple terms is deliberately left undefined.
7. **CONSTRUCT** (§16.2): an instantiation producing an illegal RDF construct is
   silently **dropped from the output graph**, not an error.
8. **Results formats**: JSON `{"type":"triple","value":{subject,predicate,object}}`
   recursively; XML `<triple><subject>…` — reifiers are ordinary IRI/bnode bindings.

### Where v1 is wrong or incomplete against this

| v1 behavior | Verdict |
|---|---|
| Synthetic id doubles as reifier **and** proposition; no `rdf:reifies` triple emitted | **Unsound vs the native model**: kills reifier multiplicity, creates the observed 2-carrier ambiguity, conflates two node kinds the spec distinguishes |
| Bare `<< … >>` and `<<( … )>>` treated identically by the rewrite | **Unsound**: one denotes the reifier, the other the triple term |
| Subject-position `<<( … )>>` patterns matched against the encoding | **Unsound**: spec-guaranteed empty (finding 5) |
| All nesting rejected (R3) | Object-side rejection is a v1 scope choice the spec does not license; **subject-side is impossible in the data model itself** (finding 1; also structurally unbuildable — oxrdf `Triple.subject: NamedOrBlankNode` has no triple arm) |
| Predicate-position rejection (R4) | **Conformant** — predicates are IRIs, full stop. Not a gap; reclassified |
| VALUES / functions / CONSTRUCT / native results 501s | **Incomplete** — all implementable (below) |
| Cross-source rejection | **Incomplete** — the join engine already joins across sources for ordinary `rr:parentTriplesMap`; only star code blocks it |
| Path-endpoint 501 | **Not a star gap**: *any* `GraphPattern::Path` joined with anything 501s today, even a plain 2-hop sequence in a non-star query (verified in `unfold::merge`) — star inherits a general engine boundary and adds nothing |

## Decision Outcome

### D0 — The semantic frame (everything else hangs on this)

The engine's **virtual graph is a native RDF 1.2 graph** (triple terms in object
position, reifiers, `rdf:reifies`). The relational layer stores that graph's
**proposition-form encoding** — a wire format for SQL, modeled on the interop
note's basic encoding with two documented, virtualization-necessary divergences:
deterministic IRIs in place of fresh blank nodes, and hybrid visibility (the
encoding triples remain matchable; the note itself contemplates hybrid graphs and
is non-normative WIP). **Soundness and completeness are defined as answer
equivalence**: for every supported query, the engine's answers over the encoding
equal the answers of a native SPARQL 1.2 evaluator over the decoded graph. This is
operationally enforced: the conformance suite gains a **decoder** and runs the
**original** SPARQL-star query in `spareval` (verified fully star-native) over the
decoded materialization, differentially against the engine.

### D1 — Role-split identifiers (the core correction)

Two distinct deterministic id families replace v1's single id:

* **Proposition id** `urn:sf-star:pf:<shape-slug>|{cols…}` — stands in for the
  triple term itself. Pure function of the quoted (s,p,o) shape and row, shared by
  every construct quoting the same shape — realizing Semantics §5's injective `IT`
  (co-identity) structurally.
* **Reifier id** `urn:sf-star:r:<outer-map-slug>|{cols…}` — one per star-map
  declaration per row. Distinct star maps quoting the same shape yield distinct
  reifiers of the same proposition — realizing the spec's reifier multiplicity.

**Mapping emission** (supersedes ADR-0029 §B in place):

* Subject-position star map: outer subject = **reifier id**; one injected POM
  `rid rdf:reifies pfid`; the 4 proposition-description triples
  (`pfid a rdf:PropositionForm; …FormSubject/Predicate/Object`) emitted as a
  standalone synthetic triples map. Author annotations ride the reifier — exactly
  the native annotation shape.
* Object-position star map: object = **proposition id** + the standalone
  description map (v1's shape, unchanged — it was already the triple-term-stand-in
  reading).
* An optional `rml:reifierMap` (documented extension property) lets authors
  override the reifier term map when they need author-controlled reifier IRIs;
  default remains the deterministic per-map reifier.
* Decode (D2) consumes description triples and substitutes: `pfid` in object
  positions → the native triple term; `rid rdf:reifies pfid` →
  `rid rdf:reifies <<( s p o )>>`; annotations stay on `rid`.

### D2 — Native triple terms at every visible surface (basic decoding)

* New `TermDef::ComposedTriple { s, p, o: Box<TermDef> }` realized in the shared
  `exec_core::build_term` via `oxrdf::Triple::from_terms` — deliberately
  **bypassing** `sf_core::term::generate` (its `GenTerm` cannot and must not grow a
  triple arm; ADR-0006 zero-alloc design, and it explicitly rejects triple
  constants). Serializers need zero work (verified present for JSON/XML/CSV and
  Turtle; N-Triples and JSON-LD writers to be verified in-wave).
* CONSTRUCT instantiation gains the recursive `TermPattern::Triple` arm
  (`Triple::from_terms` naturally rejects illegal positions → the §16.2
  silent-drop behavior falls out). The ADR-0031 rule-9 501 guards are **removed**:
  they were the right call against silent data loss in v1, and are superseded by
  actual production of legal output + spec-defined dropping of illegal output.
* A conformance-side decoder (`sf-conformance`) turns an encoding materialization
  into the native graph for the end-to-end oracle.

### D3 — The query rewrite, corrected (supersedes ADR-0031 rules R2/R5 in place)

The rewrite maintains a **triple-term variable environment** (var → composed
(s,p,o) component vars — the correspondence v1 minted but never recorded):

* `X rdf:reifies TT-pattern` (all bare/explicit-reifier/annotation sugar
  desugars here — parser-verified): **no elision**. Rewrites to
  `X rdf:reifies ?pf` + the 4 description patterns on `?pf`. Matches only
  genuinely reified statements — bare-sugar queries stop matching unreified
  object-position triple terms (a v1 unsoundness, fixed).
* `?x <pred> <<( … )>>` (TripleTerm in object position): `?x <pred> ?pf` + the 4
  description patterns — matches triple-term objects (v1 shape, kept).
* **Matching matrix (the user-facing law, tested per cell):** `<< … >>` asks
  about *reifiers*; `<<( … )>>` asks about *triple terms*. The two are never
  interchangeable.
* Subject-position `<<( … )>>` / `<< … >>`-as-TripleTerm-subject: rewritten to a
  **statically empty** pattern (SPARQL §18.1.3), never an error, never a match.
* Object-side **nesting**: recursive, bottom-up — each level mints its own
  proposition id/description (mirrors the interop note's recursive encode).
  Subject-side nesting in ground/VALUES form is already a parser-level error
  upstream; in pattern form it falls under the statically-empty rule.
* **VALUES** with ground triple terms: decomposed — rows become component-tuple
  VALUES joined against description patterns where the variable is matched;
  where only projected, the ground native term binds directly. Nested ground
  terms decompose recursively.
* **Functions**, by engine-totality (relational data can never contain a native
  triple term, so *every* triple-term value is engine-synthesized and statically
  known to the rewrite): `SUBJECT/PREDICATE/OBJECT(?t)` on a composed var →
  the component var; on any other value → the §17.4.6 error (row-eliminating in
  FILTER, unbound in BIND). `isTRIPLE` → constant true/false. `TRIPLE(e1,e2,e3)`
  → a composed binding with §17.4.6's legality errors enforced at evaluation.
  The post-SQL evaluator's silent-`None` discipline must be audited in-wave: an
  unsupported-function `None` and a spec "error" must not conflate silently.
* **Equality/order**: `sameTerm` on composed terms → component-wise syntactic
  conjunction; `=` → recursive component `sameValue` with error propagation
  (§17.4.2.2); ORDER BY places composed terms in the highest category; order
  among triple terms is spec-undefined — the engine's deterministic
  serialization order is a permissible choice, documented.

### D4 — Cross-source star maps (completeness via existing machinery)

The quoted triples map may live on a different logical source. The description
map compiles on the **quoted** source (its terms need only that source's rows);
the outer reference (`rdf:reifies` object in subject-position form, the direct
object in object-position form) compiles as a **referencing object map** to the
standalone description map with `rr:joinCondition`s declared inside the star map
node — reusing, verbatim, the `RefObjectMap` join machinery that is already
cross-source-capable for ordinary mappings (verified: `unfold`'s Ref arm joins
two independent scans with no source-equality constraint). Because the crossing
reference is always an *object*, this stays inside R2RML's own
referencing-object asymmetry — no new vocabulary beyond accepting
`rr:joinCondition` within `rml:starMap`.

### D5 — Explicitly conformant rejections (kept, now spec-cited)

Predicate-position triple terms (Concepts §3.1); subject-position triple terms
in *data* (mapping-side: a star map in the quoted TM's subject stays a load-time
error, now citing §3.1 rather than "v1 scope"); Symmetric/Generalized RDF (§7,
non-normative, out of scope); the `rdfs14`/`rdfs:Proposition` entailment
vocabulary (RDFS-tier semantics, distinct from the interop vocabulary — out of
tier-1 scope, recorded to prevent future conflation).

### D6 — Boundaries that remain, precisely framed

* **Path-endpoint composition** inherits the engine-general "no join onto any
  path branch" boundary (path-shape-agnostic, affects non-star queries
  identically, previously mis-framed as closure-specific). Star introduces zero
  additional incompleteness; the general boundary stays separately tracked, and
  a non-star regression test pinning it is added.
* **Annotation-pattern assertion semantics — RESOLVED empirically (2026-07-18,
  W2a)**: spargebra desugars a WHERE-clause `s p o {| q v |}` to THREE patterns —
  the plain triple (asserted/matched) + the reifying triple + the annotation POM —
  i.e. annotation sugar both asserts and reifies, matching Turtle 1.2's data-side
  table. Locked by a distinguishing differential pair: annotation sugar returns
  EMPTY over a non-asserted mapping where bare sugar still matches.
* **Multi-shape unify boundary — DISCOVERED (2026-07-18, W2a), lift scheduled
  (W2b)**: `unify.rs::align_templates` conservatively 501s on template
  kind/length mismatch, which fires whenever one BGP touches ≥2 distinct quoted
  shapes' description maps through a shared variable (object-side nesting
  guarantees this by construction; it is not nesting-specific). The rewrite
  output is verified correct at the AST level; the block is engine-general.
  Planned lift: **literal-prefix disjointness** — two templates whose fixed
  literal prefixes conflict before the first column reference can never expand
  to the same value (the same percent-encoding injectivity argument the module
  already relies on), so the candidate pair is provably disjoint and is pruned
  instead of Unsupported. Sound (prune only on proof), general (benefits
  non-star mappings with distinct constant-prefixed templates), and gated by
  the full differential suites.
  **Lift landed (W2b), one divergence surfaced**: the tree path now *proves*
  the star-at-path-endpoint cell empty (prefix disjointness fires before the
  path-join restriction is reached) while the flat path still 501s
  (`unfold::merge` rejects on path-presence before ever unifying) — a
  genuine, narrow tree-exceeds-flat divergence the lift surfaced, not
  created; test-locked explicitly. Follow-up: mirror the prefix check in
  `unfold::merge` (or reorder its guards) if flat/tree 501-parity on this
  cell is ever wanted.

## Consequences

### Breaking vs v1 (deliberate, spec-driven; v1 shipped same-day, blast radius = our own tests/docs)

1. Subject-position star maps now emit reifier + `rdf:reifies` + description
   (5 triples) with **new id values**; annotations move to the reifier.
2. Bare `<< … >>` query sugar no longer matches unreified object-position triple
   terms (matrix above) — v1's conflated matching corrected.
3. Subject-position triple-term patterns return **empty** instead of matching.
4. The CONSTRUCT 501 guards are replaced by real instantiation + §16.2 dropping.
5. `differential_star.rs` expectations, the published spec/guide HTML, and
   ADR-0029/0031 texts are updated in the same work (living documents).

### Good

* Every visible surface (SELECT bindings, CONSTRUCT graphs, serializations)
  becomes native RDF 1.2 reification form; the encoding becomes invisible wire
  format. The headline divergence of v1 **dissolves** rather than being merely
  documented.
* Co-identity and reifier-multiplicity match the normative semantics *by
  construction* (injective proposition ids; per-declaration reifier ids).
* The equivalence theorem gets a real enforcement mechanism (decoder +
  spareval-native oracle), converting "sound and complete" from prose into a
  differential test obligation per construct.

### Bad / risk

* The rewrite grows a genuine environment-passing pass (nontrivial plumbing
  through every function in `star.rs`).
* Encoding-visibility (hybrid stance) means a query *can* observe
  `rdf:PropositionForm` triples alongside native answers; documented, revisit
  only if it bites.
* The interop note may change (it is WIP); our divergences are already
  documented per-item, and only the decoder + emitter would track it.

## Test plan (first-class)

End-to-end oracle: materialize the mapping output, **decode**, load into
`spareval`, run the **original** query, diff against the engine — per construct:
the matching matrix (4 cells), annotation + explicit-reifier sugar, nesting
(2 and 3 deep), VALUES (matched + projected-only + multi-row), all five
functions (incl. error rows), `=`/`sameTerm`/ORDER BY over composed terms,
CONSTRUCT (object-position produced; illegal positions dropped; round-trip
through Turtle 1.2), cross-source joins, reifier multiplicity (two maps, one
proposition, two reifiers), subject-position-empty, and the non-star
path-boundary pin. RED-first throughout; `differential_tree` 168/0 must hold.

## Rules

* **R1** — The virtual graph is native RDF 1.2; the encoding exists only below
  the SQL line; every visible surface speaks native reification form.
* **R2** — Proposition ids are a pure injective function of the quoted shape;
  reifier ids are per-declaration; the two families never collide.
* **R3** — `<< … >>` matches reifiers; `<<( … )>>` matches triple terms; the
  rewrite never conflates them.
* **R4** — Spec-impossible constructs (triple terms as predicates anywhere, as
  subjects in data) stay explicit load-time errors citing Concepts §3.1;
  spec-empty patterns (triple-term-subject patterns) return empty, never error,
  never match.
* **R5** — No silent conflation of SPARQL expression *errors* with unbound:
  function-error semantics follow §17.4.6 observably.
* **R6** — Answer equivalence with the decoded-graph oracle is the acceptance
  bar for every construct this ADR claims.
