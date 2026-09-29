//! Implementation of [SPARQL Query Results XML Format](https://www.w3.org/TR/rdf-sparql-XMLres/)

mod reader;
mod state;
mod terms;
mod writer;

pub use self::reader::{
    ReaderXmlQueryResultsParserOutput, ReaderXmlSolutionsParser, SliceXmlQueryResultsParserOutput,
    SliceXmlSolutionsParser,
};
#[cfg(feature = "async-tokio")]
pub use self::reader::{
    TokioAsyncReaderXmlQueryResultsParserOutput, TokioAsyncReaderXmlSolutionsParser,
};
#[cfg(feature = "async-tokio")]
pub use self::writer::{
    TokioAsyncWriterXmlSolutionsSerializer, tokio_async_write_boolean_xml_result,
};
pub use self::writer::{WriterXmlSolutionsSerializer, write_boolean_xml_result};
