use self::writer::InnerRdfXmlWriter;
use oxiri::{Iri, IriParseError};
use oxrdf::TripleRef;
use quick_xml::Writer;
use quick_xml::events::Event;
use std::collections::BTreeMap;
use std::io;
use std::io::Write;
#[cfg(feature = "async-tokio")]
use std::sync::Arc;
#[cfg(feature = "async-tokio")]
use tokio::io::AsyncWrite;

#[cfg(test)]
mod lexical_tests;
#[cfg(test)]
#[expect(clippy::panic_in_result_fn)]
mod tests;
mod writer;

/// A [RDF/XML](https://www.w3.org/TR/rdf-syntax-grammar/) serializer.
///
/// ```
/// use oxrdf::{LiteralRef, NamedNodeRef, TripleRef};
/// use oxrdf::vocab::rdf;
/// use oxrdfxml::RdfXmlSerializer;
///
/// let mut serializer = RdfXmlSerializer::new().with_prefix("schema", "http://schema.org/")?.for_writer(Vec::new());
/// serializer.serialize_triple(TripleRef::new(
///     NamedNodeRef::new("http://example.com#me")?,
///     rdf::TYPE,
///     NamedNodeRef::new("http://schema.org/Person")?,
/// ))?;
/// serializer.serialize_triple(TripleRef::new(
///     NamedNodeRef::new("http://example.com#me")?,
///     NamedNodeRef::new("http://schema.org/name")?,
///     LiteralRef::new_language_tagged_literal_unchecked("Foo Bar", "en"),
/// ))?;
/// assert_eq!(
///     b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xmlns:schema=\"http://schema.org/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n\t<schema:Person rdf:about=\"http://example.com#me\">\n\t\t<schema:name xml:lang=\"en\">Foo Bar</schema:name>\n\t</schema:Person>\n</rdf:RDF>",
///     serializer.finish()?.as_slice()
/// );
/// # Result::<_, Box<dyn std::error::Error>>::Ok(())
/// ```
#[derive(Default, Clone)]
#[must_use]
pub struct RdfXmlSerializer {
    prefixes: BTreeMap<String, String>,
    base_iri: Option<Iri<String>>,
}

impl RdfXmlSerializer {
    /// Builds a new [`RdfXmlSerializer`].
    #[inline]
    pub fn new() -> Self {
        Self {
            prefixes: BTreeMap::new(),
            base_iri: None,
        }
    }

    #[inline]
    pub fn with_prefix(
        mut self,
        prefix_name: impl Into<String>,
        prefix_iri: impl Into<String>,
    ) -> Result<Self, IriParseError> {
        let prefix_name = prefix_name.into();
        if prefix_name == "oxprefix" {
            return Ok(self); // It is reserved
        }
        self.prefixes
            .insert(prefix_name, Iri::parse(prefix_iri.into())?.into_inner());
        Ok(self)
    }

    /// ```
    /// use oxrdf::{NamedNodeRef, TripleRef};
    /// use oxrdfxml::RdfXmlSerializer;
    ///
    /// let mut serializer = RdfXmlSerializer::new()
    ///     .with_base_iri("http://example.com")?
    ///     .with_prefix("ex", "http://example.com/ns#")?
    ///     .for_writer(Vec::new());
    /// serializer.serialize_triple(TripleRef::new(
    ///     NamedNodeRef::new("http://example.com#me")?,
    ///     NamedNodeRef::new("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")?,
    ///     NamedNodeRef::new("http://example.com/ns#Person")?,
    /// ))?;
    /// serializer.serialize_triple(TripleRef::new(
    ///     NamedNodeRef::new("http://example.com#me")?,
    ///     NamedNodeRef::new("http://example.com/ns#parent")?,
    ///     NamedNodeRef::new("http://example.com#other")?,
    /// ))?;
    /// assert_eq!(
    ///     b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xml:base=\"http://example.com\" xmlns:ex=\"http://example.com/ns#\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n\t<ex:Person rdf:about=\"#me\">\n\t\t<ex:parent rdf:resource=\"#other\"/>\n\t</ex:Person>\n</rdf:RDF>",
    ///     serializer.finish()?.as_slice()
    /// );
    /// # Result::<_,Box<dyn std::error::Error>>::Ok(())
    /// ```
    #[inline]
    pub fn with_base_iri(mut self, base_iri: impl Into<String>) -> Result<Self, IriParseError> {
        self.base_iri = Some(Iri::parse(base_iri.into())?);
        Ok(self)
    }

    /// Serializes a RDF/XML file to a [`Write`] implementation.
    ///
    /// This writer does unbuffered writes.
    ///
    /// ```
    /// use oxrdf::{LiteralRef, NamedNodeRef, TripleRef};
    /// use oxrdf::vocab::rdf;
    /// use oxrdfxml::RdfXmlSerializer;
    ///
    /// let mut serializer = RdfXmlSerializer::new().with_prefix("schema", "http://schema.org/")?.for_writer(Vec::new());
    /// serializer.serialize_triple(TripleRef::new(
    ///     NamedNodeRef::new("http://example.com#me")?,
    ///     rdf::TYPE,
    ///     NamedNodeRef::new("http://schema.org/Person")?,
    /// ))?;
    /// serializer.serialize_triple(TripleRef::new(
    ///     NamedNodeRef::new("http://example.com#me")?,
    ///     NamedNodeRef::new("http://schema.org/name")?,
    ///     LiteralRef::new_language_tagged_literal_unchecked("Foo Bar", "en"),
    /// ))?;
    /// assert_eq!(
    ///     b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xmlns:schema=\"http://schema.org/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n\t<schema:Person rdf:about=\"http://example.com#me\">\n\t\t<schema:name xml:lang=\"en\">Foo Bar</schema:name>\n\t</schema:Person>\n</rdf:RDF>",
    ///     serializer.finish()?.as_slice()
    /// );
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    pub fn for_writer<W: Write>(self, writer: W) -> WriterRdfXmlSerializer<W> {
        WriterRdfXmlSerializer {
            writer: Writer::new_with_indent(writer, b'\t', 1),
            inner: self.inner_writer(),
        }
    }

    /// Serializes a RDF/XML file to a [`AsyncWrite`] implementation.
    ///
    /// This writer does unbuffered writes.
    ///
    /// ```
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use oxrdf::{NamedNodeRef, TripleRef, LiteralRef};
    /// use oxrdf::vocab::rdf;
    /// use oxrdfxml::RdfXmlSerializer;
    ///
    /// let mut serializer = RdfXmlSerializer::new().with_prefix("schema", "http://schema.org/")?.for_tokio_async_writer(Vec::new());
    /// serializer.serialize_triple(TripleRef::new(
    ///     NamedNodeRef::new("http://example.com#me")?,
    ///     rdf::TYPE,
    ///     NamedNodeRef::new("http://schema.org/Person")?,
    /// )).await?;
    /// serializer.serialize_triple(TripleRef::new(
    ///     NamedNodeRef::new("http://example.com#me")?,
    ///     NamedNodeRef::new("http://schema.org/name")?,
    ///     LiteralRef::new_language_tagged_literal_unchecked("Foo Bar", "en"),
    /// )).await?;
    /// assert_eq!(
    ///     b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xmlns:schema=\"http://schema.org/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n\t<schema:Person rdf:about=\"http://example.com#me\">\n\t\t<schema:name xml:lang=\"en\">Foo Bar</schema:name>\n\t</schema:Person>\n</rdf:RDF>",
    ///     serializer.finish().await?.as_slice()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "async-tokio")]
    pub fn for_tokio_async_writer<W: AsyncWrite + Unpin>(
        self,
        writer: W,
    ) -> TokioAsyncWriterRdfXmlSerializer<W> {
        TokioAsyncWriterRdfXmlSerializer {
            writer: Writer::new_with_indent(writer, b'\t', 1),
            inner: self.inner_writer(),
        }
    }

    fn inner_writer(self) -> InnerRdfXmlWriter {
        InnerRdfXmlWriter::new(self.prefixes, self.base_iri)
    }
}

/// Serializes a RDF/XML file to a [`Write`] implementation.
///
/// Can be built using [`RdfXmlSerializer::for_writer`].
///
/// ```
/// use oxrdf::{LiteralRef, NamedNodeRef, TripleRef};
/// use oxrdf::vocab::rdf;
/// use oxrdfxml::RdfXmlSerializer;
///
/// let mut serializer = RdfXmlSerializer::new().with_prefix("schema", "http://schema.org/")?.for_writer(Vec::new());
/// serializer.serialize_triple(TripleRef::new(
///     NamedNodeRef::new("http://example.com#me")?,
///     rdf::TYPE,
///     NamedNodeRef::new("http://schema.org/Person")?,
/// ))?;
/// serializer.serialize_triple(TripleRef::new(
///     NamedNodeRef::new("http://example.com#me")?,
///     NamedNodeRef::new("http://schema.org/name")?,
///     LiteralRef::new_language_tagged_literal_unchecked("Foo Bar", "en"),
/// ))?;
/// assert_eq!(
///     b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xmlns:schema=\"http://schema.org/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n\t<schema:Person rdf:about=\"http://example.com#me\">\n\t\t<schema:name xml:lang=\"en\">Foo Bar</schema:name>\n\t</schema:Person>\n</rdf:RDF>",
///     serializer.finish()?.as_slice()
/// );
/// # Result::<_, Box<dyn std::error::Error>>::Ok(())
/// ```
#[must_use]
pub struct WriterRdfXmlSerializer<W: Write> {
    writer: Writer<W>,
    inner: InnerRdfXmlWriter,
}

impl<W: Write> WriterRdfXmlSerializer<W> {
    /// Serializes an extra triple.
    pub fn serialize_triple<'a>(&mut self, t: impl Into<TripleRef<'a>>) -> io::Result<()> {
        let mut buffer = Vec::new();
        self.inner.serialize_triple(t, &mut buffer)?;
        self.flush_buffer(&mut buffer)
    }

    /// Ends the write process and returns the underlying [`Write`].
    pub fn finish(mut self) -> io::Result<W> {
        let mut buffer = Vec::new();
        self.inner.finish(&mut buffer);
        self.flush_buffer(&mut buffer)?;
        Ok(self.writer.into_inner())
    }

    fn flush_buffer(&mut self, buffer: &mut Vec<Event<'_>>) -> io::Result<()> {
        for event in buffer.drain(0..) {
            self.writer.write_event(event)?;
        }
        Ok(())
    }
}

/// Serializes a RDF/XML file to a [`AsyncWrite`] implementation.
///
/// Can be built using [`RdfXmlSerializer::for_tokio_async_writer`].
///
/// ```
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use oxrdf::{NamedNodeRef, TripleRef, LiteralRef};
/// use oxrdf::vocab::rdf;
/// use oxrdfxml::RdfXmlSerializer;
///
/// let mut serializer = RdfXmlSerializer::new().with_prefix("schema", "http://schema.org/")?.for_tokio_async_writer(Vec::new());
/// serializer.serialize_triple(TripleRef::new(
///     NamedNodeRef::new("http://example.com#me")?,
///     rdf::TYPE,
///     NamedNodeRef::new("http://schema.org/Person")?,
/// )).await?;
/// serializer.serialize_triple(TripleRef::new(
///     NamedNodeRef::new("http://example.com#me")?,
///     NamedNodeRef::new("http://schema.org/name")?,
///     LiteralRef::new_language_tagged_literal_unchecked("Foo Bar", "en"),
/// )).await?;
/// assert_eq!(
///     b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xmlns:schema=\"http://schema.org/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n\t<schema:Person rdf:about=\"http://example.com#me\">\n\t\t<schema:name xml:lang=\"en\">Foo Bar</schema:name>\n\t</schema:Person>\n</rdf:RDF>",
///     serializer.finish().await?.as_slice()
/// );
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "async-tokio")]
#[must_use]
pub struct TokioAsyncWriterRdfXmlSerializer<W: AsyncWrite + Unpin> {
    writer: Writer<W>,
    inner: InnerRdfXmlWriter,
}

#[cfg(feature = "async-tokio")]
impl<W: AsyncWrite + Unpin> TokioAsyncWriterRdfXmlSerializer<W> {
    /// Serializes an extra triple.
    pub async fn serialize_triple<'a>(&mut self, t: impl Into<TripleRef<'a>>) -> io::Result<()> {
        let mut buffer = Vec::new();
        self.inner.serialize_triple(t, &mut buffer)?;
        self.flush_buffer(&mut buffer).await
    }

    /// Ends the write process and returns the underlying [`Write`].
    pub async fn finish(mut self) -> io::Result<W> {
        let mut buffer = Vec::new();
        self.inner.finish(&mut buffer);
        self.flush_buffer(&mut buffer).await?;
        Ok(self.writer.into_inner())
    }

    async fn flush_buffer(&mut self, buffer: &mut Vec<Event<'_>>) -> io::Result<()> {
        for event in buffer.drain(0..) {
            self.writer
                .write_event_async(event)
                .await
                .map_err(map_err)?;
        }
        Ok(())
    }
}

#[cfg(feature = "async-tokio")]
fn map_err(error: quick_xml::Error) -> io::Error {
    if let quick_xml::Error::Io(error) = error {
        Arc::try_unwrap(error).unwrap_or_else(|error| io::Error::new(error.kind(), error))
    } else {
        io::Error::other(error)
    }
}
