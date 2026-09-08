//! Request-local, constant-origin proof and bounded public identifiers (ADR-0017).
//! No source execution, row keys, identity values or persisted provenance.
use std::sync::Arc;

use axum::response::Response;
use serde_json::{json, Value};
use sf_core::query_control::{QueryCharge, QueryControl};
use sha2::{Digest, Sha256};

use crate::{
    activation::RuntimeSnapshotLease,
    budget::RequestBudget,
    config::QueryMode,
    deadline::{self, CompilerRunError},
    problem::{self, ProblemCode},
    ServeConfig,
};

pub(crate) const MEDIA_TYPE: &str = "application/vnd.semantic-fabric.lineage+json-seq";

#[path = "lineage_federation.rs"]
mod federation;
#[path = "lineage_graph.rs"]
mod graph;
#[path = "lineage_multiple.rs"]
mod multiple;

/// Immutable proof is created from the SAME pinned snapshot and query string as
/// execution. It never decorates a caller-supplied/mutable plan. Authorization
/// can only remove rows from this constant-origin profile, not add origins.
pub(crate) struct Lineage {
    federated: Option<Box<FederatedOrigins>>,
    pub(crate) header: Value,
    pub(crate) multi_origin: bool,
    request: String,
    source: String,
    mapping: String,
    snapshot: String,
    plan: String,
    policy: String,
}
type FederatedOrigins = [(sf_core::SourceId, Arc<Lineage>); 2];

pub(crate) async fn prepare(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: &str,
    accept: Option<&str>,
    budget: &RequestBudget,
) -> Result<Option<Arc<Lineage>>, Response> {
    // This extension deliberately requires an exact media type. Reject variant
    // parameters/quality lists mentioning it, rather than silently dropping a
    // provenance request or interpreting q=0 as opt-in.
    let Some(accept) = accept else {
        return Ok(None);
    };
    if !accept
        .to_ascii_lowercase()
        .contains("application/vnd.semantic-fabric.lineage")
    {
        return Ok(None);
    }
    if !accept.trim().eq_ignore_ascii_case(MEDIA_TYPE) {
        return Err(problem::response(ProblemCode::InvalidRequest));
    }
    cfg.query_admission
        .validate(budget)
        .map_err(problem::response)?;
    crate::request_compile::charge_input(query, budget).map_err(problem::response_for_control)?;
    let source_id = match cfg.query_mode() {
        QueryMode::Single(source_id) => source_id,
        QueryMode::SourceAffineUnion(sources) => {
            return federation::prepare(cfg, snapshot, sources, query, budget)
                .await
                .map(Some)
        }
    };
    let query = query.to_owned();
    let result = deadline::run_compiler(budget.clone(), cfg.compiler_permits(), move |control| {
        let binding = snapshot
            .snapshot()
            .registry()
            .binding(source_id)
            .ok_or_else(|| {
                sf_sparql::Error::Unsupported("lineage source is not admitted".into())
            })?;
        let (mapping_id, mapping_ids) =
            match binding.compiler().constant_mapping_origin(&query, &control) {
                Ok(id) => (Some(id), Vec::new()),
                Err(sf_sparql::Error::Unsupported(_)) => {
                    let (_, spec) = binding.compiler().compile_lineage(&query, &control)?;
                    (None, spec.mapping_ids().to_vec())
                }
                Err(error) => return Err(error),
            };
        // Fixed metadata is bounded independently of source/result size; charge
        // it before construction. Every serialized byte is charged by SharedBuf.
        control.consume(QueryCharge::RetainedBytes, 32768)?;
        if mapping_id.is_none() {
            control.consume(QueryCharge::RetainedBytes, 262144)?;
        }
        let scope = binding.compiler().scope();
        let digests = scope.digests();
        let epoch = scope.epoch().0.to_be_bytes();
        let source_index = (source_id.index() as u64).to_be_bytes();
        let snapshot = identifier(
            "snapshot",
            &[
                snapshot.snapshot().lineage_identity().as_bytes(),
                &epoch,
                &source_index,
                digests.mapping().as_bytes(),
                digests.ontology().as_bytes(),
                digests.semantic_admission().as_bytes(),
                digests.schema().as_bytes(),
                digests.constraint_policy().as_bytes(),
                digests.capability().as_bytes(),
            ],
        );
        let policy = control
            .security_context()
            .map(|context| identifier("policy", &[&context.policy_lineage_digest()]))
            .unwrap_or_else(|| "urn:semantic-fabric:policy:unrestricted-development-v1".into());
        // This is the logical compiler-input identity, not an attestation of
        // physical SQL, row-policy parameters, source contents or a release.
        let plan = identifier(
            "logical-plan",
            &[snapshot.as_bytes(), query.as_bytes(), policy.as_bytes()],
        );
        let source = identifier("source", &[snapshot.as_bytes(), &source_index]);
        let mapping = identifier("mapping-document", &[digests.mapping().as_bytes()]);
        let mut header = json!({"type":"header", "profile":"constant-mapping-source-v1",
            "mappingId":mapping_id, "sourceId":source_id.index(), "snapshot":snapshot,
            "logicalPlan":plan, "policy":policy, "rowKeys":"not-provided"});
        let multi_origin = mapping_id.is_none();
        if multi_origin {
            header["profile"] = "bounded-mapping-source-v1".into();
            header.as_object_mut().unwrap().remove("mappingId");
            header["mappingCatalog"] = json!(mapping_ids);
            header["maxWitnessesPerRelation"] = sf_sparql::lineage::MAX_WITNESSES.into();
        }
        Ok(Arc::new(Lineage {
            federated: None,
            multi_origin,
            header,
            request: control.correlation_id().as_str().into(),
            source,
            mapping,
            snapshot,
            plan,
            policy,
        }))
    })
    .await;
    match result {
        Ok(Ok(lineage)) => Ok(Some(lineage)),
        Ok(Err(error)) => Err(problem::response_for_sparql(&error)),
        Err(CompilerRunError::Control(error)) => Err(problem::response_for_control(error)),
        Err(_) => Err(problem::response(ProblemCode::Internal)),
    }
}

impl Lineage {
    /// Only called by the final result sink, never for candidates or dropped rows.
    pub(crate) fn bundle(&self, ordinal: u64) -> Value {
        let root = format!("urn:semantic-fabric:result:{}:{ordinal}", self.request);
        let activity = format!("{root}:activity");
        json!({"@context":{"prov":"http://www.w3.org/ns/prov#",
        "sf":"urn:semantic-fabric:lineage:"},
        "@id":format!("{root}:bundle"), "@type":"prov:Bundle", "@graph":[
            {"@id":activity, "@type":"prov:Activity", "prov:used":[
                {"@id":self.source}, {"@id":self.mapping}],
                "sf:snapshot":{"@id":self.snapshot}, "sf:logicalPlan":{"@id":self.plan},
                "sf:policy":{"@id":self.policy}},
            {"@id":root, "@type":"prov:Entity", "prov:wasGeneratedBy":{"@id":activity}},
            {"@id":self.mapping, "@type":"prov:Entity", "sf:mappingId":self.header["mappingId"]},
            {"@id":self.source, "@type":"prov:Entity", "sf:sourceId":self.header["sourceId"]}
        ]})
    }
}

fn identifier(domain: &str, inputs: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"semantic-fabric/lineage/v1");
    for input in std::iter::once(domain.as_bytes()).chain(inputs.iter().copied()) {
        hash.update((input.len() as u64).to_be_bytes());
        hash.update(input);
    }
    format!("urn:semantic-fabric:{domain}:{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use sf_core::query_control::QueryLimits;
    use std::time::Duration;

    fn proof() -> Arc<Lineage> {
        Arc::new(Lineage {
            federated: None,
            multi_origin: false,
            header: json!({"type":"header", "mappingId":"urn:map", "sourceId":0}),
            request: "test".into(),
            source: "urn:source".into(),
            mapping: "urn:mapping".into(),
            snapshot: "urn:snapshot".into(),
            plan: "urn:plan".into(),
            policy: "urn:policy".into(),
        })
    }

    #[tokio::test]
    async fn lineage_source_or_cleanup_failure_cannot_emit_successful_completion() {
        let budget = RequestBudget::after(
            Duration::from_secs(30),
            QueryLimits::new(10000, 10000, 10000, 100000),
        );
        let body = crate::stream::select_body_streaming_controlled(
            |mut sink| {
                Box::pin(async move {
                    sink(vec![]).await?;
                    Err(sf_sparql::Error::Sql(
                        "secret cleanup failure must not escape".into(),
                    ))
                })
            },
            crate::stream::SelectFormat::Lineage(proof()),
            vec![],
            budget,
        );
        assert_eq!(
            body.collect().await.unwrap_err().to_string(),
            "result stream failed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn lineage_deadline_terminates_a_producer_even_before_a_chunk_is_ready() {
        let budget = RequestBudget::after(
            Duration::from_secs(1),
            QueryLimits::new(10000, 10000, 10000, 100000),
        );
        let body = crate::stream::select_body_streaming_controlled(
            |mut sink| {
                Box::pin(async move {
                    sink(vec![]).await?;
                    std::future::pending().await
                })
            },
            crate::stream::SelectFormat::Lineage(proof()),
            vec![],
            budget,
        );
        assert_eq!(
            body.collect().await.unwrap_err().to_string(),
            "result stream failed"
        );
    }

    #[tokio::test]
    async fn graph_lineage_source_or_cleanup_failure_cannot_complete() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let reached_cleanup = Arc::new(AtomicBool::new(false));
        let cleanup = reached_cleanup.clone();
        let budget = RequestBudget::after(
            Duration::from_secs(30),
            QueryLimits::new(10000, 10000, 10000, 100000),
        );
        let mut body = crate::stream::construct_body_streaming_controlled(
            move |mut sink| {
                Box::pin(async move {
                    let triple = oxrdf::Triple::new(
                        oxrdf::NamedNode::new("urn:s").unwrap(),
                        oxrdf::NamedNode::new("urn:p").unwrap(),
                        oxrdf::Literal::new_simple_literal("x".repeat(20000)),
                    );
                    sink(vec![triple]).await?;
                    cleanup.store(true, Ordering::SeqCst);
                    Err(sf_sparql::Error::Sql("secret cleanup failure".into()))
                })
            },
            crate::stream::GraphFormat::Lineage(proof()),
            budget,
        );
        let mut prefix = Vec::new();
        let mut failed = false;
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => {
                    if let Ok(data) = frame.into_data() {
                        prefix.extend_from_slice(&data);
                    }
                }
                Err(error) => {
                    assert_eq!(error.to_string(), "result stream failed");
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed);
        assert!(reached_cleanup.load(Ordering::SeqCst));
        let prefix = String::from_utf8(prefix).unwrap();
        assert!(!prefix.contains("\"type\":\"complete\"") && !prefix.contains("secret"));
    }

    #[tokio::test(start_paused = true)]
    async fn graph_lineage_deadline_terminates_before_a_chunk_is_ready() {
        let budget = RequestBudget::after(
            Duration::from_secs(1),
            QueryLimits::new(10000, 10000, 10000, 100000),
        );
        let body = crate::stream::construct_body_streaming_controlled(
            |_sink| Box::pin(std::future::pending()),
            crate::stream::GraphFormat::Lineage(proof()),
            budget,
        );
        assert_eq!(
            body.collect().await.unwrap_err().to_string(),
            "result stream failed"
        );
    }

    fn multiple_proof() -> Arc<Lineage> {
        let mut proof = Arc::try_unwrap(proof()).ok().unwrap();
        proof.multi_origin = true;
        proof.header = json!({"type":"header","profile":"bounded-mapping-source-v1","sourceId":0,"mappingCatalog":["urn:a","urn:b"]});
        Arc::new(proof)
    }

    #[tokio::test]
    async fn multiple_lineage_cleanup_failure_cannot_complete() {
        let budget = RequestBudget::after(
            Duration::from_secs(30),
            QueryLimits::new(10000, 10000, 10000, 100000),
        );
        let body = crate::stream::multiple_lineage_body(
            |mut sink| {
                Box::pin(async move {
                    sink(sf_sparql::exec_core::LineageSolution {
                        output: sf_sparql::exec_core::LineageOutput::Row(vec![]),
                        mappings: 3,
                    })
                    .await?;
                    Err(sf_sparql::Error::Sql("private cleanup failure".into()))
                })
            },
            multiple_proof(),
            sf_sparql::PlanForm::Select { vars: vec![] },
            budget,
        );
        assert_eq!(
            body.collect().await.unwrap_err().to_string(),
            "result stream failed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn multiple_lineage_stalled_driver_is_deadline_bound() {
        let budget = RequestBudget::after(
            Duration::from_secs(1),
            QueryLimits::new(10000, 10000, 10000, 100000),
        );
        let body = crate::stream::multiple_lineage_body(
            |_| Box::pin(std::future::pending()),
            multiple_proof(),
            sf_sparql::PlanForm::Select { vars: vec![] },
            budget,
        );
        assert_eq!(
            body.collect().await.unwrap_err().to_string(),
            "result stream failed"
        );
    }
}
