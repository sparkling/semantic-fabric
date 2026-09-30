use crate::csv::SliceTsvSolutionsParser;
use crate::error::QueryResultsSyntaxError;
use crate::json::SliceJsonSolutionsParser;
use crate::solution::QuerySolution;
use crate::xml::SliceXmlSolutionsParser;
use oxrdf::Variable;
use std::sync::Arc;

/// The reader for a given read of a results file.
///
/// It is either a read boolean ([`bool`]) or a streaming reader of a set of solutions ([`SliceSolutionsParser`]).
///
/// Example in TSV (the API is the same for JSON and XML):
/// ```
/// use oxrdf::{Literal, Variable};
/// use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
///
/// let tsv_parser = QueryResultsParser::from_format(QueryResultsFormat::Tsv);
///
/// // boolean
/// if let ReaderQueryResultsParserOutput::Boolean(v) =
///     tsv_parser.clone().for_reader(b"true".as_slice())?
/// {
///     assert_eq!(v, true);
/// }
///
/// // solutions
/// if let ReaderQueryResultsParserOutput::Solutions(solutions) =
///     tsv_parser.for_reader(b"?foo\t?bar\n\"test\"\t".as_slice())?
/// {
///     assert_eq!(
///         solutions.variables(),
///         &[Variable::new("foo")?, Variable::new("bar")?]
///     );
///     for solution in solutions {
///         assert_eq!(
///             solution?.iter().collect::<Vec<_>>(),
///             vec![(&Variable::new("foo")?, &Literal::from("test").into())]
///         );
///     }
/// }
/// # Result::<_, Box<dyn std::error::Error>>::Ok(())
/// ```
pub enum SliceQueryResultsParserOutput<'a> {
    Solutions(SliceSolutionsParser<'a>),
    Boolean(bool),
}

/// A streaming parser of a set of [`QuerySolution`] solutions.
///
/// It implements the [`Iterator`] API to iterate over the solutions.
///
/// Example in JSON (the API is the same for XML and TSV):
/// ```
/// use sparesults::{QueryResultsFormat, QueryResultsParser, SliceQueryResultsParserOutput};
/// use oxrdf::{Literal, Variable};
///
/// let json_parser = QueryResultsParser::from_format(QueryResultsFormat::Json);
/// if let SliceQueryResultsParserOutput::Solutions(solutions) = json_parser.for_slice(r#"{"head":{"vars":["foo","bar"]},"results":{"bindings":[{"foo":{"type":"literal","value":"test"}}]}}"#)? {
///     assert_eq!(solutions.variables(), &[Variable::new("foo")?, Variable::new("bar")?]);
///     for solution in solutions {
///         assert_eq!(solution?.iter().collect::<Vec<_>>(), vec![(&Variable::new("foo")?, &Literal::from("test").into())]);
///     }
/// }
/// # Result::<_, Box<dyn std::error::Error>>::Ok(())
/// ```
pub struct SliceSolutionsParser<'a> {
    pub(super) variables: Arc<[Variable]>,
    pub(super) solutions: SliceSolutionsParserKind<'a>,
}

pub(super) enum SliceSolutionsParserKind<'a> {
    Xml(SliceXmlSolutionsParser<'a>),
    Json(SliceJsonSolutionsParser<'a>),
    Tsv(SliceTsvSolutionsParser<'a>),
}

impl SliceSolutionsParser<'_> {
    /// Ordered list of the declared variables at the beginning of the results.
    ///
    /// Example in TSV (the API is the same for JSON and XML):
    /// ```
    /// use oxrdf::Variable;
    /// use sparesults::{QueryResultsFormat, QueryResultsParser, SliceQueryResultsParserOutput};
    ///
    /// let tsv_parser = QueryResultsParser::from_format(QueryResultsFormat::Tsv);
    /// if let SliceQueryResultsParserOutput::Solutions(solutions) =
    ///     tsv_parser.for_slice("?foo\t?bar\n\"ex1\"\t\"ex2\"")?
    /// {
    ///     assert_eq!(
    ///         solutions.variables(),
    ///         &[Variable::new("foo")?, Variable::new("bar")?]
    ///     );
    /// }
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    #[inline]
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }
}

impl Iterator for SliceSolutionsParser<'_> {
    type Item = Result<QuerySolution, QueryResultsSyntaxError>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(
            match &mut self.solutions {
                SliceSolutionsParserKind::Xml(reader) => reader.parse_next(),
                SliceSolutionsParserKind::Json(reader) => reader.parse_next(),
                SliceSolutionsParserKind::Tsv(reader) => reader.parse_next(),
            }
            .transpose()?
            .map(|values| (Arc::clone(&self.variables), values).into()),
        )
    }
}
