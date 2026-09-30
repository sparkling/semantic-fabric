use super::entities::EntityRegistry;
use super::state::{
    NodeOrText, RDF_RDF, RESERVED_RDF_ELEMENTS, RdfXmlState, is_object_defined, is_whitespace,
};
use crate::error::{RdfXmlParseError, RdfXmlSyntaxError};
use oxrdf::{NamedOrBlankNode, Triple};
use quick_xml::Error;
use quick_xml::escape::unescape_with;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::{LocalName, ResolveResult};
use quick_xml::{NsReader, XmlVersion};
use std::collections::HashSet;

#[derive(PartialEq, Eq)]
enum RdfXmlNextProduction {
    Rdf,
    NodeElt,
    PropertyElt { subject: NamedOrBlankNode },
}

pub(super) struct InternalRdfXmlParser<R> {
    pub(super) reader: NsReader<R>,
    pub(super) state: Vec<RdfXmlState>,
    pub(super) custom_entities: EntityRegistry,
    pub(super) in_literal_depth: usize,
    pub(super) known_rdf_id: HashSet<String>,
    pub(super) is_end: bool,
    pub(super) lenient: bool,
    pub(super) xml_version: XmlVersion,
    pub(super) text_buffer: Option<String>,
}

impl<R> InternalRdfXmlParser<R> {
    pub(super) fn parse_event(
        &mut self,
        event: Event<'_>,
        results: &mut Vec<Triple>,
    ) -> Result<(), RdfXmlParseError> {
        match event {
            Event::Start(event) => {
                // We make sure we always run both, even if one fail for better error recovery
                let text_error = if let Some(text_buffer) = self.text_buffer.take() {
                    self.parse_text_event(text_buffer)
                } else {
                    Ok(())
                };
                let start_error = self.parse_start_event(&event, results);
                text_error.and(start_error)
            }
            Event::End(event) => {
                // We make sure we always run both, even if one fail for better error recovery
                let text_error = if let Some(text_buffer) = self.text_buffer.take() {
                    self.parse_text_event(text_buffer)
                } else {
                    Ok(())
                };
                let end_error = self.parse_end_event(&event, results);
                text_error.and(end_error)
            }
            Event::Empty(_) => unreachable!("The expand_empty_elements option must be enabled",),
            Event::Text(event) => {
                self.text_buffer
                    .get_or_insert_default()
                    .push_str(&event.xml_content(self.xml_version)?);
                Ok(())
            }
            Event::GeneralRef(event) => {
                self.decode_xml_entity(&event)?;
                Ok(())
            }
            Event::CData(event) => {
                self.text_buffer
                    .get_or_insert_default()
                    .push_str(&event.xml_content(self.xml_version)?);
                Ok(())
            }
            Event::Comment(_) | Event::PI(_) => Ok(()),
            Event::Decl(event) => {
                self.xml_version = event.xml_version()?;
                if let Some(encoding) = event.encoding() {
                    if !is_utf8(&encoding?) {
                        return Err(RdfXmlSyntaxError::msg(
                            "Only UTF-8 is supported by the RDF/XML parser",
                        )
                        .into());
                    }
                }
                Ok(())
            }
            Event::DocType(dt) => self.parse_doctype(&dt),
            Event::Eof => {
                self.is_end = true;
                if let Some(text_buffer) = self.text_buffer.take() {
                    self.parse_text_event(text_buffer)?;
                }
                Ok(())
            }
        }
    }

    fn parse_start_event(
        &mut self,
        event: &BytesStart<'_>,
        results: &mut Vec<Triple>,
    ) -> Result<(), RdfXmlParseError> {
        // Literal case
        if let Some(RdfXmlState::ParseTypeLiteralPropertyElt { .. }) = self.state.last() {
            return self.parse_literal_start_event(event);
        }

        let (tag_namespace, tag_local_name) = self.reader.resolver().resolve_element(event.name());
        let tag_name = self.resolve_ns_name(tag_namespace, tag_local_name)?;

        // We read attributes
        let attributes = self.parse_start_attributes(event)?;

        let expected_production = match self.state.last() {
            Some(RdfXmlState::Doc { .. }) => RdfXmlNextProduction::Rdf,
            Some(
                RdfXmlState::Rdf { .. }
                | RdfXmlState::PropertyElt { .. }
                | RdfXmlState::ParseTypeCollectionPropertyElt { .. },
            ) => RdfXmlNextProduction::NodeElt,
            Some(RdfXmlState::NodeElt { subject, .. }) => RdfXmlNextProduction::PropertyElt {
                subject: subject.clone(),
            },
            Some(RdfXmlState::ParseTypeLiteralPropertyElt { .. }) => {
                return Err(
                    RdfXmlSyntaxError::msg("ParseTypeLiteralPropertyElt production children should never be considered as a RDF/XML content").into()
                );
            }
            #[cfg(feature = "rdf-12")]
            Some(RdfXmlState::ParseTypeTriplePropertyElt { .. }) => RdfXmlNextProduction::NodeElt,
            None => {
                return Err(RdfXmlSyntaxError::msg(
                    "No state in the stack: the XML is not balanced",
                )
                .into());
            }
        };

        let new_state = match expected_production {
            RdfXmlNextProduction::Rdf => {
                if *tag_name == *RDF_RDF {
                    RdfXmlState::Rdf {
                        base_iri: attributes.base_iri,
                        language: attributes.language,
                        #[cfg(feature = "rdf-12")]
                        base_direction: attributes.base_direction,
                        #[cfg(feature = "rdf-12")]
                        rdf_version: attributes.rdf_version,
                    }
                } else if RESERVED_RDF_ELEMENTS.contains(&&*tag_name) {
                    return Err(RdfXmlSyntaxError::msg(format!(
                        "Invalid node element tag name: {tag_name}"
                    ))
                    .into());
                } else {
                    self.build_node_elt(self.parse_iri(tag_name)?, attributes, results)?
                }
            }
            RdfXmlNextProduction::NodeElt => {
                if RESERVED_RDF_ELEMENTS.contains(&&*tag_name) {
                    return Err(RdfXmlSyntaxError::msg(format!(
                        "Invalid property element tag name: {tag_name}"
                    ))
                    .into());
                }
                self.build_node_elt(self.parse_iri(tag_name)?, attributes, results)?
            }
            RdfXmlNextProduction::PropertyElt { subject } => {
                self.build_property_elt(tag_name, subject, attributes, results)?
            }
        };
        self.state.push(new_state);
        Ok(())
    }

    fn parse_end_event(
        &mut self,
        event: &BytesEnd<'_>,
        results: &mut Vec<Triple>,
    ) -> Result<(), RdfXmlParseError> {
        // Literal case
        if self.in_literal_depth > 0 {
            let Some(RdfXmlState::ParseTypeLiteralPropertyElt { writer, .. }) =
                self.state.last_mut()
            else {
                unreachable!()
            };
            writer.write_event(Event::End(BytesEnd::new(
                self.reader.decoder().decode(event.name().as_ref())?,
            )))?;
            self.in_literal_depth -= 1;
            return Ok(());
        }

        if let Some(current_state) = self.state.pop() {
            self.end_state(current_state, results)?;
        }
        Ok(())
    }

    fn parse_text_event(&mut self, text: String) -> Result<(), RdfXmlParseError> {
        match self.state.last_mut() {
            Some(RdfXmlState::PropertyElt { object, .. }) => {
                if is_object_defined(object) {
                    if text.bytes().all(is_whitespace) {
                        Ok(()) // whitespace anyway, we ignore
                    } else {
                        Err(
                            RdfXmlSyntaxError::msg(format!("Unexpected text event: '{text}'"))
                                .into(),
                        )
                    }
                } else {
                    *object = Some(NodeOrText::Text(text));
                    Ok(())
                }
            }
            Some(RdfXmlState::ParseTypeLiteralPropertyElt { writer, .. }) => {
                writer.write_event(Event::Text(BytesText::new(&text)))?;
                Ok(())
            }
            _ => {
                if text.bytes().all(is_whitespace) {
                    Ok(())
                } else {
                    Err(RdfXmlSyntaxError::msg(format!("Unexpected text event: '{text}'")).into())
                }
            }
        }
    }

    pub(super) fn resolve_ns_name(
        &self,
        namespace: ResolveResult<'_>,
        local_name: LocalName<'_>,
    ) -> Result<String, RdfXmlParseError> {
        match namespace {
            ResolveResult::Bound(ns) => {
                let mut value = Vec::with_capacity(ns.as_ref().len() + local_name.as_ref().len());
                value.extend_from_slice(ns.as_ref());
                value.extend_from_slice(local_name.as_ref());
                Ok(unescape_with(&self.reader.decoder().decode(&value)?, |e| {
                    self.custom_entities.resolve(e)
                })
                .map_err(Error::from)?
                .to_string())
            }
            ResolveResult::Unbound => {
                Err(RdfXmlSyntaxError::msg("XML namespaces are required in RDF/XML").into())
            }
            ResolveResult::Unknown(v) => Err(RdfXmlSyntaxError::msg(format!(
                "Unknown prefix {}:",
                self.reader.decoder().decode(&v)?
            ))
            .into()),
        }
    }
}

fn is_utf8(encoding: &[u8]) -> bool {
    matches!(
        encoding.to_ascii_lowercase().as_slice(),
        b"unicode-1-1-utf-8"
            | b"unicode11utf8"
            | b"unicode20utf8"
            | b"utf-8"
            | b"utf8"
            | b"x-unicode20utf8"
    )
}
