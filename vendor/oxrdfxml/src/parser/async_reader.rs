use super::events::InternalRdfXmlParser;
use super::prefixes::RdfXmlPrefixesIter;
use crate::error::RdfXmlParseError;
use oxrdf::Triple;
use tokio::io::{AsyncRead, BufReader as AsyncBufReader};

/// Parses a RDF/XML file from a [`AsyncRead`] implementation.
///
/// Can be built using [`RdfXmlParser::for_tokio_async_reader`](crate::RdfXmlParser::for_tokio_async_reader).
///
/// Count the number of people:
/// ```
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use oxrdf::NamedNodeRef;
/// use oxrdf::vocab::rdf;
/// use oxrdfxml::RdfXmlParser;
///
/// let file = r#"<?xml version="1.0"?>
/// <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:schema="http://schema.org/">
///   <rdf:Description rdf:about="http://example.com/foo">
///     <rdf:type rdf:resource="http://schema.org/Person" />
///     <schema:name>Foo</schema:name>
///   </rdf:Description>
///   <schema:Person rdf:about="http://example.com/bar" schema:name="Bar" />
/// </rdf:RDF>"#;
///
/// let schema_person = NamedNodeRef::new("http://schema.org/Person")?;
/// let mut count = 0;
/// let mut parser = RdfXmlParser::new().for_tokio_async_reader(file.as_bytes());
/// while let Some(triple) = parser.next().await {
///     let triple = triple?;
///     if triple.predicate == rdf::TYPE && triple.object == schema_person.into() {
///         count += 1;
///     }
/// }
/// assert_eq!(2, count);
/// # Ok(())
/// # }
/// ```
#[must_use]
pub struct TokioAsyncReaderRdfXmlParser<R: AsyncRead + Unpin> {
    pub(super) results: Vec<Triple>,
    pub(super) parser: InternalRdfXmlParser<AsyncBufReader<R>>,
    pub(super) reader_buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> TokioAsyncReaderRdfXmlParser<R> {
    /// Reads the next triple or returns `None` if the file is finished.
    pub async fn next(&mut self) -> Option<Result<Triple, RdfXmlParseError>> {
        loop {
            if let Some(triple) = self.results.pop() {
                return Some(Ok(triple));
            } else if self.parser.is_end {
                return None;
            }
            if let Err(e) = self.parse_step().await {
                return Some(Err(e));
            }
        }
    }

    /// The list of IRI prefixes considered at the current step of the parsing.
    ///
    /// This method returns (prefix name, prefix value) tuples.
    /// It is empty at the beginning of the parsing and gets updated when prefixes are encountered.
    /// It should be full at the end of the parsing (but if a prefix is overridden, only the latest version will be returned).
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    /// let mut parser = RdfXmlParser::new().for_tokio_async_reader(file.as_bytes());
    /// assert_eq!(parser.prefixes().collect::<Vec<_>>(), []); // No prefix at the beginning
    ///
    /// parser.next().await.unwrap()?; // We read the first triple
    /// assert_eq!(
    ///     parser.prefixes().collect::<Vec<_>>(),
    ///     [
    ///         ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ///         ("schema", "http://schema.org/")
    ///     ]
    /// ); // There are now prefixes
    /// //
    /// # Ok(())
    /// # }
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
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use oxrdfxml::RdfXmlParser;
    ///
    /// let file = r#"<?xml version="1.0"?>
    /// <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xml:base="http://example.com/">
    ///  <rdf:Description rdf:about="foo">
    ///    <rdf:type rdf:resource="http://schema.org/Person" />
    ///  </rdf:Description>
    /// </rdf:RDF>"#;
    ///
    /// let mut parser = RdfXmlParser::new().for_tokio_async_reader(file.as_bytes());
    /// assert!(parser.base_iri().is_none()); // No base at the beginning because none has been given to the parser.
    ///
    /// parser.next().await.unwrap()?; // We read the first triple
    /// assert_eq!(parser.base_iri(), Some("http://example.com/")); // There is now a base IRI.
    /// # Ok(())
    /// # }
    /// ```
    pub fn base_iri(&self) -> Option<&str> {
        Some(self.parser.current_base_iri()?.as_str())
    }

    /// The current byte position in the input data.
    pub fn buffer_position(&self) -> u64 {
        self.parser.reader.buffer_position()
    }

    async fn parse_step(&mut self) -> Result<(), RdfXmlParseError> {
        self.reader_buffer.clear();
        let event = self
            .parser
            .reader
            .read_event_into_async(&mut self.reader_buffer)
            .await?;
        self.parser.parse_event(event, &mut self.results)
    }
}
