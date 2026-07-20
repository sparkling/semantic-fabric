//! `sf-conformance` — the W3C RDB2RDF correctness gate (ADR-0005) and the engine
//! fitness function (the non-degradation half of the Path-B loop, ADR-0013).
//!
//! For each vendored W3C case (`tests/w3c/rdb2rdf/cases/`, base IRI fixed at
//! `http://example.com/base/`) the harness builds an in-memory SQLite fixture,
//! loads the mapping (R2RML parsed by `sf-mapping`, or Direct Mapping
//! auto-generated from introspection), runs `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p
//! ?o }` through the virtualiser (`sf-sparql`), and adjudicates the produced
//! triples against the expected graph by **blank-node-aware isomorphism**
//! (`oxrdf` RDFC-1.0), cross-checked through the in-memory oracle (zero-JVM). It
//! emits EARL (`earl-semantic-fabric-{r2rml,direct}.ttl`) and exposes the
//! cross-repo `M ⋈ T` SHACL gate ([`shacl_gate`], rudof `shacl` Native).
//!
//! The [`oracle`] module is the **independent second evaluator** (ADR-0005): the
//! W3C gate uses its identity-dump pass, and its real `spareval`-backed
//! [`oracle::evaluate`] runs the *same* SPARQL over the expected graph — general
//! BGP/JOIN/OPTIONAL/FILTER and property paths `P+`/`P*` — for the ADR-0012
//! native-oracle differential where there is no gold file (`tests/`). `spareval`
//! is sanctioned here for the in-memory oracle only, never the OBDA hot path
//! (ADR-0004).

use std::path::Path;

pub mod earl;
pub mod graph;
pub mod manifest;
pub mod oracle;
pub mod pg;
pub mod runner;
pub mod shacl_gate;
pub mod sqlite;
pub mod star_decode;

pub use manifest::Kind;
pub use shacl_gate::{validate as mapping_conforms_to_t, GateOutcome};

/// The adjudication of one case (the EARL outcome space).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Positive case matched (isomorphic); error case correctly signalled.
    Passed,
    /// Positive case produced the wrong graph; error case not rejected — a real,
    /// honest failure on a supported feature.
    Failed,
    /// A feature the engine defers to `501` (documented known-skip), or a fixture
    /// the embedded SQLite cannot load. Never a silent pass.
    Skipped,
}

/// The adjudication of one named test case.
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub id: String,
    pub kind: Kind,
    pub status: Status,
    /// A short, human-readable reason (the 501/skip cause, or the mismatch).
    pub reason: String,
}

/// W3C cases the engine deliberately does not pass, each with a standards-grounded
/// rationale (ADR-0015 §Identifier resolution). These are NOT silent skips and NOT
/// gamed: the EARL report still records each `earl:failed` (truthful), and
/// [`Report::failures`] still lists them. They are excluded only from the
/// *regression* gate ([`Report::unexpected_failures`]) so a NEW, undocumented
/// failure still turns the bar red. Keep this list tiny and every entry justified.
pub const EXPECTED_DEVIATIONS: &[(&str, &str)] = &[(
    "R2RMLTC0002f",
    "negative test: a regular identifier {Name} referencing the delimited, \
     mixed-case column \"Name\" — expected to be rejected per R2RML §5 \
     (SQL:2008 comparison). semantic-fabric resolves identifiers against the \
     live introspected schema (exact, then unique case-insensitive) and \
     accepts it on both suite dialects. The honest status is per-dialect \
     (ADR-0015, corrected 2026-07-20): on SQLite accepting is CORRECT for the \
     database — identifiers are case-insensitive regardless of quoting and \
     introspection preserves no delimited-vs-regular provenance, so strict \
     rejection there would also break the positive twins built on the same \
     pattern (R2RMLTC0002a, R2RMLTC0018a / D018). On PostgreSQL, whose \
     unquoted identifiers case-fold and genuinely do not match, rejection IS \
     achievable without breaking those twins — Ontop v4.1.0 passes 0002f \
     alongside both on PG (rml.io R2RML implementation report, verified \
     2026-07-20) — so on PG this is an OPEN PARITY ITEM (a delimited-aware \
     resolver), not an impossibility. Until that resolver exists, 0002f is a \
     documented deviation on both dialects.",
)];

/// The rationale for an expected deviation, if `id` is one.
pub fn expected_deviation(id: &str) -> Option<&'static str> {
    EXPECTED_DEVIATIONS
        .iter()
        .find(|(c, _)| *c == id)
        .map(|(_, r)| *r)
}

/// The full suite outcome.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub cases: Vec<CaseResult>,
}

impl Report {
    fn count(&self, status: Status, kind: Option<Kind>) -> usize {
        self.cases
            .iter()
            .filter(|c| c.status == status && kind.is_none_or(|k| c.kind == k))
            .count()
    }

    /// Passed cases (optionally filtered by kind).
    pub fn passed(&self, kind: Option<Kind>) -> usize {
        self.count(Status::Passed, kind)
    }
    /// Failed cases.
    pub fn failed(&self, kind: Option<Kind>) -> usize {
        self.count(Status::Failed, kind)
    }
    /// Skipped (501-deferred / unloadable-fixture) cases.
    pub fn skipped(&self, kind: Option<Kind>) -> usize {
        self.count(Status::Skipped, kind)
    }
    /// Adjudicated total = passed + failed (skips are untested, excluded).
    pub fn adjudicated(&self, kind: Option<Kind>) -> usize {
        self.passed(kind) + self.failed(kind)
    }

    /// `"<passed>/<adjudicated>"` for a kind (the EARL split form).
    pub fn split(&self, kind: Kind) -> String {
        format!(
            "{}/{}",
            self.passed(Some(kind)),
            self.adjudicated(Some(kind))
        )
    }

    /// Failed-case identifiers + reasons (for honest reporting).
    pub fn failures(&self) -> Vec<String> {
        self.cases
            .iter()
            .filter(|c| c.status == Status::Failed)
            .map(|c| format!("{}: {}", c.id, c.reason))
            .collect()
    }

    /// Skipped-case identifiers + reasons.
    pub fn skips(&self) -> Vec<String> {
        self.cases
            .iter()
            .filter(|c| c.status == Status::Skipped)
            .map(|c| format!("{}: {}", c.id, c.reason))
            .collect()
    }

    /// Failed cases that are KNOWN, documented deviations ([`EXPECTED_DEVIATIONS`])
    /// — excluded from the regression gate, but still `earl:failed` and still in
    /// [`failures`](Self::failures). Each is reported with its standards rationale.
    pub fn expected_deviations(&self) -> Vec<String> {
        self.cases
            .iter()
            .filter(|c| c.status == Status::Failed)
            .filter_map(|c| expected_deviation(&c.id).map(|r| format!("{}: {r}", c.id)))
            .collect()
    }

    /// Failed cases that are NOT documented deviations — i.e. real regressions. The
    /// conformance gate asserts this is empty (a strictly tighter check than the
    /// pass-count baseline: it also catches a pass↔fail swap at constant count).
    pub fn unexpected_failures(&self) -> Vec<String> {
        self.cases
            .iter()
            .filter(|c| c.status == Status::Failed && expected_deviation(&c.id).is_none())
            .map(|c| format!("{}: {}", c.id, c.reason))
            .collect()
    }
}

/// Run the vendored W3C RDB2RDF suite rooted at `cases_dir` (each child directory
/// is one `D###` scenario with a `manifest.ttl`). Skips a directory with no
/// manifest. I/O errors propagate; per-case engine errors are adjudicated, not
/// propagated (ADR-0005 honesty contract).
pub fn run_suite(cases_dir: &Path) -> std::io::Result<Report> {
    let mut dirs: Vec<_> = std::fs::read_dir(cases_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let mut cases = Vec::new();
    for dir in dirs {
        let manifest_path = dir.join("manifest.ttl");
        let Ok(manifest_text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let parsed = match manifest::parse(&manifest_text) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("manifest parse failed for {}: {e}", dir.display());
                continue;
            }
        };
        for case in &parsed {
            cases.push(runner::run_case(&dir, case));
        }
    }
    Ok(Report { cases })
}

/// Convenience: run the suite and write both EARL reports into `out_dir`.
pub fn run_and_report(cases_dir: &Path, out_dir: &Path) -> std::io::Result<Report> {
    let report = run_suite(cases_dir)?;
    earl::write(
        &report.cases,
        Kind::R2rml,
        &out_dir.join("earl-semantic-fabric-r2rml.ttl"),
    )?;
    earl::write(
        &report.cases,
        Kind::DirectMapping,
        &out_dir.join("earl-semantic-fabric-direct.ttl"),
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    //! Unit tests for `Report`'s deviation-filtering logic — the guard between a
    //! real W3C regression and a silently-absorbed failure. `EXPECTED_DEVIATIONS`
    //! is a real, non-empty const (`R2RMLTC0002f`), so these tests exercise it
    //! directly rather than mocking it: a typo in that list, or a case id that
    //! merely resembles a documented deviation, must never remove a genuine
    //! failure from `unexpected_failures()`.

    use super::*;

    fn case(id: &str, status: Status) -> CaseResult {
        CaseResult {
            id: id.to_string(),
            kind: Kind::R2rml,
            status,
            reason: "test fixture".to_string(),
        }
    }

    /// The one real, documented deviation id — used so these tests exercise the
    /// actual `EXPECTED_DEVIATIONS` const, not a stand-in.
    const KNOWN_DEVIATION_ID: &str = "R2RMLTC0002f";

    #[test]
    fn should_list_documented_failure_in_expected_deviations_when_id_matches() {
        let report = Report {
            cases: vec![case(KNOWN_DEVIATION_ID, Status::Failed)],
        };

        let deviations = report.expected_deviations();
        assert_eq!(deviations.len(), 1);
        assert!(deviations[0].starts_with(KNOWN_DEVIATION_ID));
    }

    #[test]
    fn should_not_list_documented_failure_in_unexpected_failures_when_id_matches() {
        let report = Report {
            cases: vec![case(KNOWN_DEVIATION_ID, Status::Failed)],
        };

        assert!(report.unexpected_failures().is_empty());
    }

    #[test]
    fn should_surface_undocumented_failure_in_unexpected_failures() {
        let report = Report {
            cases: vec![case("R2RMLTC9999z", Status::Failed)],
        };

        let unexpected = report.unexpected_failures();
        assert_eq!(unexpected.len(), 1);
        assert!(unexpected[0].starts_with("R2RMLTC9999z"));
    }

    #[test]
    fn should_not_list_undocumented_failure_in_expected_deviations() {
        let report = Report {
            cases: vec![case("R2RMLTC9999z", Status::Failed)],
        };

        assert!(report.expected_deviations().is_empty());
    }

    #[test]
    fn should_not_count_a_passing_case_as_an_expected_deviation_even_if_its_id_is_listed() {
        // A deviation that has since been fixed: the case now passes, but its id
        // is still (stale-ly) present in EXPECTED_DEVIATIONS. A pass must never
        // be filed as a deviation or a regression — both filters key on
        // `Status::Failed`, so a passing case must vanish from both lists.
        let report = Report {
            cases: vec![case(KNOWN_DEVIATION_ID, Status::Passed)],
        };

        assert!(report.expected_deviations().is_empty());
        assert!(report.unexpected_failures().is_empty());
        assert!(report.failures().is_empty());
        assert_eq!(report.passed(None), 1);
    }

    #[test]
    fn should_surface_genuine_failure_when_its_id_only_resembles_a_documented_deviation() {
        // Simulates a typo: this failing case's id is one character off from the
        // real EXPECTED_DEVIATIONS entry ("R2RMLTC0002f" -> "...0002g"), so it
        // matches no known deviation. A typo anywhere in this relationship (in
        // the const, or in a case id that was meant to match it) must not hide a
        // real failure from the regression gate.
        let typo_id = "R2RMLTC0002g";
        assert!(
            expected_deviation(typo_id).is_none(),
            "fixture id must NOT match a real documented deviation"
        );

        let report = Report {
            cases: vec![case(typo_id, Status::Failed)],
        };

        let unexpected = report.unexpected_failures();
        assert_eq!(unexpected.len(), 1);
        assert!(unexpected[0].starts_with(typo_id));
        assert!(report.expected_deviations().is_empty());
    }

    #[test]
    fn should_return_empty_lists_when_report_has_no_cases() {
        let report = Report { cases: vec![] };

        assert!(report.failures().is_empty());
        assert!(report.expected_deviations().is_empty());
        assert!(report.unexpected_failures().is_empty());
    }

    #[test]
    fn should_return_empty_failure_lists_when_all_cases_pass() {
        let report = Report {
            cases: vec![
                case("R2RMLTC0001a", Status::Passed),
                case("R2RMLTC0003a", Status::Passed),
                case(KNOWN_DEVIATION_ID, Status::Passed),
            ],
        };

        assert!(report.expected_deviations().is_empty());
        assert!(report.unexpected_failures().is_empty());
        assert_eq!(report.passed(None), 3);
    }

    #[test]
    fn should_report_none_for_expected_deviation_lookup_on_unknown_id() {
        assert!(expected_deviation("not-a-real-id").is_none());
    }

    #[test]
    fn should_report_some_for_expected_deviation_lookup_on_known_id() {
        assert!(expected_deviation(KNOWN_DEVIATION_ID).is_some());
    }
}
