//! One immutable provenance catalog for a sealed two-source UNION or join.
use super::*;
use sf_core::SourceId;

pub(super) async fn prepare(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    sources: [SourceId; 2],
    query: &str,
    budget: &RequestBudget,
) -> Result<Arc<Lineage>, Response> {
    let query = query.to_owned();
    let policy_id = cfg.query_admission.policy();
    let result = deadline::run_compiler(budget.clone(), cfg.compiler_permits(), move |control| {
        let bound = snapshot.snapshot().compile_federated_lineage(sources, &query, &control, policy_id)?;
        // Two bounded source catalogs plus their serialized header/temporary
        // PROV-O objects; the same cumulative request budget covers both arms.
        control.consume(QueryCharge::RetainedBytes, 1_048_576)?;
        let policy = control.security_context()
            .map(|c| identifier("policy", &[&c.policy_lineage_digest()]))
            .unwrap_or_else(|| "urn:semantic-fabric:policy:unrestricted-development-v1".into());
        let mut fragments = Vec::with_capacity(2);
        for source in sources {
            let fragment = bound.plan().fragments().iter().find(|f| f.source_id() == source)
                .ok_or_else(|| sf_sparql::Error::Mapping("lineage source mismatch".into()))?;
            let mapping_ids = if let Some(join) = bound.plan().bounded_join() {
                let origins = join.mapping_origins().ok_or_else(|| sf_sparql::Error::Mapping("join origins missing".into()))?;
                vec![origins.iter().find(|(id,_)| *id == source).ok_or_else(|| sf_sparql::Error::Mapping("join source mismatch".into()))?.1.clone()]
            } else {
                fragment.lineage().ok_or_else(|| sf_sparql::Error::Mapping("lineage recipe missing".into()))?.mapping_ids().to_vec()
            };
            let binding = snapshot.snapshot().registry().binding(source).unwrap();
            let scope = binding.compiler().scope();
            let d = scope.digests();
            let source_index = (source.index() as u64).to_be_bytes();
            let source_identity = identifier("source", &[snapshot.snapshot().lineage_identity().as_bytes(), &scope.epoch().0.to_be_bytes(), &source_index, d.mapping().as_bytes(), d.ontology().as_bytes(), d.semantic_admission().as_bytes(), d.schema().as_bytes(), d.constraint_policy().as_bytes(), d.capability().as_bytes()]);
            fragments.push((source, Lineage {
                federated: None, multi_origin: true,
                header: json!({"sourceId":source.index(), "source":source_identity, "mappingCatalog":mapping_ids, "mappingDocument":identifier("mapping-document", &[d.mapping().as_bytes()])}),
                source: source_identity,
                mapping: identifier("mapping-document", &[d.mapping().as_bytes()]),
                request: control.correlation_id().as_str().into(),
                snapshot: String::new(), plan: String::new(), policy: policy.clone(),
            }));
        }
        let snapshot_id = identifier("snapshot", &[snapshot.snapshot().lineage_identity().as_bytes(), fragments[0].1.source.as_bytes(), fragments[1].1.source.as_bytes()]);
        let plan_id = identifier("logical-plan", &[snapshot_id.as_bytes(), query.as_bytes(), policy.as_bytes()]);
        for (_, proof) in &mut fragments { proof.snapshot = snapshot_id.clone(); proof.plan = plan_id.clone(); }
        let mut header = json!({"type":"header", "profile":"bounded-federated-union-lineage-v1", "sources":fragments.iter().map(|(_,p)| &p.header).collect::<Vec<_>>(), "snapshot":snapshot_id, "logicalPlan":plan_id, "policy":policy, "rowKeys":"not-provided"});
        if let Some(join) = bound.plan().bounded_join() {
            header["profile"] = "bounded-federated-join-lineage-v1".into();
            header["maxBuildTriples"] = join.max_build_rows().into();
        } else {
            header["maxWitnessesPerRelation"] = sf_sparql::lineage::MAX_WITNESSES.into();
        }
        let right = fragments.pop().unwrap(); let left = fragments.pop().unwrap();
        Ok(Arc::new(Lineage {
            federated: Some(Box::new([(left.0,Arc::new(left.1)),(right.0,Arc::new(right.1))])),
            multi_origin:true, header, request:control.correlation_id().as_str().into(),
            source:String::new(), mapping:String::new(), snapshot:snapshot_id, plan:plan_id, policy,
        }))
    }).await;
    match result {
        Ok(Ok(proof)) => Ok(proof),
        Ok(Err(error)) => Err(problem::response_for_sparql(&error)),
        Err(CompilerRunError::Control(error)) => Err(problem::response_for_control(error)),
        Err(_) => Err(problem::response(ProblemCode::Internal)),
    }
}

impl Lineage {
    pub(crate) fn matches_join(
        &self,
        join: &sf_sparql::federation::BoundedJoin,
        sources: [SourceId; 2],
    ) -> bool {
        self.header["profile"] == "bounded-federated-join-lineage-v1"
            && join.mapping_origins().is_some_and(|origins| {
                origins.iter().zip(sources).all(|((id, map), source)| {
                    *id == source
                        && self
                            .for_source(Some(source))
                            .is_ok_and(|p| p.header["mappingCatalog"] == json!([map]))
                })
            })
    }

    /// Both mandatory fragments actually contributed to this exact joined row.
    /// Merge their entities and used edges into one activity/result bundle.
    pub(crate) fn bundle_join(&self, ordinal: u64) -> std::io::Result<Value> {
        let sources = self
            .federated
            .as_ref()
            .ok_or_else(|| std::io::Error::other("join sources missing"))?;
        let bundle = |proof: &Lineage| -> std::io::Result<Value> {
            let mut bundle = proof.bundle_for(ordinal, 1)?;
            for node in bundle["@graph"].as_array_mut().unwrap() {
                if node.get("sf:mappingId").is_some() {
                    node["sf:source"] = json!({"@id":proof.source});
                }
            }
            Ok(bundle)
        };
        let mut left = bundle(&sources[0].1)?;
        let right = bundle(&sources[1].1)?;
        let graph = left["@graph"].as_array_mut().unwrap();
        graph[0]["prov:used"].as_array_mut().unwrap().extend(
            right["@graph"][0]["prov:used"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        graph.extend(right["@graph"].as_array().unwrap().iter().skip(2).cloned());
        Ok(left)
    }
    pub(crate) fn for_source(&self, source: Option<SourceId>) -> std::io::Result<&Lineage> {
        match (&self.federated, source) {
            (None, None) => Ok(self),
            (Some(fragments), Some(source)) => fragments
                .iter()
                .find(|(id, _)| *id == source)
                .map(|(_, p)| p.as_ref())
                .ok_or_else(|| std::io::Error::other("lineage source mismatch")),
            _ => Err(std::io::Error::other("lineage source domain mismatch")),
        }
    }
    pub(crate) fn matches_source(
        &self,
        source: SourceId,
        spec: &sf_sparql::lineage::LineageSpec,
    ) -> bool {
        self.for_source(Some(source))
            .is_ok_and(|p| p.header["mappingCatalog"] == json!(spec.mapping_ids()))
    }
}
