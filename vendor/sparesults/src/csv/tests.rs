use super::tsv::InnerTsvSolutionsSerializer;
use super::writer::InnerCsvSolutionsSerializer;
use super::{ReaderTsvQueryResultsParserOutput, SliceTsvQueryResultsParserOutput};
use oxrdf::vocab::xsd;
use oxrdf::*;
use std::error::Error;
use std::io;

fn build_example() -> (Vec<Variable>, Vec<Vec<Option<Term>>>) {
    (
        vec![
            Variable::new_unchecked("x"),
            Variable::new_unchecked("literal"),
        ],
        vec![
            vec![
                Some(NamedNode::new_unchecked("http://example/x").into()),
                Some(Literal::new_simple_literal("String").into()),
            ],
            vec![
                Some(NamedNode::new_unchecked("http://example/x").into()),
                Some(Literal::new_simple_literal("String-with-dquote\"").into()),
            ],
            vec![
                Some(BlankNode::new_unchecked("b0").into()),
                Some(Literal::new_simple_literal("Blank node").into()),
            ],
            vec![
                None,
                Some(Literal::new_simple_literal("Missing 'x'").into()),
            ],
            vec![None, None],
            vec![
                Some(NamedNode::new_unchecked("http://example/x").into()),
                None,
            ],
            vec![
                Some(BlankNode::new_unchecked("b1").into()),
                Some(
                    Literal::new_language_tagged_literal_unchecked("String-with-lang", "en").into(),
                ),
            ],
            vec![
                Some(BlankNode::new_unchecked("b1").into()),
                Some(Literal::new_typed_literal("123", xsd::INTEGER).into()),
            ],
            vec![
                None,
                Some(Literal::new_simple_literal("escape,\t\r\n").into()),
            ],
            #[cfg(feature = "sparql-12")]
            vec![
                None,
                Some(
                    Literal::new_directional_language_tagged_literal_unchecked(
                        "String-with-dir",
                        "en",
                        BaseDirection::Ltr,
                    )
                    .into(),
                ),
            ],
        ],
    )
}

#[test]
fn test_csv_serialization() {
    let (variables, solutions) = build_example();
    let mut buffer = String::new();
    let serializer = InnerCsvSolutionsSerializer::start(&mut buffer, variables.clone());
    for solution in solutions {
        serializer.write(
            &mut buffer,
            variables
                .iter()
                .zip(&solution)
                .filter_map(|(v, s)| s.as_ref().map(|s| (v.as_ref(), s.as_ref()))),
        );
    }
    #[cfg_attr(not(feature = "sparql-12"), expect(unused_mut))]
    let mut expected = "x,literal\r\nhttp://example/x,String\r\nhttp://example/x,\"String-with-dquote\"\"\"\r\n_:b0,Blank node\r\n,Missing 'x'\r\n,\r\nhttp://example/x,\r\n_:b1,String-with-lang\r\n_:b1,123\r\n,\"escape,\t\r\n\"\r\n".to_owned();
    #[cfg(feature = "sparql-12")]
    {
        expected.push_str(",String-with-dir\r\n")
    }
    assert_eq!(buffer, expected);
}

#[test]
fn test_tsv_roundtrip() -> Result<(), Box<dyn Error>> {
    let (variables, solutions) = build_example();

    // Write
    let mut buffer = String::new();
    let serializer = InnerTsvSolutionsSerializer::start(&mut buffer, variables.clone());
    for solution in &solutions {
        serializer.write(
            &mut buffer,
            variables
                .iter()
                .zip(solution)
                .filter_map(|(v, s)| s.as_ref().map(|s| (v.as_ref(), s.as_ref()))),
        );
    }
    #[cfg_attr(not(feature = "sparql-12"), expect(unused_mut))]
    let mut expected = "?x\t?literal\n<http://example/x>\t\"String\"\n<http://example/x>\t\"String-with-dquote\\\"\"\n_:b0\t\"Blank node\"\n\t\"Missing 'x'\"\n\t\n<http://example/x>\t\n_:b1\t\"String-with-lang\"@en\n_:b1\t123\n\t\"escape,\\t\\r\\n\"\n".to_owned();
    #[cfg(feature = "sparql-12")]
    {
        expected.push_str("\t\"String-with-dir\"@en--ltr\n")
    }
    assert_eq!(buffer, expected);

    // Read
    if let SliceTsvQueryResultsParserOutput::Solutions {
        solutions: mut solutions_iter,
        variables: actual_variables,
    } = SliceTsvQueryResultsParserOutput::read(buffer.as_bytes())?
    {
        assert_eq!(actual_variables.as_slice(), variables.as_slice());
        let mut rows = Vec::new();
        while let Some(row) = solutions_iter.parse_next()? {
            rows.push(row);
        }
        assert_eq!(rows, solutions);
    } else {
        unreachable!()
    }

    Ok(())
}

#[test]
fn test_bad_tsv() {
    let mut bad_tsvs = vec![
        "?",
        "?p",
        "?p?o",
        "?p\n<",
        "?p\n_",
        "?p\n_:",
        "?p\n\"",
        "?p\n<<",
        "?p\n<<(",
        "?p\n1\t2\n",
        "?p\n\n",
    ];
    let a_lot_of_strings = format!("?p\n{}\n", "<".repeat(100_000));
    bad_tsvs.push(&a_lot_of_strings);
    for bad_tsv in bad_tsvs {
        if let Ok(ReaderTsvQueryResultsParserOutput::Solutions { mut solutions, .. }) =
            ReaderTsvQueryResultsParserOutput::read(bad_tsv.as_bytes())
        {
            while let Ok(Some(_)) = solutions.parse_next() {}
        }
    }
}

#[test]
fn test_no_columns_csv_serialization() {
    let mut buffer = String::new();
    let serializer = InnerCsvSolutionsSerializer::start(&mut buffer, Vec::new());
    serializer.write(&mut buffer, []);
    assert_eq!(buffer, "\r\n\r\n");
}

#[test]
fn test_no_columns_tsv_serialization() {
    let mut buffer = String::new();
    let serializer = InnerTsvSolutionsSerializer::start(&mut buffer, Vec::new());
    serializer.write(&mut buffer, []);
    assert_eq!(buffer, "\n\n");
}

#[test]
fn test_no_columns_tsv_parsing() -> io::Result<()> {
    if let ReaderTsvQueryResultsParserOutput::Solutions {
        mut solutions,
        variables,
    } = ReaderTsvQueryResultsParserOutput::read(b"\n\n".as_slice())?
    {
        assert_eq!(variables, Vec::<Variable>::new());
        assert_eq!(solutions.parse_next()?, Some(Vec::new()));
        assert_eq!(solutions.parse_next()?, None);
    } else {
        unreachable!()
    }
    Ok(())
}

#[test]
fn test_no_results_csv_serialization() {
    let mut buffer = String::new();
    InnerCsvSolutionsSerializer::start(&mut buffer, vec![Variable::new_unchecked("a")]);
    assert_eq!(buffer, "a\r\n");
}

#[test]
fn test_no_results_tsv_serialization() {
    let mut buffer = String::new();
    InnerTsvSolutionsSerializer::start(&mut buffer, vec![Variable::new_unchecked("a")]);
    assert_eq!(buffer, "?a\n");
}

#[test]
fn test_no_results_tsv_parsing() -> io::Result<()> {
    if let ReaderTsvQueryResultsParserOutput::Solutions {
        mut solutions,
        variables,
    } = ReaderTsvQueryResultsParserOutput::read(b"?a\n".as_slice())?
    {
        assert_eq!(variables, vec![Variable::new_unchecked("a")]);
        assert_eq!(solutions.parse_next()?, None);
    } else {
        unreachable!()
    }
    Ok(())
}
