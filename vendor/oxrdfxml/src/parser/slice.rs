use super::events::InternalRdfXmlParser;
use super::prefixes::RdfXmlPrefixesIter;
use crate::error::{RdfXmlParseError, RdfXmlSyntaxError};
use oxrdf::Triple;

/// Parses a RDF/XML file from a byte slice.
///
/// Can be built using [`RdfXmlParser::for_slice`](crate::RdfXmlParser::for_slice).
///
/// Count the number of people:
/// ```
/// use oxrdf::NamedNodeRef;
/// use oxrdf::vocab::rdf;
/// use oxrdfxml::RdfXmlParser;
///
/// let file = r#"<?xml version="1.0"?>
/// <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:schema="http://schema.org/">
///  <rdf:Description rdf:about="http://example.com/foo">
///    <rdf:type rdf:resource="http://schema.org/Person" />
///    <schema:name>Foo</schema:name>
///  </rdf:Description>
///  <schema:Person rdf:about="http://example.com/bar" schema:name="Bar" />
/// </rdf:RDF>"#;
///
/// let schema_person = NamedNodeRef::new("http://schema.org/Person")?;
/// let mut count = 0;
/// for triple in RdfXmlParser::new().for_slice(file) {
///     let triple = triple?;
///     if triple.predicate == rdf::TYPE && triple.object == schema_person.into() {
///         count += 1;
///     }
/// }
/// assert_eq!(2, count);
/// # Result::<_, Box<dyn std::error::Error>>::Ok(())
/// ```
#[must_use]
pub struct SliceRdfXmlParser<'a> {
    pub(super) results: Vec<Triple>,
    pub(super) parser: InternalRdfXmlParser<&'a [u8]>,
}

impl Iterator for SliceRdfXmlParser<'_> {
    type Item = Result<Triple, RdfXmlSyntaxError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(triple) = self.results.pop() {
                return Some(Ok(triple));
            } else if self.parser.is_end {
                return None;
            }
            if let Err(RdfXmlParseError::Syntax(e)) = self.parse_step() {
                // I/O errors can't happen
                return Some(Err(e));
            }
        }
    }
}

impl SliceRdfXmlParser<'_> {
    /// The list of IRI prefixes considered at the current step of the parsing.
    ///
    /// This method returns (prefix name, prefix value) tuples.
    /// It is empty at the beginning of the parsing and gets updated when prefixes are encountered.
    /// It should be full at the end of the parsing (but if a prefix is overridden, only the latest version will be returned).
    ///
    /// ```
    /// use oxrdfxml::RdfXmlParser;
    ///
    /// let file = r#"<?xml version="1.0"?>
    /// <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:schema="http://schema.org/">
    ///  <rdf:Description rdf:about="http://example.com/foo">
    ///    <rdf:type rdf:resource="http://schema.org/Person" />
    ///    <schema:name>Foo</schema:name>
    ///  </rdf:Description>
    ///  <schema:Person rdf:about="http://example.com/bar" schema:name="Bar" />
    /// </rdf:RDF>"#;
    ///
    /// let mut parser = RdfXmlParser::new().for_slice(file);
    /// assert_eq!(parser.prefixes().collect::<Vec<_>>(), []); // No prefix at the beginning
    ///
    /// parser.next().unwrap()?; // We read the first triple
    /// assert_eq!(
    ///     parser.prefixes().collect::<Vec<_>>(),
    ///     [
    ///         ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ///         ("schema", "http://schema.org/")
    ///     ]
    /// ); // There are now prefixes
    /// //
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    pub fn prefixes(&self) -> RdfXmlPrefixesIter<'_> {
        RdfXmlPrefixesIter {
            inner: self.parser.reader.resolver().bindings(),
            decoder: self.parser.reader.decoder(),
            lenient: self.parser.lenient,
        }
    }

    /// The base IRI considered at the current step of the parsing.
    ///
    /// ```
    /// use oxrdfxml::RdfXmlParser;
    ///
    /// let file = r#"<?xml version="1.0"?>
    /// <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xml:base="http://example.com/">
    ///  <rdf:Description rdf:about="foo">
    ///    <rdf:type rdf:resource="http://schema.org/Person" />
    ///  </rdf:Description>
    /// </rdf:RDF>"#;
    ///
    /// let mut parser = RdfXmlParser::new().for_slice(file);
    /// assert!(parser.base_iri().is_none()); // No base at the beginning because none has been given to the parser.
    ///
    /// parser.next().unwrap()?; // We read the first triple
    /// assert_eq!(parser.base_iri(), Some("http://example.com/")); // There is now a base IRI.
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    pub fn base_iri(&self) -> Option<&str> {
        Some(self.parser.current_base_iri()?.as_str())
    }

    /// The current byte position in the input data.
    pub fn buffer_position(&self) -> u64 {
        self.parser.reader.buffer_position()
    }

    fn parse_step(&mut self) -> Result<(), RdfXmlParseError> {
        let event = self.parser.reader.read_event()?;
        self.parser.parse_event(event, &mut self.results)
    }
}
