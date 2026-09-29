#![allow(clippy::large_enum_variant)]

#[cfg(feature = "async-tokio")]
mod async_reader;
mod reader;
mod slice;

use crate::csv::{ReaderTsvQueryResultsParserOutput, SliceTsvQueryResultsParserOutput};
use crate::error::{QueryResultsParseError, QueryResultsSyntaxError};
use crate::format::QueryResultsFormat;
use crate::json::{ReaderJsonQueryResultsParserOutput, SliceJsonQueryResultsParserOutput};
use crate::xml::{ReaderXmlQueryResultsParserOutput, SliceXmlQueryResultsParserOutput};
use std::io::Read;

#[cfg(feature = "async-tokio")]
use crate::csv::TokioAsyncReaderTsvQueryResultsParserOutput;
#[cfg(feature = "async-tokio")]
use crate::json::TokioAsyncReaderJsonQueryResultsParserOutput;
#[cfg(feature = "async-tokio")]
use crate::xml::TokioAsyncReaderXmlQueryResultsParserOutput;
#[cfg(feature = "async-tokio")]
use tokio::io::AsyncRead;

#[cfg(feature = "async-tokio")]
use self::async_reader::TokioAsyncReaderSolutionsParserKind;

#[cfg(feature = "async-tokio")]
pub use self::async_reader::{
    TokioAsyncReaderQueryResultsParserOutput, TokioAsyncReaderSolutionsParser,
};

use self::reader::ReaderSolutionsParserKind;

pub use self::reader::{ReaderQueryResultsParserOutput, ReaderSolutionsParser};

use self::slice::SliceSolutionsParserKind;

pub use self::slice::{SliceQueryResultsParserOutput, SliceSolutionsParser};

/// Parsers for [SPARQL query](https://www.w3.org/TR/sparql11-query/) results serialization formats.
///
/// It currently supports the following formats:
/// * [SPARQL Query Results XML Format](https://www.w3.org/TR/rdf-sparql-XMLres/) ([`QueryResultsFormat::Xml`](QueryResultsFormat::Xml)).
/// * [SPARQL Query Results JSON Format](https://www.w3.org/TR/sparql11-results-json/) ([`QueryResultsFormat::Json`](QueryResultsFormat::Json)).
/// * [SPARQL Query Results TSV Format](https://www.w3.org/TR/sparql11-results-csv-tsv/) ([`QueryResultsFormat::Tsv`](QueryResultsFormat::Tsv)).
///
/// Example in JSON (the API is the same for XML and TSV):
/// ```
/// use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
/// use oxrdf::{Literal, Variable};
///
/// let json_parser = QueryResultsParser::from_format(QueryResultsFormat::Json);
/// // boolean
/// if let ReaderQueryResultsParserOutput::Boolean(v) = json_parser.clone().for_reader(br#"{"boolean":true}"#.as_slice())? {
///     assert_eq!(v, true);
/// }
/// // solutions
/// if let ReaderQueryResultsParserOutput::Solutions(solutions) = json_parser.for_reader(br#"{"head":{"vars":["foo","bar"]},"results":{"bindings":[{"foo":{"type":"literal","value":"test"}}]}}"#.as_slice())? {
///     assert_eq!(solutions.variables(), &[Variable::new("foo")?, Variable::new("bar")?]);
///     for solution in solutions {
///         assert_eq!(solution?.iter().collect::<Vec<_>>(), vec![(&Variable::new("foo")?, &Literal::from("test").into())]);
///     }
/// }
/// # Result::<_, Box<dyn std::error::Error>>::Ok(())
/// ```
#[must_use]
#[derive(Clone)]
pub struct QueryResultsParser {
    format: QueryResultsFormat,
}

impl QueryResultsParser {
    /// Builds a parser for the given format.
    #[inline]
    pub fn from_format(format: QueryResultsFormat) -> Self {
        Self { format }
    }

    /// Reads a result file from a [`Read`] implementation.
    ///
    /// Reads are automatically buffered.
    ///
    /// Example in XML (the API is the same for JSON and TSV):
    /// ```
    /// use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
    /// use oxrdf::{Literal, Variable};
    ///
    /// let xml_parser = QueryResultsParser::from_format(QueryResultsFormat::Xml);
    ///
    /// // boolean
    /// if let ReaderQueryResultsParserOutput::Boolean(v) = xml_parser.clone().for_reader(br#"<sparql xmlns="http://www.w3.org/2005/sparql-results#"><head/><boolean>true</boolean></sparql>"#.as_slice())? {
    ///     assert_eq!(v, true);
    /// }
    ///
    /// // solutions
    /// if let ReaderQueryResultsParserOutput::Solutions(solutions) = xml_parser.for_reader(br#"<sparql xmlns="http://www.w3.org/2005/sparql-results#"><head><variable name="foo"/><variable name="bar"/></head><results><result><binding name="foo"><literal>test</literal></binding></result></results></sparql>"#.as_slice())? {
    ///     assert_eq!(solutions.variables(), &[Variable::new("foo")?, Variable::new("bar")?]);
    ///     for solution in solutions {
    ///         assert_eq!(solution?.iter().collect::<Vec<_>>(), vec![(&Variable::new("foo")?, &Literal::from("test").into())]);
    ///     }
    /// }
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    pub fn for_reader<R: Read>(
        self,
        reader: R,
    ) -> Result<ReaderQueryResultsParserOutput<R>, QueryResultsParseError> {
        Ok(match self.format {
            QueryResultsFormat::Xml => match ReaderXmlQueryResultsParserOutput::read(reader)? {
                ReaderXmlQueryResultsParserOutput::Boolean(r) => ReaderQueryResultsParserOutput::Boolean(r),
                ReaderXmlQueryResultsParserOutput::Solutions {
                    solutions,
                    variables,
                } => ReaderQueryResultsParserOutput::Solutions(ReaderSolutionsParser {
                    variables: variables.into(),
                    solutions: ReaderSolutionsParserKind::Xml(solutions),
                }),
            },
            QueryResultsFormat::Json => match ReaderJsonQueryResultsParserOutput::read(reader)? {
                ReaderJsonQueryResultsParserOutput::Boolean(r) => ReaderQueryResultsParserOutput::Boolean(r),
                ReaderJsonQueryResultsParserOutput::Solutions {
                    solutions,
                    variables,
                } => ReaderQueryResultsParserOutput::Solutions(ReaderSolutionsParser {
                    variables: variables.into(),
                    solutions: ReaderSolutionsParserKind::Json(solutions),
                }),
            },
            QueryResultsFormat::Csv => return Err(QueryResultsSyntaxError::msg("CSV SPARQL results syntax is lossy and can't be parsed to a proper RDF representation").into()),
            QueryResultsFormat::Tsv => match ReaderTsvQueryResultsParserOutput::read(reader)? {
                ReaderTsvQueryResultsParserOutput::Boolean(r) => ReaderQueryResultsParserOutput::Boolean(r),
                ReaderTsvQueryResultsParserOutput::Solutions {
                    solutions,
                    variables,
                } => ReaderQueryResultsParserOutput::Solutions(ReaderSolutionsParser {
                    variables: variables.into(),
                    solutions: ReaderSolutionsParserKind::Tsv(solutions),
                }),
            },
        })
    }

    /// Reads a result file from a Tokio [`AsyncRead`] implementation.
    ///
    /// Reads are automatically buffered.
    ///
    /// Example in XML (the API is the same for JSON and TSV):
    /// ```
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use sparesults::{QueryResultsFormat, QueryResultsParser, TokioAsyncReaderQueryResultsParserOutput};
    /// use oxrdf::{Literal, Variable};
    ///
    /// let xml_parser = QueryResultsParser::from_format(QueryResultsFormat::Xml);
    ///
    /// // boolean
    /// if let TokioAsyncReaderQueryResultsParserOutput::Boolean(v) = xml_parser.clone().for_tokio_async_reader(br#"<sparql xmlns="http://www.w3.org/2005/sparql-results#"><head/><boolean>true</boolean></sparql>"#.as_slice()).await? {
    ///     assert_eq!(v, true);
    /// }
    ///
    /// // solutions
    /// if let TokioAsyncReaderQueryResultsParserOutput::Solutions(mut solutions) = xml_parser.for_tokio_async_reader(br#"<sparql xmlns="http://www.w3.org/2005/sparql-results#"><head><variable name="foo"/><variable name="bar"/></head><results><result><binding name="foo"><literal>test</literal></binding></result></results></sparql>"#.as_slice()).await? {
    ///     assert_eq!(solutions.variables(), &[Variable::new("foo")?, Variable::new("bar")?]);
    ///     while let Some(solution) = solutions.next().await {
    ///         assert_eq!(solution?.iter().collect::<Vec<_>>(), vec![(&Variable::new("foo")?, &Literal::from("test").into())]);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "async-tokio")]
    pub async fn for_tokio_async_reader<R: AsyncRead + Unpin>(
        self,
        reader: R,
    ) -> Result<TokioAsyncReaderQueryResultsParserOutput<R>, QueryResultsParseError> {
        Ok(match self.format {
            QueryResultsFormat::Xml => match TokioAsyncReaderXmlQueryResultsParserOutput::read(reader).await? {
                TokioAsyncReaderXmlQueryResultsParserOutput::Boolean(r) => TokioAsyncReaderQueryResultsParserOutput::Boolean(r),
                TokioAsyncReaderXmlQueryResultsParserOutput::Solutions {
                    solutions,
                    variables,
                } => TokioAsyncReaderQueryResultsParserOutput::Solutions(TokioAsyncReaderSolutionsParser {
                    variables: variables.into(),
                    solutions: TokioAsyncReaderSolutionsParserKind::Xml(solutions),
                }),
            },
            QueryResultsFormat::Json => match TokioAsyncReaderJsonQueryResultsParserOutput::read(reader).await? {
                TokioAsyncReaderJsonQueryResultsParserOutput::Boolean(r) => TokioAsyncReaderQueryResultsParserOutput::Boolean(r),
                TokioAsyncReaderJsonQueryResultsParserOutput::Solutions {
                    solutions,
                    variables,
                } => TokioAsyncReaderQueryResultsParserOutput::Solutions(TokioAsyncReaderSolutionsParser {
                    variables: variables.into(),
                    solutions: TokioAsyncReaderSolutionsParserKind::Json(solutions),
                }),
            },
            QueryResultsFormat::Csv => return Err(QueryResultsSyntaxError::msg("CSV SPARQL results syntax is lossy and can't be parsed to a proper RDF representation").into()),
            QueryResultsFormat::Tsv => match TokioAsyncReaderTsvQueryResultsParserOutput::read(reader).await? {
                TokioAsyncReaderTsvQueryResultsParserOutput::Boolean(r) => TokioAsyncReaderQueryResultsParserOutput::Boolean(r),
                TokioAsyncReaderTsvQueryResultsParserOutput::Solutions {
                    solutions,
                    variables,
                } => TokioAsyncReaderQueryResultsParserOutput::Solutions(TokioAsyncReaderSolutionsParser {
                    variables: variables.into(),
                    solutions: TokioAsyncReaderSolutionsParserKind::Tsv(solutions),
                }),
            },
        })
    }

    /// Reads a result file from a [`Read`] implementation.
    ///
    /// Reads are automatically buffered.
    ///
    /// Example in XML (the API is the same for JSON and TSV):
    /// ```
    /// use sparesults::{QueryResultsFormat, QueryResultsParser, SliceQueryResultsParserOutput};
    /// use oxrdf::{Literal, Variable};
    ///
    /// let xml_parser = QueryResultsParser::from_format(QueryResultsFormat::Xml);
    ///
    /// // boolean
    /// if let SliceQueryResultsParserOutput::Boolean(v) = xml_parser.clone().for_slice(r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#"><head/><boolean>true</boolean></sparql>"#)? {
    ///     assert_eq!(v, true);
    /// }
    ///
    /// // solutions
    /// if let SliceQueryResultsParserOutput::Solutions(solutions) = xml_parser.for_slice(r#"<sparql xmlns="http://www.w3.org/2005/sparql-results#"><head><variable name="foo"/><variable name="bar"/></head><results><result><binding name="foo"><literal>test</literal></binding></result></results></sparql>"#)? {
    ///     assert_eq!(solutions.variables(), &[Variable::new("foo")?, Variable::new("bar")?]);
    ///     for solution in solutions {
    ///         assert_eq!(solution?.iter().collect::<Vec<_>>(), vec![(&Variable::new("foo")?, &Literal::from("test").into())]);
    ///     }
    /// }
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    pub fn for_slice(
        self,
        slice: &(impl AsRef<[u8]> + ?Sized),
    ) -> Result<SliceQueryResultsParserOutput<'_>, QueryResultsSyntaxError> {
        Ok(match self.format {
            QueryResultsFormat::Xml => {
                match SliceXmlQueryResultsParserOutput::read(slice.as_ref())? {
                    SliceXmlQueryResultsParserOutput::Boolean(r) => {
                        SliceQueryResultsParserOutput::Boolean(r)
                    }
                    SliceXmlQueryResultsParserOutput::Solutions {
                        solutions,
                        variables,
                    } => SliceQueryResultsParserOutput::Solutions(SliceSolutionsParser {
                        variables: variables.into(),
                        solutions: SliceSolutionsParserKind::Xml(solutions),
                    }),
                }
            }
            QueryResultsFormat::Json => {
                match SliceJsonQueryResultsParserOutput::read(slice.as_ref())? {
                    SliceJsonQueryResultsParserOutput::Boolean(r) => {
                        SliceQueryResultsParserOutput::Boolean(r)
                    }
                    SliceJsonQueryResultsParserOutput::Solutions {
                        solutions,
                        variables,
                    } => SliceQueryResultsParserOutput::Solutions(SliceSolutionsParser {
                        variables: variables.into(),
                        solutions: SliceSolutionsParserKind::Json(solutions),
                    }),
                }
            }
            QueryResultsFormat::Csv => {
                return Err(QueryResultsSyntaxError::msg(
                    "CSV SPARQL results syntax is lossy and can't be parsed to a proper RDF representation",
                ));
            }
            QueryResultsFormat::Tsv => {
                match SliceTsvQueryResultsParserOutput::read(slice.as_ref())? {
                    SliceTsvQueryResultsParserOutput::Boolean(r) => {
                        SliceQueryResultsParserOutput::Boolean(r)
                    }
                    SliceTsvQueryResultsParserOutput::Solutions {
                        solutions,
                        variables,
                    } => SliceQueryResultsParserOutput::Solutions(SliceSolutionsParser {
                        variables: variables.into(),
                        solutions: SliceSolutionsParserKind::Tsv(solutions),
                    }),
                }
            }
        })
    }
}

impl From<QueryResultsFormat> for QueryResultsParser {
    fn from(format: QueryResultsFormat) -> Self {
        Self::from_format(format)
    }
}
