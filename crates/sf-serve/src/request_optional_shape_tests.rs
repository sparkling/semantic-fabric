//! Calibrate the actual initial OPTIONAL shape scan before public admission.
use super::*;
use std::sync::Mutex;
use tracing::{span::Attributes, Id, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

pub(super) fn helper_work(query: &str, maps: &[sf_core::ir::TriplesMap]) -> (u64, u64, u64) {
    use sf_core::query_control::QueryBudget;
    struct Marker;
    struct Observe {
        budget: Arc<QueryBudget>,
        bounds: Arc<Mutex<Vec<(u64, u64)>>>,
    }
    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            if attrs.metadata().name() == "sf.compiler.optional_shape" {
                ctx.span(id).unwrap().extensions_mut().insert(Marker);
            }
        }
        fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
            if ctx.span(id).unwrap().extensions().get::<Marker>().is_some() {
                let work = self.budget.consumed(QueryCharge::CompilerWork);
                self.bounds.lock().unwrap().push((work, work));
            }
        }
        fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
            if ctx.span(id).unwrap().extensions().get::<Marker>().is_some() {
                self.bounds.lock().unwrap().last_mut().unwrap().1 =
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
    assert_eq!(
        bounds.len(),
        1,
        "the actual initial OPTIONAL shape scan executes"
    );
    let (start, end) = bounds[0];
    assert!(end > start + 1, "shape visits are prospectively paid");
    let input = query.len() as u64;
    (
        input + start,
        input + end,
        input + budget.consumed(QueryCharge::CompilerWork),
    )
}

#[test]
fn mapped_optional_shape_refusal_and_recovery() {
    mapped_process(
        "request_compile::tests::optional_work::shape::mapped_optional_shape_refusal_and_recovery",
        MappedProfile::Shape,
    );
}
