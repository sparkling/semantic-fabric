use crate::csv::ReaderTsvSolutionsParser;
use crate::error::QueryResultsParseError;
use crate::json::ReaderJsonSolutionsParser;
use crate::solution::QuerySolution;
use crate::xml::ReaderXmlSolutionsParser;
use oxrdf::Variable;
use std::io::Read;
use std::sync::Arc;

/// The reader for a given read of a results file.
///
/// It is either a read boolean ([`bool`]) or a streaming reader of a set of solutions ([`ReaderSolutionsParser`]).
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
///     tsv_parser.clone().for_reader("true".as_bytes())?
/// {
///     assert_eq!(v, true);
/// }
///
/// // solutions
/// if let ReaderQueryResultsParserOutput::Solutions(solutions) =
///     tsv_parser.for_reader("?foo\t?bar\n\"test\"\t".as_bytes())?
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
pub enum ReaderQueryResultsParserOutput<R: Read> {
    Solutions(ReaderSolutionsParser<R>),
    Boolean(bool),
}

/// A streaming parser of a set of [`QuerySolution`] solutions.
///
/// It implements the [`Iterator`] API to iterate over the solutions.
///
/// Example in JSON (the API is the same for XML and TSV):
/// ```
/// use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
/// use oxrdf::{Literal, Variable};
///
/// let json_parser = QueryResultsParser::from_format(QueryResultsFormat::Json);
/// if let ReaderQueryResultsParserOutput::Solutions(solutions) = json_parser.for_reader(br#"{"head":{"vars":["foo","bar"]},"results":{"bindings":[{"foo":{"type":"literal","value":"test"}}]}}"#.as_slice())? {
///     assert_eq!(solutions.variables(), &[Variable::new("foo")?, Variable::new("bar")?]);
///     for solution in solutions {
///         assert_eq!(solution?.iter().collect::<Vec<_>>(), vec![(&Variable::new("foo")?, &Literal::from("test").into())]);
///     }
/// }
/// # Result::<_, Box<dyn std::error::Error>>::Ok(())
/// ```
pub struct ReaderSolutionsParser<R: Read> {
    pub(super) variables: Arc<[Variable]>,
    pub(super) solutions: ReaderSolutionsParserKind<R>,
}

pub(super) enum ReaderSolutionsParserKind<R: Read> {
    Xml(ReaderXmlSolutionsParser<R>),
    Json(ReaderJsonSolutionsParser<R>),
    Tsv(ReaderTsvSolutionsParser<R>),
}

impl<R: Read> ReaderSolutionsParser<R> {
    /// Ordered list of the declared variables at the beginning of the results.
    ///
    /// Example in TSV (the API is the same for JSON and XML):
    /// ```
    /// use oxrdf::Variable;
    /// use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
    ///
    /// let tsv_parser = QueryResultsParser::from_format(QueryResultsFormat::Tsv);
    /// if let ReaderQueryResultsParserOutput::Solutions(solutions) =
    ///     tsv_parser.for_reader(b"?foo\t?bar\n\"ex1\"\t\"ex2\"".as_slice())?
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

impl<R: Read> Iterator for ReaderSolutionsParser<R> {
    type Item = Result<QuerySolution, QueryResultsParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(
            match &mut self.solutions {
                ReaderSolutionsParserKind::Xml(reader) => reader.parse_next(),
                ReaderSolutionsParserKind::Json(reader) => reader.parse_next(),
                ReaderSolutionsParserKind::Tsv(reader) => reader.parse_next(),
            }
            .transpose()?
            .map(|values| (Arc::clone(&self.variables), values).into()),
        )
    }
}
