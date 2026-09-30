use super::events::InternalRdfXmlParser;
#[cfg(feature = "rdf-12")]
use super::version::RdfVersion;
use oxiri::Iri;
#[cfg(feature = "rdf-12")]
use oxrdf::BaseDirection;
#[cfg(feature = "rdf-12")]
use oxrdf::BlankNode;
#[cfg(feature = "rdf-12")]
use oxrdf::Triple;
use oxrdf::{NamedNode, NamedOrBlankNode};
use quick_xml::Writer;

pub(super) const RDF_ABOUT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#about";
pub(super) const RDF_ABOUT_EACH: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#aboutEach";
pub(super) const RDF_ABOUT_EACH_PREFIX: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#aboutEachPrefix";
pub(super) const RDF_BAG_ID: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#bagID";
pub(super) const RDF_DATATYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#datatype";
pub(super) const RDF_DESCRIPTION: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#Description";
pub(super) const RDF_ID: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#ID";
pub(super) const RDF_LI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#li";
pub(super) const RDF_NODE_ID: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nodeID";
pub(super) const RDF_PARSE_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#parseType";
pub(super) const RDF_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#RDF";
pub(super) const RDF_RESOURCE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#resource";
pub(super) const RDF_VERSION: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#version";
pub(super) const RDF_ANNOTATION: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#annotation";
pub(super) const RDF_ANNOTATION_NODE_ID: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#annotationNodeID";

pub(super) const RESERVED_RDF_ELEMENTS: [&str; 11] = [
    RDF_ABOUT,
    RDF_ABOUT_EACH,
    RDF_ABOUT_EACH_PREFIX,
    RDF_BAG_ID,
    RDF_DATATYPE,
    RDF_ID,
    RDF_LI,
    RDF_NODE_ID,
    RDF_PARSE_TYPE,
    RDF_RDF,
    RDF_RESOURCE,
];

pub(super) const RESERVED_RDF_ATTRIBUTES: [&str; 5] = [
    RDF_ABOUT_EACH,
    RDF_ABOUT_EACH_PREFIX,
    RDF_LI,
    RDF_RDF,
    RDF_RESOURCE,
];

#[derive(Clone, Debug)]
pub(super) enum NodeOrText {
    Node(NamedOrBlankNode),
    Text(String),
}

pub(super) enum RdfXmlState {
    Doc {
        base_iri: Option<Iri<String>>,
    },
    Rdf {
        base_iri: Option<Iri<String>>,
        language: Option<String>,
        #[cfg(feature = "rdf-12")]
        base_direction: Option<BaseDirection>,
        #[cfg(feature = "rdf-12")]
        rdf_version: Option<RdfVersion>,
    },
    NodeElt {
        base_iri: Option<Iri<String>>,
        language: Option<String>,
        #[cfg(feature = "rdf-12")]
        base_direction: Option<BaseDirection>,
        subject: NamedOrBlankNode,
        li_counter: u64,
        #[cfg(feature = "rdf-12")]
        rdf_version: Option<RdfVersion>,
    },
    PropertyElt {
        // Resource, Literal or Empty property element
        iri: NamedNode,
        base_iri: Option<Iri<String>>,
        language: Option<String>,
        #[cfg(feature = "rdf-12")]
        base_direction: Option<BaseDirection>,
        subject: NamedOrBlankNode,
        object: Option<NodeOrText>,
        id_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")]
        annotation_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")]
        annotation_node_id_attr: Option<BlankNode>,
        datatype_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")]
        rdf_version: Option<RdfVersion>,
    },
    ParseTypeCollectionPropertyElt {
        iri: NamedNode,
        base_iri: Option<Iri<String>>,
        language: Option<String>,
        #[cfg(feature = "rdf-12")]
        base_direction: Option<BaseDirection>,
        subject: NamedOrBlankNode,
        objects: Vec<NamedOrBlankNode>,
        id_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")]
        annotation_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")]
        annotation_node_id_attr: Option<BlankNode>,
        #[cfg(feature = "rdf-12")]
        rdf_version: Option<RdfVersion>,
    },
    ParseTypeLiteralPropertyElt {
        iri: NamedNode,
        base_iri: Option<Iri<String>>,
        language: Option<String>,
        #[cfg(feature = "rdf-12")]
        base_direction: Option<BaseDirection>,
        subject: NamedOrBlankNode,
        writer: Writer<Vec<u8>>,
        id_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")]
        annotation_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")]
        annotation_node_id_attr: Option<BlankNode>,
        emit: bool, // false for parseTypeOtherPropertyElt support
        #[cfg(feature = "rdf-12")]
        rdf_version: Option<RdfVersion>,
    },
    #[cfg(feature = "rdf-12")]
    ParseTypeTriplePropertyElt {
        iri: NamedNode,
        base_iri: Option<Iri<String>>,
        language: Option<String>,
        base_direction: Option<BaseDirection>,
        subject: NamedOrBlankNode,
        id_attr: Option<NamedNode>,
        annotation_attr: Option<NamedNode>,
        annotation_node_id_attr: Option<BlankNode>,
        rdf_version: Option<RdfVersion>,
        triples: Vec<Triple>,
    },
}

impl<R> InternalRdfXmlParser<R> {
    pub(super) fn current_language(&self) -> Option<&str> {
        for state in self.state.iter().rev() {
            match state {
                RdfXmlState::Doc { .. } => (),
                RdfXmlState::Rdf { language, .. }
                | RdfXmlState::NodeElt { language, .. }
                | RdfXmlState::PropertyElt { language, .. }
                | RdfXmlState::ParseTypeCollectionPropertyElt { language, .. }
                | RdfXmlState::ParseTypeLiteralPropertyElt { language, .. } => {
                    if let Some(language) = language {
                        return Some(language);
                    }
                }
                #[cfg(feature = "rdf-12")]
                RdfXmlState::ParseTypeTriplePropertyElt { language, .. } => {
                    if let Some(language) = language {
                        return Some(language);
                    }
                }
            }
        }
        None
    }

    #[cfg(feature = "rdf-12")]
    pub(super) fn current_base_direction(&self) -> Option<BaseDirection> {
        for state in self.state.iter().rev() {
            match state {
                RdfXmlState::Doc { .. } => (),
                RdfXmlState::Rdf { base_direction, .. }
                | RdfXmlState::NodeElt { base_direction, .. }
                | RdfXmlState::PropertyElt { base_direction, .. }
                | RdfXmlState::ParseTypeCollectionPropertyElt { base_direction, .. }
                | RdfXmlState::ParseTypeLiteralPropertyElt { base_direction, .. } => {
                    if let Some(base_direction) = base_direction {
                        return Some(*base_direction);
                    }
                }
                #[cfg(feature = "rdf-12")]
                RdfXmlState::ParseTypeTriplePropertyElt { base_direction, .. } => {
                    if let Some(base_direction) = base_direction {
                        return Some(*base_direction);
                    }
                }
            }
        }
        None
    }

    pub(super) fn current_base_iri(&self) -> Option<&Iri<String>> {
        for state in self.state.iter().rev() {
            match state {
                RdfXmlState::Doc { base_iri }
                | RdfXmlState::Rdf { base_iri, .. }
                | RdfXmlState::NodeElt { base_iri, .. }
                | RdfXmlState::PropertyElt { base_iri, .. }
                | RdfXmlState::ParseTypeCollectionPropertyElt { base_iri, .. }
                | RdfXmlState::ParseTypeLiteralPropertyElt { base_iri, .. } => {
                    if let Some(base_iri) = base_iri {
                        return Some(base_iri);
                    }
                }
                #[cfg(feature = "rdf-12")]
                RdfXmlState::ParseTypeTriplePropertyElt { base_iri, .. } => {
                    if let Some(base_iri) = base_iri {
                        return Some(base_iri);
                    }
                }
            }
        }
        None
    }
}

pub(super) fn is_object_defined(object: &Option<NodeOrText>) -> bool {
    match object {
        Some(NodeOrText::Node(_)) => true,
        Some(NodeOrText::Text(t)) => !t.bytes().all(is_whitespace),
        None => false,
    }
}

pub(super) fn is_whitespace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}
