use crate::error::QueryResultsSyntaxError;
use oxrdf::vocab::{rdf, xsd};
use oxrdf::*;
use quick_xml::escape::{EscapeError, escape, resolve_xml_entity};
use quick_xml::events::{BytesEnd, BytesRef, BytesStart, BytesText, Event};
use quick_xml::{Error, XmlVersion};
use std::borrow::Cow;

pub(super) fn write_xml_term<'a>(output: &mut Vec<Event<'a>>, term: TermRef<'a>) {
    match term {
        TermRef::NamedNode(uri) => {
            output.push(Event::Start(BytesStart::new("uri")));
            output.push(Event::Text(BytesText::new(uri.as_str())));
            output.push(Event::End(BytesEnd::new("uri")));
        }
        TermRef::BlankNode(bnode) => {
            output.push(Event::Start(BytesStart::new("bnode")));
            output.push(Event::Text(BytesText::new(bnode.as_str())));
            output.push(Event::End(BytesEnd::new("bnode")));
        }
        TermRef::Literal(literal) => {
            let mut start = BytesStart::new("literal");
            if let Some(language) = literal.language() {
                start.push_attribute(("xml:lang", language));
                #[cfg(feature = "sparql-12")]
                if let Some(direction) = literal.direction() {
                    start.push_attribute((
                        "its:dir",
                        match direction {
                            BaseDirection::Ltr => "ltr",
                            BaseDirection::Rtl => "rtl",
                        },
                    ));
                    // TODO: put it in the root?
                    start.push_attribute(("xmlns:its", "http://www.w3.org/2005/11/its"));
                    start.push_attribute(("its:version", "2.0"));
                }
            } else if literal.datatype() != xsd::STRING {
                start.push_attribute(("datatype", literal.datatype().as_str()))
            }
            output.push(Event::Start(start));
            output.push(Event::Text(BytesText::from_escaped(
                escape_including_bound_whitespaces(literal.value()),
            )));
            output.push(Event::End(BytesEnd::new("literal")));
        }
        #[cfg(feature = "sparql-12")]
        TermRef::Triple(triple) => {
            output.push(Event::Start(BytesStart::new("triple")));
            output.push(Event::Start(BytesStart::new("subject")));
            write_xml_term(output, triple.subject.as_ref().into());
            output.push(Event::End(BytesEnd::new("subject")));
            output.push(Event::Start(BytesStart::new("predicate")));
            write_xml_term(output, triple.predicate.as_ref().into());
            output.push(Event::End(BytesEnd::new("predicate")));
            output.push(Event::Start(BytesStart::new("object")));
            write_xml_term(output, triple.object.as_ref());
            output.push(Event::End(BytesEnd::new("object")));
            output.push(Event::End(BytesEnd::new("triple")));
        }
    }
}

pub(super) fn build_literal(
    value: impl Into<String>,
    lang: Option<String>,
    #[cfg(feature = "sparql-12")] direction: Option<String>,
    datatype: Option<NamedNode>,
) -> Result<Literal, QueryResultsSyntaxError> {
    if let Some(lang) = lang {
        #[cfg(feature = "sparql-12")]
        if let Some(direction) = direction {
            if let Some(datatype) = datatype {
                if datatype.as_ref() != rdf::DIR_LANG_STRING {
                    return Err(QueryResultsSyntaxError::msg(format!(
                        "its:dir value '{direction}' provided with the datatype {datatype}"
                    )));
                }
            }
            return Literal::new_directional_language_tagged_literal(
                value,
                &lang,
                match direction.as_str() {
                    "ltr" => BaseDirection::Ltr,
                    "rtl" => BaseDirection::Rtl,
                    _ => {
                        return Err(QueryResultsSyntaxError::msg(format!(
                            "Invalid its:dir value '{direction}', expecting 'ltr' or 'rtl'"
                        )));
                    }
                },
            )
            .map_err(|e| {
                QueryResultsSyntaxError::msg(format!("Invalid xml:lang value '{lang}': {e}"))
            });
        }
        if let Some(datatype) = datatype {
            if datatype.as_ref() != rdf::LANG_STRING {
                return Err(QueryResultsSyntaxError::msg(format!(
                    "xml:lang value '{lang}' provided with the datatype {datatype}"
                )));
            }
        }
        Literal::new_language_tagged_literal(value, &lang).map_err(|e| {
            QueryResultsSyntaxError::msg(format!("Invalid xml:lang value '{lang}': {e}"))
        })
    } else {
        #[cfg(feature = "sparql-12")]
        if direction.is_some() {
            return Err(QueryResultsSyntaxError::msg(
                "its:dir can only be present alongside xml:lang",
            ));
        }
        Ok(if let Some(datatype) = datatype {
            Literal::new_typed_literal(value, datatype)
        } else {
            Literal::new_simple_literal(value)
        })
    }
}

/// Escapes boundary whitespace to prevent trimming and every CR to prevent XML newline normalization.
fn escape_including_bound_whitespaces(value: &str) -> Cow<'_, str> {
    let trimmed = value.trim_matches(|c| matches!(c, '\t' | '\n' | '\r' | ' '));
    let trimmed_escaped = escape(trimmed);
    // Insert CR references after escaping so their ampersands are not escaped again.
    let trimmed_escaped = if trimmed_escaped.contains('\r') {
        Cow::Owned(trimmed_escaped.replace('\r', "&#13;"))
    } else {
        trimmed_escaped
    };
    if trimmed == value {
        return trimmed_escaped;
    }
    let mut output =
        String::with_capacity(trimmed_escaped.len() + (value.len() - trimmed.len()) * 5);
    let mut prefix_len = 0;
    for c in value.chars() {
        match c {
            '\t' => output.push_str("&#9;"),
            '\n' => output.push_str("&#10;"),
            '\r' => output.push_str("&#13;"),
            ' ' => output.push_str("&#32;"),
            _ => break,
        }
        prefix_len += 1;
    }
    output.push_str(&trimmed_escaped);
    for c in value[prefix_len + trimmed.len()..].chars() {
        match c {
            '\t' => output.push_str("&#9;"),
            '\n' => output.push_str("&#10;"),
            '\r' => output.push_str("&#13;"),
            ' ' => output.push_str("&#32;"),
            _ => {
                unreachable!("Unexpected {c} at the end of the string {value:?}")
            }
        }
    }
    output.into()
}

pub(super) fn decode_xml_entity(
    event: &BytesRef<'_>,
    buffer: &mut String,
    xml_version: XmlVersion,
) -> Result<(), Error> {
    if let Some(char_ref) = event.resolve_char_ref()? {
        buffer.push(char_ref);
        return Ok(());
    }
    let reference = event.xml_content(xml_version)?;
    let Some(value) = resolve_xml_entity(&reference) else {
        return Err(EscapeError::UnrecognizedEntity(0..event.len(), reference.into()).into());
    };
    buffer.push_str(value);
    Ok(())
}
