//! Deterministic generated query-evaluation train (ADR-0005 / ADR-0012 M4).
//!
//! A normal run executes exactly 5,000 valid cases. Each case composes the
//! existing SQLite loader/introspector, R2RML parser, mapping materializer, four
//! public compiler paths, and the independent `spareval` evaluator. The train is
//! deliberately dependency-free: SplitMix64 derives every fixture and query
//! from a fixed suite seed, and `SF_GENERATED_QE_REPLAY=<ordinal>` replays one
//! bounded case after a failure.

#[path = "generated_qe_train/query.rs"]
mod query_fixture;
#[path = "generated_qe_train/r2rml.rs"]
mod r2rml_fixture;

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::Term;
use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::{graph, sqlite};
use sf_sparql::{
    exec, translate_tree, translate_unoptimized, translate_with, translate_with_flat, Error, Plan,
    Tbox,
};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};

use query_fixture::{generated_case, GeneratedCase};

const SUITE_SEED: u64 = 0x5EED_004D_3451_4531;
const FIXTURE_COUNT: usize = 50;
const CASES_PER_FIXTURE: usize = 100;
const NORMAL_CASES: usize = FIXTURE_COUNT * CASES_PER_FIXTURE;
const REPLAY_ENV: &str = "SF_GENERATED_QE_REPLAY";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

struct Fixture {
    id: usize,
    data_key: u64,
    conn: Connection,
    maps: Vec<sf_core::ir::TriplesMap>,
    schema: Vec<TableSchema>,
    graph: oxrdf::Dataset,
    buckets: [&'static str; 5],
}

#[derive(Default)]
struct Coverage {
    cases: usize,
    saw_duplicate: bool,
    saw_unbound: bool,
    saw_iri: bool,
    saw_plain_literal: bool,
    saw_language_literal: bool,
    saw_typed_literal: bool,
    saw_reversed_projection: bool,
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn fixture_seed(id: usize) -> u64 {
    splitmix64(SUITE_SEED ^ id as u64)
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn build_fixture(id: usize) -> Fixture {
    let data_key = fixture_seed(id);
    let item_table = format!("item_{id}");
    let group_table = format!("group_{id}");
    let language = if data_key & 1 == 0 { "en" } else { "de" };
    let palette = ["red", "blue", "red", "green", "blue"];
    let rotate = (data_key as usize) % palette.len();
    let buckets = std::array::from_fn(|row| palette[(row + rotate) % palette.len()]);

    let mut create = format!(
        "CREATE TABLE {group_table} (gid INTEGER PRIMARY KEY, title TEXT NOT NULL);\n\
         CREATE TABLE {item_table} (\n\
           id INTEGER PRIMARY KEY, label TEXT NOT NULL, bucket TEXT NOT NULL,\n\
           score INTEGER NOT NULL, note TEXT, peer INTEGER, gid INTEGER NOT NULL,\n\
           FOREIGN KEY (gid) REFERENCES {group_table}(gid)\n\
         );\n\
         INSERT INTO {group_table} VALUES (1, 'Alpha'), (2, 'Beta');\n"
    );
    for (row, bucket) in buckets.iter().enumerate() {
        let item_id = row + 1;
        let label = format!("label-{id}-{item_id}");
        let score = ((data_key >> (row * 5)) & 31) + item_id as u64;
        let note = if (data_key >> row) & 1 == 0 {
            "NULL".to_owned()
        } else {
            sql_literal(&format!("note-{id}-{item_id}"))
        };
        let peer = if (data_key >> (row + 8)) & 1 == 0 {
            "NULL".to_owned()
        } else {
            ((item_id % 5) + 1).to_string()
        };
        let gid = 1 + ((data_key >> (row + 16)) & 1);
        create.push_str(&format!(
            "INSERT INTO {item_table} VALUES ({item_id}, {}, {}, {score}, {note}, {peer}, {gid});\n",
            sql_literal(&label),
            sql_literal(bucket),
        ));
    }

    let r2rml = r2rml_fixture::build(id, &item_table, &group_table, language);

    let conn = sqlite::load(&create).unwrap_or_else(|error| {
        panic!("generated fixture SQL failed: fixture={id} data={data_key:#018x} error={error}")
    });
    let schema = sqlite::introspect_all(&conn).unwrap_or_else(|error| {
        panic!("generated fixture introspection failed: fixture={id} error={error}")
    });
    let maps = sf_mapping::parse_r2rml(&r2rml).unwrap_or_else(|error| {
        panic!("generated R2RML failed: fixture={id} data={data_key:#018x} error={error}")
    });
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).unwrap_or_else(|error| {
        panic!("generated materialization failed: fixture={id} error={error}")
    });
    Fixture {
        id,
        data_key,
        conn,
        maps,
        schema,
        graph: graph::quads_to_dataset(&quads),
        buckets,
    }
}

fn error_class(error: &Error) -> &'static str {
    match error {
        Error::Parse(_) => "parse",
        Error::Unsupported(_) => "unsupported-501",
        Error::Mapping(_) => "mapping",
        Error::Sql(_) => "sql",
        Error::Core(_) => "core",
        Error::QueryControl(_) => "query-control",
    }
}

fn run_path(
    name: &str,
    result: sf_sparql::Result<Plan>,
    fixture: &Fixture,
    case: &GeneratedCase,
) -> Vec<BTreeMap<String, Term>> {
    let receipt = case.receipt(fixture);
    let plan = result.unwrap_or_else(|error| {
        panic!(
            "{receipt}\npath={name} stage=compile class={} error={error}",
            error_class(&error)
        )
    });
    let solutions = exec::select(&plan, &fixture.conn).unwrap_or_else(|error| {
        panic!(
            "{receipt}\npath={name} stage=execute class={} error={error}",
            error_class(&error)
        )
    });
    let expected: Vec<String> = case.projected.iter().map(|v| (*v).to_owned()).collect();
    assert_eq!(
        solutions.vars, expected,
        "{receipt}\npath={name} projection order changed"
    );
    oracle::engine_bag(&solutions)
}

fn assert_same(
    expected_name: &str,
    expected: &[BTreeMap<String, Term>],
    actual_name: &str,
    actual: &[BTreeMap<String, Term>],
    fixture: &Fixture,
    case: &GeneratedCase,
) {
    let same = if case.ordered {
        expected == actual
    } else {
        oracle::solutions_bag_eq(expected, actual)
    };
    assert!(
        same,
        "{}\n{expected_name}={expected:?}\n{actual_name}={actual:?}",
        case.receipt(fixture)
    );
}

fn record_coverage(coverage: &mut Coverage, rows: &[BTreeMap<String, Term>], case: &GeneratedCase) {
    coverage.cases += 1;
    coverage.saw_unbound |= rows
        .iter()
        .any(|row| case.projected.iter().any(|name| !row.contains_key(*name)));
    coverage.saw_duplicate |= rows
        .iter()
        .enumerate()
        .any(|(index, row)| rows[index + 1..].contains(row));
    coverage.saw_reversed_projection |=
        matches!(case.projected.first(), Some(&"value" | &"note" | &"name"));
    for term in rows.iter().flat_map(BTreeMap::values) {
        match term {
            Term::NamedNode(_) => coverage.saw_iri = true,
            Term::Literal(literal) if literal.language().is_some() => {
                coverage.saw_language_literal = true;
            }
            Term::Literal(literal) if literal.datatype().as_str() != XSD_STRING => {
                coverage.saw_typed_literal = true;
            }
            Term::Literal(_) => coverage.saw_plain_literal = true,
            Term::BlankNode(_) | Term::Triple(_) => {}
        }
    }
}

fn replay_ordinal() -> Option<usize> {
    let raw = std::env::var_os(REPLAY_ENV)?;
    let raw = raw
        .to_str()
        .unwrap_or_else(|| panic!("{REPLAY_ENV} must be valid UTF-8"));
    let ordinal = raw.parse::<usize>().unwrap_or_else(|_| {
        panic!("{REPLAY_ENV} must be an integer in 0..{NORMAL_CASES}, got {raw:?}")
    });
    assert!(
        ordinal < NORMAL_CASES,
        "{REPLAY_ENV} must be in 0..{NORMAL_CASES}, got {ordinal}"
    );
    Some(ordinal)
}

#[test]
fn generated_sqlite_qe_train_matches_all_compilers_and_spareval() {
    let replay = replay_ordinal();
    let first_fixture = replay.map_or(0, |ordinal| ordinal / CASES_PER_FIXTURE);
    let last_fixture = replay.map_or(FIXTURE_COUNT, |ordinal| ordinal / CASES_PER_FIXTURE + 1);
    let mut coverage = Coverage::default();
    let mut case_identities = BTreeSet::new();

    for fixture_id in first_fixture..last_fixture {
        let fixture = build_fixture(fixture_id);
        let local_start = replay.map_or(0, |ordinal| ordinal % CASES_PER_FIXTURE);
        let local_end = replay.map_or(CASES_PER_FIXTURE, |ordinal| ordinal % CASES_PER_FIXTURE + 1);
        for local in local_start..local_end {
            let case = generated_case(&fixture, local);
            assert!(
                case_identities.insert((fixture.id, case.query.clone())),
                "{}\nduplicate generated fixture/query combination",
                case.receipt(&fixture)
            );
            let parsed: Query =
                SparqlParser::new()
                    .parse_query(&case.query)
                    .unwrap_or_else(|error| {
                        panic!(
                            "{}\nstage=generator-parse error={error}",
                            case.receipt(&fixture)
                        )
                    });
            let flat = run_path(
                "flat",
                translate_with_flat(
                    &parsed,
                    &fixture.maps,
                    Dialect::Sqlite,
                    &Tbox::default(),
                    &fixture.schema,
                ),
                &fixture,
                &case,
            );
            let tree = run_path(
                "tree",
                translate_tree(
                    &parsed,
                    &fixture.maps,
                    &Tbox::default(),
                    Dialect::Sqlite,
                    &fixture.schema,
                ),
                &fixture,
                &case,
            );
            let unoptimized = run_path(
                "unoptimized",
                translate_unoptimized(
                    &parsed,
                    &fixture.maps,
                    Dialect::Sqlite,
                    &Tbox::default(),
                    &fixture.schema,
                ),
                &fixture,
                &case,
            );
            let optimized = run_path(
                "optimized",
                translate_with(
                    &parsed,
                    &fixture.maps,
                    Dialect::Sqlite,
                    &Tbox::default(),
                    &fixture.schema,
                ),
                &fixture,
                &case,
            );
            let oracle_rows =
                match oracle::evaluate(&fixture.graph, &case.query).unwrap_or_else(|error| {
                    panic!("{}\nstage=spareval error={error}", case.receipt(&fixture))
                }) {
                    OracleAnswer::Solutions(rows) => rows,
                    other => panic!(
                        "{}\nstage=spareval class=wrong-form answer={other:?}",
                        case.receipt(&fixture)
                    ),
                };

            assert_same("flat", &flat, "tree", &tree, &fixture, &case);
            assert_same("flat", &flat, "unoptimized", &unoptimized, &fixture, &case);
            assert_same("tree", &tree, "optimized", &optimized, &fixture, &case);
            assert_same(
                "optimized",
                &optimized,
                "spareval",
                &oracle_rows,
                &fixture,
                &case,
            );
            record_coverage(&mut coverage, &optimized, &case);
        }
    }

    if replay.is_none() {
        assert_eq!(coverage.cases, NORMAL_CASES, "normal train case count");
        assert_eq!(
            case_identities.len(),
            NORMAL_CASES,
            "unique generated cases"
        );
        assert!(
            coverage.saw_duplicate,
            "train must exercise bag multiplicity"
        );
        assert!(coverage.saw_unbound, "train must exercise NULL as UNBOUND");
        assert!(coverage.saw_iri, "train must exercise IRI terms");
        assert!(
            coverage.saw_plain_literal,
            "train must exercise plain literals"
        );
        assert!(
            coverage.saw_language_literal,
            "train must exercise language literals"
        );
        assert!(
            coverage.saw_typed_literal,
            "train must exercise typed literals"
        );
        assert!(
            coverage.saw_reversed_projection,
            "train must vary projection order"
        );
    }
}
