use super::events::InternalRdfXmlParser;
use super::state::RdfXmlState;
use crate::error::{RdfXmlParseError, RdfXmlSyntaxError};
use oxiri::Iri;
#[cfg(feature = "rdf-12")]
use oxrdf::BaseDirection;
#[cfg(feature = "rdf-12")]
use oxrdf::BlankNode;
use oxrdf::vocab::rdf;
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Triple};
use quick_xml::Error;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::PrefixDeclaration;

impl<R> InternalRdfXmlParser<R> {
    pub(super) fn parse_literal_start_event(
        &mut self,
        event: &BytesStart<'_>,
    ) -> Result<(), RdfXmlParseError> {
        let Some(RdfXmlState::ParseTypeLiteralPropertyElt { writer, .. }) = self.state.last_mut()
        else {
            unreachable!()
        };
        let mut clean_event = BytesStart::new(
            self.reader
                .decoder()
                .decode(event.name().as_ref())?
                .to_string(),
        );
        for attr in event.attributes() {
            clean_event.push_attribute(attr.map_err(Error::InvalidAttr)?);
        }
        if self.in_literal_depth == 0 {
            for (prefix, namespace) in self.reader.resolver().bindings() {
                let namespace = self.reader.decoder().decode(namespace.into_inner())?;
                if let Err(error) = Iri::parse(namespace.as_ref()) {
                    return Err(RdfXmlSyntaxError::invalid_iri(namespace.into(), error).into());
                }
                match prefix {
                    PrefixDeclaration::Default => {
                        clean_event.push_attribute(("xmlns".as_bytes(), namespace.as_bytes()))
                    }
                    PrefixDeclaration::Named(name) => {
                        let mut attr = Vec::with_capacity(6 + name.len());
                        attr.extend_from_slice(b"xmlns:");
                        attr.extend_from_slice(name);
                        clean_event.push_attribute((attr.as_slice(), namespace.as_bytes()))
                    }
                }
            }
        }
        writer.write_event(Event::Start(clean_event))?;
        self.in_literal_depth += 1;
        Ok(())
    }

    pub(super) fn new_literal(
        &self,
        value: String,
        language: Option<String>,
        #[cfg(feature = "rdf-12")] base_direction: Option<BaseDirection>,
        datatype: Option<NamedNode>,
    ) -> Literal {
        if let Some(datatype) = datatype {
            Literal::new_typed_literal(value, datatype)
        } else if let Some(language) =
            language.or_else(|| self.current_language().map(ToOwned::to_owned))
        {
            #[cfg(feature = "rdf-12")]
            if let Some(base_direction) = base_direction {
                return Literal::new_directional_language_tagged_literal_unchecked(
                    value,
                    language,
                    base_direction,
                );
            }
            Literal::new_language_tagged_literal_unchecked(value, language)
        } else {
            Literal::new_simple_literal(value)
        }
    }

    pub(super) fn reify_and_annotation(
        &mut self,
        triple: &Triple,
        statement_id: Option<NamedNode>,
        #[cfg(feature = "rdf-12")] annotation: Option<NamedNode>,
        #[cfg(feature = "rdf-12")] annotation_node_id: Option<BlankNode>,
        results: &mut Vec<Triple>,
    ) {
        if let Some(statement_id) = statement_id {
            self.emit_triple(
                results,
                Triple::new(statement_id.clone(), rdf::TYPE, rdf::STATEMENT),
            );
            self.emit_triple(
                results,
                Triple::new(statement_id.clone(), rdf::SUBJECT, triple.subject.clone()),
            );
            self.emit_triple(
                results,
                Triple::new(
                    statement_id.clone(),
                    rdf::PREDICATE,
                    triple.predicate.as_ref(),
                ),
            );
            self.emit_triple(
                results,
                Triple::new(statement_id, rdf::OBJECT, triple.object.clone()),
            );
        }
        #[cfg(feature = "rdf-12")]
        if let Some(annotation) = annotation {
            self.emit_triple(
                results,
                Triple::new(annotation, rdf::REIFIES, triple.clone()),
            );
        }
        #[cfg(feature = "rdf-12")]
        if let Some(annotation_node_id) = annotation_node_id {
            self.emit_triple(
                results,
                Triple::new(annotation_node_id, rdf::REIFIES, triple.clone()),
            );
        }
    }

    pub(super) fn emit_property_attrs(
        &mut self,
        subject: &NamedOrBlankNode,
        literal_attributes: Vec<(NamedNode, String)>,
        language: Option<&str>,
        #[cfg(feature = "rdf-12")] base_direction: Option<BaseDirection>,
        results: &mut Vec<Triple>,
    ) {
        for (literal_predicate, literal_value) in literal_attributes {
            self.emit_triple(
                results,
                Triple::new(
                    subject.clone(),
                    literal_predicate,
                    if let Some(language) = language.or_else(|| self.current_language()) {
                        #[cfg(feature = "rdf-12")]
                        if let Some(base_direction) =
                            base_direction.or_else(|| self.current_base_direction())
                        {
                            Literal::new_directional_language_tagged_literal_unchecked(
                                literal_value,
                                language,
                                base_direction,
                            )
                        } else {
                            Literal::new_language_tagged_literal_unchecked(literal_value, language)
                        }
                        #[cfg(not(feature = "rdf-12"))]
                        {
                            Literal::new_language_tagged_literal_unchecked(literal_value, language)
                        }
                    } else {
                        Literal::new_simple_literal(literal_value)
                    },
                ),
            );
        }
    }

    #[cfg_attr(not(feature = "rdf-12"), expect(clippy::unused_self))]
    pub(super) fn emit_triple(&mut self, results: &mut Vec<Triple>, triple: Triple) {
        #[cfg(feature = "rdf-12")]
        for state in self.state.iter_mut().rev() {
            if let RdfXmlState::ParseTypeTriplePropertyElt { triples, .. } = state {
                triples.push(triple);
                return;
            }
        }
        results.push(triple);
    }
}
