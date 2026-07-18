---
status: accepted
date: 2026-07-18
updated: 2026-07-18
tags: [rdf-star, sparql-star, query-rewrite, algebra, obda, virtualization]
supersedes: []
depends-on:
  - ADR-0007
  - ADR-0023
  - ADR-0028
  - ADR-0029
implements: []
---

# RDF-star query support: algebra-level desugar of quoted-triple patterns onto the basic encoding

## Implementation status (2026-07-18, same day)

**Accepted and implemented.** `crates/sf-sparql/src/star.rs` (+ `star/tests.rs`)
implements rules 1–6 as one shared pre-pass wired at the top of both
`translate_tree` and `translate_inner_flat`; both engines see an identical,
already-desugared pattern. Tests: 7 AST-shape unit tests + 11 end-to-end cases
in `crates/sf-conformance/tests/differential_star.rs` (tree/flat parity +
identical-501 + hand-computed bindings); `differential_tree` unaffected at
168/0 — every query in it now flows through the pre-pass, so that green is a
live regression check. Three implementation findings recorded:

* **Rule 9 caught a real silent-wrong-answer path**: `exec_core::instantiate`'s
  `TermPattern → Term` wildcard silently *dropped* a quoted-triple template
  triple from CONSTRUCT output (compiled fine, lost data at execution, no
  error). Now an explicit 501 guard in both `Query::Construct` arms, RED-tested.
* **Rule 6b (path endpoints) is structurally a locked 501 in v1, not a
  bindings path**: the rewrite emits the designed `Join(Bgp(4), Path{..})`
  shape correctly, but `unfold::merge` (shared by both engines) refuses to
  join any path-carrying branch with anything else — a pre-existing,
  unrelated v1 boundary ("standalone `?s P+ ?o` only"). So *every*
  quoted-pattern-at-a-path-endpoint query 501s identically in both engines.
  Lifting that is a path-composition feature (join-with-closure), tracked
  separately — not query-rewrite scope.
* **Co-identification multiplicity (flagged follow-up, pre-existing class)**:
  per ADR-0029, the synthetic id is a pure function of the *quoted* triple's
  shape, so a subject-position and an object-position StarMap quoting the
  same shape co-identify (by design — the Jena one-reification invariant).
  When both live in one mapping, each basic-encoding triple is emitted by two
  TriplesMaps and the engine's generic overlapping-maps bag multiplicity
  multiplies join derivations (observed 2⁴×). Whether that diverges from the
  set-semantics oracle on such mappings is a **pre-existing engine-wide
  question about duplicate-emitting mappings**, not introduced here —
  recorded for a dedicated oracle comparison.

## Context and Problem Statement

`ADR-0029` (accepted, implemented) compiles `rml:StarMap` mappings to the W3C
RDF 1.2 Interoperability **basic encoding**: a synthetic proposition-form
identifier plus 4 plain triples (`rdf:type rdf:PropositionForm`,
`rdf:propositionFormSubject/Predicate/Object`). Its §C left the query half
undesigned: recognizing quoted-triple patterns in SPARQL-star and answering
them against that encoding. Today a quoted-triple pattern is a clean, traced
`Error::Unsupported` → 501 (`unfold.rs` `bind_position` wildcard; no panic, no
silent wrong answer), and there is **no prior art anywhere** for this rewrite
(`ADR-0028 §G`).

Ground truth about the parser (empirically probed against the pinned
`spargebra 0.4.6` + `sparql-12`, not read off type signatures):

* **Parenthesized syntax** `<<( s p o )>>` parses to
  `TermPattern::Triple(Box<TriplePattern>)` in place. `TriplePattern.predicate`
  is `NamedNodePattern` — a quoted triple can never occupy predicate position
  *by construction* (consistent with `ADR-0029 R4`).
* **Bare syntax** `<<s p o>>` — the form in virtually all RDF-star examples —
  is desugared *by spargebra itself* into a fresh blank node `_:b` substituted
  at the original position plus a prepended triple
  `_:b rdf:reifies <<( s p o )>>` (`spargebra/src/parser.rs:341-363`;
  `rdf:reifies` per `oxrdf/src/vocab.rs:46-47`). That is the RDF 1.2 **native**
  encoding — a vocabulary no `TriplesMap` in this engine ever asserts.
* `VALUES` accepts only the parenthesized ground form (`GroundTerm::Triple`);
  it already 501s cleanly (`unfold.rs` `ground_term_to_term` wildcard).
* `TRIPLE()/SUBJECT()/PREDICATE()/OBJECT()/isTRIPLE()` parse as ordinary
  function-call expressions — a separate evaluation surface, untouched here.

The trap this ADR exists to avoid: a pass that mechanically replaces every
`TermPattern::Triple` would leave spargebra's synthetic
`_:b rdf:reifies …` triple in the BGP. `rdf:reifies` is unmapped, so the
pattern unfolds to zero branches → `IqNode::Empty` — **every bare-syntax
query would silently return zero rows forever**, with matching data present.
The reifies wrapper must be recognized and elided, not translated.

## Decision Drivers

* **Zero downstream changes.** `ADR-0029` proved the desugar-into-existing-IR
  shape on the mapping side; the query side should mirror it: rewrite at the
  algebra level, before either engine runs, so `build.rs`, `iq/resolve.rs`,
  `iq/normalize.rs`, `iq/lower.rs`, `cascade/`, `emit.rs` never see a
  quoted-triple pattern at all.
* **Both engines, one rewrite.** The differential harness runs the tree path
  *and* the flat oracle and asserts row-bag parity / identical-501 outcomes.
  A shared pre-pass keeps them byte-equivalent (`iq/resolve.rs` reuses the
  flat `Unfolder` verbatim, so post-rewrite resolution is identical by
  construction).
* **Existing precedent.** The DESCRIBE→CBD rewrite already does exactly this
  shape of work at the top of both `translate_tree` (`lib.rs:422-460`) and
  `translate_inner_flat` (`lib.rs:280-325`): a recursive `GraphPattern`
  rebuild minting `__sf_`-prefixed synthetic variables via
  `Variable::new_unchecked` (unwritable in real query text → collision-proof).
* **Scope bookkeeping is a real trap.** `iq/node.rs` `triple_pattern_vars`
  counts only `TermPattern::Variable` — a surviving `Triple` contributes zero
  variables and silently under-reports scope. Rewriting before `build_tree`
  makes that unreachable rather than merely unlikely.
* **The fresh-variable counter must span the whole query.** The `__sf_ord_{n}`
  per-clause counter pattern is unsafe here (multiple quoted patterns across
  sibling BGPs / UNION arms / EXISTS bodies later compose); mirror
  `ResolveCx`'s single threaded counter discipline instead.

## Considered Options

* **(a) Native triple-term support through the IR/executor** — rejected: the
  engine's storage reality is the basic encoding (`ADR-0028 §G`: no atomic
  triple-term value exists to scan); native terms would demand new IR, new
  SQL shapes, and new result serialization for zero additional answerable
  queries.
* **(b) Rewrite inside the IQ pipeline (build/resolve time)** — rejected: two
  engines would need separate arms, the flat oracle would stay 501 (breaking
  differential parity), and the `triple_pattern_vars` scope trap sits exactly
  on that path.
* **(c) A shared `GraphPattern`-level pre-pass before both engines — chosen.**

## Decision Outcome

One recursive `GraphPattern → GraphPattern` rewrite, applied at the top of
both `translate_tree` and `translate_inner_flat` (shared function, CBD
precedent), with a whole-query fresh-variable counter minting
`__sf_star_{n}` via `Variable::new_unchecked`.

### Rewrite rules (per triple pattern, order matters)

1. **Subject substitution.** If `subject` is `TermPattern::Triple(tp)`:
   replace it with a fresh `?__sf_star_n` and append the **4 basic-encoding
   patterns** binding that variable to `tp` (rule 3).
2. **Reifies elision (the load-bearing arm).** If, after rule 1, the pattern
   is `X rdf:reifies TermPattern::Triple(tp)` (predicate is the constant
   `rdf:reifies`; `X` is whatever spargebra or the author put there — blank
   node from bare syntax, or an explicit variable): **drop the triple
   entirely** and append the 4 basic-encoding patterns with subject `X`.
   Blank-node `X` flows through the existing non-distinguished-join-variable
   machinery (`unfold.rs` `__bnode_{id}`) untouched.
3. **Object substitution.** Otherwise, if `object` is `TermPattern::Triple(tp)`:
   fresh variable + the 4 patterns, symmetric with rule 1.
4. **The 4 basic-encoding patterns** for identity `I` and quoted
   `tp = (s, p, o)`:
   `I rdf:type rdf:PropositionForm . I rdf:propositionFormSubject s .
   I rdf:propositionFormPredicate p . I rdf:propositionFormObject o .`
   `s`/`p`/`o` are copied verbatim (variables, constants, or a variable
   predicate — all already handled by generic unfolding; `p` is
   `NamedNodePattern`, structurally never a triple).
5. **Nesting → 501.** If `tp`'s own subject/object is another
   `TermPattern::Triple`: `Error::Unsupported` with a clear message at
   rewrite time — symmetric with `ADR-0029 R3`'s load-time rejection, and
   honest per ADR-0007 (a nested pattern can never match data the mapping
   compiler refuses to emit; answering `[]` silently would be a lie of
   omission).
6. **Recursion coverage.** The walker recurses through every `GraphPattern`
   container *including* pattern-bearing expressions (`EXISTS { … }` inside
   FILTER/BIND/ORDER BY) and property-path endpoints (`GraphPattern::Path`
   subject/object get rule 1/3 treatment, with the 4 patterns joined
   alongside the path node). `GraphPattern::Values` is untouched (rule 7).

### Explicit v1 boundaries (documented, tested, all pre-existing clean 501s)

7. **`VALUES` with a ground quoted triple** stays 501 (`ground_term_to_term`
   wildcard). Resolving a *constant* triple term to its synthetic id is a
   constant-vs-template unification problem against every candidate StarMap —
   a different mechanism, deferred.
8. **`TRIPLE()/SUBJECT()/PREDICATE()/OBJECT()/isTRIPLE()`** — v1 does not
   produce native triple terms, so these remain unsupported; the implementer
   verifies each reaches a clean 501 (not a silent wrong value) and locks it.
9. **CONSTRUCT templates containing a quoted-triple term** — same: verify the
   current failure mode, make it a clean 501 if it is not already.

### Semantics (the ADR-0029 divergence, carried through honestly)

A reifier/identity variable (explicit `?r rdf:reifies <<( … )>>`, or a
projected `__sf_star` position a user can't actually write) binds to the
**synthetic proposition-form IRI** (`urn:sf-star:…`), never to a native
triple term. `isTRIPLE` on it is therefore not `true` — v1 501s the function
rather than answering `false` misleadingly. An ordinary `rdf:reifies` pattern
whose object is *not* a quoted-triple pattern (e.g. `?r rdf:reifies ?t`) is
NOT elided — it unfolds normally (and matches nothing unless a mapping
asserts `rdf:reifies`, which `sf-mapping` never emits). This must be stated
in query-authoring docs wherever RDF-star support is described.

### Test plan (first-class, not a rider)

* New conformance file `crates/sf-conformance/tests/differential_star.rs`
  (mirrors `differential_paths.rs`; deliberately NOT `differential_tree.rs`,
  which a parallel work stream is appending to). Reuses
  `sf-mapping`'s `STAR_ASSERTED_FIXTURE` shape (census_row) with a matching
  `CREATE TABLE`.
* **Oracle strategy** (the subtlety): spareval evaluates SPARQL-star natively
  (reifies + triple terms), so the *original* query over the materialized
  basic-encoding graph would rightly return nothing there. Star tests
  therefore assert (a) hand-computed expected bindings for the original
  query through sf, (b) tree/flat row-bag parity, and (c) where the oracle is
  used, it runs the **post-rewrite 4-pattern algebra** over the materialized
  graph — validating that everything downstream of the rewrite is
  oracle-exact.
* RED-first coverage: bare syntax (reifies elision), parenthesized subject
  and object positions, explicit reifier variable projection (binds the
  synthetic IRI), non-asserted mapping (plain triple absent, quoted match
  present), nested pattern → 501, VALUES ground triple → 501 (locked),
  `isTRIPLE` → 501 (locked), star pattern inside EXISTS, star pattern at a
  property-path endpoint.

## Consequences

### Good

* Bare-syntax queries — the form every RDF-star user actually writes — work,
  because the reifies wrapper is understood rather than mistranslated.
* Zero changes to build/resolve/normalize/lower/cascade/emit; both engines
  gain the capability from one rewrite; differential parity holds by
  construction.
* Self-join elimination (`cascade`) can collapse the 4 same-table scans when
  the backing table declares a covering PK/unique key — a measured-later
  optimization, not a correctness condition.

### Bad / risk

* The 4-pattern expansion multiplies joins on quoted patterns (up to 4 extra
  scans per quoted triple pre-cascade) — acceptable v1 cost; bench receipts
  before optimizing (e.g. dropping the `rdf:type` guard pattern, recorded
  below as the considered alternative).
* The rewrite hard-codes the reifies-elision semantics: sf treats
  `X rdf:reifies <<( … )>>` as a basic-encoding accessor, which diverges from
  native RDF 1.2 semantics exactly as ADR-0029 already diverges on the
  mapping side. Documented loudly; portability caveat inherited.
* Stale module banners (`build.rs`, `iq/*.rs` "flat is the production
  engine") contradict `lib.rs`'s real M8 state and misled this design's first
  draft — fix the banners in the M5 reconciliation pass.

### Neutral

* Including the `rdf:type rdf:PropositionForm` guard pattern (4 patterns, not
  3) is a deliberate strict-decoding choice: incomplete hand-authored
  encodings (missing the type triple) do not match. The 3-pattern variant is
  recorded here as the alternative if bench receipts later justify it.

## More Information

* Depends on: `ADR-0029` (the encoding this decodes), `ADR-0028 §E/§G` (why
  basic encoding; no prior art), `ADR-0023` (tree pipeline this slots in
  front of), `ADR-0007` (sound-501 discipline governing rules 5/7/8/9).
* Parser ground truth from an empirical probe crate against pinned
  `spargebra 0.4.6` (`sparql-12`), 2026-07-18.

## Rules

* **R1** — Quoted-triple patterns are rewritten to the basic encoding at the
  `GraphPattern` level, before either engine; no downstream stage may carry a
  `TermPattern::Triple`.
* **R2** — `X rdf:reifies <<( s p o )>>` is elided into the 4 basic-encoding
  patterns on `X`; the reifies triple itself is never unfolded.
* **R3** — Fresh identity variables use the `__sf_star_{n}` namespace with a
  single whole-query counter.
* **R4** — Nested quoted-triple patterns, ground quoted triples in VALUES,
  the five triple-term functions, and quoted terms in CONSTRUCT templates
  are explicit 501s in v1 — never silent empties, never silent wrong values.
* **R5** — Identity positions bind the synthetic proposition-form IRI; v1
  never fabricates a native triple term anywhere in results.
