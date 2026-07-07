# Ontop 5.5.0 optimizer/reformulation dossier — the five Tier-2 sound-501 gaps (ADR-0025)

**Purpose.** ADR-0025 (Tier 2) lists five real Ontop capabilities that semantic-fabric (sf) currently
soundly declines (`Error::Unsupported`, ADR-0007's =_bag rule: refuse rather than risk a wrong answer).
This dossier investigates, for each, how Ontop 5.5.0 actually implements it — grounded in the Java
source at `/Users/henrik/source/ontop` — and what sf's operator-tree IR (ADR-0023) would need to match
it soundly. Two of the five turned out **not** to have a usable Ontop precedent at all (§1, §3); this is
reported honestly rather than papered over, per the investigation brief.

**Scope note.** This is a research dossier to inform milestone scoping, not an exhaustive Ontop teardown.
Each section cites the concrete class/method/file it is grounded in; where a claim could not be verified
in source, that is stated explicitly instead of guessed.

---

## Executive summary

| # | ADR-0025 Tier-2 gap | Ontop's mechanism | Verdict | sf work implied |
|---|---|---|---|---|
| 1 | Property path inside EXISTS/NOT EXISTS/MINUS | **No general precedent.** Ontop supports only one hard-coded path predicate (`rdfs:subClassOf*`, resolved via a precomputed TBox closure + `UNION`/`DISTINCT`, never a recursive SQL construct). General `+`/`*` paths over arbitrary predicates throw `OntopUnsupportedKGQueryException`; 11 W3C property-path compliance tests are explicitly skipped. `WITH RECURSIVE` appears nowhere in Ontop's codebase. | **Not found in source — sf is already ahead of Ontop here for general paths.** | sf must design its own solution (e.g. a CTE-aware `SqlCond::Exists`/`Scan` variant) — there is nothing to port from Ontop. Lower priority: no reference implementation exists to converge toward. |
| 2 | Multi-branch UNION as a join/OPTIONAL operand | **Real, two-part mechanism.** `BottomUpUnionAndBindingLiftOptimizer` conditionally *distributes* the join over union arms when per-arm term construction is incompatible (`InnerJoinNodeImpl`/`LeftJoinNodeImpl.liftIncompatibleDefinitions`, gated by `UnionNodeImpl.hasAChildWithLiftableDefinition`), recombining incompatible arms via an `ifElseNull` CASE construct; when arms *are* compatible it leaves the union as one `UNION ALL` derived table (`IQTree2SelectFromWhereConverterImpl.transformUnion`). Cross-arm typing is reconciled upstream at the mapping level (`TermTypeMappingCaster`) and, for null columns, at SQL-gen time (`TypingNullsInUnionDialectExtraNormalizer`). | **Found — real Ontop capability, worth matching.** | Extend `lower_as_subplan`/`try_as_subplan` with a `try_sql_group_over_union`-style cross-arm `KeyShape` proof (already exists for the aggregation case) before allowing a multi-branch inner plan; on failure, sf's flat model can approximate distribution by pre-flattening the union arms into separate branches before the join (sf already does eager flattening elsewhere — ADR-0023's own "structural ceiling" critique). |
| 3 | `COUNT(DISTINCT *)` | **Confirmed Ontop bug, not a design.** The DISTINCT flag is silently dropped for wildcard `COUNT` in `RDF4JValueExprTranslator` (only the `arg != null` branch checks `isDistinct()`); the arity-0 distinct DB symbol exists but is dead code; no multi-column `COUNT(DISTINCT a,b)` SQL path exists anywhere; the only test fixture is parse-only (no evaluated result). | **Not found (working) in source — Ontop does not correctly or portably solve this.** | sf's planned `SELECT DISTINCT <cols> FROM (...)` + outer `COUNT(*)` derived-table rewrite is an sf-original solution, not a port. Proceed on sf's own design; Ontop offers no shape to converge SQL output toward, and no correctness precedent to lean on either. |
| 4 | GROUP BY over a property-path closure | **Structurally possible but never exercised.** `AggregationNode` is fully opaque to its child's provenance (no type-check on the child, generic `IQTree.getVariables()` only) — Ontop's uniform tree design means grouping over *any* child, recursive or not, would compose for free if a recursive child existed. But because Ontop's own path support is limited to the `subClassOf*` special case (§1), there is no working example of GROUP BY over a genuinely recursive relation to point to. | **Partially found — architectural principle confirmed, no concrete precedent.** | The *design lesson* transfers even without a literal port: once sf's path-as-CTE is exposed as a `SubPlan`-like derived table (the same primitive gap #2 needs), grouping over it should require no special-casing beyond what `lower_as_subplan` + aggregation-over-SubPlan already generalizes to — i.e. gap #4 is subsumed by making paths SubPlan-lowerable, not a separate mechanism. |
| 5 | Post-GROUP-BY expression over a UNION aggregate | **Real and clean: never falls back to in-process aggregation.** A post-agg expression is an ordinary `ConstructionNode` sitting above `AggregationNode` (enforced: `AggregationNode`'s substitution must be pure aggregate calls, `AggregationNodeImpl.validateNode`). Ontop **never** executes GROUP BY in the JVM — no in-memory aggregator exists anywhere; `IQTree2SelectFromWhereConverterImpl` always composes the outer `ConstructionNode` substitution with the `AggregationNode` substitution into one SQL projection list over a `UNION ALL` derived table, or the query fails outright (`NotFullyTranslatableToNativeQueryException`) rather than degrading to Java-side grouping. | **Found — and it reframes the gap.** | The Ontop precedent implies sf's `rust_group` in-process fallback is the actual architectural mismatch, not the expression evaluator. The durable fix is to widen `try_sql_group_over_union`'s cross-arm proof (gap #2's same primitive) so more UNION shapes pool into one SQL derived table and never need `rust_group` at all; a Rust-side post-agg expression evaluator is the fallback-of-last-resort, not the primary fix Ontop's design suggests. |

**Cross-cutting finding.** Three of the five gaps (2, 4, 5) reduce to the **same missing primitive**: a
sound, general "pool N branches into one derived table with proven cross-arm term/type compatibility"
capability. sf already has one working instance of this proof (`try_sql_group_over_union`'s `KeyShape`
matching) but it is scoped narrowly to the aggregation-over-union case. Generalizing that proof into
`lower_as_subplan`'s multi-branch path (currently a flat 501) would likely close gaps 2, 4, and most of 5
in one milestone rather than three. Gaps 1 and 3 have no Ontop mechanism to converge toward at all — both
are sf-original engineering, unblocked by (and in 1's case, ahead of) the reference engine.

---

## 1. Property-path inner pattern inside EXISTS / NOT EXISTS / MINUS

### Ontop's approach

Ontop has **no general mechanism** for this shape, because it has no general mechanism for arbitrary
transitive property paths at all. The sole path translator is
`RDF4JTupleExprTranslator.translate(ArbitraryLengthPath)`
(`core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JTupleExprTranslator.java:274-328`),
and it is hard-coded to exactly one predicate: `rdfs:subClassOf`. It checks
`childAtom.getTerm(1).equals(subClassOfConstant)` (line 282); if the path predicate is anything else, it
throws `OntopUnsupportedKGQueryException("Unsupported arbitrary length path: " + arbitraryLengthPath)`
(line 327). For the one supported case, the method builds a `UNION` of a reflexive depth-0 branch and the
existing `IntensionalDataNode` for `rdfs:subClassOf`, wrapped in a `DistinctNode` — the source comment
reads "Depth 1. Takes advantage that Ontop computes the transitive closure of `rdfs:subClassOf`" (line
322). In other words: Ontop leans on its TBox reasoner having **already materialized** the closure
offline; there is no runtime recursive query over ABox data, ever.

This is corroborated by `test/sparql-compliance/src/test/java/it/unibz/inf/ontop/test/sparql11/MemorySPARQL11QueryTest.java:61-74`,
which explicitly skips 11 official W3C property-path compliance tests
(`pp02, pp12, pp14, pp16, pp21, pp23, pp25, pp34, pp35, pp36, pp37`, plus `pp28a`) with the comment
`// ArbitraryLengthPath not supported`. A grep for `RECURSIVE`/`WITH RECURSIVE` across the entire
Ontop tree (`core/`, `db/`, `engine/`) returns zero hits related to SQL recursion — the only "recursive"
matches are ordinary Java tree-walking visitor classes.

FILTER EXISTS / NOT EXISTS / MINUS themselves are *not* SQL `EXISTS` subqueries in Ontop's IQ at all —
`RDF4JTupleExprTranslator.translate(Filter)` (line 399), `createExistsSubtree` (line 429), and
`translateMinusOperation` (line 506) all compile these into an ordinary `LeftJoinNode` plus a synthetic
provenance variable, tested with `IS [NOT] NULL` (`termFactory.getDBIsNull(...)`) — i.e. a semi-/anti-join
expressed as `LEFT JOIN ... WHERE prov IS [NOT] NULL`, never a correlated `EXISTS (SELECT ...)` in the
generated SQL. The EXISTS-specific admissibility gate, `ExistsSubtreeVisitor`
(lines 526-548, 983-1020), only explicitly rejects `Slice` and `Group` nodes — it does not special-case
property paths at all. A path inside EXISTS simply falls through to the same
`OntopUnsupportedKGQueryException` as any other unsupported path, unless it happens to be the
`rdfs:subClassOf*` special case (which, being resolved via ordinary IQ-tree translation before EXISTS
compilation even sees it, would work anywhere a `UNION`/`DISTINCT` subtree can).

### The key SQL shape Ontop emits

None exists for the general case — there is nothing to emit. For the one working special case
(`rdfs:subClassOf*`), the "path" is not SQL-recursive at all; it is an ordinary `UNION ALL` /
`DISTINCT` over the pre-closed `rdfs:subClassOf` relation, compiled like any other IQ subtree — no CTE,
no `LATERAL`, no inline recursive expansion.

### What sf would need

Nothing ports from Ontop here — there is no reference mechanism to converge toward, and sf's existing
`WITH RECURSIVE` lowering for general paths (over arbitrary predicates, not just a precomputed
subclass hierarchy) is already a materially more general capability than Ontop's. Closing this sf gap is
original engineering: per ADR-0025, a CTE-aware `Scan`/`SqlCond` variant (e.g. generalizing `Scan` beyond
base-table scans, or a `SqlCond::WithTable` wrapping both base scans and CTE references) so
`SqlCond::Exists`/`NotExists`'s `scans: Vec<Scan>` can reference a `WITH RECURSIVE` CTE defined in the
same query — syntactically valid SQL (`WITH RECURSIVE cte AS (...) SELECT ... WHERE EXISTS (SELECT 1 FROM
cte WHERE cte.col = outer.col ...)`), but with no Ontop design to copy the proof obligations from. Given
the absence of a reference implementation, this should probably be sequenced *after* the gaps with a real
Ontop precedent (2, 4, 5), not before — ADR-0025's own recommended order (items 3, 2, 4, 5, then 1 last)
already reflects this by putting the CTE-aware `Exists` generalization last as "the largest SqlCond
generalization."

**Provenance:**
- `core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JTupleExprTranslator.java:274-328` (path translator, `subClassOf`-only)
- `core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JTupleExprTranslator.java:399,429,506,526-548,983-1020` (Filter/EXISTS/MINUS → LeftJoin+IS NULL, `ExistsSubtreeVisitor`)
- `test/sparql-compliance/src/test/java/it/unibz/inf/ontop/test/sparql11/MemorySPARQL11QueryTest.java:61-74` (11 skipped W3C property-path tests)
- sf side: `crates/sf-sparql/src/iq.rs` `SqlCond::Exists`/`NotExists` (`scans: Vec<Scan>`); ADR-0025 Tier-2 item 1

---

## 2. Multi-branch (UNION) sub-plan as a join / OPTIONAL input

### Ontop's approach

Ontop's IQ optimizer decides, per join, whether to **distribute** the join over a union child's arms or
**keep the union intact** as one derived table — driven by whether the union arms actually disagree on
how a shared variable is *constructed*.

The optimizer is `BottomUpUnionAndBindingLiftOptimizer`
(`core/optimization/src/main/java/it/unibz/inf/ontop/iq/optimizer/impl/BottomUpUnionAndBindingLiftOptimizer.java`),
implementing `UnionAndBindingLiftOptimizer`. It calls `IQTree.liftIncompatibleDefinitions(Variable,
VariableGenerator)` (`core/model/src/main/java/it/unibz/inf/ontop/iq/IQTree.java:39`) on each join child.
The actual distribution logic lives in the node implementations:

- `InnerJoinNodeImpl.liftIncompatibleDefinitions` (`core/model/.../iq/node/impl/InnerJoinNodeImpl.java:184-208`):
  for a child that decomposes into a `UnionNode`, if `UnionNodeImpl.hasAChildWithLiftableDefinition`
  (`.../UnionNodeImpl.java:61-68`) finds at least one arm whose `ConstructionNode` substitution
  constructs the shared variable differently from the others, it builds one join copy per arm and wraps
  them in a fresh `UnionNode`: `(A∪B)⋈C → (A⋈C)∪(B⋈C)`.
- `LeftJoinNodeImpl.liftIncompatibleDefinitions` (`.../LeftJoinNodeImpl.java:172-188`): the same
  distribution, but **only when the union is on the left (mandatory) side** — `if
  (leftChild.getVariables().contains(variable))`. There is no symmetric handling in this class for a
  union on the right/OPTIONAL side, i.e. Ontop's own code does not special-case sf's
  `?x :p ?y OPTIONAL { {A} UNION {B} }` shape any differently than the general "union survives as one
  child" path below.
- If arms are *compatible* (no `ConstructionNode` divergence), the union is left as a direct join child —
  confirmed in `BindingLiftTest.testUnionWithJoin`
  (`core/optimization/src/test/java/it/unibz/inf/ontop/iq/optimizer/BindingLiftTest.java:985-1071`), whose
  expected optimized tree still nests `unionNode4`/`unionNode5` directly inside `joinNode2`.

When the union survives as a join operand, SQL generation renders it as a derived table:
`IQTree2SelectFromWhereConverterImpl.transformUnion`
(`db/rdb/src/main/java/it/unibz/inf/ontop/generation/algebra/impl/IQTree2SelectFromWhereConverterImpl.java:222-228`)
turns it into a `SQLUnionExpression`; the serializer
(`db/rdb/src/main/java/it/unibz/inf/ontop/generation/serializer/impl/DefaultSelectFromWhereSerializer.java:311-327`)
renders it literally as `( (SELECT ... FROM A) UNION ALL \n (SELECT ... FROM B) ) tN` with a fresh alias,
then joined via the surrounding `ON` clause like any other table.

Cross-arm type reconciliation happens at two points, never by trying to unify heterogeneous SQL column
types inside one UNION column directly:

1. **Upstream, at the mapping level.** `TermTypeMappingCaster`
   (`mapping/core/src/main/java/it/unibz/inf/ontop/spec/mapping/transformer/impl/TermTypeMappingCaster.java`)
   rewrites every mapping assertion's lexical term into a canonical `RDF(lexicalTerm, typeConstant)` shape
   with explicit `CAST`s (`transformTopOfLexicalTerm`, lines 109-118) — so different mapping assertions
   that produce "the same" SPARQL variable from an int vs. a bigint column, or from differing URI
   templates, are already normalized to a uniform `RDFTermFunctionSymbol` wrapper *before* any union forms.
2. **When arms genuinely disagree** (e.g. one produces `generateIRIWithTemplate1`, another
   `generateURI2`) and the optimizer distributes the join (§ above), the recombining `ConstructionNode`
   reconciles the incompatible slot with an `ifElseNull(isNotNull(...), ...)`-style CASE construct rather
   than forcing one SQL column type — shown concretely in
   `BindingLiftTest.testLeftJoinAndUnionLiftSubstitution` (lines 559-647).
3. **At SQL-generation time, for the "kept intact" case**, `TypingNullsInUnionDialectExtraNormalizer`
   (`db/rdb/src/main/java/it/unibz/inf/ontop/generation/normalization/impl/TypingNullsInUnionDialectExtraNormalizer.java`)
   detects a variable that's `NULL` in some arms but has a real `DBTermType` in others and injects a typed
   null (`CAST(NULL AS <type>)`) so the rendered `UNION ALL` stays column-type-consistent per dialect.

### The key SQL shape Ontop emits

```sql
-- union kept intact as a join operand (arms compatible):
SELECT ...
FROM t1
JOIN ( (SELECT ... FROM a) UNION ALL
       (SELECT ... FROM b) ) t2
  ON t1.k = t2.k
```

or, when arms are incompatible, the join is distributed and recombined above a fresh top-level `UnionNode`
with `CASE`/`ifElseNull` reconciling the divergent slot — i.e. two structurally full joins unioned
together, not one derived-table join.

### What sf would need

This is a real, reusable Ontop precedent. sf already has the right shape of proof for a *related* case —
`try_sql_group_over_union` (`crates/sf-sparql/src/iq/lower.rs:1409`) computes a `KeyShape` per needed
variable (same `TermSpec` type/datatype/language, and for templates the same injective column structure)
across all union arms before pooling them into one derived table for aggregation. `lower_as_subplan`
(`crates/sf-sparql/src/iq/lower.rs:1010`) is the analogous join/OPTIONAL-operand lowering path, but it
currently hard-refuses a multi-branch inner plan outright (`if prepared.len() != 1 { return
Err(Error::Unsupported(...)) }`) rather than attempting the same cross-arm proof. The Ontop-informed design:

1. **Compatible-arms case (mirrors "kept intact").** Extend `lower_as_subplan` to accept a multi-branch
   inner plan when every needed variable (the join key(s) plus every variable referenced by the outer
   scope) passes a `try_sql_group_over_union`-style `KeyShape` check across all arms — reuse or factor out
   that existing proof rather than reimplementing it. On success, emit the arms as one `UNION ALL` derived
   table exactly as `lower_as_subplan` does today for the single-branch case.
2. **Incompatible-arms case (mirrors "distribute").** When the proof fails, sf's flat/eager-flattening
   model (ADR-0023's own "structural ceiling" critique) can approximate Ontop's join-distribution rewrite
   by pre-flattening: run the join separately against each union arm and re-union the results — this is
   substitution-lifting sf already performs elsewhere in the cascade, just not yet wired into the SubPlan
   path. Unlike Ontop, sf has no `ifElseNull` CASE-reconciliation primitive today for the truly
   incompatible-template case; if that's needed it is new work, though ADR-0025 Tier-2 item 2's own
   framing ("proof of cross-arm TermSpec agreement... before lowering") suggests the *intended* sf scope is
   the compatible-arms case only, sound-501-ing the genuinely incompatible one same as today.
3. **LeftJoin/OPTIONAL right-side note.** Ontop's own `LeftJoinNodeImpl.liftIncompatibleDefinitions` does
   *not* distribute for a union on the OPTIONAL (right) side either — so sf need not build that distribution
   case even for parity; the "keep as one derived table" path (item 1 above) is the one Ontop precedent
   actually covers for `OPTIONAL { {A} UNION {B} }`.

**Provenance:**
- `core/optimization/src/main/java/it/unibz/inf/ontop/iq/optimizer/impl/BottomUpUnionAndBindingLiftOptimizer.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/InnerJoinNodeImpl.java:184-208`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/LeftJoinNodeImpl.java:172-188`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/UnionNodeImpl.java:61-68`
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/algebra/impl/IQTree2SelectFromWhereConverterImpl.java:222-228`
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/serializer/impl/DefaultSelectFromWhereSerializer.java:311-327`
- `mapping/core/src/main/java/it/unibz/inf/ontop/spec/mapping/transformer/impl/TermTypeMappingCaster.java:109-118`
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/normalization/impl/TypingNullsInUnionDialectExtraNormalizer.java`
- `core/optimization/src/test/java/it/unibz/inf/ontop/iq/optimizer/BindingLiftTest.java:559-647,822-861,985-1071`
- sf side: `crates/sf-sparql/src/iq/lower.rs:1010` (`lower_as_subplan`), `:1409` (`try_sql_group_over_union`); ADR-0025 Tier-2 item 2

---

## 3. COUNT(DISTINCT *)

### Ontop's approach

There isn't a working one to document. `AggregationNode` itself
(`core/model/src/main/java/it/unibz/inf/ontop/iq/node/AggregationNode.java`) carries no DISTINCT concept —
DISTINCT lives per-function-symbol in `CountSPARQLFunctionSymbolImpl`
(`core/model/.../term/functionsymbol/impl/CountSPARQLFunctionSymbolImpl.java`), which has a `boolean
isDistinct` field and two variants: an arity-1 symbol for `COUNT(DISTINCT ?x)`, and an arity-0 symbol
intended to cover both `COUNT(*)` and `COUNT(DISTINCT *)`. Ontop does **not** lower `COUNT(DISTINCT *)`
into a `DistinctNode` inserted below `AggregationNode` — the sound, portable strategy sf is considering —
nor does it attempt multi-column `COUNT(DISTINCT col1, col2, ...)` SQL.

Worse, this is a confirmed live bug, not merely an unimplemented feature: in
`RDF4JValueExprTranslator.java` (`core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JValueExprTranslator.java`),
the `expr.getArg() == null` branch for RDF4J's `Count` algebra node calls `getFunctionalTerm(SPARQL.COUNT)`
— the plain, non-distinct arity-0 symbol — **unconditionally**, never consulting `aggExpr.isDistinct()`
(that check exists but only fires 8 lines later, in the `arg != null` branch). Since RDF4J parses
`COUNT(DISTINCT *)` as `Count(arg = null, distinct = true)`, Ontop silently treats it identically to plain
`COUNT(*)` — the DISTINCT is dropped, not merely unsupported.

The arity-0 *distinct* DB symbol still exists in code (`DBCountFunctionSymbolImpl.get0arySerializer`,
`core/model/.../term/functionsymbol/db/impl/DBCountFunctionSymbolImpl.java:~44-49`) and would emit the
non-standard SQL text `"COUNT(DISTINCT(*))"` if it were ever reached — but per the translator bug above,
it never is. No multi-column `COUNT(DISTINCT a,b)` codepath exists anywhere under
`db/rdb/src/main/java/.../generation/`.

The only related test fixture,
`test/sparql-compliance/src/test/resources/testcases-dawg-sparql-1.1/syntax-query/syntax-aggregate-02.rq`
(`SELECT (COUNT(DISTINCT *) AS ?count) {}`), is registered in its manifest as `mf:PositiveSyntaxTest11` —
a parse-only test with no expected result and no expected SQL. Ontop has never had this construct
evaluated in its own test suite.

### The key SQL shape Ontop emits

None — the working path never reaches the arity-0 distinct emitter, so nothing beyond ordinary
`COUNT(*)` is ever generated for this SPARQL construct today.

### What sf would need

Nothing to port. sf's planned rewrite — wrap the aggregation's projected columns in a derived-table
`SELECT DISTINCT col1, col2, ... FROM (...)`, then apply `COUNT(*)` over that derived table (a new
emission path in `iq/lower.rs` → `emit.rs`, per ADR-0025 Tier-2 item 3) — is sf-original engineering with
no Ontop design to match, and no Ontop correctness precedent to lean on. If anything, this section
argues sf's careful, portable derived-table approach is *more* rigorous than the reference engine's
current (broken) handling of the same SPARQL construct.

**Provenance:**
- `core/model/src/main/java/it/unibz/inf/ontop/term/functionsymbol/impl/CountSPARQLFunctionSymbolImpl.java`
- `core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JValueExprTranslator.java` (the `getArg() == null` branch, DISTINCT check omitted)
- `core/model/src/main/java/it/unibz/inf/ontop/term/functionsymbol/db/impl/DBCountFunctionSymbolImpl.java:~44-49`
- `core/model/src/main/java/it/unibz/inf/ontop/term/functionsymbol/db/impl/Serializers.java` (`getDistinctAggregationSerializer`, single-column path only)
- `test/sparql-compliance/src/test/resources/testcases-dawg-sparql-1.1/syntax-query/syntax-aggregate-02.rq` (parse-only fixture)
- sf side: `crates/sf-sparql/src/iq.rs:188` (`AggCol`); ADR-0025 Tier-2 item 3

---

## 4. GROUP BY over a property-path closure

### Ontop's approach

Two separate findings, at two different layers, both needed to answer this honestly.

**Layer 1 — `AggregationNode` is provenance-agnostic by design.** `AggregationNodeImpl`
(`core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/AggregationNodeImpl.java`) never inspects what
kind of node its child is. Its grouping/child variables come from constructor arguments, not from
inspecting the child's type; `validateNode(IQTree child)` only calls the generic `IQTree.getVariables()`
plus one `instanceof TrueNode` short-circuit for trivial leaves. SQL generation
(`IQTree2SelectFromWhereConverterImpl.transformAggregation`, ~line 164) dispatches through the generic
`IQTree` interface exactly like `transformFilter`/`transformDistinct` — there is no special-casing for
"the child is an `ExtensionalDataNode`" vs. anything else. This means, architecturally, `AggregationNode`
over a hypothetical recursive/CTE-backed child would compose for free — no new grouping-key-extraction
code would be needed on Ontop's side if such a child existed.

**Layer 2 — but no such child ever exists in practice.** As established in §1, Ontop's only property-path
translation (`RDF4JTupleExprTranslator.translate(ArbitraryLengthPath)`) supports exactly one predicate
(`rdfs:subClassOf`, via a precomputed closure + `UNION`/`DISTINCT` — never a runtime-recursive relation)
and throws for everything else. A search of all 58 `.rq` fixtures under `test/` containing `GROUP BY`
found none combining a path operator; `DestinationTest.java` and `SubClassOfStarTest.java` separately
exercise `GROUP BY` and `subClassOf*` but never together. So Layer 1's compositional generality is never
actually exercised by a genuinely recursive relation — there is no working Ontop example of "GROUP BY over
a real transitive-closure output" to point to.

### The key SQL shape Ontop emits

None — no test fixture or generation-layer evidence of this combination exists to quote.

### What sf would need

The transferable lesson is architectural, not a literal port: Layer 1's finding — that `AggregationNode`
composes over *any* child uniformly — validates the design direction ADR-0025 Tier-2 item 4 already
points at ("path-as-derived-table + aggregation-over-SubPlan"), rather than suggesting a special
path-aware grouping mechanism. Concretely: once sf's property-path CTE output is exposed as a `SubPlan`
(the same primitive gap #2 needs — a derived table with a known projected-column list), sf's *existing*
`lower_as_subplan` + ordinary aggregation-over-SubPlan composition should handle grouping over it with no
path-specific code, mirroring how Ontop's `AggregationNode` needs no path-specific code once *any* relation
is beneath it. In other words: gap #4 is not a fourth independent mechanism to build — it is subsumed by
making property paths lowerable via the same SubPlan machinery gap #2 needs, plus wiring
`group_key_columns`/`single_column_of` (`crates/sf-sparql/src/iq/lower.rs`) to read a SubPlan's exposed
`c{i}` columns instead of requiring raw base-table columns. This resequencing (build SubPlan-over-path once,
get gap #4 nearly free) is worth weighing against ADR-0025's current item ordering, which treats items 2 and
4 as separate M1/M2 milestones.

**Provenance:**
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/AggregationNodeImpl.java` (provenance-agnostic `validateNode`)
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/algebra/impl/IQTree2SelectFromWhereConverterImpl.java:~164` (`transformAggregation`, generic dispatch)
- `core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JTupleExprTranslator.java:274-328` (path translator, `subClassOf`-only — see §1 for detail)
- test search: no `.rq` fixture under `test/` combines `GROUP BY` with a path operator (58 `GROUP BY` fixtures checked, none matched)
- sf side: `crates/sf-sparql/src/iq/lower.rs` (`group_key_columns`, `single_column_of`, `lower_as_subplan`); ADR-0025 Tier-2 item 4

---

## 5. Post-GROUP-BY expression over a UNION aggregate

### Ontop's approach

A post-aggregation expression (e.g. `SUM(?x)/COUNT(?x)`) is an ordinary `ConstructionNode` sitting above
`AggregationNode` — and this is structurally enforced, not just conventional.
`AggregationNodeImpl.validateNode()` (`core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/AggregationNodeImpl.java:191-194`)
throws if any substitution entry is not a pure aggregate-function call
(`substitution.rangeAnyMatch(t -> !t.getFunctionSymbol().isAggregation())`) — a compound expression over
two aggregate outputs is illegal *inside* `AggregationNode` by construction, so it necessarily lives one
level up. The translator confirms the wiring: `RDF4JTupleExprTranslator.translate(Group group)`
(lines 173-205) builds `AggregationNode` from pure `GroupElem`s only; a SPARQL `(SUM(?x)/COUNT(?x) AS
?avg)` parses (via RDF4J) as `Extension(Group(...))`, and `translate(Extension node)` (lines 772-809)
wraps the aggregation subtree in an ordinary `ConstructionNode` whose substitution computes the division
from the two aggregate-output variables.

**The critical structural fact**, and the one that actually explains why sf hits this gap and Ontop
doesn't: Ontop **never executes aggregation outside the generated SQL.** There is no in-memory/JVM-side
GROUP BY executor anywhere in the codebase (searched `engine/` and `core/` for any grouping/merging
executor class — none exists). The execution pipeline is a single linear hand-off: `QuestStatement` →
`QueryReformulator.reformulateIntoNativeQuery` → `QuestQueryProcessor.reformulateIntoNativeQuery`
(`engine/reformulation/core/.../QuestQueryProcessor.java:79-158`) → `NativeQueryGenerator.generateSourceQuery`
→ handed directly to `JDBCConnector` for execution. The one Java-side post-processing mechanism that does
exist, `ProjectionSplitter`/`PostProcessingProjectionSplitterImpl`
(`engine/reformulation/core/.../PostProcessingProjectionSplitterImpl.java`,
`core/optimization/.../splitter/ProjectionSplitter.java`), is scoped only to peeling off a top
`ConstructionNode` for a *scalar row-wise* function the target dialect can't express — it can never peel
off (or re-implement) an `AggregationNode`. If a query truly cannot be fully translated with zero
post-processing where required, Ontop throws `NotFullyTranslatableToNativeQueryException`
(`SQLGeneratorImpl.java:272`) rather than falling back to any in-process aggregator. **Ontop has no
equivalent of sf's `rust_group` at all.**

SQL generation reflects this directly: `IQTree2SelectFromWhereConverterImpl.convert()`
(`db/rdb/src/main/java/it/unibz/inf/ontop/generation/algebra/impl/IQTree2SelectFromWhereConverterImpl.java:42-94`)
composes the outer `ConstructionNode` substitution with the `AggregationNode` substitution into one SQL
projection list (`s2.compose(c.getSubstitution()).restrictDomainTo(c.getVariables())`, lines 55-64), and
the remaining child tree (`UnionNode`, if present) is converted via `transformUnion` into one `UNION ALL`
derived table (see §2). The division/expression and the `GROUP BY` end up in exactly one `SELECT`
statement — ordinary SQL syntax, requiring no special handling for the "union beneath the aggregation"
case, because there is only ever one SQL statement, period.

### The key SQL shape Ontop emits

```sql
SELECT g, SUM(x) / COUNT(x) AS avg
FROM ( (SELECT ... FROM a) UNION ALL
       (SELECT ... FROM b) ) t
GROUP BY g
```

No fixture combining UNION + GROUP BY + a two-aggregate post-expression was found in Ontop's own test
suite (checked all 2473 `.rq` files); the closest relatives
(`test/lightweight-tests/.../distinctInAggregates/varianceDistinct.rq`, `stdevDistinct.rq`) union-feed a
`GROUP BY` but each SELECT item is a single aggregate call, not a function of two aggregate outputs, and
carry no expected-SQL fixture (they assert against a live DB).

### What sf would need

This finding reframes ADR-0025 Tier-2 item 5's diagnosis. The proposed fix ("a new post-group expression
evaluator in the Rust executor," `crates/sf-sparql/src/exec_core.rs` around `rust_group_execute` /
`rust_group_result_rows` — note the ADR's citation of `crates/sf-sql/src/exec.rs` is stale, that file does
not exist at that path in the current tree) treats the symptom. Ontop's design shows the actual
architectural fix is upstream: **never let the union-fed aggregation fall to `rust_group` in the first
place.** That is exactly the same cross-arm pooling generalization gap #2 needs
(`try_sql_group_over_union`'s `KeyShape` proof, widened and reused). Every UNION shape that proof can
promote into one SQL derived table gets post-agg expressions "for free," the same way Ontop's does — no
Rust-side expression evaluator needed at all for those cases, because ordinary `emit.rs` SQL projection
logic already handles post-agg SELECT expressions for the single-branch/promoted-union case today. A Rust
executor expression evaluator is still worth building as the fallback for the residual case where cross-arm
pooling is provably unsound (heterogeneous term construction that Ontop itself would have to distribute-
and-CASE-reconcile, §2) — but per this dossier, that residual case should be the *minority* path, not the
primary fix.

**Provenance:**
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/AggregationNodeImpl.java:191-194` (`validateNode`, pure-aggregate-substitution enforcement)
- `core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JTupleExprTranslator.java:173-205,772-809` (`translate(Group)`, `translate(Extension)`)
- `engine/reformulation/core/src/main/java/it/unibz/inf/ontop/answering/reformulation/impl/QuestQueryProcessor.java:79-158` (single linear SQL-only pipeline)
- `engine/reformulation/core/src/main/java/it/unibz/inf/ontop/answering/reformulation/generation/PostProcessingProjectionSplitterImpl.java`, `core/optimization/.../splitter/ProjectionSplitter.java` (scalar-only post-processing, never aggregation)
- `.../generation/impl/SQLGeneratorImpl.java:272` (`NotFullyTranslatableToNativeQueryException` — fail rather than fall back to Java)
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/algebra/impl/IQTree2SelectFromWhereConverterImpl.java:42-94` (outer/aggregation substitution composition into one SQL projection)
- `test/lightweight-tests/src/test/resources/distinctInAggregates/varianceDistinct.rq`, `stdevDistinct.rq` (closest, non-matching fixtures)
- sf side: `crates/sf-sparql/src/iq/lower.rs:1409` (`try_sql_group_over_union`), `crates/sf-sparql/src/exec_core.rs:207,1016` (`rust_group_execute`, `rust_group_result_rows` — actual location; ADR-0025's `sf-sql/src/exec.rs` citation is stale); ADR-0025 Tier-2 item 5

---

## Sources

**Ontop source** (`/Users/henrik/source/ontop`, read at the commit checked out locally):
- `core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JTupleExprTranslator.java`
- `core/kg-query/src/main/java/it/unibz/inf/ontop/query/translation/impl/RDF4JValueExprTranslator.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/AggregationNode.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/AggregationNodeImpl.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/InnerJoinNodeImpl.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/LeftJoinNodeImpl.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/UnionNodeImpl.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/node/impl/ConstructionNodeImpl.java`
- `core/model/src/main/java/it/unibz/inf/ontop/iq/IQTree.java`
- `core/model/src/main/java/it/unibz/inf/ontop/term/functionsymbol/impl/CountSPARQLFunctionSymbolImpl.java`
- `core/model/src/main/java/it/unibz/inf/ontop/term/functionsymbol/db/impl/DBCountFunctionSymbolImpl.java`
- `core/model/src/main/java/it/unibz/inf/ontop/term/functionsymbol/db/impl/Serializers.java`
- `core/optimization/src/main/java/it/unibz/inf/ontop/iq/optimizer/impl/BottomUpUnionAndBindingLiftOptimizer.java`
- `core/optimization/src/main/java/it/unibz/inf/ontop/iq/optimizer/splitter/ProjectionSplitter.java`
- `core/optimization/src/test/java/it/unibz/inf/ontop/iq/optimizer/BindingLiftTest.java`
- `mapping/core/src/main/java/it/unibz/inf/ontop/spec/mapping/transformer/impl/TermTypeMappingCaster.java`
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/algebra/impl/IQTree2SelectFromWhereConverterImpl.java`
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/serializer/impl/DefaultSelectFromWhereSerializer.java`
- `db/rdb/src/main/java/it/unibz/inf/ontop/generation/normalization/impl/TypingNullsInUnionDialectExtraNormalizer.java`
- `engine/reformulation/core/src/main/java/it/unibz/inf/ontop/answering/reformulation/impl/QuestQueryProcessor.java`
- `engine/reformulation/core/src/main/java/it/unibz/inf/ontop/answering/reformulation/generation/PostProcessingProjectionSplitterImpl.java`
- `test/sparql-compliance/src/test/java/it/unibz/inf/ontop/test/sparql11/MemorySPARQL11QueryTest.java`
- `test/sparql-compliance/src/test/resources/testcases-dawg-sparql-1.1/syntax-query/syntax-aggregate-02.rq`
- `test/lightweight-tests/src/test/resources/distinctInAggregates/varianceDistinct.rq`, `stdevDistinct.rq`

**semantic-fabric source and design docs:**
- `docs/adr/ADR-0025-ontop-parity-residue-closure.md` (the Tier-2 gap catalogue this dossier grounds)
- `docs/adr/ADR-0023-query-ir-architecture-flat-ucq-vs-iq-tree.md` (the operator-tree IR sf lowers to)
- `docs/design/ADR-0023-M4-optionb-worklist.md` (Tier-3 cosmetic-parity worklist, adjacent context)
- `crates/sf-sparql/src/iq.rs` (`SqlCond`, `AggCol`, `GroupKey`)
- `crates/sf-sparql/src/iq/lower.rs` (`lower_as_subplan`, `try_sql_group_over_union`)
- `crates/sf-sparql/src/exec_core.rs` (`rust_group_execute`, `rust_group_result_rows`)
