use super::*;

#[test]
fn admitted_queries_compile_cached_and_uncached_with_graph_separated_keys() {
    let binding = binding();
    let list = allow(&[A, B]);
    let mut entries = 0;
    for template in [VALUES, BGP, "ASK FROM <G> { ?s <http://ex/p> ?o }"] {
        let uncached = open(Mode::Uncached, &binding, &at(template, A), &list, FREE).unwrap();
        assert_eq!(binding.cache_len(), entries, "{template}");
        let cold = open(Mode::Cached, &binding, &at(template, A), &list, FREE).unwrap();
        let warm = open(Mode::Cached, &binding, &at(template, A), &list, FREE).unwrap();
        let other = open(Mode::Cached, &binding, &at(template, B), &list, FREE).unwrap();
        entries += 2;
        assert_eq!(binding.cache_len(), entries, "{template}");
        assert!(Arc::ptr_eq(&cold, &warm));
        assert!(!Arc::ptr_eq(&cold, &uncached));
        assert!(!Arc::ptr_eq(&cold, &other));
    }
}

#[test]
fn normalization_scopes_each_nonempty_bgp_and_keeps_identity_patterns() {
    let list = allow(&[A]);
    let query = at(
        "SELECT ?o FROM <G> FROM <G> WHERE { ?s <http://ex/p> ?o \
         OPTIONAL { ?s <http://ex/q> ?v } FILTER(?o = 'a') }",
        A,
    );
    let admitted = admit(
        &query,
        &list,
        FREE,
        |_: &str| OK,
        |_: ConstantOccurrence<'_>| OK,
    );
    let admitted = admitted.unwrap();
    let dataset = admitted.dataset().expect("normalized dataset");
    assert_eq!(dataset.default.len(), 1);
    assert_eq!(dataset.default[0].as_str(), A);
    assert!(dataset.named.as_ref().is_some_and(Vec::is_empty));
    let text = admitted.to_string();
    assert_eq!(text.matches("GRAPH <http://ex/A>").count(), 2, "{text}");
    for template in [
        "SELECT * FROM <G> WHERE { }",
        "SELECT ?x FROM <G> WHERE { VALUES ?x { 1 1 UNDEF } }",
        "ASK FROM <G> WHERE { }",
    ] {
        let text = at(template, A);
        let parsed = crate::parse_query(&text).unwrap();
        let admitted = admit(
            &text,
            &list,
            FREE,
            |_: &str| OK,
            |_: ConstantOccurrence<'_>| OK,
        );
        let admitted = admitted.unwrap();
        assert_eq!(pattern(&admitted), pattern(&parsed), "{template}");
        assert_eq!(admitted.dataset(), parsed.dataset(), "{template}");
    }
}

#[test]
fn unsupported_shapes_and_datasets_refuse_by_name_before_callbacks() {
    let list = allow(&[A, B]);
    for (template, rule) in REFUSED {
        for mode in MODES {
            let binding = binding();
            let (graph_calls, constant_calls) = (Cell::new(0), Cell::new(0));
            let graph = graphs(&graph_calls, OK);
            let constant = constants(&constant_calls, OK);
            let result = run(
                mode,
                &binding,
                &at(template, A),
                &list,
                FREE,
                graph,
                constant,
            );
            assert_eq!(rule_of(&result.unwrap_err()), Some(*rule), "{template}");
            assert_eq!(
                (graph_calls.get(), constant_calls.get()),
                (0, 0),
                "{template}"
            );
            assert_eq!(binding.cache_len(), 0, "{template}");
        }
    }
}

#[test]
fn graph_denial_is_one_rule_for_membership_and_coverage() {
    for mode in MODES {
        let binding = binding();
        let (graph_calls, constant_calls) = (Cell::new(0), Cell::new(0));
        let result = run(
            mode,
            &binding,
            &at(BGP, B),
            &allow(&[A]),
            FREE,
            graphs(&graph_calls, OK),
            constants(&constant_calls, OK),
        );
        assert_eq!(rule_of(&result.unwrap_err()), Some(R::GraphNotAdmitted));
        assert_eq!((graph_calls.get(), constant_calls.get()), (0, 0));
        let result = run(
            mode,
            &binding,
            &at(BGP, A),
            &allow(&[A]),
            FREE,
            graphs(&graph_calls, DENY),
            constants(&constant_calls, OK),
        );
        assert_eq!(rule_of(&result.unwrap_err()), Some(R::GraphNotAdmitted));
        assert_eq!((graph_calls.get(), constant_calls.get()), (1, 0));
        assert_eq!(binding.cache_len(), 0);
    }
}

#[test]
fn every_warm_call_reruns_allowlist_graph_and_constant_checks() {
    let binding = binding();
    let query = at(BGP, A);
    let cold = open(Mode::Cached, &binding, &query, &allow(&[A, B]), FREE).unwrap();
    let (graph_calls, constant_calls) = (Cell::new(0), Cell::new(0));
    let changed = run(
        Mode::Cached,
        &binding,
        &query,
        &allow(&[B]),
        FREE,
        graphs(&graph_calls, OK),
        constants(&constant_calls, OK),
    );
    assert_eq!(rule_of(&changed.unwrap_err()), Some(R::GraphNotAdmitted));
    assert_eq!((graph_calls.get(), constant_calls.get()), (0, 0));
    let list = allow(&[A]);
    let graph = graphs(&graph_calls, DENY);
    let denied = run(
        Mode::Cached,
        &binding,
        &query,
        &list,
        FREE,
        graph,
        constants(&constant_calls, OK),
    );
    assert_eq!(rule_of(&denied.unwrap_err()), Some(R::GraphNotAdmitted));
    assert_eq!((graph_calls.get(), constant_calls.get()), (1, 0));
    let constant = constants(&constant_calls, DENY);
    let denied = run(
        Mode::Cached,
        &binding,
        &query,
        &list,
        FREE,
        graphs(&graph_calls, OK),
        constant,
    );
    assert_eq!(rule_of(&denied.unwrap_err()), Some(R::ConstantCoverage));
    assert_eq!((graph_calls.get(), constant_calls.get()), (2, 1));
    assert_eq!(binding.cache_len(), 1);
    let again = open(Mode::Cached, &binding, &query, &list, FREE).unwrap();
    assert!(Arc::ptr_eq(&cold, &again));
}

#[test]
fn duplicate_from_admits_every_occurrence_then_shares_the_single_graph_key() {
    let binding = binding();
    let list = allow(&[A]);
    let triple = at(
        "SELECT ?o FROM <G> FROM <G> FROM <G> WHERE { ?s <http://ex/p> ?o }",
        A,
    );
    let calls = Cell::new(0);
    let second_denied = |_: &str| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            DENY
        } else {
            OK
        }
    };
    let constant = |_: ConstantOccurrence<'_>| OK;
    let denied = run(
        Mode::Cached,
        &binding,
        &triple,
        &list,
        FREE,
        second_denied,
        constant,
    );
    assert_eq!(rule_of(&denied.unwrap_err()), Some(R::GraphNotAdmitted));
    assert_eq!((calls.get(), binding.cache_len()), (2, 0));
    calls.set(0);
    let constant = |_: ConstantOccurrence<'_>| OK;
    let cold = run(
        Mode::Cached,
        &binding,
        &triple,
        &list,
        FREE,
        graphs(&calls, OK),
        constant,
    );
    let cold = cold.unwrap();
    assert_eq!((calls.get(), binding.cache_len()), (3, 1));
    let single = open(Mode::Cached, &binding, &at(BGP, A), &list, FREE).unwrap();
    assert!(Arc::ptr_eq(&cold, &single));
    assert_eq!(binding.cache_len(), 1);
}

#[test]
fn callbacks_see_each_graph_occurrence_and_only_original_constants() {
    let query = at(
        "SELECT ?o FROM <G> FROM <G> WHERE { ?s <http://ex/p> ?o . ?s <http://ex/q> 5 }",
        A,
    );
    let mut seen_graphs = Vec::new();
    let mut seen = Vec::new();
    let graph = |iri: &str| {
        seen_graphs.push(iri.to_owned());
        OK
    };
    let constant = |found: ConstantOccurrence<'_>| {
        seen.push((found.iri().to_owned(), found.role()));
        OK
    };
    run(
        Mode::Uncached,
        &binding(),
        &query,
        &allow(&[A]),
        FREE,
        graph,
        constant,
    )
    .unwrap();
    assert_eq!(seen_graphs, [A, A]);
    seen.sort();
    let expected = [
        (P.to_owned(), ConstantRole::Predicate),
        (Q.to_owned(), ConstantRole::Predicate),
        (XSD_INTEGER.to_owned(), ConstantRole::LiteralDatatype),
    ];
    assert_eq!(seen, expected);
}

#[test]
fn existing_generated_entry_points_still_refuse_every_dataset_clause() {
    let binding = binding();
    let refused = GeneratedQueryRefusal::Rule(ShapeRule::DatasetClause);
    for query in [at(BGP, A), at(VALUES, A)] {
        let allow_all = |_: ConstantOccurrence<'_>| OK;
        let cached = binding.compile_shared_with_generated_admission(&query, FREE, allow_all);
        let uncached =
            binding.compile_uncached_shared_with_generated_admission(&query, FREE, allow_all);
        for result in [cached, uncached] {
            let error = result.unwrap_err();
            assert!(matches!(error, GeneratedCompileError::Refused(r) if r == refused));
        }
        let deferred = binding
            .compile_shared_with_generated_admission_deferred(&query, FREE, allow_all)
            .unwrap();
        assert!(
            matches!(deferred, GeneratedDeferred::Refused { refusal, .. } if refusal == refused)
        );
    }
    assert_eq!(binding.cache_len(), 0);
}
