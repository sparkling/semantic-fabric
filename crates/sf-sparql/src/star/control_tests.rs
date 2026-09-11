//! Whole initial-rewrite parity, dynamic expansion and terminal-state proofs.
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::{Query, SparqlParser};

use super::{rewrite_query, rewrite_query_with_work_control};
use crate::Error;

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn parsed(source: &str) -> Query {
    SparqlParser::new().parse_query(source).unwrap()
}

fn fixtures() -> Vec<Query> {
    [
        "SELECT ?s WHERE { ?s ?p ?o }",
        "BASE <http://example.test/> SELECT ?s FROM <g> FROM NAMED <n> WHERE { GRAPH ?g { ?s <p> ?o } }",
        "SELECT ?t WHERE { ?r <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> ?t FILTER(isTRIPLE(?t)) }",
        "SELECT ?s WHERE { ?s <urn:p> <<( <urn:a> <urn:b> <<( <urn:c> <urn:d> 'café 東京' )>> )>> }",
        "SELECT ?t WHERE { VALUES ?t { <<( <urn:a> <urn:p> <urn:b> )>> UNDEF <<( <urn:a> <urn:p> <urn:b> )>> } }",
        "SELECT ?t WHERE { VALUES ?t { <<( <urn:a> <urn:p> <<( <urn:x> <urn:q> <urn:y> )>> )>> UNDEF } }",
        "SELECT ?t WHERE { VALUES ?t { <<( <urn:a> <urn:p> <urn:b> )>> <urn:plain> UNDEF } FILTER(isTRIPLE(?t)) }",
        "SELECT ?t WHERE { BIND(TRIPLE(<urn:s>, <urn:p>, TRIPLE(<urn:a>, <urn:b>, 'x')) AS ?t) FILTER(?t = TRIPLE(<urn:s>, <urn:p>, TRIPLE(<urn:a>, <urn:b>, 'x'))) }",
        "SELECT ?__sf_star_0 ?__sf_star_1 WHERE { ?__sf_star_0 <urn:p> <<( ?__sf_star_1 <urn:q> ?o )>> FILTER EXISTS { ?x <urn:p> <<( ?a <urn:q> ?b )>> } }",
        "SELECT (COUNT(?s) AS ?n) WHERE { ?s <urn:p> ?o OPTIONAL { ?s <urn:q> ?q FILTER(?q > 1) } } GROUP BY ?o ORDER BY ?o LIMIT 3",
        "CONSTRUCT { ?__sf_star_0 <urn:p> ?t } WHERE { ?r <urn:p> <<( ?s <urn:q> ?o )>> }",
        "ASK { ?s <urn:p>/<urn:q> <<( <urn:a> <urn:b> <urn:c> )>> }",
        "SELECT ?s WHERE { ?s <urn:p> <<( <urn:a> <urn:b> <urn:c> )>> . ?s <urn:q> <<( <urn:d> <urn:e> <<( <urn:f> <urn:g> <urn:h> )>> )>> . ?s <urn:r> <<( <urn:i> <urn:j> <urn:k> )>> }",
        "ASK { <<( <urn:a> <urn:b> <<( <urn:c> <urn:d> <urn:e> )>> )>> <urn:p>/<urn:q> <<( <urn:f> <urn:g> <urn:h> )>> }",
        "DESCRIBE <urn:s> WHERE { VALUES ?x { 1 1 UNDEF } }",
    ].into_iter().map(parsed).collect()
}

#[test]
fn star_rewrite_exact_one_short_and_default_corpus_preserve_raw_results() {
    for q in fixtures() {
        let before = q.clone();
        let raw = rewrite_query(&q).unwrap();
        let observed = budget(u64::MAX);
        assert_eq!(rewrite_query_with_work_control(&q, &observed).unwrap(), raw);
        let work = observed.consumed(QueryCharge::CompilerWork);
        assert!(work > 0 && work < 1_000_000, "work={work}: {q}");
        let exact = budget(work);
        assert_eq!(rewrite_query_with_work_control(&q, &exact).unwrap(), raw);
        let short = budget(work - 1);
        assert!(matches!(
            rewrite_query_with_work_control(&q, &short),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        assert_eq!(
            short.checkpoint(),
            Err(QueryControlError::CompilerWorkExceeded)
        );
        assert_eq!(q, before);
        assert_eq!(observed.consumed(QueryCharge::SourceWork), 0);
    }
}

struct Stop {
    budget: QueryBudget,
    calls: AtomicUsize,
    stop: usize,
    cause: QueryControlError,
}
impl QueryControl for Stop {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(&self, charge: QueryCharge, units: u64) -> Result<(), QueryControlError> {
        self.budget.consume(charge, units)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn star_rewrite_observes_cancellation_and_deadline_at_every_paid_boundary() {
    // Small representatives reach mint/collision, env fork, recursive VALUES,
    // expression copies and the normal structural visitor. Larger exactness
    // fixtures above need not multiply this O(checkpoints²) regression.
    for q in [
        fixtures().remove(2),
        fixtures().remove(5),
        fixtures().remove(6),
    ] {
        let observed = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            stop: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        rewrite_query_with_work_control(&q, &observed).unwrap();
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for stop in 1..=observed.calls.load(Ordering::SeqCst) {
                let control = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    stop,
                    cause,
                };
                assert!(
                    matches!(rewrite_query_with_work_control(&q, &control), Err(Error::QueryControl(actual)) if actual == cause),
                    "stop={stop}"
                );
                assert_eq!(control.checkpoint(), Err(cause));
            }
        }
    }
}

fn amplified(level: usize) -> String {
    let mut body = "VALUES ?leaf { 1 }".to_owned();
    for i in 0..level {
        body = format!("{{ VALUES ?t{i} {{ <<( <urn:s> <urn:p> <urn:o> )>> }} }} UNION {{ VALUES ?t{i} {{ <urn:plain> }} }} FILTER EXISTS {{ {body} }}");
    }
    format!("SELECT * WHERE {{ {body} }}")
}

#[test]
fn nested_disagreeing_union_exists_pays_each_actual_rewrite() {
    let mut previous_work = 0;
    let mut previous_output = 0;
    for level in 1..=4 {
        let q = parsed(&amplified(level));
        let control = budget(u64::MAX);
        let actual = rewrite_query_with_work_control(&q, &control).unwrap();
        assert_eq!(actual, rewrite_query(&q).unwrap());
        let work = control.consumed(QueryCharge::CompilerWork);
        let size = actual.0.to_string().len();
        assert!(work > previous_work && size > previous_output);
        if level > 1 {
            assert!(
                size > previous_output * 3 / 2,
                "fixture must actually amplify"
            );
            assert!(matches!(
                rewrite_query_with_work_control(&q, &budget(previous_work)),
                Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
            ));
        }
        previous_work = work;
        previous_output = size;
    }
}

#[test]
fn star_rewrite_keeps_funded_unsupported_and_rejects_deep_input_before_recursion() {
    for source in [
        "SELECT ?t WHERE { BIND(TRIPLE(<urn:s>, <urn:p>, <urn:o>) AS ?t) FILTER(TRIPLE(<urn:s>, <urn:p>, <urn:o>)) }",
        "ASK { VALUES ?t { <<( <urn:a> <urn:p> <urn:b> )>> <urn:plain> } }",
        "SELECT ?t WHERE { { VALUES ?t { <<( <urn:a> <urn:p> <urn:b> )>> } } UNION { VALUES ?t { <urn:plain> } } BIND(isTRIPLE(?t) AS ?flag) }",
    ] {
        let q = parsed(source);
        let raw = rewrite_query(&q).unwrap_err();
        assert!(matches!(raw, Error::Unsupported(_)));
        assert_eq!(rewrite_query_with_work_control(&q, &budget(u64::MAX)).unwrap_err().to_string(), raw.to_string());
        assert!(matches!(rewrite_query_with_work_control(&q, &budget(0)), Err(Error::QueryControl(_))));
    }
    let mut q = parsed("SELECT ?x WHERE { VALUES ?x { 1 } }");
    if let Query::Select { pattern, .. } = &mut q {
        for _ in 0..140 {
            *pattern = spargebra::algebra::GraphPattern::Distinct {
                inner: Box::new(pattern.clone()),
            };
        }
    }
    assert!(matches!(
        rewrite_query_with_work_control(&q, &budget(u64::MAX)),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
}
