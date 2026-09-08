//! One response-wide native RDF dataset: product triples in the default graph,
//! all metadata in named bundles. Borrow triple terms so nested terms/blank-node
//! identity are preserved without cloning source-sized values.
use std::io::{self, Write};

use oxrdf::{vocab::rdf, GraphNameRef, Literal, NamedNodeRef, QuadRef, TermRef, Triple};
use oxttl::NQuadsSerializer;

use super::Lineage;

fn iri(value: &str) -> NamedNodeRef<'_> {
    // Only generated identifiers and fixed vocabulary IRIs use this function.
    NamedNodeRef::new_unchecked(value)
}

impl Lineage {
    pub(crate) fn write_graph_dataset<W: Write>(
        &self,
        ordinal: u64,
        triples: &[Triple],
        writer: W,
    ) -> io::Result<()> {
        let root = format!("urn:semantic-fabric:result:{}:{ordinal}", self.request);
        let activity = format!("{root}:activity");
        let bundle = format!("{root}:bundle");
        let graph = GraphNameRef::NamedNode(iri(&bundle));
        let mut writer = NQuadsSerializer::new().for_writer(writer);
        let invalid = || io::Error::other("invalid lineage proof");
        let mapping_id =
            Literal::new_simple_literal(self.header["mappingId"].as_str().ok_or_else(invalid)?);
        let source_id = Literal::from(self.header["sourceId"].as_u64().ok_or_else(invalid)?);
        let metadata: [(&str, NamedNodeRef<'_>, TermRef<'_>); 13] = [
            (
                bundle.as_str(),
                rdf::TYPE,
                iri("http://www.w3.org/ns/prov#Bundle").into(),
            ),
            (
                activity.as_str(),
                rdf::TYPE,
                iri("http://www.w3.org/ns/prov#Activity").into(),
            ),
            (
                root.as_str(),
                rdf::TYPE,
                iri("http://www.w3.org/ns/prov#Entity").into(),
            ),
            (
                root.as_str(),
                iri("http://www.w3.org/ns/prov#wasGeneratedBy"),
                iri(&activity).into(),
            ),
            (
                activity.as_str(),
                iri("http://www.w3.org/ns/prov#used"),
                iri(&self.source).into(),
            ),
            (
                activity.as_str(),
                iri("http://www.w3.org/ns/prov#used"),
                iri(&self.mapping).into(),
            ),
            (
                activity.as_str(),
                iri("urn:semantic-fabric:lineage:snapshot"),
                iri(&self.snapshot).into(),
            ),
            (
                activity.as_str(),
                iri("urn:semantic-fabric:lineage:logicalPlan"),
                iri(&self.plan).into(),
            ),
            (
                activity.as_str(),
                iri("urn:semantic-fabric:lineage:policy"),
                iri(&self.policy).into(),
            ),
            (
                self.mapping.as_str(),
                rdf::TYPE,
                iri("http://www.w3.org/ns/prov#Entity").into(),
            ),
            (
                self.source.as_str(),
                rdf::TYPE,
                iri("http://www.w3.org/ns/prov#Entity").into(),
            ),
            (
                self.mapping.as_str(),
                iri("urn:semantic-fabric:lineage:mappingId"),
                mapping_id.as_ref().into(),
            ),
            (
                self.source.as_str(),
                iri("urn:semantic-fabric:lineage:sourceId"),
                source_id.as_ref().into(),
            ),
        ];
        for (subject, predicate, object) in metadata {
            writer.serialize_quad(QuadRef::new(iri(subject), predicate, object, graph))?;
        }
        for (index, triple) in triples.iter().enumerate() {
            writer.serialize_quad(triple.as_ref().in_graph(GraphNameRef::DefaultGraph))?;
            let reifier = format!("{root}:triple:{index}");
            writer.serialize_quad(QuadRef::new(
                iri(&reifier),
                rdf::REIFIES,
                TermRef::Triple(triple),
                graph,
            ))?;
            writer.serialize_quad(QuadRef::new(
                iri(&reifier),
                iri("http://www.w3.org/ns/prov#wasGeneratedBy"),
                iri(&activity),
                graph,
            ))?;
        }
        writer.finish();
        Ok(())
    }
}
