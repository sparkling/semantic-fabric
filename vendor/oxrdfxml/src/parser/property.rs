use super::events::InternalRdfXmlParser;
use super::node::{RdfXmlParseType, StartAttributes};
use super::state::{
    NodeOrText, RDF_DESCRIPTION, RDF_LI, RESERVED_RDF_ELEMENTS, RdfXmlState, is_object_defined,
};
use crate::error::{RdfXmlParseError, RdfXmlSyntaxError};
use oxiri::Iri;
#[cfg(feature = "rdf-12")]
use oxrdf::BaseDirection;
use oxrdf::vocab::rdf;
use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use quick_xml::Writer;
use std::str;

fn resource_or_blank_object(
    resource_attr: Option<NamedNode>,
    node_id_attr: Option<BlankNode>,
) -> Result<NamedOrBlankNode, RdfXmlSyntaxError> {
    Ok(match (resource_attr, node_id_attr) {
        (Some(resource_attr), None) => NamedOrBlankNode::from(resource_attr),
        (None, Some(node_id_attr)) => node_id_attr.into(),
        (None, None) => BlankNode::default().into(),
        (Some(_), Some(_)) => {
            return Err(RdfXmlSyntaxError::msg(
                "Not both rdf:resource and rdf:nodeID could be set at the same time",
            ));
        }
    })
}

impl<R> InternalRdfXmlParser<R> {
    pub(super) fn build_property_elt(
        &mut self,
        tag_name: String,
        subject: NamedOrBlankNode,
        attributes: StartAttributes,
        results: &mut Vec<Triple>,
    ) -> Result<RdfXmlState, RdfXmlParseError> {
        let StartAttributes {
            language,
            base_iri,
            id_attr,
            node_id_attr,
            resource_attr,
            datatype_attr,
            property_attrs,
            parse_type,
            type_attr,
            #[cfg(feature = "rdf-12")]
            base_direction,
            #[cfg(feature = "rdf-12")]
            rdf_version,
            #[cfg(feature = "rdf-12")]
            annotation_attr,
            #[cfg(feature = "rdf-12")]
            annotation_node_id_attr,
            ..
        } = attributes;
        let iri = if *tag_name == *RDF_LI {
            let Some(RdfXmlState::NodeElt { li_counter, .. }) = self.state.last_mut() else {
                return Err(RdfXmlSyntaxError::msg(format!(
                    "Invalid property element tag name: {tag_name}"
                ))
                .into());
            };
            *li_counter += 1;
            NamedNode::new_unchecked(format!(
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#_{li_counter}"
            ))
        } else if RESERVED_RDF_ELEMENTS.contains(&&*tag_name) || *tag_name == *RDF_DESCRIPTION {
            return Err(RdfXmlSyntaxError::msg(format!(
                "Invalid property element tag name: {tag_name}"
            ))
            .into());
        } else {
            self.parse_iri(tag_name)?
        };
        let has_object_attrs =
            resource_attr.is_some() || node_id_attr.is_some() || !property_attrs.is_empty();
        Ok(match parse_type {
            RdfXmlParseType::Default => {
                if has_object_attrs {
                    let object = resource_or_blank_object(resource_attr, node_id_attr)?;
                    self.emit_property_attrs(
                        &object,
                        property_attrs,
                        language.as_deref(),
                        #[cfg(feature = "rdf-12")]
                        base_direction,
                        results,
                    );
                    if let Some(type_attr) = type_attr {
                        let type_triple = Triple::new(object.clone(), rdf::TYPE, type_attr);
                        self.emit_triple(results, type_triple);
                    }
                    RdfXmlState::PropertyElt {
                        iri,
                        base_iri,
                        language,
                        #[cfg(feature = "rdf-12")]
                        base_direction,
                        subject,
                        object: Some(NodeOrText::Node(object)),
                        id_attr,
                        #[cfg(feature = "rdf-12")]
                        annotation_attr,
                        #[cfg(feature = "rdf-12")]
                        annotation_node_id_attr,
                        datatype_attr,
                        #[cfg(feature = "rdf-12")]
                        rdf_version,
                    }
                } else {
                    RdfXmlState::PropertyElt {
                        iri,
                        base_iri,
                        language,
                        #[cfg(feature = "rdf-12")]
                        base_direction,
                        subject,
                        object: None,
                        id_attr,
                        #[cfg(feature = "rdf-12")]
                        annotation_attr,
                        #[cfg(feature = "rdf-12")]
                        annotation_node_id_attr,
                        datatype_attr,
                        #[cfg(feature = "rdf-12")]
                        rdf_version,
                    }
                }
            }
            RdfXmlParseType::Literal => RdfXmlState::ParseTypeLiteralPropertyElt {
                iri,
                base_iri,
                language,
                #[cfg(feature = "rdf-12")]
                base_direction,
                subject,
                writer: Writer::new(Vec::new()),
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                emit: true,
                #[cfg(feature = "rdf-12")]
                rdf_version,
            },
            RdfXmlParseType::Resource => self.build_parse_type_resource_property_elt(
                iri,
                base_iri,
                language,
                #[cfg(feature = "rdf-12")]
                base_direction,
                #[cfg(feature = "rdf-12")]
                rdf_version,
                subject,
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                results,
            ),
            RdfXmlParseType::Collection => RdfXmlState::ParseTypeCollectionPropertyElt {
                iri,
                base_iri,
                language,
                #[cfg(feature = "rdf-12")]
                base_direction,
                subject,
                objects: Vec::new(),
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                #[cfg(feature = "rdf-12")]
                rdf_version,
            },
            #[cfg(feature = "rdf-12")]
            RdfXmlParseType::Triple => RdfXmlState::ParseTypeTriplePropertyElt {
                iri,
                base_iri,
                language,
                base_direction,
                subject,
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                #[cfg(feature = "rdf-12")]
                rdf_version,
                triples: Vec::new(),
            },
            RdfXmlParseType::Other => RdfXmlState::ParseTypeLiteralPropertyElt {
                iri,
                base_iri,
                language,
                #[cfg(feature = "rdf-12")]
                base_direction,
                subject,
                writer: Writer::new(Vec::new()),
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                emit: false,
                #[cfg(feature = "rdf-12")]
                rdf_version,
            },
        })
    }

    #[cfg_attr(feature = "rdf-12", expect(clippy::too_many_arguments))]
    pub(super) fn build_parse_type_resource_property_elt(
        &mut self,
        iri: NamedNode,
        base_iri: Option<Iri<String>>,
        language: Option<String>,
        #[cfg(feature = "rdf-12")] base_direction: Option<BaseDirection>,
        #[cfg(feature = "rdf-12")] rdf_version: Option<super::version::RdfVersion>,
        subject: NamedOrBlankNode,
        id_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")] annotation_attr: Option<NamedNode>,
        #[cfg(feature = "rdf-12")] annotation_node_id_attr: Option<BlankNode>,
        results: &mut Vec<Triple>,
    ) -> RdfXmlState {
        let object = BlankNode::default();
        let triple = Triple::new(subject, iri, object.clone());
        self.reify_and_annotation(
            &triple,
            id_attr,
            #[cfg(feature = "rdf-12")]
            annotation_attr,
            #[cfg(feature = "rdf-12")]
            annotation_node_id_attr,
            results,
        );
        self.emit_triple(results, triple);
        RdfXmlState::NodeElt {
            base_iri,
            language,
            #[cfg(feature = "rdf-12")]
            base_direction,
            subject: object.into(),
            li_counter: 0,
            #[cfg(feature = "rdf-12")]
            rdf_version,
        }
    }

    pub(super) fn end_state(
        &mut self,
        state: RdfXmlState,
        results: &mut Vec<Triple>,
    ) -> Result<(), RdfXmlSyntaxError> {
        match state {
            RdfXmlState::PropertyElt {
                iri,
                language,
                #[cfg(feature = "rdf-12")]
                base_direction,
                subject,
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                datatype_attr,
                object,
                ..
            } => {
                let object = match object {
                    Some(NodeOrText::Node(node)) => Term::from(node),
                    Some(NodeOrText::Text(text)) => self
                        .new_literal(
                            text,
                            language,
                            #[cfg(feature = "rdf-12")]
                            base_direction,
                            datatype_attr,
                        )
                        .into(),
                    None => self
                        .new_literal(
                            String::new(),
                            language,
                            #[cfg(feature = "rdf-12")]
                            base_direction,
                            datatype_attr,
                        )
                        .into(),
                };
                let triple = Triple::new(subject, iri, object);
                self.reify_and_annotation(
                    &triple,
                    id_attr,
                    #[cfg(feature = "rdf-12")]
                    annotation_attr,
                    #[cfg(feature = "rdf-12")]
                    annotation_node_id_attr,
                    results,
                );
                self.emit_triple(results, triple);
            }
            RdfXmlState::ParseTypeCollectionPropertyElt {
                iri,
                subject,
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                objects,
                ..
            } => {
                let mut current_node = NamedOrBlankNode::from(rdf::NIL);
                for object in objects.into_iter().rev() {
                    let subject = NamedOrBlankNode::from(BlankNode::default());
                    self.emit_triple(results, Triple::new(subject.clone(), rdf::FIRST, object));
                    self.emit_triple(
                        results,
                        Triple::new(subject.clone(), rdf::REST, current_node),
                    );
                    current_node = subject;
                }
                let triple = Triple::new(subject, iri, current_node);
                self.reify_and_annotation(
                    &triple,
                    id_attr,
                    #[cfg(feature = "rdf-12")]
                    annotation_attr,
                    #[cfg(feature = "rdf-12")]
                    annotation_node_id_attr,
                    results,
                );
                self.emit_triple(results, triple);
            }
            RdfXmlState::ParseTypeLiteralPropertyElt {
                iri,
                subject,
                id_attr,
                #[cfg(feature = "rdf-12")]
                annotation_attr,
                #[cfg(feature = "rdf-12")]
                annotation_node_id_attr,
                writer,
                emit,
                ..
            } => {
                if emit {
                    let object = writer.into_inner();
                    if object.is_empty() {
                        return Err(RdfXmlSyntaxError::msg(format!(
                            "No value found for rdf:XMLLiteral value of property {iri}"
                        )));
                    }
                    let triple = Triple::new(
                        subject,
                        iri,
                        Literal::new_typed_literal(
                            str::from_utf8(&object).map_err(|_| {
                                RdfXmlSyntaxError::msg(
                                    "The XML literal is not in valid UTF-8".to_owned(),
                                )
                            })?,
                            rdf::XML_LITERAL,
                        ),
                    );
                    self.reify_and_annotation(
                        &triple,
                        id_attr,
                        #[cfg(feature = "rdf-12")]
                        annotation_attr,
                        #[cfg(feature = "rdf-12")]
                        annotation_node_id_attr,
                        results,
                    );
                    self.emit_triple(results, triple);
                }
            }
            RdfXmlState::NodeElt { subject, .. } => match self.state.last_mut() {
                Some(RdfXmlState::PropertyElt { object, .. }) => {
                    if is_object_defined(object) {
                        return Err(RdfXmlSyntaxError::msg(
                            "Unexpected node, a text value is already present",
                        ));
                    }
                    *object = Some(NodeOrText::Node(subject))
                }
                Some(RdfXmlState::ParseTypeCollectionPropertyElt { objects, .. }) => {
                    objects.push(subject)
                }
                _ => (),
            },
            RdfXmlState::Doc { .. } | RdfXmlState::Rdf { .. } => (),
            #[cfg(feature = "rdf-12")]
            RdfXmlState::ParseTypeTriplePropertyElt {
                iri,
                subject,
                id_attr,
                annotation_attr,
                annotation_node_id_attr,
                triples,
                rdf_version,
                ..
            } => {
                if rdf_version
                    .unwrap_or_else(|| self.current_rdf_version())
                    .supports_triple_term()
                {
                    if triples.len() != 1 {
                        return Err(RdfXmlSyntaxError::msg(
                            "rdf:parseType=\"Triple\" can only include a single triple",
                        ));
                    }
                    let triple = Triple::new(subject, iri, triples.into_iter().next().unwrap());
                    self.reify_and_annotation(
                        &triple,
                        id_attr,
                        annotation_attr,
                        annotation_node_id_attr,
                        results,
                    );
                    self.emit_triple(results, triple);
                }
            }
        }
        Ok(())
    }
}
