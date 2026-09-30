use crate::{RdfXmlParser, RdfXmlSerializer};
use oxrdf::{Literal, NamedNode, Triple};

fn cases() -> Vec<Triple> {
    let mut triples = Vec::new();
    for value in [
        "x\ry",
        "x\r\ny",
        "\r",
        "\r\r",
        " \r\n\t ",
        "\rx\r",
        "&amp;&#13;<x>\r\u{e9}\u{1f642}",
    ] {
        let mut literals = vec![
            Literal::new_simple_literal(value),
            Literal::new_typed_literal(value, NamedNode::new("http://example.com/type").unwrap()),
            Literal::new_language_tagged_literal(value, "en").unwrap(),
        ];
        #[cfg(feature = "rdf-12")]
        literals.push(
            Literal::new_directional_language_tagged_literal(
                value,
                "ar",
                oxrdf::BaseDirection::Rtl,
            )
            .unwrap(),
        );
        for literal in literals {
            triples.push(Triple::new(
                NamedNode::new("http://example.com/s").unwrap(),
                NamedNode::new("http://example.com/p").unwrap(),
                literal,
            ));
        }
    }
    triples
}

fn serialized(triple: &Triple) -> Vec<u8> {
    let mut writer = RdfXmlSerializer::new().for_writer(Vec::new());
    writer.serialize_triple(triple).unwrap();
    writer.finish().unwrap()
}

fn assert_sync_readers(bytes: &[u8], expected: &Triple) {
    let slice = RdfXmlParser::new()
        .for_slice(bytes)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let reader = RdfXmlParser::new()
        .for_reader(bytes)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(slice, vec![expected.clone()]);
    assert_eq!(reader, vec![expected.clone()]);
}

#[test]
fn literal_cr_roundtrips_without_raw_cr() {
    for triple in cases() {
        let bytes = serialized(&triple);
        assert_sync_readers(&bytes, &triple);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(!text.contains('\r'));
        assert!(text.contains("&#13;") || text.contains("&#xD;"));
    }
}

fn raw_document(value: &str) -> String {
    format!(
        "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:e=\"http://example.com/\"><rdf:Description rdf:about=\"http://example.com/s\"><e:p>{value}</e:p></rdf:Description></rdf:RDF>"
    )
}

fn expected(value: &str) -> Triple {
    Triple::new(
        NamedNode::new("http://example.com/s").unwrap(),
        NamedNode::new("http://example.com/p").unwrap(),
        Literal::new_simple_literal(value),
    )
}

#[test]
fn raw_xml_newlines_normalize_but_references_do_not() {
    for (raw, value) in [
        ("x\ry", "x\ny"),
        ("x\r\ny", "x\ny"),
        ("x&#13;y", "x\ry"),
        ("x&#xD;y", "x\ry"),
    ] {
        assert_sync_readers(raw_document(raw).as_bytes(), &expected(value));
    }
}

#[cfg(feature = "async-tokio")]
#[tokio::test]
async fn async_writers_and_readers_preserve_literal_cr() {
    for triple in cases() {
        let mut writer = RdfXmlSerializer::new().for_tokio_async_writer(Vec::new());
        writer.serialize_triple(&triple).await.unwrap();
        let bytes = writer.finish().await.unwrap();
        assert_eq!(bytes, serialized(&triple));
        assert_sync_readers(&bytes, &triple);
        let mut reader = RdfXmlParser::new().for_tokio_async_reader(bytes.as_slice());
        assert_eq!(reader.next().await.unwrap().unwrap(), triple);
        assert!(reader.next().await.is_none());
    }
    for (raw, value) in [("x\ry", "x\ny"), ("x\r\ny", "x\ny"), ("x&#13;y", "x\ry")] {
        let bytes = raw_document(raw);
        let mut reader = RdfXmlParser::new().for_tokio_async_reader(bytes.as_bytes());
        assert_eq!(reader.next().await.unwrap().unwrap(), expected(value));
        assert!(reader.next().await.is_none());
    }
}

#[cfg(feature = "rdf-12")]
#[test]
fn quoted_triple_literal_cr_roundtrips() {
    let triple = Triple::new(
        NamedNode::new("http://example.com/s").unwrap(),
        NamedNode::new("http://example.com/p").unwrap(),
        oxrdf::Term::Triple(Box::new(expected("x\ry"))),
    );
    assert_sync_readers(&serialized(&triple), &triple);
}
