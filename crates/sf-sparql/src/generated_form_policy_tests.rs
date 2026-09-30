use std::cell::Cell;
use std::sync::Arc;

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    UncontrolledQueryControl,
};
use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;
use spargebra::SparqlParser;

use super::test_support::{isolated, parse_spans};
use super::{
    ConstantCoverageError, ConstantOccurrence, GeneratedCompileError, GeneratedQueryRefusal,
    ShapeRule,
};
use crate::{CompilerBinding, Epoch, Error, Plan, Tbox};

const WARM_Q: &str = "ASK { ?s ?p ?o }";
const UPDATE_Q: &str = "INSERT DATA { <http://e/s> <http://e/p> <http://e/o> }";
const CONSTANT_Q: &str = "ASK { <http://e/s> <http://e/p> <http://e/o> }";
const UNSUPPORTED_Q: &str = "SELECT * WHERE { ?s !<http://ex/p> ?o }";
const REFUSAL: &str = "generated-query admission refused: form-not-admitted";
const CANCELLED: QueryControlError = QueryControlError::Cancelled;
const DEADLINE: QueryControlError = QueryControlError::DeadlineExceeded;
const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;

// Each entry is confirmed by the spargebra parse_update oracle in the test below.
const UPDATES: &[&str] = &[
    "LOAD <http://e/d>",
    "LOAD SILENT <http://e/d> INTO GRAPH <http://e/g>",
    "CLEAR DEFAULT",
    "CLEAR NAMED",
    "CLEAR ALL",
    "CLEAR GRAPH <http://e/g>",
    "CLEAR SILENT GRAPH <http://e/g>",
    "DROP DEFAULT",
    "DROP SILENT ALL",
    "DROP GRAPH <http://e/g>",
    "DROP NAMED",
    "CREATE GRAPH <http://e/g>",
    "CREATE SILENT GRAPH <http://e/g>",
    "ADD <http://e/g1> TO <http://e/g2>",
    "ADD SILENT DEFAULT TO GRAPH <http://e/g>",
    "MOVE <http://e/g1> TO <http://e/g2>",
    "COPY DEFAULT TO GRAPH <http://e/g>",
    "INSERT DATA { <http://e/s> <http://e/p> <http://e/o> }",
    "INSERT DATA { GRAPH <http://e/g> { <http://e/s> <http://e/p> <http://e/o> } }",
    "DELETE DATA { <http://e/s> <http://e/p> <http://e/o> }",
    "DELETE WHERE { ?s ?p ?o }",
    "DELETE WHERE { GRAPH <http://e/g> { ?s ?p ?o } }",
    "INSERT { ?s ?p ?o } WHERE { ?s ?p ?o }",
    "WITH <http://e/g> DELETE { ?s ?p ?o } INSERT { ?s ?p 1 } WHERE { ?s ?p ?o }",
    "DELETE { ?s ?p ?o } USING <http://e/g> WHERE { ?s ?p ?o }",
    "INSERT { ?s ?p ?o } USING NAMED <http://e/g> WHERE { ?s ?p ?o }",
    "CREATE GRAPH <http://e/g> ; LOAD <http://e/d> INTO GRAPH <http://e/g>",
    "CLEAR ALL ;",
    "CLEAR ALL ; DROP ALL ;",
    "PREFIX e: <http://e/> CLEAR ALL ; INSERT DATA { e:s e:p e:o }",
    "PREFIX e: <http://e/> INSERT DATA { e:s e:p e:o }",
    "BASE <http://e/> DELETE DATA { <s> <p> <o> }",
    "# leading comment\nCLEAR ALL",
    "CLEAR ALL # trailing comment\n",
    "\n\t  CLEAR ALL",
    "clear all",
    "Drop Silent Graph <http://e/g>",
    "insert data { <http://e/s> <http://e/p> <http://e/o> }",
    "dElEtE wHeRe { ?s ?p ?o }",
    "INSERT DATA { <http://e/s> <http://e/p> 'café 日本 😀' }",
    "INSERT DATA { <http://secret.example/s> <http://e/p> 'hidden-literal' }",
];

// Prologue-only input may be a valid empty update, so it is only claimed non-query here.
// A prologue after ';' is outside the pinned update grammar and stays in this bucket.
const MALFORMED: &[&str] = &[
    "",
    "   \n",
    "# comment only",
    "PREFIX e: <http://e/>",
    "BASE <http://e/>",
    "not sparql",
    "SELECT",
    "SELECT * WHERE {",
    "ASK { ?s ?p ?o",
    "INSERT DATA {",
    "INSERT DATA { ?s <http://e/p> <http://e/o> }",
    "SELECT secret_var WHERE { ?s ?p ?o }",
    "PREFIX e: <http://e/> CLEAR ALL ; PREFIX f: <http://f/> INSERT DATA { f:s f:p f:o }",
];

// Admitted SELECT/ASK text containing UPDATE-like words; the flag says constants exist.
const ADMITTED: [(&str, bool); 4] = [
    (
        "ASK { <http://e/insert> <http://e/delete> 'INSERT DATA' }",
        true,
    ),
    ("SELECT ?load WHERE { ?load <http://e/clear> ?drop }", true),
    ("# INSERT DATA { }\nASK { ?s ?p ?o }", false),
    ("ASK { ?s <http://e/p> 'CLEAR ALL ; DROP ALL' }", true),
];

const STRUCTURAL: [(&str, ShapeRule); 5] = [
    (
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        ShapeRule::ConstructForm,
    ),
    ("DESCRIBE <http://e/x>", ShapeRule::DescribeForm),
    (
        "SELECT * FROM <http://e/g> WHERE { ?s ?p ?o }",
        ShapeRule::DatasetClause,
    ),
    (
        "ASK FROM NAMED <http://e/g> { ?s ?p ?o }",
        ShapeRule::DatasetClause,
    ),
    (
        "ASK { SERVICE <http://e/s> { ?s ?p ?o } }",
        ShapeRule::ServiceInPattern,
    ),
];

#[derive(Clone, Copy)]
enum Mode {
    Cold,
    Warm,
    Uncached,
}

const MODES: [Mode; 3] = [Mode::Cold, Mode::Warm, Mode::Uncached];

fn binding() -> CompilerBinding {
    CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        8,
    )
}

fn prepared(mode: Mode) -> CompilerBinding {
    let binding = binding();
    if matches!(mode, Mode::Warm) {
        binding.compile_shared(WARM_Q).unwrap();
    }
    binding
}

fn budget() -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX))
}

fn counting<'a>(
    calls: &'a Cell<u32>,
) -> impl FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError> + 'a {
    move |_| {
        calls.set(calls.get() + 1);
        Ok(())
    }
}

fn run<F>(
    mode: Mode,
    binding: &CompilerBinding,
    query: &str,
    control: &dyn QueryControl,
    check: F,
) -> Result<Arc<Plan>, GeneratedCompileError>
where
    F: FnMut(ConstantOccurrence<'_>) -> Result<(), ConstantCoverageError>,
{
    if matches!(mode, Mode::Uncached) {
        binding.compile_uncached_shared_with_generated_admission(query, control, check)
    } else {
        binding.compile_shared_with_generated_admission(query, control, check)
    }
}

fn control_cause(error: &GeneratedCompileError) -> Option<QueryControlError> {
    match error {
        GeneratedCompileError::Compiler(Error::QueryControl(cause)) => Some(*cause),
        _ => None,
    }
}

fn assert_form_refused(error: &GeneratedCompileError, query: &str) {
    let named = matches!(
        error,
        GeneratedCompileError::Refused(GeneratedQueryRefusal::Rule(ShapeRule::FormNotAdmitted))
    );
    assert!(named, "{query:?}: {error}");
    assert_eq!(error.to_string(), REFUSAL, "{query:?}");
    let debug = format!("{error:?}");
    assert_eq!(debug, "Refused(Rule(FormNotAdmitted))", "{query:?}");
}

fn assert_refused_on_every_seam(query: &str) {
    for mode in MODES {
        let binding = prepared(mode);
        let entries = binding.cache_len();
        let control = budget();
        let calls = Cell::new(0);
        let result = run(mode, &binding, query, &control, counting(&calls));
        assert_form_refused(&result.unwrap_err(), query);
        assert_eq!(calls.get(), 0, "{query:?}");
        assert_eq!(binding.cache_len(), entries, "{query:?}");
        assert_eq!(control.terminal(), None, "{query:?}");
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0, "{query:?}");
    }
}

#[test]
fn every_confirmed_update_is_refused_by_the_generated_seam() {
    for update in UPDATES {
        let parsed_as_update = SparqlParser::new().parse_update(update).is_ok();
        assert!(parsed_as_update, "oracle rejected update {update:?}");
        let parsed_as_query = SparqlParser::new().parse_query(update).is_ok();
        assert!(!parsed_as_query, "update parsed as query {update:?}");
        assert_refused_on_every_seam(update);
    }
}

#[test]
fn malformed_and_prologue_only_input_gets_the_same_redacted_refusal() {
    for input in MALFORMED {
        let parsed_as_query = SparqlParser::new().parse_query(input).is_ok();
        assert!(!parsed_as_query, "expected non-query {input:?}");
        assert_refused_on_every_seam(input);
    }
}

#[test]
fn select_and_ask_with_update_like_words_stay_admitted() {
    for (query, has_constants) in ADMITTED {
        let parsed = SparqlParser::new().parse_query(query);
        assert!(parsed.is_ok(), "fixture must be a query {query:?}");
        for mode in MODES {
            let binding = prepared(mode);
            let calls = Cell::new(0);
            let result = run(mode, &binding, query, FREE, counting(&calls));
            assert!(result.is_ok(), "{query:?}");
            assert_eq!(calls.get() > 0, has_constants, "{query:?}");
        }
    }
}

#[test]
fn construct_describe_dataset_and_service_keep_their_named_rules() {
    for (query, rule) in STRUCTURAL {
        for mode in MODES {
            let binding = prepared(mode);
            let entries = binding.cache_len();
            let calls = Cell::new(0);
            let error = run(mode, &binding, query, FREE, counting(&calls)).unwrap_err();
            let GeneratedCompileError::Refused(GeneratedQueryRefusal::Rule(found)) = &error else {
                panic!("expected a named rule for {query:?}, got {error:?}");
            };
            assert_eq!(*found, rule, "{query:?}");
            assert_ne!(*found, ShapeRule::FormNotAdmitted, "{query:?}");
            assert_eq!(calls.get(), 0, "{query:?}");
            assert_eq!(binding.cache_len(), entries, "{query:?}");
        }
    }
}

#[test]
fn pre_cancelled_control_precedes_parse_and_form_refusal() {
    for group in [UPDATES, MALFORMED] {
        for query in group.iter().copied() {
            for mode in MODES {
                let binding = prepared(mode);
                let entries = binding.cache_len();
                let control = budget();
                control.terminate(CANCELLED);
                let calls = Cell::new(0);
                let result = run(mode, &binding, query, &control, counting(&calls));
                let error = result.unwrap_err();
                assert_eq!(control_cause(&error), Some(CANCELLED), "{query:?}");
                assert_eq!(calls.get(), 0, "{query:?}");
                assert_eq!(control.consumed(QueryCharge::CompilerWork), 0, "{query:?}");
                assert_eq!(binding.cache_len(), entries, "{query:?}");
            }
        }
    }
}

#[test]
fn sticky_termination_from_the_callback_precedes_later_form_refusals() {
    for cause in [CANCELLED, DEADLINE] {
        let binding = binding();
        let control = budget();
        let result = run(
            Mode::Cold,
            &binding,
            CONSTANT_Q,
            &control,
            |_: ConstantOccurrence<'_>| {
                control.terminate(cause);
                Ok(())
            },
        );
        assert_eq!(control_cause(&result.unwrap_err()), Some(cause));
        assert_eq!(control.terminal(), Some(cause));
        for query in [UPDATE_Q, "not sparql"] {
            let calls = Cell::new(0);
            let result = run(Mode::Cold, &binding, query, &control, counting(&calls));
            assert_eq!(
                control_cause(&result.unwrap_err()),
                Some(cause),
                "{query:?}"
            );
            assert_eq!(calls.get(), 0, "{query:?}");
        }
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn compile_errors_after_admission_stay_typed_compiler_failures() {
    for mode in [Mode::Cold, Mode::Uncached] {
        let binding = binding();
        let calls = Cell::new(0);
        let result = run(mode, &binding, UNSUPPORTED_Q, FREE, counting(&calls));
        let error = result.unwrap_err();
        let typed = matches!(
            &error,
            GeneratedCompileError::Compiler(Error::Unsupported(_))
        );
        assert!(typed, "{error}");
        assert!(calls.get() > 0);
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn default_entry_points_keep_plain_parse_errors() {
    let binding = binding();
    for query in [UPDATE_Q, "CLEAR ALL", "not sparql", ""] {
        let results = [
            binding.compile_shared(query).map(|_| ()),
            binding.compile_uncached_shared(query).map(|_| ()),
            binding
                .compile_shared_with_work_control(query, FREE)
                .map(|_| ()),
            binding
                .compile_uncached_shared_with_work_control(query, FREE)
                .map(|_| ()),
            crate::parse_query(query).map(|_| ()),
        ];
        for result in results {
            assert!(matches!(result, Err(Error::Parse(_))), "{query:?}");
        }
    }
    assert_eq!(binding.cache_len(), 0);
}

#[test]
fn form_refusals_parse_exactly_once_and_cancelled_never_parses() {
    isolated(|| {
        let binding = binding();
        let calls = Cell::new(0);
        let steps = [
            (Mode::Cold, WARM_Q),
            (Mode::Cold, UPDATE_Q),
            (Mode::Cold, "not sparql"),
            (Mode::Warm, WARM_Q),
            (Mode::Warm, UPDATE_Q),
            (Mode::Uncached, UPDATE_Q),
            (Mode::Uncached, ""),
        ];
        let mut counts = Vec::new();
        for (mode, query) in steps {
            let (result, parses) =
                parse_spans(|| run(mode, &binding, query, FREE, counting(&calls)));
            assert_eq!(result.is_ok(), query == WARM_Q, "{query:?}");
            counts.push(parses);
        }
        assert_eq!(counts, [1, 1, 1, 1, 1, 1, 1]);
        let control = budget();
        control.terminate(CANCELLED);
        let (result, parses) =
            parse_spans(|| run(Mode::Cold, &binding, UPDATE_Q, &control, counting(&calls)));
        assert_eq!(control_cause(&result.unwrap_err()), Some(CANCELLED));
        assert_eq!(parses, 0);
    });
}
