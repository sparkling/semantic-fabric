#[cfg(feature = "async-tokio")]
mod async_reader;
mod builder;
mod entities;
mod events;
mod literal;
mod node;
mod prefixes;
mod property;
mod reader;
mod slice;
mod state;
#[cfg(feature = "rdf-12")]
mod version;

#[cfg(feature = "async-tokio")]
pub use async_reader::TokioAsyncReaderRdfXmlParser;
pub use builder::RdfXmlParser;
pub use prefixes::RdfXmlPrefixesIter;
pub use reader::ReaderRdfXmlParser;
pub use slice::SliceRdfXmlParser;
