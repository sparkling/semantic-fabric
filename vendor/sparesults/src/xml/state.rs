use super::terms::{build_literal, decode_xml_entity};
use crate::error::{QueryResultsParseError, QueryResultsSyntaxError};
use oxrdf::*;
use quick_xml::events::Event;
use quick_xml::{Decoder, Error, XmlVersion};
use std::collections::HashMap;
use std::mem::take;

enum State {
    Start,
    Result,
    Binding,
    Uri,
    BNode,
    Literal,
    Triple,
    Subject,
    Predicate,
    Object,
}

pub(super) struct XmlInnerSolutionsParser {
    decoder: Decoder,
    mapping: HashMap<String, usize>,
    state_stack: Vec<State>,
    new_bindings: Vec<Option<Term>>,
    current_var: Option<String>,
    term: Option<Term>,
    lang: Option<String>,
    #[cfg(feature = "sparql-12")]
    direction: Option<String>,
    datatype: Option<NamedNode>,
    subject_stack: Vec<Term>,
    predicate_stack: Vec<Term>,
    object_stack: Vec<Term>,
    text_buffer: String,
    xml_version: XmlVersion,
}

impl XmlInnerSolutionsParser {
    pub(super) fn new(decoder: Decoder, variables: &[Variable], xml_version: XmlVersion) -> Self {
        let mut mapping = HashMap::new();
        for (i, var) in variables.iter().enumerate() {
            mapping.insert(var.clone().into_string(), i);
        }
        Self {
            decoder,
            mapping,
            state_stack: vec![State::Start, State::Start],
            new_bindings: Vec::new(),
            current_var: None,
            term: None,
            lang: None,
            #[cfg(feature = "sparql-12")]
            direction: None,
            datatype: None,
            subject_stack: Vec::new(),
            predicate_stack: Vec::new(),
            object_stack: Vec::new(),
            text_buffer: String::new(),
            xml_version,
        }
    }

    pub(super) fn read_event(
        &mut self,
        event: Event<'_>,
    ) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        match event {
            Event::Start(event) => match self.state_stack.last().ok_or_else(|| {
                QueryResultsSyntaxError::msg("Extra XML is not allowed at the end of the document")
            })? {
                State::Start => {
                    if event.local_name().as_ref() == b"result" {
                        self.new_bindings = vec![None; self.mapping.len()];
                        self.state_stack.push(State::Result);
                        Ok(None)
                    } else {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting <result>, found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    }
                }
                State::Result => {
                    if event.local_name().as_ref() == b"binding" {
                        let Some(attr) = event
                            .attributes()
                            .filter_map(Result::ok)
                            .find(|attr| attr.key.local_name().as_ref() == b"name")
                        else {
                            return Err(QueryResultsSyntaxError::msg(
                                "No name attribute found for the <binding> tag",
                            )
                            .into());
                        };
                        self.current_var = Some(
                            attr.decoded_and_normalized_value(self.xml_version, self.decoder)?
                                .into_owned(),
                        );
                        self.state_stack.push(State::Binding);
                        Ok(None)
                    } else {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting <binding>, found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    }
                }
                State::Binding | State::Subject | State::Predicate | State::Object => {
                    if self.term.is_some() {
                        return Err(QueryResultsSyntaxError::msg(
                            "There is already a value for the current binding",
                        )
                        .into());
                    }
                    if event.local_name().as_ref() == b"uri" {
                        self.state_stack.push(State::Uri);
                        Ok(None)
                    } else if event.local_name().as_ref() == b"bnode" {
                        self.state_stack.push(State::BNode);
                        Ok(None)
                    } else if event.local_name().as_ref() == b"literal" {
                        for attr in event.attributes() {
                            let attr = attr.map_err(Error::from)?;
                            if attr.key.as_ref() == b"xml:lang" {
                                self.lang = Some(
                                    attr.decoded_and_normalized_value(
                                        self.xml_version,
                                        self.decoder,
                                    )?
                                    .into_owned(),
                                );
                            } else if attr.key.local_name().as_ref() == b"datatype" {
                                let iri = attr
                                    .decoded_and_normalized_value(self.xml_version, self.decoder)?;
                                self.datatype =
                                    Some(NamedNode::new(iri.as_ref()).map_err(|e| {
                                        QueryResultsSyntaxError::msg(format!(
                                            "Invalid datatype IRI '{iri}': {e}"
                                        ))
                                    })?);
                            }
                            #[cfg(feature = "sparql-12")]
                            if attr.key.as_ref() == b"its:dir" {
                                self.direction = Some(
                                    attr.decoded_and_normalized_value(
                                        self.xml_version,
                                        self.decoder,
                                    )?
                                    .into_owned(),
                                );
                            }
                        }
                        self.state_stack.push(State::Literal);
                        Ok(None)
                    } else if event.local_name().as_ref() == b"triple" {
                        self.state_stack.push(State::Triple);
                        Ok(None)
                    } else {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting <uri>, <bnode> or <literal> found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    }
                }
                State::Triple => {
                    if event.local_name().as_ref() == b"subject" {
                        self.state_stack.push(State::Subject);
                        Ok(None)
                    } else if event.local_name().as_ref() == b"predicate" {
                        self.state_stack.push(State::Predicate);
                        Ok(None)
                    } else if event.local_name().as_ref() == b"object" {
                        self.state_stack.push(State::Object);
                        Ok(None)
                    } else {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting <subject>, <predicate> or <object> found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    }
                }
                State::Uri => Err(QueryResultsSyntaxError::msg(format!(
                    "<uri> must only contain a string, found <{}>",
                    self.decoder.decode(event.name().as_ref())?
                ))
                .into()),
                State::BNode => Err(QueryResultsSyntaxError::msg(format!(
                    "<bnode> must only contain a string, found <{}>",
                    self.decoder.decode(event.name().as_ref())?
                ))
                .into()),
                State::Literal => Err(QueryResultsSyntaxError::msg(format!(
                    "<literal> must only contain a string, found <{}>",
                    self.decoder.decode(event.name().as_ref())?
                ))
                .into()),
            },
            Event::Text(event) => {
                self.text_buffer
                    .push_str(&event.xml_content(self.xml_version)?);
                Ok(None)
            }
            Event::End(_) => {
                let value = take(&mut self.text_buffer);
                let value = value.trim_matches(|c| matches!(c, '\t' | '\n' | '\r' | ' '));
                match self.state_stack.pop().ok_or_else(|| {
                    QueryResultsSyntaxError::msg(
                        "Extra XML is not allowed at the end of the document",
                    )
                })? {
                    State::Start => Ok(None),
                    State::Result => Ok(Some(take(&mut self.new_bindings))),
                    State::Binding => {
                        if let Some(var) = &self.current_var {
                            if let Some(var) = self.mapping.get(var) {
                                self.new_bindings[*var] = self.term.take()
                            } else {
                                return Err(
                                    QueryResultsSyntaxError::msg(format!("The variable '{var}' is used in a binding but not declared in the variables list")).into()
                                );
                            }
                        } else {
                            return Err(QueryResultsSyntaxError::msg(
                                "No name found for <binding> tag",
                            )
                            .into());
                        }
                        Ok(None)
                    }
                    State::Subject => {
                        if let Some(subject) = self.term.take() {
                            self.subject_stack.push(subject)
                        }
                        Ok(None)
                    }
                    State::Predicate => {
                        if let Some(predicate) = self.term.take() {
                            self.predicate_stack.push(predicate)
                        }
                        Ok(None)
                    }
                    State::Object => {
                        if let Some(object) = self.term.take() {
                            self.object_stack.push(object)
                        }
                        Ok(None)
                    }
                    State::Uri => {
                        self.term = Some(
                            NamedNode::new(value)
                                .map_err(|e| {
                                    QueryResultsSyntaxError::msg(format!(
                                        "Invalid IRI value '{value}': {e}"
                                    ))
                                })?
                                .into(),
                        );
                        Ok(None)
                    }
                    State::BNode => {
                        self.term = Some(
                            if value.is_empty() {
                                BlankNode::default()
                            } else {
                                BlankNode::new(value).map_err(|e| {
                                    QueryResultsSyntaxError::msg(format!(
                                        "Invalid blank node value '{value}': {e}"
                                    ))
                                })?
                            }
                            .into(),
                        );
                        Ok(None)
                    }
                    State::Literal => {
                        self.term = Some(
                            build_literal(
                                value,
                                self.lang.take(),
                                #[cfg(feature = "sparql-12")]
                                self.direction.take(),
                                self.datatype.take(),
                            )?
                            .into(),
                        );
                        Ok(None)
                    }
                    State::Triple => {
                        #[cfg(feature = "sparql-12")]
                        if let (Some(subject), Some(predicate), Some(object)) = (
                            self.subject_stack.pop(),
                            self.predicate_stack.pop(),
                            self.object_stack.pop(),
                        ) {
                            self.term = Some(
                                Triple::new(
                                    match subject {
                                        Term::NamedNode(subject) => NamedOrBlankNode::from(subject),
                                        Term::BlankNode(subject) => NamedOrBlankNode::from(subject),
                                        Term::Triple(_) => {
                                            return Err(QueryResultsSyntaxError::msg(
                                                "The <subject> value cannot be a <triple>",
                                            )
                                            .into());
                                        }
                                        Term::Literal(_) => {
                                            return Err(QueryResultsSyntaxError::msg(
                                                "The <subject> value cannot be a <literal>",
                                            )
                                            .into());
                                        }
                                    },
                                    if let Term::NamedNode(predicate) = predicate {
                                        predicate
                                    } else {
                                        return Err(QueryResultsSyntaxError::msg(
                                            "The <predicate> value must be an <uri>",
                                        )
                                        .into());
                                    },
                                    object,
                                )
                                .into(),
                            );
                            Ok(None)
                        } else {
                            Err(QueryResultsSyntaxError::msg(
                                "A <triple> must contain a <subject>, a <predicate> and an <object>",
                            )
                                .into())
                        }
                        #[cfg(not(feature = "sparql-12"))]
                        {
                            Err(QueryResultsSyntaxError::msg(
                                "The <triple> tag is only supported in RDF 1.2",
                            )
                            .into())
                        }
                    }
                }
            }
            Event::Decl(event) => {
                self.xml_version = event.xml_version()?;
                Ok(None)
            }
            Event::Eof | Event::Comment(_) | Event::PI(_) | Event::DocType(_) => Ok(None),
            Event::GeneralRef(event) => {
                decode_xml_entity(&event, &mut self.text_buffer, self.xml_version)?;
                Ok(None)
            }
            Event::Empty(_) => unreachable!("Empty events are expended"),
            Event::CData(_) => Err(QueryResultsSyntaxError::msg(
                "<![CDATA[...]]> are not supported in SPARQL XML results",
            )
            .into()),
        }
    }
}
