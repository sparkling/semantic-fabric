use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

#[test]
fn admission_work_is_finite_and_exact_at_the_boundary() {
    let list = allow(&[A]);
    for template in [BGP, VALUES] {
        for mode in MODES {
            let query = at(template, A);
            let measure = budget(u64::MAX);
            open(mode, &binding(), &query, &list, &measure).unwrap();
            let total = work(&measure);
            assert!(total > 0);
            let exact = budget(total);
            open(mode, &binding(), &query, &list, &exact).unwrap();
            assert_eq!(work(&exact), total);
            assert_eq!(exact.terminal(), None);
            let short = budget(total - 1);
            let error = open(mode, &binding(), &query, &list, &short).unwrap_err();
            assert_eq!(cause_of(&error), Some(EXCEEDED));
            assert_eq!(short.terminal(), Some(EXCEEDED));
        }
    }
}

#[test]
fn dataset_occurrences_are_bounded_before_any_callback() {
    let list = allow(&[A]);
    let many = |count: usize, graph: &str| {
        let clauses = format!("FROM <{graph}> ").repeat(count);
        format!("SELECT ?o {clauses}WHERE {{ ?s <http://ex/p> ?o }}")
    };
    let long = format!("http://ex/{}", "g".repeat(MAX_GRAPH_IRI_BYTES));
    let wide = format!("http://ex/{}", "w".repeat(3990));
    for query in [
        many(MAX_DATASET_GRAPHS + 1, A),
        many(1, &long),
        many(17, &wide),
    ] {
        let (binding, calls) = (binding(), Cell::new(0));
        let constant = |_: ConstantOccurrence<'_>| OK;
        let result = run(
            Mode::Uncached,
            &binding,
            &query,
            &list,
            FREE,
            graphs(&calls, OK),
            constant,
        );
        assert_eq!(rule_of(&result.unwrap_err()), Some(R::DatasetBounds));
        assert_eq!(calls.get(), 0);
    }
    let calls = Cell::new(0);
    let constant = |_: ConstantOccurrence<'_>| OK;
    let query = many(MAX_DATASET_GRAPHS, A);
    run(
        Mode::Uncached,
        &binding(),
        &query,
        &list,
        FREE,
        graphs(&calls, OK),
        constant,
    )
    .unwrap();
    assert_eq!(calls.get(), MAX_DATASET_GRAPHS as u32);
}

#[test]
fn allowlist_is_bounded_validated_and_redacted() {
    let too_many = DatasetGraphAllowlist::new(vec![A; MAX_DATASET_GRAPHS + 1]);
    assert_eq!(too_many.unwrap_err(), DatasetAllowlistError::TooManyGraphs);
    let long = format!("http://ex/{}", "g".repeat(MAX_GRAPH_IRI_BYTES));
    let too_long = DatasetGraphAllowlist::new([long.as_str()]);
    assert_eq!(
        too_long.unwrap_err(),
        DatasetAllowlistError::GraphIriTooLong
    );
    let wide = format!("http://ex/{}", "w".repeat(3990));
    let too_wide = DatasetGraphAllowlist::new(vec![wide.as_str(); 17]);
    assert_eq!(
        too_wide.unwrap_err(),
        DatasetAllowlistError::TotalBytesExceeded
    );
    for invalid in ["relative/g", "", "not an iri"] {
        let result = DatasetGraphAllowlist::new([invalid]);
        assert_eq!(result.unwrap_err(), DatasetAllowlistError::InvalidGraphIri);
    }
    let full = DatasetGraphAllowlist::new(vec![A; MAX_DATASET_GRAPHS]).unwrap();
    assert_eq!(full.len(), MAX_DATASET_GRAPHS);
    let secret = DatasetGraphAllowlist::new(["http://secret.example/g"]).unwrap();
    let rendered = format!("{secret:?}");
    assert!(!rendered.contains("secret"), "{rendered}");
}

#[test]
fn pre_cancelled_control_stops_before_parse_and_callbacks() {
    for mode in MODES {
        let binding = binding();
        let control = budget(u64::MAX);
        control.terminate(CANCELLED);
        let (graph_calls, constant_calls) = (Cell::new(0), Cell::new(0));
        let graph = graphs(&graph_calls, OK);
        let constant = constants(&constant_calls, OK);
        let result = run(
            mode,
            &binding,
            &at(BGP, A),
            &allow(&[A]),
            &control,
            graph,
            constant,
        );
        assert_eq!(cause_of(&result.unwrap_err()), Some(CANCELLED));
        assert_eq!((graph_calls.get(), constant_calls.get()), (0, 0));
        assert_eq!(work(&control), 0);
        assert_eq!(binding.cache_len(), 0);
    }
}

/// Parsing observes no control, so a stop raised while it runs is first seen
/// after the entry checkpoint has already passed.
struct StopAfterEntry {
    budget: QueryBudget,
    cause: QueryControlError,
    entered: AtomicBool,
}

impl QueryControl for StopAfterEntry {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        let entry = self.budget.checkpoint();
        if !self.entered.swap(true, Ordering::SeqCst) {
            self.budget.terminate(self.cause);
        }
        entry
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

#[test]
fn termination_during_parse_precedes_form_refusal_and_callbacks() {
    let list = allow(&[A]);
    let update = "INSERT DATA { <http://ex/s> <http://ex/p> <http://ex/o> }";
    let valid = at(BGP, A);
    for cause in [CANCELLED, DEADLINE] {
        for mode in MODES {
            for query in ["not sparql", update, valid.as_str()] {
                let binding = binding();
                let control = StopAfterEntry {
                    budget: budget(u64::MAX),
                    cause,
                    entered: AtomicBool::new(false),
                };
                let (graph_calls, constant_calls) = (Cell::new(0), Cell::new(0));
                let graph = graphs(&graph_calls, OK);
                let constant = constants(&constant_calls, OK);
                let result = run(mode, &binding, query, &list, &control, graph, constant);
                assert_eq!(cause_of(&result.unwrap_err()), Some(cause), "{query}");
                assert_eq!(control.budget.terminal(), Some(cause), "{query}");
                assert_eq!(work(&control.budget), 0, "{query}");
                assert_eq!((graph_calls.get(), constant_calls.get()), (0, 0), "{query}");
                assert_eq!(binding.cache_len(), 0, "{query}");
            }
        }
    }
}

#[test]
fn termination_inside_either_callback_is_sticky_even_with_a_denial() {
    let list = allow(&[A]);
    for cause in [CANCELLED, DEADLINE] {
        for verdict in [OK, DENY] {
            for in_graph in [true, false] {
                let binding = binding();
                let control = budget(u64::MAX);
                let stop = |found: Verdict| {
                    control.terminate(cause);
                    found
                };
                let query = at(BGP, A);
                let result = if in_graph {
                    let constant = |_: ConstantOccurrence<'_>| OK;
                    run(
                        Mode::Cached,
                        &binding,
                        &query,
                        &list,
                        &control,
                        |_: &str| stop(verdict),
                        constant,
                    )
                } else {
                    let constant = |_: ConstantOccurrence<'_>| stop(verdict);
                    run(
                        Mode::Cached,
                        &binding,
                        &query,
                        &list,
                        &control,
                        |_: &str| OK,
                        constant,
                    )
                };
                assert_eq!(cause_of(&result.unwrap_err()), Some(cause));
                assert_eq!(control.terminal(), Some(cause));
                assert_eq!(binding.cache_len(), 0);
            }
        }
    }
}

#[test]
fn callback_control_failures_stay_typed_control_errors() {
    let failure: Verdict = Err(ConstantCoverageError::Control(DEADLINE));
    for in_graph in [true, false] {
        let binding = binding();
        let control = budget(u64::MAX);
        let query = at(BGP, A);
        let list = allow(&[A]);
        let result = if in_graph {
            let constant = |_: ConstantOccurrence<'_>| OK;
            run(
                Mode::Cached,
                &binding,
                &query,
                &list,
                &control,
                |_: &str| failure,
                constant,
            )
        } else {
            let constant = |_: ConstantOccurrence<'_>| failure;
            run(
                Mode::Cached,
                &binding,
                &query,
                &list,
                &control,
                |_: &str| OK,
                constant,
            )
        };
        assert_eq!(cause_of(&result.unwrap_err()), Some(DEADLINE));
        assert_eq!(control.terminal(), Some(DEADLINE));
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn refusals_render_without_query_or_graph_text() {
    let secret = "http://secret.example/g";
    let list = allow(&[A]);
    let cases = [
        (at(BGP, secret), OK),
        (
            at(
                "ASK FROM <G> { SERVICE <http://secret.example/s> { ?s ?p ?o } }",
                A,
            ),
            OK,
        ),
        (
            at(
                "SELECT ?o FROM <G> WHERE { ?s <http://secret.example/p> 'hidden' }",
                A,
            ),
            DENY,
        ),
    ];
    for (query, verdict) in cases {
        let constant = |_: ConstantOccurrence<'_>| verdict;
        let result = run(
            Mode::Cached,
            &binding(),
            &query,
            &list,
            FREE,
            |_: &str| OK,
            constant,
        );
        let error = result.unwrap_err();
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains("secret"), "{rendered}");
        assert!(!rendered.contains("hidden"), "{rendered}");
    }
}

#[test]
fn each_call_parses_exactly_once_and_cancelled_calls_never_parse() {
    isolated(|| {
        let binding = binding();
        let list = allow(&[A]);
        let query = at(BGP, A);
        let graph_query = at(
            "SELECT ?o FROM <G> WHERE { GRAPH <G> { ?s <http://ex/p> ?o } }",
            A,
        );
        let mut counts = Vec::new();
        for mode in [Mode::Cached, Mode::Cached, Mode::Uncached] {
            let (result, parses) = parse_spans(|| open(mode, &binding, &query, &list, FREE));
            result.unwrap();
            counts.push(parses);
        }
        for (text, verdict) in [
            (query.as_str(), DENY),
            (graph_query.as_str(), OK),
            ("not sparql", OK),
        ] {
            let (result, parses) = parse_spans(|| {
                let constant = |_: ConstantOccurrence<'_>| OK;
                run(
                    Mode::Cached,
                    &binding,
                    text,
                    &list,
                    FREE,
                    |_: &str| verdict,
                    constant,
                )
            });
            assert!(result.is_err(), "{text}");
            counts.push(parses);
        }
        assert_eq!(counts, [1; 6]);
        let control = budget(u64::MAX);
        control.terminate(CANCELLED);
        let (result, parses) =
            parse_spans(|| open(Mode::Cached, &binding, &query, &list, &control));
        assert_eq!(cause_of(&result.unwrap_err()), Some(CANCELLED));
        assert_eq!(parses, 0);
    });
}
