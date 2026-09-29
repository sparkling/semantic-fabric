use crate::error::QueryResultsSyntaxError;
use json_event_parser::JsonEvent;
use oxrdf::vocab::rdf;
use oxrdf::*;

#[derive(Default)]
pub(super) struct JsonInnerTermReader {
    state: JsonInnerTermReaderState,
    term_type: Option<TermType>,
    value: Option<String>,
    lang: Option<String>,
    #[cfg(feature = "sparql-12")]
    direction: Option<String>,
    datatype: Option<NamedNode>,
    #[cfg(feature = "sparql-12")]
    subject: Option<Term>,
    #[cfg(feature = "sparql-12")]
    predicate: Option<Term>,
    #[cfg(feature = "sparql-12")]
    object: Option<Term>,
}

#[derive(Default)]
enum JsonInnerTermReaderState {
    #[default]
    Start,
    Middle,
    TermType,
    Value,
    Lang,
    #[cfg(feature = "sparql-12")]
    BaseDirection,
    Datatype,
    #[cfg(feature = "sparql-12")]
    InValue,
    #[cfg(feature = "sparql-12")]
    Subject(Box<JsonInnerTermReader>),
    #[cfg(feature = "sparql-12")]
    Predicate(Box<JsonInnerTermReader>),
    #[cfg(feature = "sparql-12")]
    Object(Box<JsonInnerTermReader>),
}

enum TermType {
    Uri,
    BNode,
    Literal,
    #[cfg(feature = "sparql-12")]
    Triple,
}

impl JsonInnerTermReader {
    pub(super) fn read_event(
        &mut self,
        event: JsonEvent<'_>,
    ) -> Result<Option<Term>, QueryResultsSyntaxError> {
        match &mut self.state {
            JsonInnerTermReaderState::Start => {
                if event == JsonEvent::StartObject {
                    self.state = JsonInnerTermReaderState::Middle;
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "RDF terms must be encoded using objects",
                    ))
                }
            }
            JsonInnerTermReaderState::Middle => match event {
                JsonEvent::ObjectKey(object_key) => {
                    self.state = match object_key.as_ref() {
                        "type" => JsonInnerTermReaderState::TermType,
                        "value" => JsonInnerTermReaderState::Value,
                        "datatype" => JsonInnerTermReaderState::Datatype,
                        "xml:lang" => JsonInnerTermReaderState::Lang,
                        #[cfg(feature = "sparql-12")]
                        "its:dir" => JsonInnerTermReaderState::BaseDirection,
                        _ => {
                            return Err(QueryResultsSyntaxError::msg(format!(
                                "Unsupported term key: {object_key}"
                            )));
                        }
                    };
                    Ok(None)
                }
                JsonEvent::EndObject => {
                    self.state = JsonInnerTermReaderState::Start;
                    match self.term_type.take() {
                        None => Err(QueryResultsSyntaxError::msg(
                            "Term serialization must have a 'type' key",
                        )),
                        Some(TermType::Uri) => Ok(Some(
                            NamedNode::new(self.value.take().ok_or_else(|| {
                                QueryResultsSyntaxError::msg(
                                    "uri serialization must have a 'value' key",
                                )
                            })?)
                            .map_err(|e| {
                                QueryResultsSyntaxError::msg(format!("Invalid uri value: {e}"))
                            })?
                            .into(),
                        )),
                        Some(TermType::BNode) => Ok(Some(
                            BlankNode::new(self.value.take().ok_or_else(|| {
                                QueryResultsSyntaxError::msg(
                                    "bnode serialization must have a 'value' key",
                                )
                            })?)
                            .map_err(|e| {
                                QueryResultsSyntaxError::msg(format!("Invalid bnode value: {e}"))
                            })?
                            .into(),
                        )),
                        Some(TermType::Literal) => {
                            let value = self.value.take().ok_or_else(|| {
                                QueryResultsSyntaxError::msg(
                                    "literal serialization must have a 'value' key",
                                )
                            })?;
                            Ok(Some(if let Some(lang) = self.lang.take() {
                                #[cfg(feature = "sparql-12")]
                                if let Some(direction) = self.direction.take() {
                                    if let Some(datatype) = &self.datatype {
                                        if datatype.as_ref() != rdf::DIR_LANG_STRING {
                                            return Err(QueryResultsSyntaxError::msg(format!(
                                                "xml:lang value '{lang}' and its:dir value '{direction}' provided with the datatype {datatype}"
                                            )));
                                        }
                                    }
                                    return Ok(Some(Literal::new_directional_language_tagged_literal(
                                        value,
                                        &lang,
                                        match direction.as_str() {
                                            "ltr" => BaseDirection::Ltr,
                                            "rtl" => BaseDirection::Rtl,
                                            _ => return Err(QueryResultsSyntaxError::msg(format!(
                                                "Invalid its:dir value '{direction}', expecting 'ltr' or 'rtl'"
                                            )))
                                        }
                                    ).map_err(|e| {
                                        QueryResultsSyntaxError::msg(format!(
                                            "Invalid xml:lang value '{lang}': {e}"
                                        ))
                                    })?.into()))
                                }
                                if let Some(datatype) = &self.datatype {
                                    if datatype.as_ref() != rdf::LANG_STRING {
                                        return Err(QueryResultsSyntaxError::msg(format!(
                                            "xml:lang value '{lang}' provided with the datatype {datatype}"
                                        )));
                                    }
                                }
                                Literal::new_language_tagged_literal(value, &lang)
                                    .map_err(|e| {
                                        QueryResultsSyntaxError::msg(format!(
                                            "Invalid xml:lang value '{lang}': {e}"
                                        ))
                                    })?
                            } else {
                                #[cfg(feature = "sparql-12")]
                                if self.direction.take().is_some() {
                                    return Err(QueryResultsSyntaxError::msg("its:dir can only be present alongside xml:lang"))
                                }
                                if let Some(datatype) = self.datatype.take() {
                                    Literal::new_typed_literal(value, datatype)
                                } else {
                                    Literal::new_simple_literal(value)
                                }
                            }.into()))
                        }
                        #[cfg(feature = "sparql-12")]
                        Some(TermType::Triple) => Ok(Some(
                            Triple::new(
                                match self.subject.take().ok_or_else(|| {
                                    QueryResultsSyntaxError::msg(
                                        "triple serialization must have a 'subject' key",
                                    )
                                })? {
                                    Term::NamedNode(subject) => NamedOrBlankNode::from(subject),
                                    Term::BlankNode(subject) => NamedOrBlankNode::from(subject),
                                    Term::Triple(_) => {
                                        return Err(QueryResultsSyntaxError::msg(
                                            "The 'subject' value cannot be a triple term",
                                        ));
                                    }
                                    Term::Literal(_) => {
                                        return Err(QueryResultsSyntaxError::msg(
                                            "The 'subject' value cannot be a literal",
                                        ));
                                    }
                                },
                                if let Term::NamedNode(predicate) =
                                    self.predicate.take().ok_or_else(|| {
                                        QueryResultsSyntaxError::msg(
                                            "triple serialization must have a 'predicate' key",
                                        )
                                    })?
                                {
                                    predicate
                                } else {
                                    return Err(QueryResultsSyntaxError::msg(
                                        "The 'predicate' value must be a uri",
                                    ));
                                },
                                self.object.take().ok_or_else(|| {
                                    QueryResultsSyntaxError::msg(
                                        "triple serialization must have a 'object' key",
                                    )
                                })?,
                            )
                            .into(),
                        )),
                    }
                }
                _ => unreachable!(),
            },
            JsonInnerTermReaderState::TermType => {
                self.state = JsonInnerTermReaderState::Middle;
                if let JsonEvent::String(value) = event {
                    match value.as_ref() {
                        "uri" => {
                            self.term_type = Some(TermType::Uri);
                            Ok(None)
                        }
                        "bnode" => {
                            self.term_type = Some(TermType::BNode);
                            Ok(None)
                        }
                        "literal" | "typed-literal" => {
                            self.term_type = Some(TermType::Literal);
                            Ok(None)
                        }
                        #[cfg(feature = "sparql-12")]
                        "triple" => {
                            self.term_type = Some(TermType::Triple);
                            Ok(None)
                        }
                        _ => Err(QueryResultsSyntaxError::msg(format!(
                            "Unexpected term type: '{value}'"
                        ))),
                    }
                } else {
                    Err(QueryResultsSyntaxError::msg("Term type must be a string"))
                }
            }
            JsonInnerTermReaderState::Value => match event {
                JsonEvent::String(value) => {
                    self.value = Some(value.into_owned());
                    self.state = JsonInnerTermReaderState::Middle;
                    Ok(None)
                }
                #[cfg(feature = "sparql-12")]
                JsonEvent::StartObject => {
                    self.state = JsonInnerTermReaderState::InValue;
                    Ok(None)
                }
                _ => {
                    self.state = JsonInnerTermReaderState::Middle;

                    Err(QueryResultsSyntaxError::msg("Term value must be a string"))
                }
            },
            JsonInnerTermReaderState::Lang => {
                let result = if let JsonEvent::String(value) = event {
                    self.lang = Some(value.into_owned());
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg("Term lang must be strings"))
                };
                self.state = JsonInnerTermReaderState::Middle;

                result
            }
            #[cfg(feature = "sparql-12")]
            JsonInnerTermReaderState::BaseDirection => {
                let result = if let JsonEvent::String(value) = event {
                    self.direction = Some(value.into_owned());
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "Term base directions must be strings",
                    ))
                };
                self.state = JsonInnerTermReaderState::Middle;

                result
            }
            JsonInnerTermReaderState::Datatype => {
                let result = if let JsonEvent::String(value) = event {
                    match NamedNode::new(value) {
                        Ok(datatype) => {
                            self.datatype = Some(datatype);
                            Ok(None)
                        }
                        Err(e) => Err(QueryResultsSyntaxError::msg(format!(
                            "Invalid datatype: {e}"
                        ))),
                    }
                } else {
                    Err(QueryResultsSyntaxError::msg("Term lang must be strings"))
                };
                self.state = JsonInnerTermReaderState::Middle;

                result
            }
            #[cfg(feature = "sparql-12")]
            JsonInnerTermReaderState::InValue => match event {
                JsonEvent::ObjectKey(object_key) => {
                    self.state = match object_key.as_ref() {
                        "subject" => JsonInnerTermReaderState::Subject(Box::default()),
                        "predicate" => JsonInnerTermReaderState::Predicate(Box::default()),
                        "object" => JsonInnerTermReaderState::Object(Box::default()),
                        _ => {
                            return Err(QueryResultsSyntaxError::msg(format!(
                                "Unsupported value key: {object_key}"
                            )));
                        }
                    };
                    Ok(None)
                }
                JsonEvent::EndObject => {
                    self.state = JsonInnerTermReaderState::Middle;
                    Ok(None)
                }
                _ => unreachable!(),
            },
            #[cfg(feature = "sparql-12")]
            JsonInnerTermReaderState::Subject(inner_state) => {
                if let Some(term) = inner_state.read_event(event)? {
                    self.state = JsonInnerTermReaderState::InValue;
                    self.subject = Some(term);
                }
                Ok(None)
            }
            #[cfg(feature = "sparql-12")]
            JsonInnerTermReaderState::Predicate(inner_state) => {
                if let Some(term) = inner_state.read_event(event)? {
                    self.state = JsonInnerTermReaderState::InValue;
                    self.predicate = Some(term);
                }
                Ok(None)
            }
            #[cfg(feature = "sparql-12")]
            JsonInnerTermReaderState::Object(inner_state) => {
                if let Some(term) = inner_state.read_event(event)? {
                    self.state = JsonInnerTermReaderState::InValue;
                    self.object = Some(term);
                }
                Ok(None)
            }
        }
    }
}
