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

fn raw_newline_cases() -> [(&'static str, &'static str, &'static str); 5] {
    [
        ("x\ry", "x\ny", "x\ny"),
        ("x\r\ny", "x\ny", "x\ny"),
        ("x\r\ry\r\nz", "x\n\ny\nz", "x\n\ny\nz"),
        ("\r\nx\ry\r\n", "\nx\ny\n", "&#10;x\ny&#10;"),
        ("x\r&#13;y\r\n&#13;z", "x\n\ry\n\rz", "x\n&#13;y\n&#13;z"),
    ]
}

fn cases() -> Vec<(String, Term, &'static str)> {
    let mut cases: Vec<(String, Term, &'static str)> = Vec::new();
    // Input XML, exact lexical value, and exact serialized body are independent fixtures.
    for (encoded, plain, serialized) in [
        ("&#32;x&#32;", " x ", "&#32;x&#32;"),
        ("&#9;x&#10;&#13;", "\tx\n\r", "&#9;x&#10;&#13;"),
        ("&#32;&#9;&#10;&#13;", " \t\n\r", "&#32;&#9;&#10;&#13;"),
        ("", "", ""),
        (" x ", " x ", "&#32;x&#32;"),
        (" \t\n ", " \t\n ", "&#32;&#9;&#10;&#32;"),
        (
            "&#x9;&#xA;&#xD;x&#9;&#10;&#13;",
            "\t\n\rx\t\n\r",
            "&#9;&#10;&#13;x&#9;&#10;&#13;",
        ),
        (
            " \t&#32;x&#x20;\n ",
            " \t x \n ",
            "&#32;&#9;&#32;x&#32;&#10;&#32;",
        ),
        ("&#32;x&amp;y&#32;", " x&y ", "&#32;x&amp;y&#32;"),
        ("x&#13;y", "x\ry", "x&#13;y"),
        ("x&#xD;y", "x\ry", "x&#13;y"),
        ("x&#13;\ny", "x\r\ny", "x&#13;\ny"),
        ("x&#13;&#13;y&#13;z", "x\r\ry\rz", "x&#13;&#13;y&#13;z"),
        ("x&#13;&#13;\ny", "x\r\r\ny", "x&#13;&#13;\ny"),
        (
            "x&amp;&#13;&lt;y&gt;&quot;&apos;&amp;#13;&amp;amp;",
            "x&\r<y>\"'&#13;&amp;",
            "x&amp;&#13;&lt;y&gt;&quot;&apos;&amp;#13;&amp;amp;",
        ),
        (
            "\u{e9}&#13;\u{6c34}\u{1f642}",
            "\u{e9}\r\u{6c34}\u{1f642}",
            "\u{e9}&#13;\u{6c34}\u{1f642}",
        ),
        (
            "&#32;&#9;&#13;x&#13;\ny&#13;&#10;&#32;",
            " \t\rx\r\ny\r\n ",
            "&#32;&#9;&#13;x&#13;\ny&#13;&#10;&#32;",
        ),
        (
            "&#13;\u{e9}&amp;&#13;&lt;\u{6c34}&gt;&#9;",
            "\r\u{e9}&\r<\u{6c34}>\t",
            "&#13;\u{e9}&amp;&#13;&lt;\u{6c34}&gt;&#9;",
        ),
    ]
    .into_iter()
    .chain(raw_newline_cases())
    {
        cases.push((
            document(encoded, ""),
            Literal::new_simple_literal(plain).into(),
            serialized,
        ));
        cases.push((
            document(encoded, " xml:lang=\"en\""),
            Literal::new_language_tagged_literal(plain, "en")
                .unwrap()
                .into(),
            serialized,
        ));
        cases.push((
            document(encoded, " datatype=\"http://example.com/type\""),
            Literal::new_typed_literal(plain, NamedNode::new_unchecked("http://example.com/type"))
                .into(),
            serialized,
        ));
    }
    let formatted: Vec<_> = cases
        .iter()
        .map(|(xml, term, serialized)| {
            (
                xml.replace("<result>", "\n  <result>\n    ")
                    .replace("<literal", "\n      <literal")
                    .replace("</literal>", "</literal>\n    "),
                term.clone(),
                *serialized,
            )
        })
        .collect();
    cases.extend(formatted);
    cases.push((
        document("", "").replace("<literal></literal>", "<literal/>"),
        Literal::new_simple_literal("").into(),
        "",
    ));
    cases
}

fn serialize_sync(expected: &Term) -> Vec<u8> {
    let variable = Variable::new_unchecked("x");
    let mut writer = QueryResultsSerializer::from_format(QueryResultsFormat::Xml)
        .serialize_solutions_to_writer(Vec::new(), vec![variable.clone()])
        .unwrap();
    writer
        .serialize([(variable.as_ref(), expected.as_ref())])
        .unwrap();
    writer.finish().unwrap()
}

fn assert_serialized(xml: &[u8], expected_content: &str) {
    assert!(!xml.contains(&b'\r'));
    let xml = std::str::from_utf8(xml).unwrap();
    let (_, literal) = xml.split_once("<literal").unwrap();
    let (_, content) = literal.split_once('>').unwrap();
    let (content, _) = content.split_once("</literal>").unwrap();
    assert_eq!(content, expected_content);
}

#[test]
fn xml_literal_boundary_whitespace_reader_and_slice() {
    for (xml, expected, _) in cases() {
        assert_sync(xml.as_bytes(), &expected);
    }
}

#[test]
fn xml_literal_raw_newlines_normalize_reader_and_slice() {
    for (encoded, plain, _) in raw_newline_cases() {
        let expected = Literal::new_simple_literal(plain).into();
        assert_sync(document(encoded, "").as_bytes(), &expected);
    }
}

#[test]
fn xml_literal_boundary_whitespace_serializer_roundtrip() {
    for (_, expected, serialized) in cases() {
        let xml = serialize_sync(&expected);
        assert_serialized(&xml, serialized);
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
    for (xml, expected, serialized) in cases() {
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
        assert_serialized(&xml, serialized);
        assert_eq!(xml, serialize_sync(&expected));
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
