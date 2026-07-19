//! Run 4 Wave B2 differential: mixed-composedness UNDER AN OBSERVER (a
//! function call, an EXISTS/NOT EXISTS body, a CONSTRUCT template) — the
//! boundary `sf_sparql::star::top_level::rewrite_top_level_pattern`'s ledger
//! closeout boundary A (ADR-0032 D3, ledger F4b) left 501, now partly closed.
//! Deliberately a SEPARATE file, not an addition to `differential_star.rs`
//! (owned concurrently by another work stream on the SAME boundary — its
//! `union_arms_disagreeing_on_composed_ness_wrapped_in_a_filter_is_still_a_locked_501`
//! and `values_mixed_triple_and_plain_cells_wrapped_in_a_filter_is_still_a_locked_501`
//! pin EXACTLY the shape this file's `function_over_mixed_union_...` test now
//! closes — see this file's own module-end note). Mirrors
//! `differential_star.rs`'s helper pattern (engine-vs-expected + a
//! decoded-graph spareval oracle) — a separate integration-test binary cannot
//! import another one's private items, so the pattern is replicated, not
//! shared, exactly as `differential_star.rs`'s own module doc explains it
//! replicated `differential_paths.rs`'s.
//!
//! **Oracle strategy** (identical to `differential_star.rs`): `spareval`
//! evaluates the ORIGINAL SPARQL-star query natively over the DECODED graph
//! (`sf_conformance::star_decode::decode_proposition_forms`) — the rigorous,
//! independent cross-check; `assert_oracle_agrees`/`assert_locked_501` below
//! are byte-for-byte copies of `differential_star.rs`'s own.

use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::star_decode::decode_proposition_forms;
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, PlanForm, Tbox};
use sf_sql::Dialect;
use spargebra::SparqlParser;
use std::collections::BTreeMap;

use oxrdf::{Dataset, Term};

// ============================================================================
// Fixture — the SAME `census_row` shape `differential_star.rs` uses (a
// `#PersonAge` quoted triples map keyed by `person_id`/`age`, asserted via
// `#PersonAgeAssertion`): every person has exactly one age assertion, so
// `{ ?r rdf:reifies ?t } UNION { ?t ex:assertedBy ex:CensusRecord2026 }`
// always mixes 3 composed propositions (left arm) with 3 plain reifier IRIs
// (right arm) — the disagreement every test below builds on.
// ============================================================================

const CENSUS_SQL: &str = r#"
CREATE TABLE census_row (
    person_id INTEGER PRIMARY KEY,
    age INTEGER NOT NULL
);
INSERT INTO census_row VALUES (1, 30);
INSERT INTO census_row VALUES (2, 40);
INSERT INTO census_row VALUES (3, 30);
"#;

const CENSUS_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://example.com/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [ rr:template "http://ex.org/person/{person_id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#PersonAgeAssertion>
    rr:logicalTable [ rr:tableName "census_row" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#PersonAge> ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:objectMap [ rr:constant ex:CensusRecord2026 ]
    ] .
"#;

const EX: &str = "PREFIX ex: <http://example.com/> ";
const RDF: &str = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> ";

/// A plan's executed answer, comparable as a bag (SELECT only).
fn run_select(plan: &Plan, conn: &Connection) -> Vec<BTreeMap<String, Term>> {
    let PlanForm::Select { .. } = &plan.form else {
        panic!(
            "differential_star_observers fixtures are SELECT-only, got {:?}",
            plan.form
        );
    };
    oracle::engine_bag(&exec::select(plan, conn).expect("select exec"))
}

/// The tree/flat differential over one fixture + query (byte-for-byte copy of
/// `differential_star.rs::diff` — see the module doc for why this is
/// replicated rather than shared). Both translators must either both succeed
/// with the SAME row bag, or both return `Unsupported`.
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

/// A locked boundary: both engines must 501 identically (copy of
/// `differential_star.rs::assert_locked_501`).
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

/// The converse of [`assert_locked_501`] — both engines must translate
/// WITHOUT error. Needed as an explicit, separate check for a query whose
/// CORRECT row-bag answer happens to be empty (e.g. a NOT EXISTS that
/// eliminates every row): `diff`'s `(Err(Unsupported), Err(Unsupported)) =>
/// Vec::new()` graceful-empty arm would otherwise make a translate-time 501
/// indistinguishable from a correct empty answer through `assert_oracle_agrees`
/// alone — this closes that gap by asserting translation success directly.
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

fn decoded_graph(create: &str, r2rml: &str) -> Dataset {
    let conn = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materialize");
    let encoded = graph::quads_to_dataset(&quads);
    decode_proposition_forms(&encoded)
        .expect("decode must succeed for a well-formed ADR-0032 D1 emission")
}

/// The D0 oracle answer: `query` (the ORIGINAL SPARQL-star surface syntax,
/// never rewritten) evaluated by `spareval` over the decoded native graph.
fn oracle_star_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    match oracle::evaluate(&decoded_graph(create, r2rml), query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected Solutions, got {other:?}"),
    }
}

/// The engine's (tree/flat-agreed, [`diff`]) row bag vs the decoded-graph
/// spareval oracle's row bag: both must agree EXACTLY (ADR-0032 R6's
/// acceptance bar, unchanged by this wave). Returns the agreed rows for the
/// caller's own additional assertions.
fn assert_oracle_agrees(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, Term>> {
    let engine = diff(create, r2rml, query);
    let oracle_rows = oracle_star_bag(create, r2rml, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "ADR-0032 R6 divergence on `{query}`:\n \
         engine (SQL-rewritten encoding) = {engine:#?}\n \
         oracle (decoded native graph, spareval) = {oracle_rows:#?}"
    );
    engine
}

// ============================================================================
// function-over-mixed-union — CLOSES. `star::top_level::rewrite_filter_over_union`.
// ============================================================================

/// A FILTER (`isTRIPLE`, a genuine static consumer) DIRECTLY wrapping the
/// query's own top-level mixed-composed Union. FORMERLY locked 501 (see the
/// module doc's note on `differential_star.rs`'s companion pins on this EXACT
/// query shape). `isTRIPLE(?t)` now resolves PER ARM instead of once
/// statically: `true` for the 3 propositions (left arm, composes `?t` via
/// `rdf:reifies`), `false` for the 3 plain reifier IRIs (right arm) — so the
/// FILTER keeps only the 3 propositions, never a wrong static answer for
/// either half.
#[test]
fn function_over_mixed_union_filters_to_only_the_composed_arm() {
    let query = format!(
        "{EX}{RDF}SELECT ?t WHERE {{ \
           {{ {{ ?r rdf:reifies ?t }} \
           UNION \
           {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
           FILTER(isTRIPLE(?t)) \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(
        rows.len(),
        3,
        "only the 3 propositions survive: got={rows:#?}"
    );
    assert!(
        rows.iter().all(|r| matches!(&r["t"], Term::Triple(_))),
        "every surviving row's ?t must be a native triple term: got={rows:#?}"
    );
}

/// Companion: `isTRIPLE`'s negation keeps only the non-composed arm — proves
/// the per-arm resolution is not simply "always true" by construction.
#[test]
fn function_over_mixed_union_negated_filters_to_only_the_plain_arm() {
    let query = format!(
        "{EX}{RDF}SELECT ?t WHERE {{ \
           {{ {{ ?r rdf:reifies ?t }} \
           UNION \
           {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
           FILTER(!isTRIPLE(?t)) \
         }}"
    );
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(
        rows.len(),
        3,
        "only the 3 plain reifiers survive: got={rows:#?}"
    );
    assert!(
        rows.iter().all(|r| matches!(&r["t"], Term::NamedNode(_))),
        "every surviving row's ?t must be an ordinary IRI: got={rows:#?}"
    );
}

// ============================================================================
// EXISTS-over-mixed-union — CLOSES. `rewrite_expr`'s `Exists` arm now routes
// through `rewrite_top_level_pattern` instead of plain `rewrite_pattern`.
// ============================================================================

/// A BARE mixed-composed Union AS an EXISTS body (uncorrelated with the
/// outer pattern — EXISTS only contributes a boolean, the SAME "nothing else
/// observes the per-row shape" argument the SELECT top-level relaxation
/// makes). FORMERLY locked 501 (the ordinary, unconditional
/// `rewrite_union(top_level: false)` check fired for ANY reachable Union,
/// including one buried inside an EXISTS body). The union always matches (3
/// propositions via `rdf:reifies`), so EXISTS is true for every person —
/// this test's job is proving the query no longer 501s and the row set is
/// unaffected by the (previously fatal) mixed composed-ness inside the body.
#[test]
fn exists_over_mixed_union_no_longer_501s() {
    let query = format!(
        "{EX}{RDF}SELECT ?p WHERE {{ \
           ?p ex:hasAge ?age . \
           FILTER EXISTS {{ {{ ?r rdf:reifies ?t }} UNION {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
         }}"
    );
    assert_translates_ok(CENSUS_R2RML, &query);
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(
        rows.len(),
        3,
        "EXISTS holds for every person: got={rows:#?}"
    );
}

/// Companion: `NOT EXISTS` over the identical mixed-composed body must
/// correctly resolve to false for every row (the union always matches, so
/// NOT EXISTS eliminates every row) — proves the relaxation applies to the
/// `Not(Exists(..))` desugaring too, not merely bare `EXISTS`.
#[test]
fn not_exists_over_mixed_union_no_longer_501s() {
    let query = format!(
        "{EX}{RDF}SELECT ?p WHERE {{ \
           ?p ex:hasAge ?age . \
           FILTER NOT EXISTS {{ {{ ?r rdf:reifies ?t }} UNION {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
         }}"
    );
    // The correct answer here happens to be an EMPTY row bag (every row is
    // eliminated) — the SAME shape `diff`'s graceful `(Unsupported,
    // Unsupported) => Vec::new()` arm would produce on a 501, so
    // `assert_oracle_agrees` alone cannot tell "translated and correctly
    // computed empty" from "still 501ing"; `assert_translates_ok` closes
    // that gap explicitly.
    assert_translates_ok(CENSUS_R2RML, &query);
    let rows = assert_oracle_agrees(CENSUS_SQL, CENSUS_R2RML, &query);
    assert_eq!(
        rows.len(),
        0,
        "NOT EXISTS eliminates every row: got={rows:#?}"
    );
}

// ============================================================================
// CONSTRUCT-with-mixed-union — the HONEST RESIDUAL. Still 501, by design.
// ============================================================================

/// A CONSTRUCT template referencing the query's own top-level mixed-composed
/// Union. `star::env::substitute_construct_template` is a whole-`Plan`
/// static AST rewrite of the template — ONE substitution for every `Branch`
/// — with no per-`Branch` counterpart to fork the way
/// `rewrite_filter_over_union` forks `env` for a FILTER's `expr`: a `Plan`'s
/// `PlanForm::Construct` carries exactly one template, shared by every arm at
/// execution (`exec_core::instantiate`), so there is no seam to install an
/// arm-specific substitution into. This is genuinely the "single consumer
/// requires cross-branch uniformity per-branch resolution cannot express"
/// case — `rewrite_query` routes CONSTRUCT's own top-level pattern through
/// plain `rewrite_pattern`, never `rewrite_top_level_pattern`, so this
/// query never reaches ANY of this wave's new machinery. Still locked 501.
#[test]
fn construct_with_mixed_union_is_still_a_locked_501() {
    let query = format!(
        "{EX}{RDF}CONSTRUCT {{ ex:CensusRecord2026 ex:mentions ?t }} WHERE {{ \
           {{ ?r rdf:reifies ?t }} \
           UNION \
           {{ ?t ex:assertedBy ex:CensusRecord2026 }} \
         }}"
    );
    assert_locked_501(CENSUS_R2RML, &query);
}

// ============================================================================
// A mixed-composed Union wrapped in a FILTER that is NOT the query's own
// top-level pattern — the boundary stays exactly where claimed. Still 501.
// ============================================================================

/// The IDENTICAL `FILTER(isTRIPLE(?t))`-over-mixed-Union shape
/// `function_over_mixed_union_filters_to_only_the_composed_arm` closes above,
/// but joined against an outer BGP (`?p ex:hasAge ?age .`) — the Union is no
/// longer (modulo pass-through wrappers) the query's ENTIRE top-level
/// pattern, so `rewrite_top_level_pattern`'s `Filter` arm is never reached at
/// all (the outer `Join` falls through its catch-all straight to ordinary
/// `rewrite_pattern`, whose OWN `Filter`/`Union` arms keep the original,
/// unconditional `rewrite_union(top_level: false)` check). Proves the
/// widened boundary is still exactly "a Filter directly wrapping the
/// pattern's own top-level Union", not "a Filter-wrapped mixed Union
/// anywhere reachable from a SELECT".
#[test]
fn filter_over_mixed_union_nested_under_a_join_is_still_a_locked_501() {
    let query = format!(
        "{EX}{RDF}SELECT ?p ?t WHERE {{ \
           ?p ex:hasAge ?age . \
           {{ {{ ?r rdf:reifies ?t }} UNION {{ ?t ex:assertedBy ex:CensusRecord2026 }} }} \
           FILTER(isTRIPLE(?t)) \
         }}"
    );
    assert_locked_501(CENSUS_R2RML, &query);
}

// ============================================================================
// NOTE for whoever next edits `differential_star.rs` (cannot be done from
// this file/wave — that file is owned by a concurrent work stream):
//
// `union_arms_disagreeing_on_composed_ness_wrapped_in_a_filter_is_still_a_locked_501`
// and `values_mixed_triple_and_plain_cells_wrapped_in_a_filter_is_still_a_locked_501`
// (both in `differential_star.rs`) assert `assert_locked_501` on EXACTLY the
// shape `function_over_mixed_union_filters_to_only_the_composed_arm` (union
// variant) and a mixed-VALUES analog of it close in THIS file — Run 4 Wave B2
// (`sf_sparql::star::top_level::rewrite_filter_over_union`) makes both of
// those queries succeed now. Those two tests need the SAME flip this file's
// `star::tests::top_level_union_disagreement_wrapped_in_filter_now_resolves_per_arm`
// / `top_level_mixed_values_wrapped_in_filter_now_resolves_per_arm` unit
// tests already got: replace `assert_locked_501(...)` with an
// `assert_oracle_agrees`-based success assertion, mirroring the ALREADY-
// FLIPPED sibling test immediately above each of them in that same file
// (`..._resolves_at_the_top_level`).
// ============================================================================
