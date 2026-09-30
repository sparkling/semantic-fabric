use super::writer::split_iri;
use super::*;
use oxrdf::NamedNodeRef;
use oxrdf::vocab::rdf;
use std::error::Error;

#[test]
fn test_split_iri() {
    assert_eq!(
        split_iri("http://schema.org/Person"),
        ("http://schema.org/", "Person")
    );
    assert_eq!(split_iri("http://schema.org/"), ("http://schema.org/", ""));
    assert_eq!(
        split_iri("http://schema.org#foo"),
        ("http://schema.org#", "foo")
    );
    assert_eq!(split_iri("urn:isbn:foo"), ("urn:isbn:", "foo"));
}

#[test]
fn test_custom_rdf_ns() -> Result<(), Box<dyn Error>> {
    let output = RdfXmlSerializer::new()
        .with_prefix("rdf", "http://example.com/")?
        .for_writer(Vec::new())
        .finish()?;
    assert_eq!(output, b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n</rdf:RDF>");
    Ok(())
}

#[test]
fn test_custom_empty_ns() -> Result<(), Box<dyn Error>> {
    let mut serializer = RdfXmlSerializer::new()
        .with_prefix("", "http://example.com/")?
        .for_writer(Vec::new());
    serializer.serialize_triple(TripleRef::new(
        NamedNodeRef::new("http://example.com/s")?,
        rdf::TYPE,
        NamedNodeRef::new("http://example.org/o")?,
    ))?;
    serializer.serialize_triple(TripleRef::new(
        NamedNodeRef::new("http://example.com/s")?,
        NamedNodeRef::new("http://example.com/p")?,
        NamedNodeRef::new("http://example.com/o2")?,
    ))?;
    let output = serializer.finish()?;
    assert_eq!(
        String::from_utf8_lossy(&output),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rdf:RDF xmlns=\"http://example.com/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:its=\"http://www.w3.org/2005/11/its\">\n\t<oxprefix:o xmlns:oxprefix=\"http://example.org/\" rdf:about=\"http://example.com/s\">\n\t\t<p rdf:resource=\"http://example.com/o2\"/>\n\t</oxprefix:o>\n</rdf:RDF>"
    );
    Ok(())
}
