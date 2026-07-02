//! ADR-0023 OPTION_B **empirical probe** — for a representative scenario from each
//! optimizer family in the OPTION_B analysis (union-structural, boolean-push,
//! join-elim, projection-and-true) this constructs a minimal SPARQL + R2RML +
//! SQLite fixture and runs THREE evaluators:
//!
//!   * `translate_with_flat` — the proven FLAT path (the `=_bag` set-faithful oracle),
//!   * `translate_tree`      — the operator-tree (IQ) path, now the default since M8,
//!   * `spareval`            — the INDEPENDENT in-memory SPARQL oracle (ADR-0004/0005).
//!
//! The load-bearing question is NOT "does the tree reproduce Ontop's node-shape"
//! (the analyses already say most rewrites are cosmetic for the bag) but the
//! purely empirical: **does the TREE result `=_bag` spareval?** That is the real
//! test of whether the tree handles the scenario, trusting the probe over any
//! conceptual guess (harness rule).
//!
//! Each scenario carries an EXPECTED disposition from the analysis:
//!   * `FreePass`    — the analysis predicts `free-pass-likely`; we ASSERT
//!     `tree =_bag spareval` (a real regression gate — these must hold).
//!   * `NeedsRewrite`— the analysis predicts `needs-tree-rewrite`; we RECORD the
//!     verdict and `eprintln!` a work-list line, but do NOT fail the build
//!     (these are the backlog, not regressions). If one empirically already
//!     matches, that is reported too (a free win), but never asserted.
//!
//! The test PRINTS a categorized table (scenario -> flat / tree / spareval /
//! verdict). Run with:
//!   cargo test -p sf-conformance --test option_b_probe -- --nocapture

use std::collections::BTreeMap;

use oxrdf::Term;
use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::sqlite;
use sf_sparql::{exec, translate_tree, translate_with_flat, Plan, PlanForm, Tbox};
use sf_sql::Dialect;
use spargebra::{Query, SparqlParser};

const BASE: &str = "http://ex/";
const PFX: &str = "PREFIX ex: <http://ex/>";

/// The disposition the OPTION_B analysis assigned the scenario.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Disp {
    FreePass,
    NeedsRewrite,
}

/// One probe scenario: a self-contained fixture + query, tagged with its family
/// and the analysis's predicted disposition.
struct Scenario {
    name: &'static str,
    family: &'static str,
    disp: Disp,
    create: &'static str,
    r2rml: &'static str,
    ttl: &'static str,
    query: String,
}

/// A translation+execution outcome, comparable as a bag.
enum Outcome {
    Rows(Vec<BTreeMap<String, Term>>),
    Err(String),
}

impl Outcome {
    fn label(&self) -> String {
        match self {
            Outcome::Rows(r) => format!("Ok({})", r.len()),
            Outcome::Err(e) => {
                let one = e.lines().next().unwrap_or("");
                let short: String = one.chars().take(28).collect();
                format!("Err[{short}]")
            }
        }
    }
}

/// The empirical verdict — the only thing that matters: does the TREE `=_bag`
/// the independent spareval oracle?
#[derive(PartialEq, Eq)]
enum Verdict {
    /// tree Ok and `tree =_bag spareval`.
    Match,
    /// tree Ok but `tree !=_bag spareval` (a real divergence).
    Mismatch,
    /// tree returned Err (translate or exec) — the scenario is not handled.
    TreeErr,
    /// spareval itself failed (probe-authoring error; never expected).
    OracleErr,
}

impl Verdict {
    fn label(&self) -> &'static str {
        match self {
            Verdict::Match => "MATCH(tree=_bag spareval)",
            Verdict::Mismatch => "MISMATCH",
            Verdict::TreeErr => "TREE-ERR",
            Verdict::OracleErr => "ORACLE-ERR",
        }
    }
}

fn parse(q: &str) -> Query {
    SparqlParser::new().parse_query(q).expect("query parses")
}

/// Translate (flat or tree) and, if Ok and a SELECT, execute into a row bag.
fn run_path(plan: sf_sparql::Result<Plan>, conn: &Connection) -> Outcome {
    match plan {
        Err(e) => Outcome::Err(format!("{e}")),
        Ok(p) => match &p.form {
            PlanForm::Select { .. } => match exec::select(&p, conn) {
                Ok(sols) => Outcome::Rows(oracle::engine_bag(&sols)),
                Err(e) => Outcome::Err(format!("{e}")),
            },
            other => Outcome::Err(format!("non-SELECT plan form: {other:?}")),
        },
    }
}

/// Independent spareval oracle over the hand-authored expected graph.
fn run_oracle(ttl: &str, query: &str) -> Result<Vec<BTreeMap<String, Term>>, String> {
    let g = sf_conformance::graph::parse_turtle(ttl, BASE).map_err(|e| format!("{e:?}"))?;
    match oracle::evaluate(&g, query)? {
        OracleAnswer::Solutions(rows) => Ok(rows),
        other => Err(format!("oracle returned non-SELECT answer: {other:?}")),
    }
}

/// Probe one scenario: run flat, tree, spareval; return (flat, tree, oracle, verdict).
fn probe(s: &Scenario) -> (Outcome, Outcome, String, Verdict) {
    let conn = sqlite::load(s.create).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspect");
    let maps = sf_mapping::parse_r2rml(s.r2rml).expect("R2RML parses");
    let q = parse(&s.query);

    let flat = run_path(
        translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema),
        &conn,
    );
    let tree = run_path(
        translate_tree(&q, &maps, &Tbox::default(), Dialect::Sqlite, &schema),
        &conn,
    );

    let (oracle_label, verdict) = match run_oracle(s.ttl, &s.query) {
        Err(e) => (format!("Err[{e}]"), Verdict::OracleErr),
        Ok(orows) => {
            let lbl = format!("Ok({})", orows.len());
            let v = match &tree {
                Outcome::Err(_) => Verdict::TreeErr,
                Outcome::Rows(tr) => {
                    if oracle::solutions_bag_eq(tr, &orows) {
                        Verdict::Match
                    } else {
                        Verdict::Mismatch
                    }
                }
            };
            (lbl, v)
        }
    };
    (flat, tree, oracle_label, verdict)
}

// ============================================================================
// Fixtures (minimal, reused across scenarios).
// ============================================================================

// Fixture P — person ⟕ dept (the proven differential fixture). PK subjects, an FK
// dept join, a nullable `email`. Set-faithful for OPTIONAL arms.
const P_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    dept_id INTEGER NOT NULL,
    email TEXT,
    FOREIGN KEY (dept_id) REFERENCES dept(id)
);
INSERT INTO dept VALUES (10, 'Sales');
INSERT INTO person VALUES (1, 'Ann', 10, 'ann@x');
INSERT INTO person VALUES (2, 'Bob', 10, NULL);
INSERT INTO person VALUES (3, 'Zed', 10, 'zed@x');
"#;

const P_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name  ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:email ; rr:objectMap [ rr:column "email" ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:dept ;
        rr:objectMap [
            rr:parentTriplesMap <#Dept> ;
            rr:joinCondition [ rr:child "dept_id" ; rr:parent "id" ]
        ]
    ] .
<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

const P_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/person/1> ex:name "Ann" ; ex:dept <http://ex/dept/10> ; ex:email "ann@x" .
<http://ex/person/2> ex:name "Bob" ; ex:dept <http://ex/dept/10> .
<http://ex/person/3> ex:name "Zed" ; ex:dept <http://ex/dept/10> ; ex:email "zed@x" .
<http://ex/dept/10> ex:label "Sales" .
"#;

// Fixture U — one wide table whose columns map to ex:x / ex:y / ex:z. All NOT
// NULL ⇒ set-faithful for UNION / projection probes.
const U_SQL: &str = r#"
CREATE TABLE u (id INTEGER PRIMARY KEY, x TEXT NOT NULL, y TEXT NOT NULL, z TEXT NOT NULL);
INSERT INTO u VALUES (1, 'x1', 'y1', 'z1');
INSERT INTO u VALUES (2, 'x2', 'y2', 'z2');
"#;

const U_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#U>
    rr:logicalTable [ rr:tableName "u" ] ;
    rr:subjectMap [ rr:template "http://ex/u/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:x ; rr:objectMap [ rr:column "x" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:y ; rr:objectMap [ rr:column "y" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:z ; rr:objectMap [ rr:column "z" ] ] .
"#;

const U_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/u/1> ex:x "x1" ; ex:y "y1" ; ex:z "z1" .
<http://ex/u/2> ex:x "x2" ; ex:y "y2" ; ex:z "z2" .
"#;

fn scenarios() -> Vec<Scenario> {
    vec![
        // --- union-structural ---------------------------------------------
        Scenario {
            name: "union-structural/flattenUnion (nested UNION)",
            family: "union-structural",
            disp: Disp::FreePass,
            create: U_SQL,
            r2rml: U_R2RML,
            ttl: U_TTL,
            // Nested UNION (X over (X over (X))); n-ary Union + Vec<Branch>
            // concatenation flattens for free. Bag {x*,y*,z*} = 6.
            query: format!(
                "{PFX} SELECT ?v WHERE {{ {{ ?s ex:x ?v }} UNION \
                 {{ {{ ?s ex:y ?v }} UNION {{ ?s ex:z ?v }} }} }}"
            ),
        },
        Scenario {
            name: "union-structural/ValuesNode constant-fold (BIND-union)",
            family: "union-structural",
            disp: Disp::NeedsRewrite,
            create: U_SQL,
            r2rml: U_R2RML,
            ttl: U_TTL,
            // Union of constant-only arms (Construct{X=c}/True) — Ontop folds to a
            // ValuesNode (UnionAndBindingLift). Non-load-bearing for =_bag.
            query: format!(
                "{PFX} SELECT ?x WHERE {{ {{ BIND(1 AS ?x) }} UNION {{ BIND(2 AS ?x) }} }}"
            ),
        },
        // --- boolean-push -------------------------------------------------
        Scenario {
            name: "boolean-push/JoiningCondition (join + FILTER pushdown)",
            family: "boolean-push",
            disp: Disp::FreePass,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            // InnerJoin chain + conjunctive FILTER; conjunct placement is cosmetic
            // for the bag (inner-join assoc/comm). Ann/Sales, Zed/Sales.
            query: format!(
                "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name . ?p ex:dept ?d . \
                 ?d ex:label ?label . FILTER(?name != \"Bob\") }}"
            ),
        },
        // --- join-elim ----------------------------------------------------
        Scenario {
            name: "join-elim/self-leftjoin on PK (OPTIONAL single-scan right)",
            family: "join-elim",
            disp: Disp::FreePass,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            // OPTIONAL with a single-scan right on the PK subject — the canonical
            // self-leftjoin shape (cascade joinelim). Bob's email stays unbound.
            query: format!(
                "{PFX} SELECT ?name ?email WHERE {{ ?p ex:name ?name \
                 OPTIONAL {{ ?p ex:email ?email }} }}"
            ),
        },
        Scenario {
            name: "join-elim/JoinTransfer (OPTIONAL over multi-atom InnerJoin right)",
            family: "join-elim",
            disp: Disp::NeedsRewrite,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            // The OPTIONAL right is a 2-scan InnerJoin (dept refObject join + label):
            // v1 lowering can only put a single scan in Branch.opts ⇒ the flat cascade
            // never sees this shape (the multi-scan-right obstacle). All in dept 10.
            query: format!(
                "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
                 OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label }} }}"
            ),
        },
        Scenario {
            name: "join-elim/LJReductionWithLJOnTheRight (right-nested OPTIONAL)",
            family: "join-elim",
            // ADR-0023 M4 wave 3 — CLOSED: the top OPTIONAL's right is itself an OPTIONAL
            // (right-nested LeftJoin). The tree lowers the right operand OPTS-FREE via the
            // `(P⋈R)∪(P−R)` decomposition (§5.3), then re-feeds it into the outer
            // `left_join_branches`. The FLAT path still 501s (frozen); the tree EXCEEDS it.
            disp: Disp::FreePass,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            query: format!(
                "{PFX} SELECT ?name ?label ?email WHERE {{ ?p ex:name ?name \
                 OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label \
                 OPTIONAL {{ ?p ex:email ?email }} }} }}"
            ),
        },
        Scenario {
            name: "join-elim/FDOnRight (DISTINCT over L OPT (R1 OPT R2), shared FD-det)",
            family: "join-elim",
            disp: Disp::NeedsRewrite,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            // ADR-0023 optimizer-residue Wave B pre-work: DISTINCT over a right-nested
            // OPTIONAL (Group C's decomposition), projecting away the innermost
            // ?email so only the FD-determined (?p -> ?d -> ?label) columns survive.
            // Ontop's FDOnRight collapses this via FD-driven right-side elimination;
            // the tree closes it via Group C's decomposition + per-branch dedup
            // instead — added here (per the "oracle law") to get an EMPIRICAL
            // verdict before any Group D implementation, since this scenario had
            // zero probe coverage. Ann/Sales, Bob/Sales, Zed/Sales (each name
            // already unique, so DISTINCT is a no-op on the bag).
            query: format!(
                "{PFX} SELECT DISTINCT ?name ?label WHERE {{ ?p ex:name ?name \
                 OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label \
                 OPTIONAL {{ ?p ex:email ?email }} }} }}"
            ),
        },
        Scenario {
            name: "join-elim/FDSimplification (nested OPT + per-right FILTER + ancestor FILTER)",
            family: "join-elim",
            disp: Disp::NeedsRewrite,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            // ADR-0023 optimizer-residue Wave B pre-work: a right-nested OPTIONAL
            // (Group C decomposition) with a FILTER inside the OPTIONAL right
            // (per-right, on the FD-determined ?label) PLUS an outer ancestor
            // FILTER on ?name — the combination Ontop's FDSimplification targets.
            // No dept is labelled "X" (inner FILTER a no-op); Bob is dropped by the
            // outer FILTER. Expected bag: Ann/Sales, Zed/Sales.
            query: format!(
                "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
                 OPTIONAL {{ ?p ex:dept ?d . ?d ex:label ?label FILTER(?label != \"X\") }} \
                 FILTER(?name != \"Bob\") }}"
            ),
        },
        Scenario {
            name: "join-elim/PaddingForUnsatisfiableRight (UNION right, ALL arms unsat)",
            family: "join-elim",
            disp: Disp::NeedsRewrite,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            // ADR-0023 optimizer-residue Wave B pre-work: the OPTIONAL right is a UNION
            // where EVERY arm is provably unsatisfiable (no dept is labelled "NoSuchA"/
            // "NoSuchB") -- Ontop's PaddingForUnsatisfiableRight (UNION-right variant)
            // NULL-pads the OPTIONAL rather than distributing the unsat-prune INTO the
            // union (which would wrongly make the whole OPTIONAL disappear instead of
            // padding). FILTERs on ?label (a plain `rr:column` binding), NOT ?d (the
            // refObjectMap-derived dept IRI) -- filtering ?d hits an UNRELATED,
            // orthogonal v1 limitation ("FILTER on ?d needs a plain column binding")
            // that would confound this probe with a different gap entirely. Group C's
            // generic decomposition re-feeds the (opts-free, still 2-armed) union into
            // `left_join_branches`'s multi-branch path, which emits its OWN correlated
            // NOT-EXISTS no-match branch over BOTH arms together -- added here (zero
            // prior probe coverage) to empirically confirm padding, not silent
            // disappearance. Expected: 3 rows (Ann/Bob/Zed), ?label unbound on every row.
            query: format!(
                "{PFX} SELECT ?name ?label WHERE {{ ?p ex:name ?name \
                 OPTIONAL {{ {{ ?p ex:dept ?d . ?d ex:label ?label . FILTER(?label = \"NoSuchA\") }} \
                 UNION {{ ?p ex:dept ?d . ?d ex:label ?label . FILTER(?label = \"NoSuchB\") }} }} }}"
            ),
        },
        Scenario {
            name: "join-elim/OPTIONAL over a UNION right (multi-branch opts-free right)",
            family: "join-elim",
            // ADR-0023 M4 wave 3 — the OPTIONAL right is a UNION. The right lowers to
            // multi-branch opts-free branches and re-feeds `left_join_branches` (multi-
            // branch decomposition). Over the all-NOT-NULL U fixture (set-faithful vs
            // spareval): each ?s has both ?y and ?z, so the OPTIONAL union matches twice
            // per row ⇒ {(x1,y1),(x1,z1),(x2,y2),(x2,z2)} = 4 rows.
            disp: Disp::FreePass,
            create: U_SQL,
            r2rml: U_R2RML,
            ttl: U_TTL,
            query: format!(
                "{PFX} SELECT ?v ?w WHERE {{ ?s ex:x ?v \
                 OPTIONAL {{ {{ ?s ex:y ?w }} UNION {{ ?s ex:z ?w }} }} }}"
            ),
        },
        // --- projection-and-true ------------------------------------------
        Scenario {
            name: "projection-and-true/projection-shrink over UNION",
            family: "projection-and-true",
            disp: Disp::FreePass,
            create: U_SQL,
            r2rml: U_R2RML,
            ttl: U_TTL,
            // Outer projection keeps only ?v; the union arms shrink (unused cols
            // dropped). Bag {x1,x2,y1,y2} = 4.
            query: format!("{PFX} SELECT ?v WHERE {{ {{ ?s ex:x ?v }} UNION {{ ?s ex:y ?v }} }}"),
        },
        Scenario {
            name: "projection-and-true/PullOutVariable (shared-var self join)",
            family: "projection-and-true",
            disp: Disp::FreePass,
            create: P_SQL,
            r2rml: P_R2RML,
            ttl: P_TTL,
            // Shared variable across two InnerJoin atoms → implicit equality lowered
            // to a ColEq (the explicit-equality artifact stays at LOWER).
            query: format!(
                "{PFX} SELECT ?name ?name2 WHERE {{ ?p ex:name ?name . ?p ex:name ?name2 }}"
            ),
        },
    ]
}

#[test]
fn option_b_empirical_probe() {
    let scen = scenarios();

    // Probe every scenario first, then print the categorized table, then gate.
    let mut results: Vec<(usize, Outcome, Outcome, String, Verdict)> = Vec::new();
    for (i, s) in scen.iter().enumerate() {
        let (flat, tree, oracle_label, verdict) = probe(s);
        results.push((i, flat, tree, oracle_label, verdict));
    }

    eprintln!("\n========================= OPTION_B EMPIRICAL PROBE =========================");
    eprintln!(
        "{:<6} {:<22} {:<10} {:<10} {:<10} VERDICT",
        "DISP", "FAMILY", "FLAT", "TREE", "SPAREVAL"
    );
    eprintln!("---------------------------------------------------------------------------");
    for (i, flat, tree, oracle_label, verdict) in &results {
        let s = &scen[*i];
        let disp = match s.disp {
            Disp::FreePass => "FREE",
            Disp::NeedsRewrite => "NEED",
        };
        eprintln!(
            "{:<6} {:<22} {:<10} {:<10} {:<10} {}",
            disp,
            s.family,
            flat.label(),
            tree.label(),
            oracle_label,
            verdict.label()
        );
        eprintln!("        scenario: {}", s.name);
    }
    eprintln!("---------------------------------------------------------------------------");

    // Work-list: NeedsRewrite scenarios that do NOT yet match (the backlog) and any
    // that already match (free wins). Never fail the build on these.
    for (i, _flat, _tree, _oracle, verdict) in &results {
        let s = &scen[*i];
        if s.disp == Disp::NeedsRewrite {
            match verdict {
                Verdict::Match => eprintln!(
                    "[OPTION_B free-win] needs-rewrite scenario ALREADY tree=_bag spareval: {}",
                    s.name
                ),
                Verdict::Mismatch | Verdict::TreeErr | Verdict::OracleErr => eprintln!(
                    "[OPTION_B work-list] needs-rewrite ({}) — tree does NOT yet match spareval: {}",
                    verdict.label(),
                    s.name
                ),
            }
        }
    }
    eprintln!("===========================================================================\n");

    // GATE: assert only the FreePass scenarios — these are the regression contract
    // (the analysis predicts the tree already handles them =_bag). NeedsRewrite
    // scenarios are recorded above but never asserted (work-list, not regressions).
    for (i, _flat, _tree, _oracle, verdict) in &results {
        let s = &scen[*i];
        if s.disp == Disp::FreePass {
            assert!(
                *verdict == Verdict::Match,
                "FREE-PASS regression: scenario `{}` expected tree=_bag spareval but got {}",
                s.name,
                verdict.label()
            );
        }
    }
}
