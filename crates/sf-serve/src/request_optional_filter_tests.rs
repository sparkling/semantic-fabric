//! Refuse separately inside actual FILTER construction and source validation.
use super::*;
use std::sync::Mutex;
use tracing::{span::Attributes, Id, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

pub(super) const QUERIES: [&str; 2] = [
    "SELECT ?value ?optional WHERE { ?item <http://example.test/a> ?value OPTIONAL { ?item <http://example.test/b> ?optional FILTER(?value = \"one\" && ?optional = \"x\" && ?item = <http://example.test/item/1>) } }",
    "SELECT ?value ?optional WHERE { ?item <http://example.test/a> ?value OPTIONAL { { ?item <http://example.test/b> ?optional } UNION { ?item <http://example.test/b> ?optional } FILTER(?value = \"one\" && ?optional = \"x\" && ?item = <http://example.test/item/1>) } }",
];

pub(super) fn helper_work(query: &str, maps: &[sf_core::ir::TriplesMap]) -> (u64, u64, u64) {
    use sf_core::query_control::QueryBudget;
    struct Marker(usize);
    struct Observe {
        budget: Arc<QueryBudget>,
        bounds: Arc<Mutex<Vec<(usize, u64, u64)>>>,
    }
    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            let kind = match attrs.metadata().name() {
                "sf.compiler.optional_filter_construct" => 0,
                "sf.compiler.optional_filter_validate" => 1,
                _ => return,
            };
            ctx.span(id).unwrap().extensions_mut().insert(Marker(kind));
        }
        fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
            if let Some(marker) = ctx.span(id).unwrap().extensions().get::<Marker>() {
                let work = self.budget.consumed(QueryCharge::CompilerWork);
                self.bounds.lock().unwrap().push((marker.0, work, work));
            }
        }
        fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
            if ctx.span(id).unwrap().extensions().get::<Marker>().is_some() {
                self.bounds.lock().unwrap().last_mut().unwrap().2 =
                    self.budget.consumed(QueryCharge::CompilerWork);
            }
        }
    }
    let budget = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let bounds = Arc::new(Mutex::new(Vec::new()));
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps.to_vec()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        64, // Match RuntimeBinding's cache geometry, not a capacity-one cache.
    );
    tracing::subscriber::with_default(
        tracing_subscriber::registry().with(Observe {
            budget: budget.clone(),
            bounds: bounds.clone(),
        }),
        || {
            binding
                .compile_shared_with_work_control(query, budget.as_ref())
                .inspect(|plan| crate::admission::admit(plan, 0, budget.as_ref()).unwrap())
                .unwrap();
        },
    );
    let bounds = bounds.lock().unwrap();
    let variant = QUERIES.iter().position(|q| *q == query).unwrap();
    assert_eq!(
        bounds.len(),
        [2, 8][variant],
        "both phases of every actual FILTER execute"
    );
    for pair in bounds.chunks_exact(2) {
        assert_eq!((pair[0].0, pair[1].0), (0, 1));
        assert!(pair[0].2 > pair[0].1 + 1024);
        assert_eq!(pair[0].2, pair[1].1);
        assert!(
            pair[1].2 > pair[1].1 + 2,
            "actual IRI source validation is paid"
        );
    }
    let construction = bounds[bounds.len() - 2].2;
    let validation = bounds.last().unwrap().2;
    let input = query.len() as u64;
    // The first refusal is the construction reservation, not the preceding
    // expression/input/branch copies. The second is the validation traversal.
    (
        input + construction - 1,
        input + validation,
        input + budget.consumed(QueryCharge::CompilerWork),
    )
}

#[test]
fn mapped_optional_filter_work_refusal_and_exact_recovery() {
    mapped_process("request_compile::tests::optional_work::filter::mapped_optional_filter_work_refusal_and_exact_recovery", MappedProfile::Filter);
}

// The generic key fixture binds objects as IRI templates and cannot lower this
// literal FILTER. Measure an actual raw-populated hit with these exact maps;
// never calibrate the warm budget from a cold compile or remove the FILTER.
pub(super) fn key_work(query: &str, maps: &[sf_core::ir::TriplesMap]) -> u64 {
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps.to_vec()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        64, // Same geometry for the independently measured warm hit.
    );
    let raw = binding.compile_shared(query).unwrap();
    let control = sf_core::query_control::QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ));
    let warm = binding
        .compile_shared_with_work_control(query, &control)
        .inspect(|plan| crate::admission::admit(plan, 0, &control).unwrap())
        .unwrap();
    assert!(
        Arc::ptr_eq(&raw, &warm),
        "calibrate only key work on a proven hit"
    );
    control.consumed(QueryCharge::CompilerWork)
}
