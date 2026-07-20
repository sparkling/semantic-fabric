//! Run 4 Wave B adversarial refutation pass (commit 51895dd) — the three B
//! items attacked where they are most likely to be wrong:
//!
//! * **B3** `SqlCond::TemplateEq` (differently-shaped templates compared via a
//!   rendered SQL string CONCAT) — SURFACES 1/2/3 below.
//! * **B2** per-arm observer resolution (`rewrite_filter_over_union`, EXISTS
//!   routing through `rewrite_top_level_pattern`) — SURFACE 4.
//! * **B1** AVG §18.5.1 non-numeric-operand error semantics (`nums.len() <
//!   vals.len()`) — SURFACE 5.
//!
//! **Method.** Every surface builds the smallest real fixture and runs BOTH
//! translators (tree `translate_with` and flat `translate_with_flat`) against
//! the ADR-0005 `spareval` oracle. The oracle graph is the mapping's OWN
//! materialization (`exec::dump_quads` → `graph::quads_to_dataset`) — so
//! materialization is the reference and the SQL-rewrite (OBDA) path is the
//! system-under-test; the two are genuinely independent code (Rust
//! `sf_core::term::generate_into` template rendering vs SQL `||`/`CONCAT`
//! rendering). A separate integration-test binary cannot import another one's
//! private helpers, so the `diff`/oracle pattern is replicated from
//! `differential_star_observers.rs`, exactly as that file replicated
//! `differential_star.rs`'s (see those files' module docs).
//!
//! Any test carrying a `BUG:` marker comment is a REFUTED verdict left
//! deliberately red (an exact repro), never a lock. Every other test is a
//! SURVIVED regression lock.
//!
//! **Run 4 B-repair.** The refute pass's four REFUTED verdicts (Surface 1,
//! Surface 3b, Surface 4b, Surface 5c) are fixed: `leftjoin::null_safe`'s
//! `TemplateEq` arm (FIX 1), `render_template_concat`'s percent-encoding
//! (FIX 2), `composed_agreement`'s union-of-arms variable scan (FIX 3), and
//! the AVG/SUM pushdown numeric gate (FIX 4, `iq/lower.rs` +
//! `unfold.rs::agg_needs_rust_group`). The four former `BUG:` repros are now
//! LOCKs (doc comments rewritten past-tense).
//!
//! **Main-loop review, second pass.** FIX 2's first draft covered a curated
//! 10-byte encode set (forced by a `sqlparser`-PostgreSQL-dialect parsing
//! pathology in a naive `REPLACE`-chain design); reworked to
//! `emit::percent_encode_col`, a per-dialect, O(1)-parse-depth encoder
//! covering the FULL RFC 3987 byte range on all three production dialects
//! (`s3d1`/`s3d2` lock bytes outside the old 10-byte set; `s3e` is a live-PG
//! differential, not just PG emission well-formedness). FIX 4's single-branch
//! tree AVG double carve-out is also closed (`s5f`) — the SAME NaN-coercion
//! bug one call site over from `s5c`. Every test in this file is green.

use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::star_decode::decode_proposition_forms;
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, PlanForm, Tbox};
use sf_sql::Dialect;
use spargebra::SparqlParser;
use std::collections::BTreeMap;

use oxrdf::{Dataset, Term};

const EX: &str = "PREFIX ex: <http://example.com/> ";
const RDF: &str = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ";

// ============================================================================
// Shared helpers — engine (tree ∧ flat) vs the materialized-graph spareval
// oracle. NO proposition decode: these fixtures are ordinary (mostly
// non-star) R2RML mappings, so the oracle graph is `dump_quads`'s direct
// materialization.
// ============================================================================

fn run_select(plan: &Plan, conn: &Connection) -> Vec<BTreeMap<String, Term>> {
    let PlanForm::Select { .. } = &plan.form else {
        panic!(
            "adversarial_run4b fixtures are SELECT-only, got {:?}",
            plan.form
        );
    };
    oracle::engine_bag(&exec::select(plan, conn).expect("select exec"))
}

/// Both translators must either both succeed with the SAME row bag, or both
/// return `Unsupported`. Returns the tree engine's rows (empty on a shared
/// 501). Mirrors `differential_star.rs::diff`.
fn diff(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");

    let flat = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    let tree = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema);

    match (&flat, &tree) {
        (Ok(fp), Ok(tp)) => {
            let fa = run_select(fp, &conn);
            let ta = run_select(tp, &conn);
            assert!(
                oracle::solutions_bag_eq(&fa, &ta),
                "flat vs tree row-bag divergence on `{query}`:\n flat={fa:#?}\n tree={ta:#?}"
            );
            ta
        }
        (Err(Error::Unsupported(_)), Err(Error::Unsupported(_))) => Vec::new(),
        _ => panic!(
            "501-set mismatch on `{query}` (flat and tree must agree on Unsupported):\n \
             flat={flat:?}\n tree={tree:?}"
        ),
    }
}

/// The mapping's own materialization as an in-memory RDF dataset — the oracle
/// graph. `dump_quads` (Rust-side template rendering) is a genuinely different
/// code path from `translate_with`'s SQL rewrite, so a divergence pinpoints a
/// rewrite bug.
fn materialized(create: &str, r2rml: &str) -> Dataset {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    graph::quads_to_dataset(&quads)
}

/// The spareval oracle's SELECT row bag over the materialized graph.
fn oracle_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    match oracle::evaluate(&materialized(create, r2rml), query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected Solutions, got {other:?}"),
    }
}

/// Engine (tree∧flat-agreed) vs oracle — the acceptance bar. Returns the
/// agreed rows for the caller's own additional assertions.
fn assert_oracle_agrees(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let engine = diff(create, r2rml, query);
    let oracle_rows = oracle_bag(create, r2rml, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "engine vs oracle divergence on `{query}`:\n \
         engine (SQL-rewritten) = {engine:#?}\n \
         oracle (materialized graph, spareval) = {oracle_rows:#?}"
    );
    engine
}

/// Both engines must translate WITHOUT error (a correct-empty answer is
/// otherwise indistinguishable from a 501 through `diff`'s graceful arm).
fn assert_translates_ok(r2rml: &str, query: &str) {
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let flat = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    let tree = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    assert!(
        flat.is_ok(),
        "expected flat to translate `{query}`, got {flat:?}"
    );
    assert!(
        tree.is_ok(),
        "expected tree to translate `{query}`, got {tree:?}"
    );
}

/// Both engines must 501 identically (a locked, still-deferred boundary).
fn assert_locked_501(r2rml: &str, query: &str) {
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let flat = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    let tree = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    assert!(
        matches!(flat, Err(Error::Unsupported(_))),
        "expected 501 on the flat path for `{query}`, got {flat:?}"
    );
    assert!(
        matches!(tree, Err(Error::Unsupported(_))),
        "expected 501 on the tree path for `{query}`, got {tree:?}"
    );
}

// ============================================================================
// SURFACE 1 — TemplateEq × OPTIONAL shared variables (the sharpest attack) —
// LOCK (Run 4 B-repair FIX 1; was REFUTED before the fix).
//
// ADR-0007 R1: a shared OPTIONAL variable's join condition must be
// `(a = b OR a IS NULL OR b IS NULL)` — an unbound value is compatible with
// anything. Before this fix, `leftjoin::null_safe` implemented R1 with arms
// ONLY for `ColEq` (→ `NullSafeEq`) and `Cmp(_,Eq,_)`; a `SqlCond::TemplateEq`
// (Wave B3's new variant) hit the `other => other` catch-all and passed
// through as a BARE `render(t1) = render(t2)`, dropping R1's null-compat
// disjuncts whenever `left_nullable` was true. `null_safe` now has its own
// `TemplateEq` arm (see `s1_templateeq_optional_shared_var_admits_null_padded_row`'s
// doc comment below for the fix mechanism).
//
// Fixture reaches that path via `(base OPT leftv) OPT rightv` sharing ?v:
//   * ?v's LEFT def is `leftv`'s object template (a prior-OPTIONAL alias ⇒
//     `left_nullable` true), an IRI template `http://ex.org/v/{va}`.
//   * ?v's RIGHT def is `rightv`'s object template `http://ex.org/v/{vb1}/{vb2}`
//     — a DIFFERENT shape (2 vs 4 segments), same `http://ex.org/v/` leading
//     prefix ⇒ not disjoint ⇒ `align_templates` emits `TemplateEq`.
//   * Person 2 has NO leftv row (NULL `va` ⇒ R2RML emits no triple), so ?v's
//     left value is NULL there — exactly R1's "unbound left, compatible with
//     any right" case the dropped disjunct is supposed to admit.
// ============================================================================

// A `-` separator (IRI-unreserved: RFC 3987 template expansion does NOT
// percent-encode it) between the two-column template's parts, and column
// values with NO encodable characters, so the single-column template's
// expansion (`http://ex.org/v/X-Y`) and the two-column template's expansion
// (`http://ex.org/v/X-Y`) are RDF-EQUAL AND raw-concat-equal — isolating the
// null-padding question from the percent-encoding question (which SURFACE 3's
// `s3_templateeq_percent_encoding_*` attacks separately).
const S1_SQL: &str = r#"
CREATE TABLE t (
    id INTEGER PRIMARY KEY,
    va TEXT,
    vb1 TEXT NOT NULL,
    vb2 TEXT NOT NULL
);
INSERT INTO t VALUES (1, 'X-Y', 'X', 'Y');
INSERT INTO t VALUES (2, NULL,  'M', 'N');
"#;

const S1_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .

<#Base>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:base ; rr:objectMap [ rr:constant ex:Anchor ] ] .

<#LeftV>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:leftv ; rr:objectMap [ rr:template "http://ex.org/v/{va}" ] ] .

<#RightV>
    rr:logicalTable [ rr:tableName "t" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:rightv ; rr:objectMap [ rr:template "http://ex.org/v/{vb1}-{vb2}" ] ] .
"#;

/// LOCK (Run 4 B-repair FIX 1): a shared OPTIONAL variable bound by
/// differently-shaped IRI templates emits `SqlCond::TemplateEq`. Before this
/// fix, `leftjoin::null_safe`'s `other => other` catch-all (leftjoin.rs:433)
/// dropped the R1 null-compatibility disjuncts for it — a left row whose ?v
/// was UNBOUND (person 2, no leftv row) failed to join the compatible right
/// row, wrongly leaving ?v unbound instead of taking the right value (R1,
/// `(a = b OR a IS NULL OR b IS NULL)`, was not being applied to the
/// `TemplateEq` variant). `null_safe` now has a `TemplateEq` arm that wraps
/// it as `SqlCond::Or([templateeq, IsNull(col) for every referenced column on
/// BOTH sides])`, so `?v = http://ex.org/v/M-N` is admitted for person 2,
/// matching the oracle.
#[test]
fn s1_templateeq_optional_shared_var_admits_null_padded_row() {
    let query = format!(
        "{EX}SELECT ?p ?v WHERE {{ \
           ?p ex:base ?anchor . \
           OPTIONAL {{ ?p ex:leftv ?v }} \
           OPTIONAL {{ ?p ex:rightv ?v }} \
         }}"
    );
    let rows = assert_oracle_agrees(S1_SQL, S1_R2RML, &query);
    assert_eq!(rows.len(), 2, "both persons projected: {rows:#?}");
}

// ============================================================================
// SURFACE 3 — TemplateEq semantics corners.
//
// (a) cross-KIND (IRI template vs plain-literal template): must never produce a
//     wrong equality — LOCK.
// (b) percent-encoding: TWO templates whose RENDERED IRIs differ ONLY in the
//     percent-encoding of a character — RDF IRI equality is codepoint-exact, so
//     the pair is UNequal, and the engine must agree — LOCK (Run 4 B-repair
//     FIX 2; was REFUTED before the fix).
// (c) PG-dialect translate-only emission — assert well-formed SQL + params.
// ============================================================================

// --- 3(b) percent-encoding — the LOCK ---------------------------------------
//
// `render_template_concat` (emit.rs) renders each `Segment::Column` as a RAW
// `colref` and each `Segment::Literal` as the raw template text, concatenated
// with `||`. But R2RML/RFC 3987 IRI template expansion PERCENT-ENCODES column
// values (all but IRI-unreserved chars). So for two DIFFERENTLY-shaped
// templates whose column values contain an encodable character across a
// segment boundary, the SQL raw-concat and the real RDF IRIs disagree:
//   * single-column `http://ex.org/v/{va}` with va = "X/Y" expands (RDF) to
//     `http://ex.org/v/X%2FY` — the `/` is encoded.
//   * two-column `http://ex.org/v/{vb1}/{vb2}` with vb1="X", vb2="Y" expands
//     (RDF) to `http://ex.org/v/X/Y` — the `/` is a LITERAL template char.
//   * these are DIFFERENT IRIs (`X%2FY` ≠ `X/Y`), so a shared-var join on ?v
//     has NO solution. But the SQL `TemplateEq` compares the RAW concats
//     `'http://ex.org/v/' || 'X/Y'` vs `'http://ex.org/v/' || 'X' || '/' || 'Y'`
//     — BOTH `http://ex.org/v/X/Y` — and reports them EQUAL, a false-positive.

const S3B_SQL: &str = r#"
CREATE TABLE t3 (id INTEGER PRIMARY KEY, va TEXT NOT NULL, vb1 TEXT NOT NULL, vb2 TEXT NOT NULL);
INSERT INTO t3 VALUES (1, 'X/Y', 'X', 'Y');
"#;
const S3B_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#L> rr:logicalTable [ rr:tableName "t3" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:leftv ; rr:objectMap [ rr:template "http://ex.org/v/{va}" ] ] .
<#R> rr:logicalTable [ rr:tableName "t3" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:rightv ; rr:objectMap [ rr:template "http://ex.org/v/{vb1}/{vb2}" ] ] .
"#;

/// LOCK (Run 4 B-repair FIX 2): the two objects for person 1 are DIFFERENT
/// IRIs (`http://ex.org/v/X%2FY` vs `http://ex.org/v/X/Y`), so the shared-
/// variable join `?p ex:leftv ?v . ?p ex:rightv ?v` has NO solution (oracle =
/// empty). Before this fix, `SqlCond::TemplateEq` compared the RAW
/// pre-encoding column concatenations, which ARE equal, and wrongly returned
/// person 1 — unsound whenever a column value contains a character IRI-
/// template expansion would percent-encode across a differently-shaped
/// segment boundary. `render_template_concat` now wraps each IRI-kind
/// `Segment::Column` in a percent-encoding call (`emit::percent_encode_col`,
/// a per-dialect, O(1)-parse-depth encoder — see its own doc comment for the
/// full design history, including why an earlier `REPLACE`-chain draft was
/// replaced) before comparing, so the `/` in `va = "X/Y"` is correctly
/// encoded and the join comes back empty, matching the oracle. The encode
/// set is now the FULL RFC 3987 complement (every ASCII byte outside
/// `iunreserved`, including every control byte) — see
/// [`s3d1_templateeq_percent_encoding_false_positive_paren`] and
/// [`s3d2_templateeq_percent_encoding_false_negative_pipe`] for a
/// regression lock over bytes OUTSIDE this repro's own `/`, in both
/// directions.
#[test]
fn s3b_templateeq_percent_encoding_prevents_false_positive_inner_join() {
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:leftv ?v . ?p ex:rightv ?v }}");
    let rows = assert_oracle_agrees(S3B_SQL, S3B_R2RML, &query);
    assert!(
        rows.is_empty(),
        "the two IRIs differ by encoding: {rows:#?}"
    );
}

// --- 3(d) full-byte-range regression lock — ADDED post-review --------------
//
// Main-loop review of Run 4 B-repair: FIX 2's first draft covered a curated
// 10-byte subset (forced by a PostgreSQL `sqlparser` parsing pathology in a
// naive `REPLACE`-chain design); the reworked `percent_encode_col` closes
// that gap with a per-dialect, O(1)-parse-depth encoder covering the FULL
// RFC 3987 complement. `(` and `|` are two bytes the OLD 10-byte set did NOT
// cover — these two fixtures lock in that they are now handled correctly,
// in BOTH directions the refute report's own s3b anatomy generalizes to:
//
// * FALSE POSITIVE (3d-i, `(`): the right template's own LITERAL text is
//   `(` (its column separator) — a left value containing a RAW `(` raw-
//   concats to the SAME string as the right's raw concat, but the TRUE
//   (encoded) left IRI differs (`%28` vs a literal `(`), so the answer must
//   be empty.
// * FALSE NEGATIVE (3d-ii, `|`): the right template's own LITERAL text is
//   the ALREADY-encoded form `%7C` — a left value containing a RAW `|`
//   raw-concats to a DIFFERENT string than the right's raw concat, but the
//   TRUE (encoded) left IRI COINCIDES with the right's literal text, so the
//   answer must be non-empty (a match a raw-concat-only comparison would
//   wrongly miss).
const S3D1_SQL: &str = r#"
CREATE TABLE t3d1 (id INTEGER PRIMARY KEY, va TEXT NOT NULL, vb1 TEXT NOT NULL, vb2 TEXT NOT NULL);
INSERT INTO t3d1 VALUES (1, 'X(Y', 'X', 'Y');
"#;
const S3D1_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#L> rr:logicalTable [ rr:tableName "t3d1" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:leftv ; rr:objectMap [ rr:template "http://ex.org/v/{va}" ] ] .
<#R> rr:logicalTable [ rr:tableName "t3d1" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:rightv ; rr:objectMap [ rr:template "http://ex.org/v/{vb1}({vb2}" ] ] .
"#;

/// FALSE POSITIVE direction (`(`, 0x28): `va = "X(Y"` expands (RDF) to
/// `http://ex.org/v/X%28Y` (`(` percent-encoded); `vb1="X", vb2="Y"` through
/// `http://ex.org/v/{vb1}({vb2}` expands to `http://ex.org/v/X(Y` (the `(`
/// is the template's OWN literal separator, never encoded) — DIFFERENT
/// IRIs, so the shared-variable join must be empty. The RAW, pre-encoding
/// concatenations coincide (`"X(Y"` both ways) — exactly the class of bug
/// FIX 2 closes, now for a byte outside the original 10-byte set.
#[test]
fn s3d1_templateeq_percent_encoding_false_positive_paren() {
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:leftv ?v . ?p ex:rightv ?v }}");
    let rows = assert_oracle_agrees(S3D1_SQL, S3D1_R2RML, &query);
    assert!(
        rows.is_empty(),
        "X%28Y and X(Y are different IRIs: {rows:#?}"
    );
}

const S3D2_SQL: &str = r#"
CREATE TABLE t3d2 (id INTEGER PRIMARY KEY, va TEXT NOT NULL, vb1 TEXT NOT NULL, vb2 TEXT NOT NULL);
INSERT INTO t3d2 VALUES (1, 'X|Y', 'X', 'Y');
"#;
const S3D2_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#L> rr:logicalTable [ rr:tableName "t3d2" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:leftv ; rr:objectMap [ rr:template "http://ex.org/v/{va}" ] ] .
<#R> rr:logicalTable [ rr:tableName "t3d2" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:rightv ; rr:objectMap [ rr:template "http://ex.org/v/{vb1}%7C{vb2}" ] ] .
"#;

/// FALSE NEGATIVE direction (`|`, 0x7C): `va = "X|Y"` expands (RDF) to
/// `http://ex.org/v/X%7CY` (`|` percent-encoded); `vb1="X", vb2="Y"` through
/// `http://ex.org/v/{vb1}%7C{vb2}` expands to the SAME
/// `http://ex.org/v/X%7CY` (the template's own literal text is ALREADY the
/// string `%7C`, authored that way, never itself encoded) — the SAME IRI,
/// so the shared-variable join must find this person. The RAW, pre-encoding
/// concatenations DIFFER (`"X|Y"` vs `"X%7CY"`), so a comparison that only
/// ever compares raw concatenations would wrongly MISS this match — the
/// mirror-image failure mode from 3d1's, both closed by the same fix.
#[test]
fn s3d2_templateeq_percent_encoding_false_negative_pipe() {
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:leftv ?v . ?p ex:rightv ?v }}");
    let rows = assert_oracle_agrees(S3D2_SQL, S3D2_R2RML, &query);
    assert_eq!(
        rows.len(),
        1,
        "X%7CY (encoded) and X%7CY (literal) are the SAME IRI: {rows:#?}"
    );
}

// --- 3(a) cross-KIND — the LOCK -------------------------------------------

// W5b repair: previously `va`/`vb1`/`vb2` fed DIFFERENTLY-shaped templates
// (`{va}` vs `{vb1}/{vb2}`) rendering DIFFERENT text ("X" vs "X/Y"), so
// `s3a_cross_kind_shared_join_is_empty` below was empty even with BOTH
// cross-kind guards (unify.rs:137 and :263) removed — an OVER-DETERMINED
// lock (verified empirically in the W5 run; see `adversarial_testaudit.rs`'s
// Cell D, added to close the gap this left). Fixed to the SAME single-column,
// same-shape template on both sides — mirrors Cell D's `CROSS_KIND_SQL`/
// `CROSS_KIND_R2RML` exactly — so this JOIN cell now depends on the guard
// (belt-and-braces with Cell D, not a replacement for it).
//
// Used by the JOIN-form test below AND (Run 5 W6) its same-shape FILTER-form
// companion `s3a_cross_kind_filter_eq_same_shape_is_empty` — deliberately NOT
// by `s3a_cross_kind_filter_eq_stays_locked_501` just above, which keeps its
// OWN, differently-shaped fixture. The JOIN form resolves cross-kind
// disjointness via `unify_derived`'s `term_map_type` pre-check (sound for
// same-shape templates too), but the FILTER form's `?a = ?b` resolves via
// `unify::cmp` -> `align_templates`, whose "same segment shape ⇒
// pairwise-column-equal" fast path (`unify.rs::align_templates`, the
// `sx.len() == sy.len()` loop) used to NOT check `spec.term_type` at all
// before emitting `SqlCond::ColEq` — discovered live while writing the W5b
// repair: pointing the FILTER test at THIS same-shape fixture made it
// wrongly TRANSLATE (a raw column-vs-column equality) instead of the correct
// empty answer. That was a genuine, pre-existing soundness gap (same-shape
// cross-kind templates were not provably disjoint in `align_templates`),
// orthogonal to the over-determination repair this comment is about, reported
// separately rather than silently absorbed by loosening either test — and now
// FIXED (Run 5 W6): `align_templates` itself proves a same-shape, cross-KIND
// (or cross-normalised-literal) pair disjoint before ever reaching the
// column-equality loop, so BOTH callers (this join path AND the FILTER path)
// get the correct `Unify::Empty`.
const S3A_SQL: &str = r#"
CREATE TABLE t4 (id INTEGER PRIMARY KEY, v TEXT NOT NULL);
INSERT INTO t4 VALUES (1, 'X');
"#;
// `ex:iriv` binds an IRI-typed object template; `ex:litv` binds a PLAIN-LITERAL
// object template (rr:termType rr:Literal) over the IDENTICAL template shape
// and column, so both render the SAME lexical text `http://ex.org/v/X`. An
// IRI can never equal a plain literal (SPARQL term equality), regardless of
// lexical text.
const S3A_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#Iri> rr:logicalTable [ rr:tableName "t4" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:iriv ; rr:objectMap [ rr:template "http://ex.org/v/{v}" ] ] .
<#Lit> rr:logicalTable [ rr:tableName "t4" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:litv ;
        rr:objectMap [ rr:template "http://ex.org/v/{v}" ; rr:termType rr:Literal ] ] .
"#;

// The FILTER test's OWN fixture — differently-shaped templates (`{va}` vs
// `{vb1}/{vb2}`, the ORIGINAL pre-W5b s3a shape). Kept separate from
// `S3A_R2RML` above deliberately; see that constant's doc comment for why
// sharing it would trip an unrelated `align_templates` gap instead of
// exercising the (sound) `template_eq_or_unsupported` path this test locks.
const S3A_FILTER_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#Iri> rr:logicalTable [ rr:tableName "t4f" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:iriv ; rr:objectMap [ rr:template "http://ex.org/v/{va}" ] ] .
<#Lit> rr:logicalTable [ rr:tableName "t4f" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:litv ;
        rr:objectMap [ rr:template "http://ex.org/v/{vb1}/{vb2}" ; rr:termType rr:Literal ] ] .
"#;

/// LOCK: a FILTER `?iri = ?lit` over an IRI template vs a DIFFERENT-shaped
/// plain-literal template must NEVER match them via a wrong `TemplateEq` —
/// `template_eq_or_unsupported`'s `spec1.term_type == spec2.term_type` guard
/// keeps this a sound `Unsupported` (501), never a rendered-string equality
/// between two different-kind terms. This SHAPE mismatch stays 501 even after
/// the Run 5 W6 fix below (only the SAME-shape path was ever missing its
/// term_type check) — the "Enhancement note" this doc comment used to flag (a
/// provably-unequal `Unify::Empty` would be sound AND complete, so a
/// SAME-shape cross-kind pair could answer empty instead of 501) is now
/// realized by `s3a_cross_kind_filter_eq_same_shape_is_empty` below, over
/// `S3A_R2RML`'s same-shape fixture. Uses its OWN fixture
/// (`S3A_FILTER_R2RML`), NOT `S3A_R2RML` — see that constant's doc comment
/// for why sharing it would exercise a different `align_templates` path than
/// this shape-mismatch test means to lock.
#[test]
fn s3a_cross_kind_filter_eq_stays_locked_501() {
    let query =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:iriv ?a . ?p ex:litv ?b . FILTER(?a = ?b) }}");
    assert_locked_501(S3A_FILTER_R2RML, &query);
}

/// LOCK (Run 5 W6 fix): the FILTER-form companion over the SAME-SHAPE fixture
/// (`S3A_R2RML`, the join cell's own fixture below) instead of
/// `S3A_FILTER_R2RML`'s differently-shaped one. `?a`/`?b` are both a
/// single-column `http://ex.org/v/{v}` template over the SAME column, one
/// side IRI-typed, the other `rr:termType rr:Literal`, so `align_templates`
/// takes its same-shape pairwise-column-equality path (the `sx.len() ==
/// sy.len()` loop), never reaching `template_eq_or_unsupported`. Before the
/// fix, that loop never consulted `spec.term_type` and emitted a plain
/// `ColEq` on the shared `v` column, wrongly MATCHING an IRI against a
/// same-lexical-text Literal (SPARQL term equality: never — an IRI and a
/// Literal are never `sameTerm` regardless of lexical form). Now
/// `align_templates` proves this disjoint (`Unify::Empty`) up front — the
/// FILTER becomes constant-false, an empty answer, agreeing with the oracle
/// (which treats `?a = ?b` between an IRI and a Literal as a type error,
/// excluding the row).
#[test]
fn s3a_cross_kind_filter_eq_same_shape_is_empty() {
    let query =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:iriv ?a . ?p ex:litv ?b . FILTER(?a = ?b) }}");
    let rows = assert_oracle_agrees(S3A_SQL, S3A_R2RML, &query);
    assert!(
        rows.is_empty(),
        "IRI != literal even with identical lexical text and template shape: {rows:#?}"
    );
}

/// LOCK companion: the shared-variable JOIN form of the same cross-kind pair —
/// `unify_derived`'s `term_map_type` pre-check proves IRI vs Literal disjoint
/// (`Unify::Empty`), so the engine returns an empty bag, matching the oracle
/// (the two objects are never the same term). Both engines translate and
/// agree. The fixture's IRI and Literal templates render the IDENTICAL
/// lexical text (W5b repair, see the fixture's own doc comment above), so
/// this emptiness depends SOLELY on the cross-kind guard, not on the two
/// sides happening to render different strings.
#[test]
fn s3a_cross_kind_shared_join_is_empty() {
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:iriv ?v . ?p ex:litv ?v }}");
    let rows = assert_oracle_agrees(S3A_SQL, S3A_R2RML, &query);
    assert!(
        rows.is_empty(),
        "IRI ≠ literal, join must be empty: {rows:#?}"
    );
}

// --- 3(c) PG-dialect translate-only emission — the LOCK --------------------

/// LOCK: a `TemplateEq` (different-shaped IRI templates, distinct vars, joined
/// by a FILTER equality) emitted for `Dialect::Postgres` renders each side as a
/// parenthesised `||` concat with NUMBERED (`$N`) placeholders for the literal
/// segments and a column reference for the column segments, and the params
/// vector carries exactly the literal-segment texts in placeholder order — no
/// malformed SQL, no placeholder/param count mismatch. Post-review (FIX 2
/// reworked to a per-dialect encoder covering the full byte range, closing
/// the raw-column percent-encoding gap this test's ORIGINAL doc comment
/// flagged as a noted limitation): each column reference is now additionally
/// wrapped in `percent_encode_col_postgres`'s `unnest(string_to_array(...))
/// WITH ORDINALITY` scalar subquery, asserted below.
#[test]
fn s3c_pg_templateeq_emission_is_wellformed() {
    let query =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:leftv ?a . ?p ex:rightv ?b . FILTER(?a = ?b) }}");
    let maps = sf_mapping::parse_r2rml(S3B_R2RML).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(&query)
        .expect("query parses");
    let plan = translate_with(&q, &maps, Dialect::Postgres, &Tbox::default(), &[])
        .expect("PG translate of a TemplateEq FILTER must succeed");
    let emitted = plan.emitted().expect("emit PG SQL");
    assert_eq!(emitted.len(), 1, "single branch");
    let e = &emitted[0];
    // The rendered TemplateEq: `(<..> || <..>) = (<..> || <..> || <..>)`.
    assert!(
        e.sql.contains("||"),
        "PG TemplateEq must render a `||` concat: {}",
        e.sql
    );
    assert!(
        e.sql.contains("$1"),
        "PG must use numbered placeholders for the literal segments: {}",
        e.sql
    );
    // Every placeholder used in the SQL must be backed by a param, and every
    // param must be referenced — a mismatch is a malformed prepared statement.
    let max_ph = (1..=e.params.len())
        .rev()
        .find(|n| e.sql.contains(&format!("${n}")))
        .unwrap_or(0);
    assert_eq!(
        max_ph,
        e.params.len(),
        "highest $N placeholder ({max_ph}) must equal param count ({}): sql={} params={:?}",
        e.params.len(),
        e.sql,
        e.params
    );
    // The literal-segment texts ('http://ex.org/v/' and the '/' separator) must
    // appear among the bound params (they are parameters, not inlined SQL text).
    assert!(
        e.params.iter().any(|p| p == "http://ex.org/v/"),
        "the shared IRI prefix must be a bound param: {:?}",
        e.params
    );
    assert!(
        e.params.iter().any(|p| p == "/"),
        "the two-column template's literal '/' separator must be a bound param: {:?}",
        e.params
    );
    // Every column reference is percent-encoded (not a bare colref): the
    // `unnest(string_to_array(...))  WITH ORDINALITY` encoder shape must
    // appear once per encoded COLUMN — one on the left (`ex:leftv`'s single
    // `{va}` slot) plus two on the right (`ex:rightv`'s `{vb1}`/`{vb2}`
    // slots), three total — not left as bare column refs.
    assert_eq!(
        e.sql.matches("WITH ORDINALITY").count(),
        3,
        "every TemplateEq column (1 left + 2 right) must be percent-encoded \
         via the per-character encoder: {}",
        e.sql
    );
}

// --- 3(e) live PostgreSQL differential — ADDED post-review -----------------
//
// W5b hermeticity repair: the original version connected with NO `dbname=`,
// landing on whatever database happens to default-resolve for `$USER` (often
// a database SHARED with other live-PG suites), then reused the bare table
// name `t3` there with no isolation. Two concurrent PG differentials racing
// `DROP TABLE IF EXISTS t3` / `CREATE TABLE t3` on the SAME database hit a
// genuine PostgreSQL race — both `CREATE TABLE t3` calls can interleave past
// each other's `DROP`, and the loser's implicit `pg_type` row for the
// table's row type collides on `pg_type_typname_nsp_index` (duplicate key).
// Fixed by mirroring `differential_pg_sqlite.rs`'s own convention exactly:
// probe the maintenance `postgres` database first (absence ⇒ graceful skip,
// unchanged), then create a PER-PROCESS throwaway database (`sf_s3e_<pid>`)
// so `t3` lives in a database no other test can ever see.

/// Base connection params (host/port/user, **no** dbname): `SF_PG_URL` if set,
/// else a local trust-auth default keyed on `$USER` (duplicated from
/// `differential_pg_sqlite.rs` / `sf_conformance::pg` per this file's own
/// copied-helper convention — a Cargo integration test is its own crate, no
/// `pub` cross-file surface to import).
fn base_conn() -> String {
    std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    })
}

/// Connect and spawn the driver task, returning the live client.
async fn connect(conn_str: &str) -> Result<tokio_postgres::Client, String> {
    let (client, connection) = tokio_postgres::connect(conn_str, tokio_postgres::NoTls)
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

/// Live-PG companion to s3b: the SAME percent-encoding false-positive
/// repro, executed end to end against a REAL PostgreSQL server — not just
/// emission well-formedness (s3c's own, narrower scope), but the full
/// translate → emit → EXECUTE → reconstruct pipeline through
/// `percent_encode_col_postgres`'s `unnest(string_to_array(...))  WITH
/// ORDINALITY` encoder. Gracefully skips when no PostgreSQL server is
/// reachable (mirrors `differential_pg_sqlite.rs`'s own `SF_PG_URL`
/// graceful-skip convention, so CI stays green without a live server), and
/// runs in a per-process throwaway database so concurrent PG suites can
/// never collide with it (see the hermeticity repair note above).
#[tokio::test]
async fn s3e_live_pg_templateeq_percent_encoding_false_positive() {
    let base = base_conn();
    // Probe via the maintenance database; absence ⇒ graceful skip (mirrors
    // `differential_pg_sqlite.rs::select_and_ask_agree_across_sqlite_and_pg`).
    let admin = match connect(&format!("{base} dbname=postgres")).await {
        Ok(c) => c,
        Err(_) => {
            eprintln!("no PostgreSQL server reachable — skipping s3e live PG differential");
            return;
        }
    };
    let dbname = format!("sf_s3e_{}", std::process::id());
    admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .await
        .expect("drop pre-existing throwaway db");
    admin
        .batch_execute(&format!("CREATE DATABASE {dbname}"))
        .await
        .expect("create throwaway db");

    let client = connect(&format!("{base} dbname={dbname}"))
        .await
        .expect("connect work db");

    // Same table name/shape/data as S3B_SQL (SQLite) — safe to reuse verbatim
    // now that `t3` lives in a database no other test can ever see.
    client
        .batch_execute(
            "CREATE TABLE t3 (id INTEGER PRIMARY KEY, va TEXT NOT NULL, \
             vb1 TEXT NOT NULL, vb2 TEXT NOT NULL); \
             INSERT INTO t3 VALUES (1, 'X/Y', 'X', 'Y');",
        )
        .await
        .expect("create+insert");

    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:leftv ?v . ?p ex:rightv ?v }}");
    let maps = sf_mapping::parse_r2rml(S3B_R2RML).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(&query)
        .expect("query parses");
    let plan = translate_with(&q, &maps, Dialect::Postgres, &Tbox::default(), &[])
        .expect("PG translate must succeed");
    let solutions = sf_sparql::exec_pg::select_pg(&plan, &client)
        .await
        .expect("PG execution");
    let engine = oracle::engine_bag(&solutions);

    // Oracle: the SAME logical data, materialized via SQLite (dialect-
    // independent ground truth — the two IRIs differ by encoding regardless
    // of which SQL engine executes the rewrite).
    let oracle_rows = oracle_bag(S3B_SQL, S3B_R2RML, &query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "live-PG engine vs oracle divergence:\n engine = {engine:#?}\n oracle = {oracle_rows:#?}"
    );
    assert!(
        engine.is_empty(),
        "the two IRIs differ by encoding, live PG must agree: {engine:#?}"
    );

    drop(client);
    let _ = admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} WITH (FORCE)"))
        .await;
}

// ============================================================================
// SURFACE 4 — per-arm observer resolution (B2). Fixture is the census
// proposition graph from `differential_star_observers.rs`: every person has
// exactly one hasAge proposition asserted by ex:CensusRecord2026, so
// `{ ?r rdf:reifies ?t } UNION { ?t ex:assertedBy ex:CensusRecord2026 }`
// mixes 3 composed propositions (left) with 3 plain reifier IRIs (right). The
// oracle evaluates the ORIGINAL star query over the DECODED native RDF-1.2
// graph (`decode_proposition_forms`).
// ============================================================================

const STAR_SQL: &str = r#"
CREATE TABLE census_row (
    person_id INTEGER PRIMARY KEY,
    age INTEGER NOT NULL
);
INSERT INTO census_row VALUES (1, 30);
INSERT INTO census_row VALUES (2, 40);
INSERT INTO census_row VALUES (3, 30);
"#;

const STAR_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:hasAge ; rr:objectMap [ rr:column "age" ] ] .

<#PersonAgeAssertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAge> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:assertedBy ; rr:objectMap [ rr:constant ex:CensusRecord2026 ] ] .
"#;

fn decoded_graph(create: &str, r2rml: &str) -> Dataset {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let encoded = graph::quads_to_dataset(&quads);
    decode_proposition_forms(&encoded).expect("decode ADR-0032 D1 emission")
}

fn oracle_star_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    match oracle::evaluate(&decoded_graph(create, r2rml), query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected Solutions, got {other:?}"),
    }
}

/// Engine (tree∧flat) vs the DECODED-graph spareval oracle (ADR-0032 R6).
fn assert_star_oracle_agrees(
    create: &str,
    r2rml: &str,
    query: &str,
) -> Vec<BTreeMap<String, Term>> {
    let engine = diff(create, r2rml, query);
    let oracle_rows = oracle_star_bag(create, r2rml, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "ADR-0032 R6 divergence on `{query}`:\n \
         engine = {engine:#?}\n oracle = {oracle_rows:#?}"
    );
    engine
}

// --- 4(b) a variable composed in ONE arm only — the LOCK --------------------

/// LOCK (Run 4 B-repair FIX 3): `{ ?r rdf:reifies ?t1 } UNION { ?r rdf:reifies
/// ?t2 }` binds ?t1 (composed) in the LEFT arm only and ?t2 (composed) in the
/// RIGHT arm only. Before this fix, `composed_agreement` intersected the two
/// arms' variable sets — so neither ?t1 nor ?t2 (each in ONE arm) appeared,
/// the "all agree" fast path fired vacuously, and `FILTER(isTRIPLE(?t1))` was
/// resolved ONCE against the whole-query env where ?t1 is composed ⇒
/// statically `true`, wrongly applied to BOTH arms (the 3 RIGHT-arm rows,
/// where ?t1 is UNBOUND, survived alongside the 3 correct LEFT-arm rows:
/// engine 6, oracle 3).
///
/// `composed_agreement` now scans the UNION of the two arms' variable sets,
/// not the intersection — a variable composed in only one arm is now
/// reported as a genuine per-arm disagreement (`left_composes`/
/// `right_composes` naturally differ, since `collect_pattern_vars(rw_arm)`
/// is false for the arm that never mentions it), and the EXISTING per-arm
/// env-fork machinery in `rewrite_filter_over_union` resolves
/// `isTRIPLE(?t1)` correctly per arm with no further change needed there:
/// `true` in the composing left arm, `false` (composed-info removed from
/// that arm's env fork) in the right. Only the 3 left-arm rows survive,
/// matching the oracle.
///
/// Attribution: the whole-query-static composed-ness model (ADR-0032 D3)
/// applied to a UNION arm that does not bind the composed variable; it
/// pre-dates Wave B2 (the empty-`composed_agreement` fast path in
/// `rewrite_filter_over_union` reproduced the same resolve-once behaviour
/// the pre-B2 `rewrite_pattern` Filter arm had).
#[test]
fn s4b_istriple_over_var_composed_in_one_arm_only() {
    let query = format!(
        "{EX}{RDF}SELECT ?t1 ?t2 WHERE {{ \
           {{ {{ ?r rdf:reifies ?t1 }} UNION {{ ?r rdf:reifies ?t2 }} }} \
           FILTER(isTRIPLE(?t1)) \
         }}"
    );
    let rows = assert_star_oracle_agrees(STAR_SQL, STAR_R2RML, &query);
    assert_eq!(
        rows.len(),
        3,
        "only left-arm (?t1-bound) rows may survive: {rows:#?}"
    );
}

// --- 4(a) mixed static || dynamic disjunct — a sound-501 boundary LOCK -----

/// LOCK (sound-but-incomplete boundary): `FILTER(isTRIPLE(?t) || sameTerm(?t,
/// ex:X))` over the mixed union does NOT achieve 4(a)'s "keep the dynamic
/// disjunct live in both arms" — the per-arm fork rewrites the WHOLE `expr`
/// against the composed-arm env too, where `sameTerm(?t, ex:X)` on a composed
/// ?t has no plain-column rendering, so it 501s. A sound completeness limit
/// (never a wrong answer), NOT the success the ticket hoped for — locked so a
/// future improvement (short-circuiting the statically-`true` `isTRIPLE`
/// disjunct in the composed arm before rewriting the rest) is a visible change.
#[test]
fn s4a_dynamic_disjunct_on_composed_var_is_sound_501() {
    let query = format!(
        "{EX}{RDF}SELECT ?t WHERE {{ \
           {{ {{ ?r rdf:reifies ?t }} UNION {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
           FILTER(isTRIPLE(?t) || sameTerm(?t, ex:CensusRecord2026)) \
         }}"
    );
    assert_locked_501(STAR_R2RML, &query);
}

// --- 4(c) NOT EXISTS whose body is itself a filter-over-mixed-union — LOCK --

/// LOCK: a `NOT EXISTS` body that is ITSELF a `FILTER(isTRIPLE(?t))` over the
/// mixed-composed union (the exact shape B2 closes as a top-level pattern, now
/// nested one level deeper inside a NOT EXISTS). The `Exists` arm of
/// `rewrite_expr` routes the body through `rewrite_top_level_pattern`, so the
/// inner filter-over-union gets per-arm resolution; the body always matches (the
/// composed arm has 3 propositions), so NOT EXISTS is false for every person and
/// the answer is empty — engine and oracle agree.
#[test]
fn s4c_not_exists_body_is_filter_over_mixed_union() {
    let query = format!(
        "{EX}{RDF}SELECT ?p WHERE {{ \
           ?p ex:hasAge ?age . \
           FILTER NOT EXISTS {{ \
             {{ {{ ?r rdf:reifies ?t }} UNION {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
             FILTER(isTRIPLE(?t)) \
           }} \
         }}"
    );
    assert_translates_ok(STAR_R2RML, &query);
    let rows = assert_star_oracle_agrees(STAR_SQL, STAR_R2RML, &query);
    assert!(
        rows.is_empty(),
        "NOT EXISTS is false for every person: {rows:#?}"
    );
}

// --- 4(d) annotation sugar inside ONE union arm only — LOCK ----------------

/// LOCK: annotation sugar `?p ex:hasAge ?age {| ex:assertedBy ?t |}` in the
/// RIGHT arm binds ?t to the reifier (an ordinary IRI), while the LEFT arm's
/// `?r rdf:reifies ?t` composes ?t — a genuine per-arm composed-ness
/// disagreement on the shared ?t, wrapped in `FILTER(isTRIPLE(?t))`. Per-arm
/// resolution keeps only the composed left arm; engine and oracle agree.
#[test]
fn s4d_annotation_sugar_in_one_arm() {
    let query = format!(
        "{EX}{RDF}SELECT ?t WHERE {{ \
           {{ {{ ?r rdf:reifies ?t }} UNION {{ ?p ex:hasAge ?age {{| ex:assertedBy ?t |}} }} }} \
           FILTER(isTRIPLE(?t)) \
         }}"
    );
    let rows = assert_star_oracle_agrees(STAR_SQL, STAR_R2RML, &query);
    assert_eq!(
        rows.len(),
        3,
        "only the composed left arm survives: {rows:#?}"
    );
    assert!(
        rows.iter().all(|r| matches!(&r["t"], Term::Triple(_))),
        "every surviving ?t is a triple term: {rows:#?}"
    );
}

const AVG_SQL: &str = r#"
CREATE TABLE d (id INTEGER PRIMARY KEY, n INTEGER NOT NULL, num_str TEXT NOT NULL, dbl_str TEXT NOT NULL);
INSERT INTO d VALUES (1, 10, '12', 'NaN');
INSERT INTO d VALUES (2, 20, '34', '2.0');
INSERT INTO d VALUES (3, 30, '56', '4.0');
"#;
const AVG_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<#Num> rr:logicalTable [ rr:tableName "d" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:num ; rr:objectMap [ rr:column "n" ] ] .
<#Str> rr:logicalTable [ rr:tableName "d" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:str ; rr:objectMap [ rr:column "num_str" ] ] .
<#Dbl> rr:logicalTable [ rr:tableName "d" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:dbl ; rr:objectMap [ rr:column "dbl_str" ; rr:datatype xsd:double ] ] .
"#;

// AVG-over-UNION forces the Rust aggregation path (`rust_agg` — the B1 target):
// the FLAT engine 501s on this shape ("BIND references unbound"), so these use
// a TREE-only differential (the production default since ADR-0023 M8), not the
// flat∧tree `diff`. The flat/tree split is itself a sound boundary (flat 501s,
// never a wrong answer) noted in the final report.
fn tree_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let plan = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("tree translate");
    oracle::engine_bag(&exec::select(&plan, &conn).expect("select exec"))
}

fn assert_tree_oracle_agrees(
    create: &str,
    r2rml: &str,
    query: &str,
) -> Vec<BTreeMap<String, Term>> {
    let engine = tree_bag(create, r2rml, query);
    let oracle_rows = oracle_bag(create, r2rml, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "tree engine vs oracle divergence on `{query}`:\n \
         engine = {engine:#?}\n oracle = {oracle_rows:#?}"
    );
    engine
}

/// The one-row aggregate answer's `?a` term, or `None` if the aggregate is
/// unbound (errored group) — `engine_bag` drops unbound vars, so an errored
/// group is the empty solution `{}`.
fn avg_result(rows: &[BTreeMap<String, Term>]) -> Option<&Term> {
    assert_eq!(
        rows.len(),
        1,
        "implicit grouping yields exactly one row: {rows:#?}"
    );
    rows[0].get("a")
}

// --- 5(a)/5(d) non-numeric operand errors the group — the B1 LOCK ----------

/// LOCK (validates B1): AVG over a group MIXING xsd:integer operands and
/// `"12"/"34"/"56"^^xsd:string` operands (numeric-LOOKING lexical form, but a
/// NON-numeric DATATYPE) must error the whole aggregate ⇒ ?a UNBOUND, NOT the
/// average of just the integer subset. `rust_agg`'s `nums.len() < vals.len()`
/// gate (the B1 fix) rejects the string operands; the tree engine and the
/// spareval oracle both return an unbound ?a. (The old `nums.is_empty()` gate
/// would have averaged the 3 integers alone — a `=_bag` wrong answer.)
#[test]
fn s5d_mixed_numeric_and_string_datatype_errors_group() {
    let query = format!(
        "{EX}SELECT (AVG(?v) AS ?a) WHERE {{ {{ ?x ex:num ?v }} UNION {{ ?y ex:str ?v }} }}"
    );
    let rows = assert_tree_oracle_agrees(AVG_SQL, AVG_R2RML, &query);
    assert!(
        avg_result(&rows).is_none(),
        "mixed group errors ⇒ unbound: {rows:#?}"
    );
}

/// LOCK (validates B1, decimal arm): AVG over an ALL-non-numeric group (every
/// operand `"12"/"34"/"56"^^xsd:string`) errors ⇒ ?a unbound. Exercises
/// `rust_agg`'s decimal branch `nums.len() < vals.len()` with `nums` empty —
/// the tree engine's type-aware lowering routes an all-string operand column to
/// `rust_agg` (not SQL AVG), so it correctly errors rather than casting the
/// numeric-looking strings. Matches the oracle's unbound.
#[test]
fn s5a_all_non_numeric_string_group_errors() {
    let query = format!(
        "{EX}SELECT (AVG(?v) AS ?a) WHERE {{ {{ ?x ex:str ?v }} UNION {{ ?y ex:str ?v }} }}"
    );
    let rows = assert_tree_oracle_agrees(AVG_SQL, AVG_R2RML, &query);
    assert!(
        avg_result(&rows).is_none(),
        "all-string group errors ⇒ unbound: {rows:#?}"
    );
}

// --- 5(b) empty group — the LOCK -------------------------------------------

/// LOCK: AVG over a GENUINELY empty group (an implicit-grouping pattern that
/// matches ZERO rows) ⇒ `"0"^^xsd:integer` (SPARQL §11, `rust_agg`'s
/// `rows.is_empty()` arm), matching the oracle — distinct from AVG over rows
/// that all leave the operand unbound (⇒ unbound, not 0), which the B1-adjacent
/// C.4 discrimination on `rows` handles.
#[test]
fn s5b_avg_over_empty_group_is_integer_zero() {
    let query = format!(
        "{EX}SELECT (AVG(?v) AS ?a) WHERE {{ {{ ?x ex:missing ?v }} UNION {{ ?y ex:missing2 ?v }} }}"
    );
    let rows = assert_tree_oracle_agrees(AVG_SQL, AVG_R2RML, &query);
    let a = avg_result(&rows).expect("empty-group AVG is 0, not unbound");
    assert_eq!(
        a.to_string(),
        "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>",
        "empty group AVG ⇒ 0^^integer: {a:?}"
    );
}

// --- 5(c) NaN propagation — the LOCK ----------------------------------------

/// LOCK (Run 4 B-repair FIX 4): AVG over `"NaN"^^xsd:double`,
/// `"2.0"^^xsd:double`, `"4.0"^^xsd:double` must propagate NaN ⇒
/// `"NaN"^^xsd:double` (NaN is a NUMERIC value; it must NOT error and must
/// NOT be dropped). The oracle returns `"NaN"^^xsd:double`. Before this fix,
/// the tree engine returned `"2"^^xsd:decimal`: this all-`xsd:double` AVG was
/// pooled by `try_sql_group_over_union` and pushed to SQL aggregation (NOT
/// `rust_agg` — the operand column is uniformly typed, so the tree lowering
/// chose `SUM`/`AVG` pushdown), and SQLite's `AVG` CAST the underlying
/// `'NaN'` TEXT to `0.0`, averaging `(0 + 2 + 4 + 0 + 2 + 4) / 6 = 2` — a
/// wrong finite answer for a NaN input.
///
/// Scope: the SQL-aggregation PUSHDOWN path, NOT B1's `rust_agg` fix (which
/// already correctly handles the mixed/non-numeric cases above); pre-existing
/// (SQLite text→numeric coercion in a pushed `AVG`), reached whenever an
/// `xsd:double`-typed column carries a lexical form SQLite cannot parse to
/// the same value. `try_sql_group_over_union` now gates AVG/SUM pooling on
/// `agg_operand_is_exact_numeric` — the operand's declared `rr:datatype` must
/// be `xsd:integer`/`xsd:decimal` — so this all-`xsd:double` case bails to
/// `RustGroup`, which already propagates NaN correctly, matching the oracle.
#[test]
fn s5c_nan_double_avg_propagation() {
    let query = format!(
        "{EX}SELECT (AVG(?v) AS ?a) WHERE {{ {{ ?x ex:dbl ?v }} UNION {{ ?y ex:dbl ?v }} }}"
    );
    let rows = assert_tree_oracle_agrees(AVG_SQL, AVG_R2RML, &query);
    let a = avg_result(&rows).expect("NaN AVG is bound (NaN), not unbound");
    assert_eq!(
        a.to_string(),
        "\"NaN\"^^<http://www.w3.org/2001/XMLSchema#double>",
        "NaN must propagate through AVG: {a:?}"
    );
}

// --- 5(f) single-pattern xsd:double AVG NaN propagation — the RESIDUAL -----
//
// Main-loop review of Run 4 B-repair: s5c closed the UNION-pooled
// (`try_sql_group_over_union`) instance of the double-AVG-NaN-coercion bug;
// this is the SAME bug one call site over. `iq/lower.rs::lower_aggregation`'s
// single-branch `avg_needs_exact_decimal` gate (predates this repair, M3 fix
// 1) has a "provably double is exempt" carve-out — double stays on the SQL
// pushdown path since M3's OWN, unrelated precision concern considers f64
// arithmetic harmless for an already-f64 type. That same exemption ALSO lets
// SQLite coerce 'NaN' text to 0.0 for a plain, non-union `AVG(?v)` over
// `ex:dbl`, averaging `(0 + 2 + 4) / 3 = 2` instead of propagating NaN.
#[test]
fn s5f_single_pattern_double_avg_nan_propagation() {
    let query = format!("{EX}SELECT (AVG(?v) AS ?a) WHERE {{ ?x ex:dbl ?v }}");
    let rows = assert_tree_oracle_agrees(AVG_SQL, AVG_R2RML, &query);
    let a = avg_result(&rows).expect("NaN AVG is bound (NaN), not unbound");
    assert_eq!(
        a.to_string(),
        "\"NaN\"^^<http://www.w3.org/2001/XMLSchema#double>",
        "NaN must propagate through AVG: {a:?}"
    );
}

// --- 5(e) flat single-pattern all-string AVG — the ENHANCEMENT LOCK -------
//
// The refute report's enhancement note: "flat's single-pattern all-string
// AVG returns a wrong value the same way" as s5c's NaN case, via a DIFFERENT
// mechanism — `unfold.rs::group`'s single-branch path had NO type gate at
// all (unlike the tree engine's pre-existing `avg_needs_exact_decimal`), so
// it pushed AVG straight to SQL even for an `xsd:string`-datatyped column,
// letting SQLite's `AVG` coerce the numeric-LOOKING lexical forms
// "12"/"34"/"56" to a wrong finite average instead of erroring the whole
// aggregate (SPARQL §11: a non-numeric-DATATYPE operand errors, regardless
// of how its lexical form reads — the same B1 rule s5a/s5d lock for the
// union-shaped case).
//
// Run 4 B-repair FIX 4 (`unfold.rs::agg_needs_rust_group`) closes the WRONG
// ANSWER by routing this shape to the SAME Rust-group path a multi-branch
// inner already uses — but that path then hits the PRE-EXISTING, ALREADY
// documented limitation s5a-s5d's own module comment names ("the FLAT
// engine 501s on this shape ('BIND references unbound')"): `rust_group_plan`
// reuses the inner pattern's own bindings verbatim (`..t`), never installing
// the aggregate's own synthetic output variable, so the ENCLOSING
// `BIND(?synthetic AS ?a)` (spargebra's own AVG-desugaring) can't resolve it
// — a SOUND 501, not the silent wrong answer this fix replaces. Tree is
// unaffected (its own pre-existing single-branch gate already routed this
// shape correctly, before this fix ever runs) and gives the right (unbound)
// answer — so, like s5a-s5d, this stays a TREE-ONLY differential; the fix's
// observable effect on FLAT is "wrong answer → sound 501", verified below.
#[test]
fn s5e_flat_single_pattern_all_string_avg_errors_group() {
    let query = format!("{EX}SELECT (AVG(?v) AS ?a) WHERE {{ ?x ex:str ?v }}");
    let maps = sf_mapping::parse_r2rml(AVG_R2RML).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(&query)
        .expect("query parses");
    let flat = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    assert!(
        matches!(flat, Err(Error::Unsupported(_))),
        "flat must SOUNDLY 501 this shape (rust_group_plan's pre-existing \
         BIND-unbound limitation, same as s5a-s5d's UNION case), not silently \
         coerce the non-numeric strings: {flat:?}"
    );
    let rows = assert_tree_oracle_agrees(AVG_SQL, AVG_R2RML, &query);
    assert!(
        avg_result(&rows).is_none(),
        "all-string single-pattern AVG errors ⇒ unbound: {rows:#?}"
    );
}

const S2_SQL: &str = r#"
CREATE TABLE t2sj (id INTEGER PRIMARY KEY, c1 TEXT NOT NULL, c2a TEXT NOT NULL, c2b TEXT NOT NULL);
INSERT INTO t2sj VALUES (1, 'A-B', 'A', 'B');
INSERT INTO t2sj VALUES (2, 'C',   'D', 'E');
"#;
const S2_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#A> rr:logicalTable [ rr:tableName "t2sj" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:a ; rr:objectMap [ rr:template "http://ex.org/v/{c1}" ] ] .
<#B> rr:logicalTable [ rr:tableName "t2sj" ] ;
    rr:subjectMap [ rr:template "http://ex.org/p/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:b ; rr:objectMap [ rr:template "http://ex.org/v/{c2a}-{c2b}" ] ] .
"#;

const S2FK_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    dept_id INTEGER NOT NULL,
    a TEXT NOT NULL,
    b TEXT NOT NULL,
    FOREIGN KEY (dept_id) REFERENCES dept(id)
);
INSERT INTO dept VALUES (5), (6);
INSERT INTO person VALUES (1, 5, '5', 'x');
INSERT INTO person VALUES (2, 6, 'zzz', 'q');
"#;
const S2FK_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .
<#Dept> rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex.org/dept/{id}" ] .
<#Person> rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:dept ;
        rr:objectMap [ rr:parentTriplesMap <#Dept> ; rr:joinCondition [ rr:child "dept_id" ; rr:parent "id" ] ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:other ;
        rr:objectMap [ rr:template "http://ex.org/dept/{a}-{b}" ] ] .
"#;

/// Emit the tree plan's single branch to SQL (for cascade-shape assertions).
fn tree_sql(create: &str, r2rml: &str, query: &str) -> (String, Vec<String>) {
    let conn = sqlite::load(create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let plan = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("tree translate");
    let emitted = plan.emitted().expect("emit");
    assert_eq!(emitted.len(), 1, "single branch expected");
    (emitted[0].sql.clone(), emitted[0].params.clone())
}

// --- 2(b) self-join merge over a TemplateEq — the LOCK ---------------------

/// LOCK: two scans of the SAME table sharing the PK-keyed subject ?p are merged
/// into ONE scan by `self_join_elimination`; a `FILTER(?x = ?y)` over two
/// differently-shaped IRI templates on that table produces a `TemplateEq` whose
/// RIGHT side (`http://ex.org/v/{c2a}-{c2b}`) has TWO columns at the dropped
/// alias. `cascade::mod::rewrite_cond_alias`'s `TemplateEq` arm reassigns the
/// per-side alias once PER column; this proves no mixed-alias corruption — both
/// c2a and c2b end at the KEPT alias, so the merged SQL scans the table ONCE and
/// both `TemplateEq` sides reference that one alias. Answer matches the oracle.
#[test]
fn s2b_self_join_merge_preserves_template_eq() {
    let query = format!("{EX}SELECT ?p WHERE {{ ?p ex:a ?x . ?p ex:b ?y . FILTER(?x = ?y) }}");
    let (sql, _params) = tree_sql(S2_SQL, S2_R2RML, &query);
    assert_eq!(
        sql.matches("t2sj").count(),
        1,
        "self-join must merge to ONE scan of t2sj: {sql}"
    );
    // Both TemplateEq sides reference the surviving alias t0 — no dangling
    // alias. (Run 4 B-repair FIX 2 wraps every `Segment::Column` in a
    // percent-encoding `REPLACE` chain, so the exact pre-fix bare-colref
    // concat string no longer appears verbatim; checking for each column's
    // merged-alias reference still verifies what this LOCK is actually
    // about — no mixed/dangling alias after the self-join merge.)
    assert!(
        sql.contains("t0.\"c1\"") && sql.contains("t0.\"c2a\"") && sql.contains("t0.\"c2b\""),
        "both TemplateEq sides must reference the merged alias t0: {sql}"
    );
    let rows = assert_oracle_agrees(S2_SQL, S2_R2RML, &query);
    assert_eq!(
        rows.len(),
        1,
        "only person 1's v/A-B == v/A-B holds: {rows:#?}"
    );
}

// --- 2(a) FK/PK join-elimination over a TemplateEq — the LOCK --------------

/// LOCK: `?p ex:dept ?d` joins person→dept via the FK (dept_id → id);
/// `fk_pk_join_elimination` eliminates the parent (dept) scan because the FK
/// gives dept.id = person.dept_id. A `FILTER(?d = ?t)` over ?d (the parent
/// subject template `http://ex.org/dept/{id}`) and ?t (a differently-shaped
/// child template) produces a `TemplateEq` referencing the PARENT alias's
/// `id` column. `collect_cond_cols`'s `TemplateEq` arm reports that column, so
/// `parent_referenced_only_via` sees the reference is the (rewritable) join key
/// and `rewrite_parent_template_segments` rewrites dept.id → person.dept_id.
/// Result: the dept scan is gone, BOTH TemplateEq sides reference the person
/// alias (no dangling parent alias), and the answer matches the oracle.
#[test]
fn s2a_fk_pk_elimination_rewrites_template_eq_parent_segment() {
    let query =
        format!("{EX}SELECT ?p WHERE {{ ?p ex:dept ?d . ?p ex:other ?t . FILTER(?d = ?t) }}");
    let (sql, _params) = tree_sql(S2FK_SQL, S2FK_R2RML, &query);
    assert_eq!(
        sql.matches("\"dept\"").count(),
        0,
        "FK-PK elimination must remove the parent dept scan: {sql}"
    );
    assert!(
        sql.contains("t0.\"dept_id\""),
        "the TemplateEq's parent id segment must be rewritten to person.dept_id: {sql}"
    );
    // No match (a 1-col `dept/{id}` can never equal a 2-col `dept/{a}-{b}`), so
    // both engine and oracle are empty — the machinery ran without a dangling
    // alias or wrong row.
    let rows = assert_oracle_agrees(S2FK_SQL, S2FK_R2RML, &query);
    assert!(rows.is_empty(), "differently-shaped, no match: {rows:#?}");
}
