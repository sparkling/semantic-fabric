//! Format only actual origin bits supplied by the owned execution recipe.
use super::*;
use std::io::{self, Write};

impl Lineage {
    fn origin_proofs(&self, bits: u64) -> io::Result<Vec<Lineage>> {
        let ids = self.header["mappingCatalog"]
            .as_array()
            .ok_or_else(|| io::Error::other("missing lineage catalog"))?;
        if bits == 0 || ids.len() > 64 || (ids.len() < 64 && bits >> ids.len() != 0) {
            return Err(io::Error::other("invalid lineage origins"));
        }
        ids.iter()
            .enumerate()
            .filter(|(i, _)| bits & (1u64 << i) != 0)
            .map(|(_, id)| {
                let id = id
                    .as_str()
                    .ok_or_else(|| io::Error::other("invalid mapping identity"))?;
                Ok(Lineage {
                    header: json!({"mappingId":id,"sourceId":self.header["sourceId"]}),
                    multi_origin: false,
                    request: self.request.clone(),
                    source: self.source.clone(),
                    mapping: identifier("mapping-entry", &[self.mapping.as_bytes(), id.as_bytes()]),
                    snapshot: self.snapshot.clone(),
                    plan: self.plan.clone(),
                    policy: self.policy.clone(),
                })
            })
            .collect()
    }

    pub(crate) fn bundle_for(&self, ordinal: u64, bits: u64) -> io::Result<Value> {
        let proofs = self.origin_proofs(bits)?;
        let mut bundle = proofs[0].bundle(ordinal);
        let graph = bundle["@graph"].as_array_mut().unwrap();
        for proof in proofs.iter().skip(1) {
            graph[0]["prov:used"]
                .as_array_mut()
                .unwrap()
                .push(json!({"@id":proof.mapping}));
            graph.push(json!({"@id":proof.mapping,"@type":"prov:Entity","sf:mappingId":proof.header["mappingId"]}));
        }
        graph[0]["prov:used"]
            .as_array_mut()
            .unwrap()
            .push(json!({"@id":self.mapping}));
        Ok(bundle)
    }

    pub(crate) fn write_graph_for<W: Write>(
        &self,
        ordinal: u64,
        triples: &[oxrdf::Triple],
        bits: u64,
        mut writer: W,
    ) -> io::Result<()> {
        for (i, proof) in self.origin_proofs(bits)?.iter().enumerate() {
            // One activity/bundle IRI across origins; repeated metadata has RDF
            // set semantics. Product triples/reifiers are written only once.
            proof.write_graph_dataset(ordinal, if i == 0 { triples } else { &[] }, &mut writer)?;
        }
        let activity = format!(
            "urn:semantic-fabric:result:{}:{ordinal}:activity",
            self.request
        );
        let bundle = format!(
            "urn:semantic-fabric:result:{}:{ordinal}:bundle",
            self.request
        );
        let mut rdf = oxttl::NQuadsSerializer::new().for_writer(&mut writer);
        rdf.serialize_quad(oxrdf::QuadRef::new(
            oxrdf::NamedNodeRef::new_unchecked(&activity),
            oxrdf::NamedNodeRef::new_unchecked("http://www.w3.org/ns/prov#used"),
            oxrdf::NamedNodeRef::new_unchecked(&self.mapping),
            oxrdf::NamedNodeRef::new_unchecked(&bundle),
        ))?;
        rdf.finish();
        Ok(())
    }
}
