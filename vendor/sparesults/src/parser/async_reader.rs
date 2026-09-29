use crate::csv::TokioAsyncReaderTsvSolutionsParser;
use crate::error::QueryResultsParseError;
use crate::json::TokioAsyncReaderJsonSolutionsParser;
use crate::solution::QuerySolution;
use crate::xml::TokioAsyncReaderXmlSolutionsParser;
use oxrdf::Variable;
use std::sync::Arc;
use tokio::io::AsyncRead;

#[cfg(doc)]
use super::ReaderSolutionsParser;

/// The reader for a given read of a results file.
///
/// It is either a read boolean ([`bool`]) or a streaming reader of a set of solutions ([`ReaderSolutionsParser`]).
///
/// Example in TSV (the API is the same for JSON and XML):
/// ```
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use oxrdf::{Literal, Variable};
/// use sparesults::{
///     QueryResultsFormat, QueryResultsParser, TokioAsyncReaderQueryResultsParserOutput,
/// };
///
/// let tsv_parser = QueryResultsParser::from_format(QueryResultsFormat::Tsv);
///
/// // boolean
/// if let TokioAsyncReaderQueryResultsParserOutput::Boolean(v) = tsv_parser
///     .clone()
///     .for_tokio_async_reader(b"true".as_slice())
///     .await?
/// {
///     assert_eq!(v, true);
/// }
///
/// // solutions
/// if let TokioAsyncReaderQueryResultsParserOutput::Solutions(mut solutions) = tsv_parser
///     .for_tokio_async_reader(b"?foo\t?bar\n\"test\"\t".as_slice())
///     .await?
/// {
///     assert_eq!(
///         solutions.variables(),
///         &[Variable::new("foo")?, Variable::new("bar")?]
///     );
///     while let Some(solution) = solutions.next().await {
///         assert_eq!(
///             solution?.iter().collect::<Vec<_>>(),
///             vec![(&Variable::new("foo")?, &Literal::from("test").into())]
///         );
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub enum TokioAsyncReaderQueryResultsParserOutput<R: AsyncRead + Unpin> {
    Solutions(TokioAsyncReaderSolutionsParser<R>),
    Boolean(bool),
}

/// A streaming parser of a set of [`QuerySolution`] solutions.
///
/// It implements the [`Iterator`] API to iterate over the solutions.
///
/// Example in JSON (the API is the same for XML and TSV):
/// ```
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use sparesults::{QueryResultsFormat, QueryResultsParser, TokioAsyncReaderQueryResultsParserOutput};
/// use oxrdf::{Literal, Variable};
///
/// let json_parser = QueryResultsParser::from_format(QueryResultsFormat::Json);
/// if let TokioAsyncReaderQueryResultsParserOutput::Solutions(mut solutions) = json_parser.for_tokio_async_reader(br#"{"head":{"vars":["foo","bar"]},"results":{"bindings":[{"foo":{"type":"literal","value":"test"}}]}}"#.as_slice()).await? {
///     assert_eq!(solutions.variables(), &[Variable::new("foo")?, Variable::new("bar")?]);
///     while let Some(solution) = solutions.next().await {
///         assert_eq!(solution?.iter().collect::<Vec<_>>(), vec![(&Variable::new("foo")?, &Literal::from("test").into())]);
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub struct TokioAsyncReaderSolutionsParser<R: AsyncRead + Unpin> {
    pub(super) variables: Arc<[Variable]>,
    pub(super) solutions: TokioAsyncReaderSolutionsParserKind<R>,
}

pub(super) enum TokioAsyncReaderSolutionsParserKind<R: AsyncRead + Unpin> {
    Json(TokioAsyncReaderJsonSolutionsParser<R>),
    Xml(TokioAsyncReaderXmlSolutionsParser<R>),
    Tsv(TokioAsyncReaderTsvSolutionsParser<R>),
}

impl<R: AsyncRead + Unpin> TokioAsyncReaderSolutionsParser<R> {
    /// Ordered list of the declared variables at the beginning of the results.
    ///
    /// Example in TSV (the API is the same for JSON and XML):
    /// ```
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use oxrdf::Variable;
    /// use sparesults::{
    ///     QueryResultsFormat, QueryResultsParser, TokioAsyncReaderQueryResultsParserOutput,
    /// };
    ///
    /// let tsv_parser = QueryResultsParser::from_format(QueryResultsFormat::Tsv);
    /// if let TokioAsyncReaderQueryResultsParserOutput::Solutions(solutions) = tsv_parser
    ///     .for_tokio_async_reader(b"?foo\t?bar\n\"ex1\"\t\"ex2\"".as_slice())
    ///     .await?
    /// {
    ///     assert_eq!(
    ///         solutions.variables(),
    ///         &[Variable::new("foo")?, Variable::new("bar")?]
    ///     );
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    /// Reads the next solution or returns `None` if the file is finished.
    pub async fn next(&mut self) -> Option<Result<QuerySolution, QueryResultsParseError>> {
        Some(
            match &mut self.solutions {
                TokioAsyncReaderSolutionsParserKind::Json(reader) => reader.parse_next().await,
                TokioAsyncReaderSolutionsParserKind::Xml(reader) => reader.parse_next().await,
                TokioAsyncReaderSolutionsParserKind::Tsv(reader) => reader.parse_next().await,
            }
            .transpose()?
            .map(|values| (Arc::clone(&self.variables), values).into()),
        )
    }
}
