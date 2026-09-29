//! Implementation of [SPARQL 1.1 Query Results CSV and TSV Formats](https://www.w3.org/TR/sparql11-results-csv-tsv/)

mod reader;
mod tsv;
mod writer;

pub use reader::{
    ReaderTsvQueryResultsParserOutput, ReaderTsvSolutionsParser, SliceTsvQueryResultsParserOutput,
    SliceTsvSolutionsParser,
};
#[cfg(feature = "async-tokio")]
pub use reader::{TokioAsyncReaderTsvQueryResultsParserOutput, TokioAsyncReaderTsvSolutionsParser};

#[cfg(feature = "async-tokio")]
pub use tsv::TokioAsyncWriterTsvSolutionsSerializer;
pub use tsv::WriterTsvSolutionsSerializer;

#[cfg(feature = "async-tokio")]
pub use writer::{TokioAsyncWriterCsvSolutionsSerializer, tokio_async_write_boolean_csv_result};
pub use writer::{WriterCsvSolutionsSerializer, write_boolean_csv_result};

#[cfg(test)]
#[expect(clippy::panic_in_result_fn)]
mod tests;
