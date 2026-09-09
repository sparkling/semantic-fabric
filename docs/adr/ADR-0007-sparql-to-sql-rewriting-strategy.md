---
status: accepted
date: 2026-06-27
updated: 2026-09-09
tags: [obda, virtualization, sparql-to-sql, rewriting, intermediate-query, optional, null-semantics, optimizer-cascade, correctness, term-construction-lifting, plan-cache, cost-driven]
supersedes: []
depends-on:
  - ADR-0003
  - ADR-0004
  - ADR-0006
implements:
  - ADR-0001
---

# SPARQL→SQL rewriting strategy and cascade correctness (virtualization / OBDA)

## Context and Problem Statement

The virtualizer (ADR-0003) turns a SPARQL 1.2 query over the virtual graph into SQL over the live source. Scope is R2RML-only (no OWL reasoning on this path; tier-1 hierarchy via T-mapping UNION expansion is ADR-0008), which bounds the rewriting blow-up by the number of triples-map references in the query rather than by ontology entailment.

The deep difficulty is a semantic mismatch: **SPARQL solution mappings are partial functions under bag + three-valued semantics, whereas SQL tuples are total functions where "absent" is NULL under a *different* three-valued logic.** Every correctness bug on this path is a leak between those two NULL regimes, and SPARQL `OPTIONAL` → SQL `LEFT JOIN` is the largest leak (it took the reference system, Ontop, roughly a decade to stabilise self-left-join elimination with nullable determinants).

## Decision Drivers

* Correctness first — a sound, complete base translation (Chebotko/Pérez semantics) with OPTIONAL/NULL handled exactly.
* Performance — an Ontop-style structural + semantic optimizer cascade is where the real-world speed lives.
* Reuse — `spargebra` (parse/algebra), `sqlparser` (dialect emission, ADR-0004), and the shared native SQLite/PostgreSQL/MySQL driver layer (execution, ADR-0006).

## Considered Options

* **Xiao/Kontchakov et al. (ISWC 2018) OPTIONAL-to-SQL base translation** — adopt as the unoptimized ground truth (`LEFT JOIN` + `COALESCE` of shared variables + explicit compatibility filter), proven bag- and 3VL-faithful.
* **Invent a bespoke base translation** — rejected; getting OPTIONAL/NULL exact across the two different three-valued NULL regimes from scratch is the largest correctness leak (Ontop took ~a decade to stabilise it).
* **Wire the `sparopt` algebra optimizer into `sf-sparql` (path A)** — not pursued; the order-disciplined IQ→IQ cascade is the sole optimiser (filter-pushdown = pass 5, join-elimination = passes 2/4), so the opt-in pre-stage adds nothing needed and whether its API integrates at our use-site is unproven.

## Decision Outcome

### Base translation = the unoptimized ground truth

Adopt the **Xiao/Kontchakov et al. (ISWC 2018)** OPTIONAL-to-SQL translation as the *unoptimized ground truth* — `LEFT JOIN` + `COALESCE` of shared variables + an explicit compatibility filter, proven bag- and 3VL-faithful. Do **not** invent a base translation; the cascade is then only semantics-preserving IQ→IQ rewrites on a translation that is already correct.

NULL / left-join rules the base translation obeys:

* **R1** — a shared OPTIONAL variable is never a plain `ON a = b`; the condition is `(a = b OR a IS NULL OR b IS NULL)` (an unbound variable is compatible with any value).
* **R2** — a shared variable is one SPARQL variable but two SQL columns after a LEFT JOIN; project it as `COALESCE(left, right)`.
* **R3** — R2RML mappings cannot emit NULL, so the mapping filters NULLs out; OPTIONAL re-introduces them (the padding effect / `IS NOT NULL` guards).
* **R4** — preserve bag semantics / multiplicities everywhere except inside an established DISTINCT/uniqueness context.
* **R5** — FILTER *inside* OPTIONAL belongs in the LEFT JOIN `ON` condition; FILTER *after* OPTIONAL is a later WHERE — an outer FILTER must never be pushed onto the preserved (left) side.

**NULL/cascade correction (2026-09-08):** base atoms now guard every generated
subject/predicate/object (including referenced parent subjects); class shortcuts
guard their subject before projection or correlation. DISTINCT may remove an
OPTIONAL only when projected bindings, WHERE and other OPTIONAL conditions no
longer consume its alias, including nested existential correlations. Its own ON
conditions alone do not prevent pruning. Ordered/aggregate/path/SubPlan consumers
remain outside this pruning proof. Verified NOT-NULL/PK guards are tautologies for
the existing same-row self-left-join proof, but multiple nullable guards remain
conditional and cannot be discarded. Required NULL-term, pruning and independent
bag-oracle regressions preserve both absence and valid OPTIONAL results; serving
retains its unverified-constraint quarantine.

**Equality correction (2026-09-08):** authored `rr:joinCondition` and portable
row-policy predicates retain separate native SQL condition variants. Native
equality is not proof of identical RDF lexical values, even with an FK/PK claim;
RDF substitution/elimination rules cannot consume that marker. Structural visitors
preserve it; same-terms elimination cannot orphan a residual native join. Ordinary
RDF text comparisons use live decoder recipes and exact comparison, without
blanket-casting other native types. Ref atoms now join/filter both native sources
before per-atom dedup when every key has live text/CHAR decoder proof; other
families retain prior per-source D1. Policy values remain bound inside the relation.
Their established OPTIONAL decomposition is retained. Raw output descriptors and
blank-node graph scope survive; general term-key limits remain in ADR-0034.
Ordinary D1 policy predicates now run as bound native guards before decoded
representative selection, without adding policy-only columns to the RDF key.
Every newly enabled policy key needs live text/CHAR or the SQLite lexical-only
IRI-template consumer/decoder proof in ADR-0034; raw outputs stay native. Lexical D1 identity
is separate from numeric literal value comparisons. Only all-IRI-template
consumers gain the new lexical comparison recipe; native guards are unchanged.

### Pipeline (`sf-sparql`)

1. **Parse** — `spargebra::SparqlParser` → `GraphPattern`. Since 2026-09-08 public serving uses an explicit prepared Rust process and bounded QueryV1 transfer before parent AST ownership (ADR-0055 parser-lifetime repair); raw diagnostic APIs retain their caller-owned contract. Existing semantic/compiler/cache profiles are not promoted to `GovernedV1`.
2. **Algebra pre-optimizer — unwired.** `sparopt` compiles as a transitive evidence dependency, but `sf-sparql` neither depends on nor invokes it. The order-disciplined cascade below is the sole product optimizer; `sparopt` is reference material, not an opt-in current stage.
3. **Unfold** — replace each triple pattern with the SQL sub-expressions of the matching mapping-IR entries → an IQ-style relational tree (the ISWC-2018 base translation).
4. **Tier-0 elimination (up front)** — a refObjectMap with no `rr:joinCondition` ⇒ inline the parent's subject IRI (no join); parent == child triples-map on a PK ⇒ collapse to a scan (redundant self-join elimination).
5. **Optimizer cascade — order is load-bearing:** (i) IRI-template-mismatch pruning → (ii) self-join / self-left-join elimination → (iii) functional-dependency inference (transitive closure, through unions) → (iv) FK/PK join elimination → (v) selection pushdown → (vi) distinct removal.
6. **Emit** — translate the optimized tree to a `sqlparser` AST and render the target dialect; values are bound parameters only (ADR-0010).
7. **Execute & reconstruct** — probe and validate every live result-column catalog, pre-emit every branch, then open cursors through the shared `SqlBackend` over the native SQLite/PostgreSQL/MySQL adapters; map rows to bindings; serialize with `sparesults`/RDF writers under the evidenced result profile (streamed where admitted, ADR-0010).

Steps 4–5 describe compiler capability, not unconditional serving behaviour.
Explicit frozen-schema/conformance callers may exercise the constraint-driven
rules through the raw translation APIs. Current `sf-serve` instead compiles only
through an immutable `CompilerBinding` whose `ConstraintAuthority::Unverified`
schema has PK, UNIQUE, FK, functional-dependency and NOT-NULL facts removed;
those rules are therefore conservative no-ops while structural rules continue.

### Cascade order is load-bearing (invariants)

* IRI-template-mismatch pruning **must precede** self-join elimination — empty branches are pruned, and the IRI-term equalities that license a self-join merge are established first.
* FD inference (with transitive closure) **must precede** FK/PK join elimination — eliminating a join is sound only when uniqueness *and* match-guarantee hold; firing earlier drops rows and violates bag semantics.
* Every rule preserves `=_bag` w.r.t. the base translation; preserves the COALESCE/compatibility semantics; fires only when its integrity-constraint precondition is already established by an explicit frozen/verified authority; left-join elimination preserves the right-side-bound provenance marker; the cascade runs to a fixpoint whose result is order-independent among commuting rules. A mutable startup catalogue observation is not such authority.

### Term-construction lifting (translation discipline)

Lift IRI/literal construction to the final projection **only where raw-column comparisons are proved equivalent to generated RDF identity**. Indexed native joins remain desirable, but encoding, CHAR padding, signed zero, processor bases and source collations invalidate a universal raw-key rule. Otherwise compare decoder-owned generated keys at the correct relational boundary (ADR-0034), retaining Rust term generation and bound query values. The former unconditional "never constructed strings" wording was incorrect: it licensed encoded IRI constants to match raw percent text and select the wrong row.

**Static constant correction (2026-09-09):** single-slot static IRI templates now compare forward-generated, byte-exact IRIs for fixed subject/object matches and `=`/`!=`/`sameTerm` filters. SQLite uses the shared live row decoder; native text/CHAR, integer, PostgreSQL boolean/BYTEA and MySQL binary string/blob/NEWDECIMAL/BIT/TIME/TIMESTAMP templates use separately proved live lexical recipes. Scalar authority is not text-column, constraint or general numeric-comparison authority. MySQL decimal/BIT/TIME/TIMESTAMP proofs are cleared across multi-branch SubPlans because UNION can change native representation. Exhaustive scalar alphabets bypass the generic byte encoder; TIME/TIMESTAMP escape colons and preserve wire fractions/timezone. MySQL DATE/DATETIME lexical-only D1 now converts both selected and partition values before temporary-field revalidation; required native checks cover partial/invalid/zero dates, fractional DATETIME, constant lookup and exact duplicate/explicit-literal bags. Original consumer proof prevents native/natural-key substitution, and unprotected raw temporal facts do not cross Projection/RefAtom/SubPlan. PostgreSQL numeric non-finite errors and native float/remaining temporal spellings remain required (ADR-0034). Fixed prefix/suffix disjointness still prunes mappings before source work. Multi-slot constant and template/template boundaries are unchanged; unproved native scalar families fail explicitly and remain required follow-up, not a completed or removed product capability.

**Natural temporal identity delta (2026-09-09):** MySQL natural DATE/DATETIME now retains native payload and datatype while decoder-qualified identity keys match canonical Rust output; query constants remain verbatim. Required owned TLS CLI checks cover canonical/noncanonical fixed and sameTerm matches, DATE/leap/year-zero/extrema and DATETIME fractions, duplicate bags, nested projection, mixed literal/IRI joins, NULL/negation, invalid hidden SELECT/COUNT/ASK terms, cap-one recovery and denied-invalid-row policy/existential/OPTIONAL isolation. Coercing mixed temporal SubPlans retain a rejection marker instead of falling back to raw equality. Wider natural/native identity and native-consumer copies remain open. ADR-0015/0024 describe the decoder and authorization boundaries. This closes the qualified natural temporal slice, not general native identity or release admission.

### v1 SPARQL coverage

**Supported:** BGP, `JOIN`, `FILTER`, `OPTIONAL` (null-safe), `UNION`, `BIND`, `VALUES`, projection, `DISTINCT`/`REDUCED`, `LIMIT`/`OFFSET`, `ORDER BY` (with explicit SPARQL NULL/UNBOUND ordering), aggregates, `GRAPH`, `MINUS`, and the characterized variable-endpoint property-path profile: `P+`/`P*`, single-predicate `p?`, negated property sets, inverse, sequence and alternative. Recursive paths use exact finite-pair fixed points under ADR-0049. Residual bound-endpoint, nested-closure, shape-mismatched and multi-mapping/refObjectMap path shapes return `501`, as do the parsed `LATERAL` extension, `SERVICE`, and OWL 2 QL tier-2 entailment.

### Performance

**Path-search controls (2026-09-08):** public compiler misses now prospectively
charge mapping/graph candidate searches, negated-complement comparisons and actual
term-map/source copies under the same request control. Borrowing complement IRIs
and moving first-hop endpoints preserves `Nps` bag identity and raw restrictions.
Exact six-pair HTTP, no-source-admission rejection and path-oracle tests pass.
Graph inventory/reflexive eligibility, shape construction and total compiler/source
work remain open under ADR-0010; no supported shape or cache authority changes.

**Plan cache (hot path).** The implemented single-source `CompilerBinding` inseparably owns `SourceMapping`, dialect, T-box, compiler-safe schema, constraint authority, column-type authority and a bounded `quick_cache`. Its key includes a process-unique binding/generation scope (including both authorities), dialect, a structural hash, and the full canonical algebra; cached values carry and recheck the same scope. This conservative form safely keys all constants and prevents cross-source/dialect/binding/policy reuse, but it may miss reusable data-constant plans. PostgreSQL statements use the native client preparation path; there is no `deadpool` prepared-statement cache. The target refinement parameterises *data* constants while keying *schema-selecting* constants, then replaces process-local identity with immutable ontology/mapping/schema/capability/policy digests and atomic generation changes.

**Implemented serving quarantine (`24a0e20`; hardened 2026-09-02).** `sf-serve` converts every startup observation
to `CompilerSchema::from_unverified_observation`. Both fresh and cached plans see
names, SQL types and estimates but no PK, UNIQUE, FK, functional dependency or
NOT-NULL proof, so later constraint DDL cannot make a constraint-sensitive pass
change a serving answer. Duplicate safety consequently keeps its conservative
deduplication path. This closes the earlier stale-constraint P0 without claiming
a watcher, fingerprint, reload, or verified-constraint mode.

`ColumnTypeAuthority::Unverified` now also prevents mutable startup type strings
from proving positional PostgreSQL pooling safe across different physical
columns; missing type facts fail closed, while an identical source/column pair
is structurally one live expression. When compatibility remains unproven, each
fallback arm captures its full BGP-boundary term definitions, including an active
graph variable, before outer projection. After all rewrites, the scope and key
remap only through physically key-preserving pure unary SubPlan chains; execution
overlays them onto a private branch clone and hashes the explicit key. Runtime
repeats the shape/key proof before metadata I/O. Joined/OPTIONAL/path/aggregate,
modifier-bearing or multi-branch nested wrappers, key-dropping nested projections,
orphaned/multiply owned markers, or groups with fewer than two executable arms
return `501`. Fully ground overlapping arms use an exact SQL unit-relation pool.
The exact shared set remains a source-sized raw/conformance fallback that serving rejects pre-I/O. The raw
translation APIs retain an explicit caller-authorized frozen-schema contract. At execution, every
distinct base Table/Query source recursively reachable through correlated
conditions and nested SubPlans is probed, missing/duplicate/ambiguous live
columns reject, and all branches are emitted before the first cursor opens.
Fresh derived aliases are allocated above aliases hidden in nested IQ/SQL
conditions. Composed paths retain typed scan recipes until live emission
(ADR-0033, 2026-09-08), which resolves their recursively probed leaf columns;
their outer aliases confer no table or portable-policy authority. Live nested
emission overlays probed catalogs for base-source references. Offline/synthetic derived aliases and translate-time immediate
wrappers retain the bounded `col_is_unquoted_alias` lexical heuristic, which is
not SQL-token-aware and never live metadata authority. Probe and emission errors therefore cannot
produce a partial multi-branch result. SELECT/CONSTRUCT executor failures expose
only a stable body error after HTTP commitment.

Compiler SQL-shape tests show that the tested no-primary-key Direct-Mapping path
forms retain SQLite `rowid` and substitute PostgreSQL `(ctid)::text` for the
synthetic `rowid` sentinel; the D1 Table→Query wrapper preserves the logical
`rowid` output name and only base-table aliases read `ctid`. This removes the
known nonexistent-`rowid` emission for those shapes, but is not a general row
identity proof: a real PostgreSQL table column named `rowid` is still
indistinguishable from the sentinel and `ctid` is snapshot-local. Required owned
native query fixtures now cover decimal-digit VARCHAR paths and joined-column
folding, not this no-PK identity profile or general key equality (ADR-0049).

Ordinary probes are sequential, can race DDL after preflight and do not establish
a coherent generation. Authored reload now fences observed drift and activates
an immutable replacement binding/cache namespace; the closed PostgreSQL Direct
profile adds a verified backend lease (ADR-0050/0055). Neither gives every source
or authored query a DDL-stable lease. General structural/type lifecycle and total
metadata work controls remain open; a digest alone cannot close the DDL race.

Direct Mapping remains a separate lifecycle. `serve --direct-mapping-base` now
generates mappings for the closed PostgreSQL 16.9/16.15 PK-backed profile under
its verified generation owner; authored serving remains separate (ADR-0055).
No-PK synthetic identity and additional Direct backends remain open. Redacting
optimizer facts after generation cannot replace a mapping-generation lease.

### Correctness anchor

Chebotko/Pérez relational-algebra semantics is the soundness/completeness reference (proof target: `eval_SQL(τ(Q,M),D) =_bag eval_SPARQL(Q, RDF(M,D))`).

### Consequences

* Good, because a proven base translation + an order-disciplined cascade captures Ontop-class speed, with blow-up bounded by R2RML-only scope.
* Bad, because the cascade is the single hardest correctness surface; NULL-across-left-joins demands the invariants above plus the ADR-0012 test strategy.
* Bad, because deferred features return `501` in v1 (documented, not silent).

### Confirmation

The implemented fixed native-oracle and NoREC differentials provide regression
evidence. ADR-0012 defines the broader verification target: generated MR1
constraint toggling for unsound rewrites and per-rule bounded equivalence
checking remain planned; no VeriEQL integration or generated MR1 gate is
currently claimed.

## More Information

* **Architecture:** ADR-0003. **Substrate:** ADR-0004. **Execution substrate:** ADR-0006. **Scope:** ADR-0002. **Reasoning:** ADR-0008. **Governance (injection-safety, recursion bounds, streaming):** ADR-0010. **Test strategy:** ADR-0012.
* **Research:** `docs/research/` — `cascade-correctness` (Xiao/Kontchakov ISWC-2018, Chebotko, Pérez, cascade order + invariants), `ontop`, `foundations-benchmarks`.
* **Cost-driven design (baked in here):** term-construction lifting + the plan cache are the rewriter-side half; the term-gen allocation discipline + cross-source semi-join cost are the ADR-0006 half. Both promoted from the ADR-0020 research register.

**Update (2026-07-19, Run 4 Wave B3) — a second condition-only exception to
term-construction lifting.** The lifting invariant ("the SELECT projects raw
key columns; RDF terms are built during reconstruction") is unchanged, but the
WHERE clause now has a second place where term-LEXICAL text is computed in SQL
(the first being `StrMatch`'s LIKE/regex pushdown): `SqlCond::TemplateEq`
renders two differently-shaped templates as dialect-correct CONCAT expressions
and compares them, resolving template-shape-mismatch equality (`unify.rs`,
`emit.rs::render_template_concat`). Boolean-condition-only — never SELECTed,
never reconstructed from — so the lifting economics are untouched. Restricted
to term classes where lexical equality IS term equality (IRIs, plain/
`xsd:string` literals) and to dialects whose NULL-propagation through concat
matches R2RML §11 term-absence (`||` on PG/SQLite, `CONCAT` on MySQL; every
other dialect stays a sound 501).
