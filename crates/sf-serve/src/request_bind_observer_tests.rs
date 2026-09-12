//! Observe actual substitution/projection/output phases, including nested exits.
use super::*;
use std::sync::Mutex;
use tracing::{span::Attributes, Id, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

pub(super) fn work(query: &str) -> (Vec<u64>, u64) {
    phase_work(query, &[0, 1, 2])
}

pub(super) fn projection_work(query: &str) -> (Vec<u64>, u64) {
    phase_work(
        query,
        if query.contains("UNION") {
            &[3, 4, 5]
        } else {
            &[3, 4]
        },
    )
}

pub(super) fn base_work(query: &str) -> (Vec<u64>, u64) {
    phase_work(
        query,
        if query.contains("VALUES") {
            &[6, 7, 8]
        } else {
            &[6]
        },
    )
}

fn phase_work(query: &str, kinds: &[usize]) -> (Vec<u64>, u64) {
    use sf_core::query_control::QueryBudget;
    struct Marker {
        kind: usize,
        index: usize,
    }
    struct Observe {
        control: Arc<QueryBudget>,
        bounds: Arc<Mutex<Vec<(usize, u64, u64)>>>,
    }
    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Observe {
        fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
            let kind = match attrs.metadata().name() {
                "sf.compiler.substitution" => 0,
                "sf.compiler.substitution_expression" => 1,
                "sf.compiler.substitution_binding" => 2,
                "sf.compiler.projection_retention" => 3,
                "sf.compiler.construction_output" => 4,
                "sf.compiler.union_output" => 5,
                "sf.compiler.leaf_output" => 6,
                "sf.compiler.values_materialization" => 7,
                "sf.compiler.join_seed" => 8,
                _ => return,
            };
            ctx.span(id).unwrap().extensions_mut().insert(Marker {
                kind,
                index: usize::MAX,
            });
        }
        fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
            let span = ctx.span(id).unwrap();
            let mut extensions = span.extensions_mut();
            if let Some(marker) = extensions.get_mut::<Marker>() {
                let mut bounds = self.bounds.lock().unwrap();
                marker.index = bounds.len();
                let consumed = self.control.consumed(QueryCharge::CompilerWork);
                bounds.push((marker.kind, consumed, consumed));
            }
        }
        fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
            let span = ctx.span(id).unwrap();
            let extensions = span.extensions();
            if let Some(marker) = extensions.get::<Marker>() {
                self.bounds.lock().unwrap()[marker.index].2 =
                    self.control.consumed(QueryCharge::CompilerWork);
            }
        }
    }
    let control = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let bounds = Arc::new(Mutex::new(Vec::new()));
    let binding = sf_sparql::CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), mapped_fixture()),
        sf_sql::Dialect::Sqlite,
        Default::default(),
        vec![],
        Default::default(),
        64, // Match RuntimeBinding's cache geometry, not a capacity-one cache.
    );
    tracing::subscriber::with_default(
        tracing_subscriber::registry().with(Observe {
            control: control.clone(),
            bounds: bounds.clone(),
        }),
        || {
            binding
                .compile_shared_with_work_control(query, control.as_ref())
                .inspect(|plan| crate::admission::admit(plan, 0, control.as_ref()).unwrap())
                .unwrap();
        },
    );
    let bounds = bounds.lock().unwrap();
    let input = query.len() as u64;
    let cuts = kinds
        .iter()
        .map(|&kind| {
            let (_, start, end) = *bounds
                .iter()
                .rev()
                .find(|b| b.0 == kind)
                .expect("every required actual phase executes");
            assert!(end > start + 1, "actual phase pays work");
            input + end - 1
        })
        .collect();
    (cuts, input + control.consumed(QueryCharge::CompilerWork))
}
