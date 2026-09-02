use std::future::pending;
use std::time::Duration;

use http_body_util::BodyExt;
use oxrdf::{Literal, NamedNode, Term, Triple};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError, QueryLimits};
use sparesults::QueryResultsFormat;
use tokio::sync::oneshot;

use crate::budget::RequestBudget;
use crate::stream::{
    construct_body_streaming_controlled, select_body_streaming_controlled, RdfFormat,
};

fn budget(max_bytes: u64) -> RequestBudget {
    RequestBudget::after(
        Duration::from_secs(60),
        QueryLimits::new(u64::MAX, u64::MAX, max_bytes),
    )
}

fn one_row_body(fmt: QueryResultsFormat, budget: RequestBudget) -> axum::body::Body {
    select_body_streaming_controlled(
        |mut sink| {
            Box::pin(async move {
                sink(vec![Some(Term::Literal(Literal::new_simple_literal(
                    "value",
                )))])
                .await
            })
        },
        fmt,
        vec!["value".to_owned()],
        budget,
    )
}

fn one_triple_body(fmt: RdfFormat, budget: RequestBudget) -> axum::body::Body {
    construct_body_streaming_controlled(
        |mut sink| {
            Box::pin(async move {
                sink(vec![Triple::new(
                    NamedNode::new_unchecked("urn:subject"),
                    NamedNode::new_unchecked("urn:predicate"),
                    Literal::new_simple_literal("object"),
                )])
                .await
            })
        },
        fmt,
        budget,
    )
}

async fn collect(body: axum::body::Body) -> Result<Vec<u8>, String> {
    body.collect()
        .await
        .map(|collected| collected.to_bytes().to_vec())
        .map_err(|error| error.to_string())
}

#[tokio::test]
async fn every_result_serializer_honours_the_exact_byte_boundary() {
    for fmt in [
        QueryResultsFormat::Json,
        QueryResultsFormat::Xml,
        QueryResultsFormat::Csv,
        QueryResultsFormat::Tsv,
    ] {
        let measuring = budget(u64::MAX);
        let bytes = collect(one_row_body(fmt, measuring.clone()))
            .await
            .expect("measure result serialization");
        let size = u64::try_from(bytes.len()).expect("serialized size fits u64");
        assert!(size > 0);
        assert_eq!(measuring.consumed(QueryCharge::SerializedBytes), size);

        let exact = budget(size);
        assert_eq!(
            collect(one_row_body(fmt, exact.clone()))
                .await
                .expect("exact byte budget"),
            bytes
        );
        assert_eq!(exact.consumed(QueryCharge::SerializedBytes), size);

        let below = budget(size - 1);
        assert_eq!(
            collect(one_row_body(fmt, below.clone())).await,
            Err("result stream failed".to_owned())
        );
        assert!(below.consumed(QueryCharge::SerializedBytes) < size);
    }
}

#[tokio::test]
async fn every_rdf_serializer_honours_the_exact_byte_boundary() {
    for fmt in [RdfFormat::Turtle, RdfFormat::NTriples, RdfFormat::JsonLd] {
        let measuring = budget(u64::MAX);
        let bytes = collect(one_triple_body(fmt, measuring.clone()))
            .await
            .expect("measure RDF serialization");
        let size = u64::try_from(bytes.len()).expect("serialized size fits u64");
        assert!(size > 0);
        assert_eq!(measuring.consumed(QueryCharge::SerializedBytes), size);

        let exact = budget(size);
        assert_eq!(
            collect(one_triple_body(fmt, exact.clone()))
                .await
                .expect("exact byte budget"),
            bytes
        );
        assert_eq!(exact.consumed(QueryCharge::SerializedBytes), size);

        let below = budget(size - 1);
        assert_eq!(
            collect(one_triple_body(fmt, below.clone())).await,
            Err("result stream failed".to_owned())
        );
        assert!(below.consumed(QueryCharge::SerializedBytes) < size);
    }
}

struct DropSignal(Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[tokio::test]
async fn dropping_a_sub_chunk_body_cancels_a_stalled_driver() {
    let budget = budget(u64::MAX);
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let body = select_body_streaming_controlled(
        move |_sink| {
            Box::pin(async move {
                let _guard = DropSignal(Some(dropped_tx));
                let _ = started_tx.send(());
                pending::<sf_sparql::Result<()>>().await
            })
        },
        QueryResultsFormat::Json,
        vec!["value".to_owned()],
        budget.clone(),
    );
    started_rx.await.expect("driver reached pending state");

    drop(body);
    tokio::time::timeout(Duration::from_secs(1), dropped_rx)
        .await
        .expect("driver was dropped promptly")
        .expect("drop signal");
    assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
}
