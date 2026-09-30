use super::events::InternalRdfXmlParser;
use crate::error::{RdfXmlParseError, RdfXmlSyntaxError};
use quick_xml::Error;
use quick_xml::escape::{EscapeError, resolve_xml_entity, unescape_with};
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesRef, BytesText};
use std::borrow::Cow;
use std::collections::HashMap;

const MAX_ENTITY_NESTING: usize = 1024;

#[derive(Default)]
pub(super) struct EntityRegistry(HashMap<String, String>);

impl EntityRegistry {
    pub(super) fn resolve(&self, e: &str) -> Option<&str> {
        resolve_xml_entity(e).or_else(|| self.0.get(e).map(String::as_str))
    }
}

impl<R> InternalRdfXmlParser<R> {
    pub(super) fn parse_doctype(&mut self, dt: &BytesText<'_>) -> Result<(), RdfXmlParseError> {
        // we extract entities
        for input in dt.xml_content(self.xml_version)?.split('<').skip(1) {
            if let Some(input) = input.strip_prefix("!ENTITY") {
                let input = input.trim_start().strip_prefix('%').unwrap_or(input);
                let (entity_name, input) = input.trim_start().split_once(|c: char| c.is_ascii_whitespace()).ok_or_else(|| {
                    RdfXmlSyntaxError::msg(
                        "<!ENTITY declarations should contain both an entity name and an entity value",
                    )
                })?;
                let input = input.trim_start().strip_prefix('\"').ok_or_else(|| {
                    RdfXmlSyntaxError::msg("<!ENTITY values should be enclosed in double quotes")
                })?;
                let (entity_value, input) = input.split_once('\"').ok_or_else(|| {
                    RdfXmlSyntaxError::msg(
                        "<!ENTITY declarations values should be enclosed in double quotes",
                    )
                })?;
                input.trim_start().strip_prefix('>').ok_or_else(|| {
                    RdfXmlSyntaxError::msg("<!ENTITY declarations values should end with >")
                })?;

                // Resolves custom entities within the current entity definition.
                let entity_value = unescape_with(entity_value, |e| self.custom_entities.resolve(e))
                    .map_err(Error::from)?;
                self.custom_entities
                    .0
                    .insert(entity_name.to_owned(), entity_value.to_string());
            }
        }
        Ok(())
    }

    pub(super) fn convert_attribute<'a>(
        &self,
        attribute: &Attribute<'a>,
    ) -> Result<Cow<'a, str>, RdfXmlParseError> {
        Ok(attribute.decoded_and_normalized_value_with(
            self.xml_version,
            self.reader.decoder(),
            MAX_ENTITY_NESTING,
            |e| self.custom_entities.resolve(e),
        )?)
    }

    pub(super) fn decode_xml_entity(&mut self, event: &BytesRef<'_>) -> Result<(), Error> {
        if let Some(char_ref) = event.resolve_char_ref()? {
            self.text_buffer.get_or_insert_default().push(char_ref);
            return Ok(());
        }
        let reference = event.xml_content(self.xml_version)?;
        let Some(value) = self.custom_entities.resolve(&reference) else {
            return Err(EscapeError::UnrecognizedEntity(0..event.len(), reference.into()).into());
        };
        self.text_buffer.get_or_insert_default().push_str(value);
        Ok(())
    }
}
