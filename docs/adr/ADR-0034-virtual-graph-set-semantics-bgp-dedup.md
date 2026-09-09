---
status: accepted
date: 2026-07-19
updated: 2026-09-09
tags: [set-semantics, bgp-dedup, duplicate-rows, soundness, union-dedup, key-elision]
supersedes: []
depends-on:
  - ADR-0007
  - ADR-0025
  - ADR-0032
  - ADR-0033
implements: []
---

# Virtual-graph set semantics: BGP-level dedup for duplicate rows and cross-map same-triple emission

## Current wrapper boundary (2026-09-08)

D1 DISTINCT and D2 rendered-width wrappers use `ScanSource::Projection`: owned
input scan, ordered raw-column/template recipes, NULL or bound native-equality
guards and a DISTINCT flag. SQL is emitted only after original Table/Query metadata is
available. Generated wrappers are not authored queries or catalog/constraint
authority. Stable quoted output labels carry resolved native columns through
nested wrappers; PostgreSQL synthetic rowid becomes CTID only at a table leaf.

Raw columns retain native descriptors. Live-proven text template operands now
normalize to decoder text; transparent non-text operands retain their descriptors
on PostgreSQL/SQLite. Template encoding and parameter isolation remain.
Only guard-free same-named raw-column DISTINCT over one table may be restored by
the existing bounded-join proof. Its portable row authorization
adds same-input native-equality guards before dedup, without widening RDF outputs
or keys; nonempty guards revoke restore authority. The separately proved rendered
IRI atom below admits policy insertion but never table restoration. Other
computed/nested wrappers gain no such authority. Shared RDF-term dedup follows relation aliases, not
authored-source authority, and remains source-sized/fail-closed on serving paths.

Required owned TLS public SELECT now resolves mapping SRC/DST against native
lowercase columns on PostgreSQL16.15/MySQL8.4.11. Boundary tests cover original
probes, nested/conditional wrappers, missing/ambiguous names before cursors,
descriptor propagation, parameter separation and clone-work measurement.
Ordinary text-only D1 comparisons now use decoded, exact keys in a native window
partition while returning original raw values. SELECT DISTINCT and SQL-pooled
outputs normalize text keys too; SQLite metadata twins preserve decoder identity.
Required public SELECT/ASK/COUNT and JOIN/OPTIONAL/EXISTS/NOT EXISTS/MINUS checks
cover SQLite NOCASE plus pinned PostgreSQL/MySQL native collations and CHAR widths.

`NativeColEq` and `NativeCmp` preserve authored Ref joins and row-policy predicates.
`ScanSource::RefAtom` now seals the complete native child/parent relation before
D1, joins and projection narrowing. Both original leaves and their policies are
filtered before decoded-key window dedup, enabled only when every key has live
text/CHAR decoder proof (or the key tuple is empty). Unknown families retain the
previous per-source D1/native join, preserving distinct signed-zero IRIs rather
than applying SQL numeric equality. Policy-only/join-only columns cannot widen
its RDF key. Raw outputs retain individual decoder descriptors, including
child-owned graph scope for parent-generated blank nodes. Parameters follow SQL
text order through nested projections, OPTIONAL and EXISTS. A Ref remains on the
established OPTIONAL decomposition path and never gains table/constraint authority.
Column-level origins preserve PostgreSQL pooling without inspecting generated SQL.
Join-less references retain their established path. The unknown-family fallback
clones only the admitted two-source plan payload, not source rows; it adds no
claim of compiler-work metering or total source-work control.

The formerly ignored Ref-witness regression is now active and passing. Required
HTTP checks cover conflicting collations, projection bags, policy ordering,
CHAR padding, named blank-node scope and unknown-family signed-zero preservation; owned PostgreSQL16.15/MySQL8.4.11 checks
cover folded columns, SELECT/DISTINCT/COUNT, fixed values and correlations. The
rejected universal SubPlan experiment remains unintegrated. These proofs cover
fixed injective templates with decoder-proven text/CHAR keys, not universal term
identity: base-resolved column IRIs and general mixed/natural scalar keys remain
open, as do synthetic row identity, total source-work controls and exact-release
qualification. No broad admission is promoted.

**Ordinary policy correction (2026-09-08):** native row equality runs inside the
existing D1 projection before raw representative selection. The newly enabled
policy window initially required live text/CHAR proof for every projected key;
the proven SQLite lexical subset below extends this without a partial-key gate.
Public SELECT/COUNT, projection bags, correlations and
policy-before-representative checks pass, as do owned PostgreSQL16.15/MySQL8.4.11
text/CHAR policy queries. Missing guard columns and wrong operators/aliases reject
before cursors; parameters follow SQL order through nested/conditional projections.
A virtual-column fixture verifies native signed-zero values and catches removal
of the all-key gate. Raw descriptors remain intact; general key identity is not closed.

**SQLite lexical mixed-key correction (2026-09-08):** D1 now captures original
term consumers before synthetic raw-column recipes erase that information. With
live SQLite declaration/storage-fallback and padding facts, decoded lexical keys use
the same Rust decoder as result rows, not SQL CAST or numeric equality. Signed
zero IRIs survive while integer/REAL lexical duplicates and CHAR duplicates
collapse before projection; raw outputs and descriptors remain unchanged. A
query-owned scalar charges source work, retains cancellation/lease ownership and
cleans up on completion, drop and errors without replacing application callbacks.
Lexical identity does not license literal value comparisons: original IRI-template
and explicit column-literal consumers supply D1 keys, including constant-only
literal constraints. Natural literals, blank nodes or column IRIs revoke that
raw lexical proof. Literal conditions retain construction specs and distinguish
RDF identity from FILTER values; they confer no raw-key optimizer authority.
Required public SELECT/COUNT, self-join/OPTIONAL/UNION, BLOB/date and
numeric-filter checks pass. Owned PostgreSQL/MySQL mixed integer/case-insensitive
text checks protect the existing native window path. This does not promote general natural
literal identity/value comparison, base-resolved column IRIs, native scalar identity,
mixed-decoder SubPlans/paths or unknown Ref keys. The single-slot static
template/constant FILTER boundary is extended by the correction below; no release flag changes.

**Literal comparison correction (2026-09-09):** explicit SQLite column literals
now compare decoded lexical/datatype/language tuples for BGP identity and sameTerm,
preserving NULL expression errors under negation/OPTIONAL. Signed-zero terms remain
distinct through SELECT/COUNT, constant matching and joins; numeric FILTER equality
matches both. IRI/literal dual-use keeps identity distinct from value predicates.
The query-owned Rust numeric callback compares the four base numeric datatypes with
SPARQL promotion, exact integer/decimal arithmetic and NaN semantics. Invalid
lexicals produce expression error; representational overflow fails closed. Other
FILTER families keep their previous decoded-text/native-value path and cannot
borrow D1 lexical authority. Missing Ref/SubPlan decoder facts do not fabricate
TEXT callback inputs. Literal/IRI plain-column FILTER mismatches never fall back
to identity-aware raw SQL equality; a condition-owned expression error preserves
unbound behavior under NOT/OPTIONAL. Numeric VALUES use the same Rust promotion
on all dialects and retain existing nonnumeric VALUES variable-pair equality.
Datatype/language identity components use byte-exact, NO PAD comparison.
Natural SQLite constant matching uses declared decoder
datatype/canonicalization; undeclared natural serving maps remain rejected by
existing admission. Native unknown natural descriptors keep legacy comparison,
not invented xsd:string. General natural derived-pair identity, native numeric
identity/value semantics and descriptor propagation remain required follow-up;
the retained paths are compatibility boundaries, not completion evidence.

**Resolved column-IRI correction (2026-09-09):** `IriCmp` preserves each operand's
base for constant matching, BGP identity, `=` and `sameTerm`. Live SQLite keys
decode and generate through shared Rust functions; ordered IRI comparisons remain
expression errors. Source UNIQUE keys cannot elide base-resolved RDF dedup.
D1 retains separate resolved/decoded keys for multiple consumers and keeps raw
outputs. Ref atoms join/filter first, then dedup resolved subjects and decoded
blank labels; original SQLite descriptors survive outer predicates. Unknown
consumer keys cannot silently fall through to raw DISTINCT.
The existing standalone reconstructed-term fallback admits column IRIs, not
noninjective IRI templates. Cross-map groups retain complete pattern keys,
including hidden graph variables; source-sized fallback remains rejected by
serving. Compiler projection-layout inspection no longer emits speculative SQL
without live decoder authority. Public regression coverage includes SELECT/COUNT,
constant/BGP/FILTER matching, unique keys, unbound errors and native Ref witnesses;
W3C compatibility expectations remain unchanged. Native resolved-key execution,
general mixed/natural identities and broader DISTINCT/GROUP/SubPlan qualification
remain open. The late-template update below narrows its earlier gap. Processor-base configuration
now flows independently of Turtle document bases through the public parser and
both serving sources, with exact compiler identity and reload retention. The shared
column generator follows R2RML §11.2 verbatim prefixing, not RFC3986 normalization;
relative `../x` and absolute `base/../x` dedup together, but not with `base-parent/x`.
Required authenticated CLI/HTTP and parser tests cover this correction. No
release/admission flag is promoted by this slice.

**Template separator correction (2026-09-09):** nonempty separators are not an
injectivity proof. ASCII unreserved text, Unicode and percent triplets can occur
inside encoded substitutions; `{a}-{b}` has real collisions. The proof now requires
a delimiter absent from encoded values. Unit collision witnesses cover `-` and `%`;
fixtures intended to test injective grouping/transitive pooling now use proven
delimiters while retaining their original grouping, width and collision assertions.
For a single SQLite atom containing only static IRI-template bindings/constants
and NULL guards, D1 renders every bound RDF key inside the existing typed Projection
before joins or final projection. Live decoders precede percent encoding; exact
rendered keys dedup in SQL, including final IRI validation under COUNT. Synthetic
outputs have generated string/no-padding authority, never source constraints.
A separate original-table proof places portable policy predicates before ranking;
policy-only columns are not RDF keys. Required authenticated tests cover collisions,
SELECT/DISTINCT/COUNT, hidden graph/object bags, rendered-to-rendered BGP/FILTER,
CHAR padding, signed zero, NULL/empty values and policy-before-dedup. No source-sized
Rust set or release-admission promotion is introduced. Native rendered atoms,
multi-arm noninjective pooling and general same-shape template unification remain
open; this is not universal template identity.

**Unicode template correction (2026-09-09):** reconstruction and SQL identity
now share the RFC3987 `ucschar` alphabet (ADR-0015), excluding C1 controls,
private-use and noncharacters rather than passing every non-ASCII byte through.
Required SQLite HTTP SELECT/COUNT distinguishes escaped private-use characters
from literal percent text and deduplicates repeated generated terms. The owned
native CLI profile checks actual PostgreSQL/MySQL results with and without
different-shape `=`/`sameTerm` filters and with a negated-filter COUNT. Malformed
SQLite UTF-8 remains an error, including after NUL; allowed Unicode remains raw.
Existing limits and release flags are unchanged. Late processor-base selection,
same-shape/mixed template comparisons and the other open identities above are
not closed by this alphabet repair.

**Late-template atom correction (2026-09-09):** the parser/core now implement
post-expansion processor-base selection (ADR-0015). Typed IRI operands retain
literal parts, individual source-column aliases and the base. Live SQLite
decoding, encoding and finalization precede equality; visitors, alias rewrites
and plan accounting retain each part. Static/static native lowering is unchanged.
Single-source IRI-template/constant atoms seal finalized keys before projection,
including constant-bound subjects whose original recipe now lives only in a
condition. Source-local IRI conditions remain inside the typed projection; NULL,
data errors and hidden object/graph bags survive SELECT/COUNT and correlation.
Already sealed atoms are not wrapped again or given source-constraint authority.
Portable row policies filter original rows before dedup, including fully bound
queries. Mixed rendered/static BGP and FILTER comparisons now use whole IRIs.
Newly introduced OPTIONAL constants use the existing matched/unmatched
decomposition so an absent match stays unbound, including under negation.

Required commands: `cargo test --locked -p sf-sparql --test late_templates`
and `cargo test --locked -p sf-serve --test query_budget late_template_identity`.
They cover differing processor bases, fixed matching, bags, policy, invalid
expansions, zero-slot callback ownership and qualified rejection boundaries;
unit tests separately cover per-part alias rewrites and callback registration.
Native resolved-template execution, mixed natural/literal-column atoms, reference
atoms, recursive hop identity, late proposition components and wider pooling are
not qualified. They reject instead of borrowing raw tuple identity. The same-shape
static-template and broader identity backlog remains open; no release flag changes.

**Static-template constant correction (2026-09-09):** single-slot static IRI
templates use `IriCmp`, decoding then encoding substitutions exactly once and
comparing the complete IRI byte-exactly. Raw `a/b` must match `a%2Fb`; raw
`a%2Fb` must instead match `a%252Fb`. Lowercase/noncanonical escapes are distinct.
SQLite retains every live storage/declared/padding decoder, including mixed
INTEGER/REAL/BLOB and signed zero. Native text/CHAR reuses its own decoder;
native scalars carry a separate live wire-type lexical proof through raw scans,
Ref outputs and compatible SubPlan positions. Names-only refresh revokes that proof;
it never licenses text comparisons, native-join substitution or source constraints.
Authenticated SQLite tests cover fixed subject/object matches, reversed equality,
sameTerm, negation, NULL, empty/NUL/Unicode, COUNT and OPTIONAL. Required owned
PostgreSQL16.15/MySQL8.4.11 CLI checks add encoded text, CHAR and integer spelling
(`1` versus `01`, `%2B1`, `1.0`), alongside the existing native profile aggregate.
Signed integer limits, unsigned u64, MySQL YEAR zero and ZEROFILL are checked:
an exact decimal intermediate removes native display padding without narrowing.
Static multi-slot constant and template/template boundaries are unchanged.
The required native aggregate also restores PostgreSQL boolean/BYTEA and MySQL
binary string/blob/NEWDECIMAL fixed/`=`/`sameTerm` matches, COUNT, OPTIONAL and NULL/negation.
Tests compare listed decoder values with lookup results: empty/leading/trailing
zero bytes, uppercase hex, true/false, full decimal precision/scale and ZEROFILL.
SQL AST round-trips and metadata tests protect raw projection, names-only revocation,
provider separation and loss of decimal proof across scale-coercing UNIONs.
MySQL BIT, PostgreSQL numeric and floating/temporal recipes remain unqualified:
plain numeric casts would bypass PostgreSQL decoder errors for NaN/infinity;
BIT needs byte-width proof, and temporal/float SQL spelling is not Rust spelling.
Unsupported recipes can reject formerly valid
native lookups; restoring their decoder-exact behavior remains required work,
not a scope deferral or a whole identity/release gate closure. Path-endpoint FILTERs
retain pre-source rejection through OPTIONAL ON/outer and SubPlan projections;
the check follows the referenced output, not unrelated columns sharing a path query.
Focused commands: `cargo test --locked -p sf-serve --test query_budget` and
`cargo test --locked -p sf-cli --no-default-features --test source_tls_live native_static_template_constants_are_exact -- --ignored --test-threads=1`.

## Implementation status (2026-07-19, same day — accepted, implemented, Run 4 C0)

All 9 red-phase cells green against the spareval oracle: 34→4, 4→3, 66→3, 34→3,
130→3, 514→3, 4→3 (CONSTRUCT), 405→2, plus the bare-reifies twin. New general
locks: plain-pattern duplicate-row `=_bag` + COUNT-below-GROUP-BY (dedup lands
under aggregation), frozen-schema PK-covered elision (no DISTINCT in emitted SQL), disjoint
arms stay unpooled (no UNION), and the D2 non-injective+non-disjoint sound-501
pin. Gates: `differential_star` 65/0, `differential_tree` 178/0,
`differential_paths` 23/0, `adversarial_adr0033_refute` 24/0, observers 6/0.

**Serving reconciliation (2026-09-01).** The D1/D2 algorithm and its historical
frozen-fixture tests remain implemented. Current `sf-serve`, however, constructs
`CompilerSchema` with `ConstraintAuthority::Unverified`; PK, UNIQUE, FK,
functional-dependency and NOT-NULL observations are removed before translation.
D1 therefore cannot use PK-covered elision in serving and keeps its conservative
duplicate-safety path (a derived-table `DISTINCT` for injective scans). D2
template-disjointness is structural and unchanged.
Raw translation/conformance APIs may still exercise key elision with an explicit
frozen `TableSchema`, but that is compiler/test capability, not product serving
authority. Direct Mapping conformance likewise generates mappings from its frozen
fixture schema; current `sf-serve` consumes authored R2RML and does not generate
Direct Mapping from a mutable source.

**Where it lives:** D1 proof `cascade::force_distinct_for_dup_safety` /
`scan_key_covered`; flat hook `unfold::bgp` + tree hook
`iq/resolve.rs` `Intensional` arm (both BEFORE joining/projection narrowing —
timing is load-bearing); aggregate wrap `cascade::dedup_before_aggregate` (the
below-GROUP-BY commitment); D2 pooling `unfold::pool_pattern_relation` (flat) /
`Filter{Distinct{Union}}` bridge (tree — the `Filter` wrapper dodges
`lower_spine`/`normalize` unwrapping), both funneling into the pre-existing
UNION-dedup + injectivity gates.

**Review-hardened during landing:** (1) composite-key coverage now unions the
columns of every *individually-injective* binding on the alias (two rows
agreeing on all injective outputs must agree on each binding's read columns —
contrapositive — hence on the union; union covers a declared key ⇒ same
physical row). The single-binding-covers-key original missed `om_mid`-shaped
keys split across two variables. (2) `(Const,Const)` case added to arm
disjointness. (3) A `leftjoin` guard widened: D1's INNER-joined SubPlans now
take the same shared-reads check as LEFT-joined ones (converted a malformed-SQL
crash into a sound 501).

**Scope guards:** EXISTS/NOT-EXISTS/MINUS bodies are exempt (existence and
anti-join questions are duplicate-insensitive — §18.4/§8.3); NPS hops keep
§18.2.2 bag semantics via `Branch.nps` (D1's flag never ORs across an
NPS-carrying merge). Path closures never reach D1 at all (they resolve via
`IqNode::Path`; their relations are set-semantic by construction). If D1 is
ever re-applied post-`convert_path_branches`, the correct per-scan
set-semantics formula is `!b.nps` at the conversion point — worked out, not
shipped (no reachable call site today).

**Update (same day, Wave C0b) — D1 rebuilt as a per-scan derived-table wrap
after the r2 refute pass proved the branch-flag design wrong on flat.** The
per-Branch `distinct` flag degraded into a projected-subset final DISTINCT
after projection narrowing (under-count) and was dropped across NPS merges
(over-count) — both flat-only wrong answers, tree was correct. D1 now
rewrites the uncovered SCAN itself: `Table("t")` →
`Query("SELECT DISTINCT sfs{alias}.cols … FROM t")`, alias-preserved
(ADR-0033 precedent), columns = the branch's full per-alias read set
(bindings + where_conds + OPTIONAL ons + SubPlan correlations;
`cascade::alias_used_columns`). Survives merges, NPS, and projection by
construction. **Injectivity gate**: the wrap fires only when every binding
on the alias is injective (raw-tuple DISTINCT equals term dedup only then —
the C.3 argument); non-injective aliases keep the old branch-flag path,
whose C.3 sound-501 still guards them (the W3C TC0005b carve-out remains
load-bearing — verified by instrumentation, not assumption). Because the
decision function is shared, the tree got the same fix for free and TWO
pinned completeness costs UN-PINNED (the M5 group-by shape and the unkeyed
OPTIONAL-right-path both answer again, oracle-verified). Also landed: §16.2
CONSTRUCT set-dedup at SQL level when the template's vars are a strict
subset of the pattern's and bnode-free (projection-level DISTINCT is
correct for CONSTRUCT's set output, wrong for SELECT — streaming-safe,
constant-memory invariant intact). Residual, flagged in-code: cross-branch
CONSTRUCT same-triple dedup (two maps instantiating one triple) is
unimplemented. Found during landing: SQLite's bare-double-quoted-identifier
string-literal fallback would have silently converted a wrap-projection
typo into a bogus value — wrap columns are alias-qualified, which restores
hard errors on every dialect.

**Update (same day, Waves C0c+C0d) — W3C conformance restored to baseline 62;
phase 2 implemented.** C0's D2 pooled ALL arms the moment ANY pair failed
disjointness; now `disjoint_groups` (union-find connected components) pools
only genuinely-colliding groups, singletons stay bag-union — plus term-kind/
language disjointness (`term_specs_disjoint`; datatype deliberately excluded —
a pinned refute test requires datatype-mismatched pairs to route through the
agreement gate). Phase 2, both mechanisms: (A) **term-level rust-side dedup**
as the third D1 path — `distinct` + non-injective + STANDALONE branch (≤1
row-contributor, no join partner: post-exec dedup of a joined relation would
dedup too late — those keep C.3) executes without SQL DISTINCT and dedups on
the full reconstructed solution tuple, an O(distinct-results) HashSet that
deliberately relaxes the constant-memory invariant for exactly this class;
restricted to Literal/BlankNode non-injectivity (**the IRI-exclusion rule**:
IRI templates can always be made injective by adding a separator — RFC-3987
escaping protects it — so IRI shapes stay 501 as an avoidable mapping-design
gap, preserving the C.3/s2a adversarial pins). Group extension covers D2
multi-arm pools (UNION ALL + outer term-dedup). (B) **rendered-projection
pooling** for width-mismatched arms: each arm projects its RENDERED term per
shared var (reusing the percent-encoding machinery), making widths uniform by
construction; gated on term classes where rendered-lexical+kind is term
identity. The flat/tree C.3 dump carve-out is RETIRED (proven dead after the
fix). W3C construct conformance: 53→62/63 adjudicated (baseline met); every
restored case verified by full oracle isomorphism, not just non-error.

**Update (2026-07-20, C0e) — PG lane restored, `w3c_pg_suite` 1/1.** Three
mechanisms, all in the wrap/pooling SQL builders: (1) the wrap now mirrors
`colref`'s rowid→`ctid` translation (12 Direct-Mapping cases); (2) bare
`rr:sqlQuery` aliases fold on PG — `col_is_unquoted_alias` (a precise
byte-scan, not a name-shape heuristic: the broad version regressed 4 cases
and was narrowed) drives quoting in the wrap and the rendered pooling, and a
new `synthetic_subplan_catalog` seeds folded names into `Plan::emitted()`'s
previously-EMPTY SubPlan catalog (a gap since ADR-0023 M5, exposed only now
that D2 routes these shapes into SubPlans); (3) R2RMLTC0012e is a SOUND
PG-only refusal (`group_has_unsafe_float_slot_mismatch`): pooling a
float-family slot against text forces one UNION column type, and PG's
`float8out` lexicalization (scientific notation, `-0.0`) provably diverges
from the native read path's Rust formatting — a CAST would risk silent
lexical drift, live-disproven rather than assumed. NOTE the honest
accounting: main DID pass 0012e on PG (bag-union output, set-isomorphic
W3C comparison), so this refusal is a real single-case completeness
regression, taken deliberately over a wrong-answer risk; `R2RML_PG_BASELINE`
57→56 with the rationale in the test file. Restoration path (ledgered):
cross-branch SHARED seen-set term-dedup for standalone top-level groups —
executing the group's arms as separate per-branch queries with one shared
dedup set needs no SQL UNION at all, sidestepping the type-alignment wall.

**Known completeness costs (sound 501s, pinned, with restoration paths):**
(1) GROUP-BY-over-multibranch-OPTIONAL on unkeyed tables — D1's dedup wrap
routes through the SubPlan mechanism and hits the ADR-0023 M5 boundary; both
engines now honestly refuse (`differential_tree` pin). Restoration: a tagged
bare-DISTINCT `IqNode` distinct from the SubPlan mechanism (a lightweight fast
path was built, fixed 6 shapes but broke 8 `item1d_*` sound-501s relying on
SubPlan wrapping — net loss, reverted; the tag is the right fix). (2) W3C
TC0005b dump: a NON-injective blank-node template (`{fname}_{lname}`) on an
unkeyed table — tree surfaces ADR-0025 C.3 at translate time, flat answers
lazily (correct on collision-free data); pinned as a documented Ok/Err
asymmetry. Restoration: term-level rust-side dedup for non-injective
templates. (3) The unkeyed OPTIONAL-right-path variant is pinned as a sound
501 (`differential_paths`); the keyed forms of all these shapes work — the
path-suite fixtures gained their semantically-faithful PKs.

## Context and Problem Statement

R2RML defines the output dataset as an RDF **graph — a set of triples**. SPARQL
§18.3 evaluates a BGP over that set: each distinct solution mapping μ with
μ(BGP) ⊆ G has cardinality **1** (the instance-mapping multiplicity clause
concerns blank-node instance mappings, not repeated triples — a duplicate source
row does not create a second triple, and two maps emitting the same triple
still describe one triple). The engines instead return one solution per
**source-row combination**: a duplicate row in a logical table, or two candidate
maps producing the identical triple, inflate the answer bag. The spareval oracle
(evaluating the decoded graph, which materializes as a set) is right; the
engines are wrong. A3 proved this is **general R2RML behavior, not
star-specific** — the plain-pattern baseline diverges 4v3 with one duplicated
row; star's extra shared-variable join positions only amplify the same
mechanism multiplicatively (66v3, 130v3, 514v3).

Every prior `=_bag` gate passed only because no fixture ever contained (D1) a
logical source with duplicate rows over the projected columns, or (D2) two
candidate maps agreeing on a triple.

## Decision

Dedup at the **BGP-block boundary**, where SPARQL's own semantics puts it —
never at the final result (projection/UNION above the BGP create *legitimate*
duplicates that must survive).

**D1 — within-branch (duplicate rows).** A branch whose joined tables do not
all contribute an authority-admitted declared key over the branch's
output-determining columns gets `SELECT DISTINCT`, reusing the existing single-branch DISTINCT pushdown
discipline (`iq.rs` — SELECT list restricted to output-determining columns,
per-branch, already proven for query-level DISTINCT).

**D2 — cross-branch (same triple from two maps).** A multi-branch pattern
relation joins its arms with `UNION` (set) instead of `UNION ALL`, under the
already-stated precondition (`emit_subplan_sql`, ADR-0025 Tier-2 gap 2): SQL
raw-column dedup equals SPARQL term dedup **only when cross-arm reconstruction
is injective**. Where arm reconstructions are not provably injective-compatible,
phase 1 refuses (sound 501, pinned); the general fallback (dedup over rendered
term expressions — the same fully-rendered-lexical lesson as the Fix-1 `pf:` id
repair) is phase 2 if a real mapping ever needs it.

**Elision — the performance story under frozen/verified authority.** Raw
introspection captures `TableSchema.primary_key` and `.unique`, but those fields
authorize elision only after entering an explicit frozen/verified compiler
schema. Current serving intentionally admits neither:

- D1 elides in frozen-schema/compiler tests when every joined table's projected
  columns are covered by an admitted PK/UNIQUE key (duplicate rows impossible).
  Unverified serving instead keeps `DISTINCT`, including for commonly
  PK-templated subjects.
- D2 elides when the arms' subject/object templates are pairwise **provably
  disjoint** (`unify::templates_provably_disjoint` — existing machinery, ADR-0032
  D6): disjoint arms cannot produce the same mapping, so `UNION ALL` is already
  set-correct.

Under a frozen/verified schema, a well-keyed, disjointly-templated mapping emits
the historically pinned SQL. Under the current unverified serving policy, D1
pays a conservative DISTINCT cost even when the mutable source still has the
observed key; correctness takes priority over an unleased performance proof. D2
still elides UNION dedup for structurally disjoint arms.

**Interactions.**
- Aggregates: the BGP block sits below GROUP BY, so dedup-before-aggregation is
  automatic (COUNT over a duplicate-carrying source becomes correct, not just
  cosmetically deduped).
- Property paths: closure relations already dedup internally
  (`SELECT DISTINCT sf_s, sf_o`, iq.rs); the NPS `UNION ALL` bag exception is
  arm-disjoint by construction (a triple's predicate matches exactly one arm),
  so D2-elision applies to it verbatim; D1 still applies to its underlying
  scans.
- Both engines: the mechanism lives in branch emission + the shared
  branch-union seam, below the flat/tree fork — one implementation, two
  engines, same as ADR-0033's conversion.

## Consequences

- The 9 red cells go green; `=_bag` vs the oracle becomes unconditional rather
  than fixture-lucky. This closes a **soundness** gap in the project's own
  definition (answer equivalence with the native evaluator over the decoded
  graph).
- Frozen/verified compiler SQL shape changes only where duplicates are possible;
  its elision cells pin NO DISTINCT. Current quarantine result controls prove
  conservative answers, but no serving-path SQL-shape assertion or performance
  receipt yet pins the DISTINCT cost. Both are required before a serving
  performance claim or future verified mode is admitted.
- The phase-1 non-injective cross-arm 501 is a new, honest, pinned boundary
  (expected to be unreachable for realistic mappings; revisit only on evidence).

## Test contract

1. All 9 `differential_star` set-semantics cells green, `=_bag` with spareval.
2. New plain-pattern (non-star) duplicate-row cells in `differential_tree` —
   the bug is general; its regression lock must be too.
3. Existing elision SQL-shape cells prove an explicitly frozen PK-covered
   compiler fixture emits no DISTINCT and a disjoint-arm fixture emits UNION ALL.
   Add a serving-path assertion that raw PK metadata under unverified authority
   still emits the conservative DISTINCT shape before claiming that shape as
   serving evidence.
4. Full suites: differential_tree/paths/star, adversarial_adr0033_refute, no
   regressions; bench before/after receipts on the standard suite.
