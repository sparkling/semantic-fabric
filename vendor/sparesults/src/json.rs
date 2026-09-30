//! Implementation of [SPARQL Query Results JSON Format](https://www.w3.org/TR/sparql11-results-json/)

#![allow(clippy::large_enum_variant)]

mod reader;
mod state;
mod terms;
mod writer;

pub use reader::{
    ReaderJsonQueryResultsParserOutput, ReaderJsonSolutionsParser,
    SliceJsonQueryResultsParserOutput, SliceJsonSolutionsParser,
};
#[cfg(feature = "async-tokio")]
pub use reader::{
    TokioAsyncReaderJsonQueryResultsParserOutput, TokioAsyncReaderJsonSolutionsParser,
};
#[cfg(feature = "async-tokio")]
pub use writer::{TokioAsyncWriterJsonSolutionsSerializer, tokio_async_write_boolean_json_result};
pub use writer::{WriterJsonSolutionsSerializer, write_boolean_json_result};
