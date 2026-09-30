use crate::{QueryResultsFormat, QueryResultsParser, QueryResultsSerializer};
use crate::{ReaderQueryResultsParserOutput, SliceQueryResultsParserOutput};
use oxrdf::{Literal, NamedNode, Term, Variable};

fn document(content: &str, attributes: &str) -> String {
    format!(
        "<sparql xmlns=\"http://www.w3.org/2005/sparql-results#\"><head><variable name=\"x\"/></head><results><result><binding name=\"x\"><literal{attributes}>{content}</literal></binding></result></results></sparql>"
    )
}

fn assert_sync(xml: &[u8], expected: &Term) {
    let parser = QueryResultsParser::from_format(QueryResultsFormat::Xml);
    let ReaderQueryResultsParserOutput::Solutions(mut rows) =
        parser.clone().for_reader(xml).unwrap()
    else {
        panic!("expected solutions");
    };
    assert_eq!(rows.next().unwrap().unwrap().get("x"), Some(expected));
    assert!(rows.next().is_none());
    let SliceQueryResultsParserOutput::Solutions(mut rows) = parser.for_slice(xml).unwrap() else {
        panic!("expected solutions");
    };
    assert_eq!(rows.next().unwrap().unwrap().get("x"), Some(expected));
    assert!(rows.next().is_none());
}

fn cases() -> Vec<(String, Term)> {
    let mut cases: Vec<(String, Term)> = Vec::new();
    for (encoded, plain) in [
        ("&#32;x&#32;", " x "),
        ("&#9;x&#10;&#13;", "\tx\n\r"),
        ("&#32;&#9;&#10;&#13;", " \t\n\r"),
        ("", ""),
        (" x ", " x "),
        (" \t\n ", " \t\n "),
        ("&#x9;&#xA;&#xD;x&#9;&#10;&#13;", "\t\n\rx\t\n\r"),
        (" \t&#32;x&#x20;\n ", " \t x \n "),
        ("&#32;x&amp;y&#32;", " x&y "),
    ] {
        cases.push((
            document(encoded, ""),
            Literal::new_simple_literal(plain).into(),
        ));
        cases.push((
            document(encoded, " xml:lang=\"en\""),
            Literal::new_language_tagged_literal(plain, "en")
                .unwrap()
                .into(),
        ));
        cases.push((
            document(encoded, " datatype=\"http://example.com/type\""),
            Literal::new_typed_literal(plain, NamedNode::new_unchecked("http://example.com/type"))
                .into(),
        ));
    }
    let formatted: Vec<_> = cases
        .iter()
        .map(|(xml, term)| {
            (
                xml.replace("<result>", "\n  <result>\n    ")
                    .replace("<literal", "\n      <literal")
                    .replace("</literal>", "</literal>\n    "),
                term.clone(),
            )
        })
        .collect();
    cases.extend(formatted);
    cases.push((
        document("", "").replace("<literal></literal>", "<literal/>"),
        Literal::new_simple_literal("").into(),
    ));
    cases
}

#[test]
fn xml_literal_boundary_whitespace_reader_and_slice() {
    for (xml, expected) in cases() {
        assert_sync(xml.as_bytes(), &expected);
    }
}

#[test]
fn xml_literal_boundary_whitespace_serializer_roundtrip() {
    let variable = Variable::new_unchecked("x");
    for (_, expected) in cases() {
        let mut writer = QueryResultsSerializer::from_format(QueryResultsFormat::Xml)
            .serialize_solutions_to_writer(Vec::new(), vec![variable.clone()])
            .unwrap();
        writer
            .serialize([(variable.as_ref(), expected.as_ref())])
            .unwrap();
        let xml = writer.finish().unwrap();
        assert_sync(&xml, &expected);
    }
}

#[test]
fn xml_literal_cdata_remains_rejected() {
    let xml = document("&#32;<![CDATA[x]]>&#32;", "");
    let parser = QueryResultsParser::from_format(QueryResultsFormat::Xml);
    let ReaderQueryResultsParserOutput::Solutions(mut rows) =
        parser.clone().for_reader(xml.as_bytes()).unwrap()
    else {
        panic!("expected solutions");
    };
    assert!(rows.next().unwrap().is_err());
    let SliceQueryResultsParserOutput::Solutions(mut rows) =
        parser.for_slice(xml.as_bytes()).unwrap()
    else {
        panic!("expected solutions");
    };
    assert!(rows.next().unwrap().is_err());
}

#[cfg(feature = "async-tokio")]
#[tokio::test]
async fn xml_literal_boundary_whitespace_async() {
    use crate::TokioAsyncReaderQueryResultsParserOutput;
    for (xml, expected) in cases() {
        let TokioAsyncReaderQueryResultsParserOutput::Solutions(mut rows) =
            QueryResultsParser::from_format(QueryResultsFormat::Xml)
                .for_tokio_async_reader(xml.as_bytes())
                .await
                .unwrap()
        else {
            panic!("expected solutions");
        };
        assert_eq!(
            rows.next().await.unwrap().unwrap().get("x"),
            Some(&expected)
        );
        assert!(rows.next().await.is_none());
        let variable = Variable::new_unchecked("x");
        let mut writer = QueryResultsSerializer::from_format(QueryResultsFormat::Xml)
            .serialize_solutions_to_tokio_async_write(Vec::new(), vec![variable.clone()])
            .await
            .unwrap();
        writer
            .serialize([(variable.as_ref(), expected.as_ref())])
            .await
            .unwrap();
        let xml = writer.finish().await.unwrap();
        assert_sync(&xml, &expected);
        let TokioAsyncReaderQueryResultsParserOutput::Solutions(mut rows) =
            QueryResultsParser::from_format(QueryResultsFormat::Xml)
                .for_tokio_async_reader(xml.as_slice())
                .await
                .unwrap()
        else {
            panic!("expected solutions");
        };
        assert_eq!(
            rows.next().await.unwrap().unwrap().get("x"),
            Some(&expected)
        );
        assert!(rows.next().await.is_none());
    }
    let xml = document("&#32;<![CDATA[x]]>&#32;", "");
    let TokioAsyncReaderQueryResultsParserOutput::Solutions(mut rows) =
        QueryResultsParser::from_format(QueryResultsFormat::Xml)
            .for_tokio_async_reader(xml.as_bytes())
            .await
            .unwrap()
    else {
        panic!("expected solutions");
    };
    assert!(rows.next().await.unwrap().is_err());
}
