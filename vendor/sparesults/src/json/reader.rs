use super::state::{JsonInnerQueryResults, JsonInnerReader, JsonInnerSolutions};
use super::terms::JsonInnerTermReader;
use crate::error::{QueryResultsParseError, QueryResultsSyntaxError};
#[cfg(feature = "async-tokio")]
use json_event_parser::TokioAsyncReaderJsonParser;
use json_event_parser::{JsonEvent, ReaderJsonParser, SliceJsonParser};
use oxrdf::{Term, Variable};
use std::collections::HashMap;
use std::io::Read;
use std::mem::take;
#[cfg(feature = "async-tokio")]
use tokio::io::AsyncRead;

pub enum ReaderJsonQueryResultsParserOutput<R: Read> {
    Solutions {
        variables: Vec<Variable>,
        solutions: ReaderJsonSolutionsParser<R>,
    },
    Boolean(bool),
}

impl<R: Read> ReaderJsonQueryResultsParserOutput<R> {
    pub fn read(reader: R) -> Result<Self, QueryResultsParseError> {
        let mut json_parser = ReaderJsonParser::new(reader);
        let mut inner = JsonInnerReader::new();
        loop {
            if let Some(result) = inner.read_event(json_parser.parse_next()?)? {
                return match result {
                    JsonInnerQueryResults::Solutions {
                        variables,
                        solutions,
                    } => Ok(Self::Solutions {
                        variables,
                        solutions: ReaderJsonSolutionsParser {
                            inner: solutions,
                            json_parser,
                        },
                    }),
                    JsonInnerQueryResults::Boolean(value) => Ok(Self::Boolean(value)),
                };
            }
        }
    }
}

pub struct ReaderJsonSolutionsParser<R: Read> {
    inner: JsonInnerSolutions,
    json_parser: ReaderJsonParser<R>,
}

impl<R: Read> ReaderJsonSolutionsParser<R> {
    pub fn parse_next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        match &mut self.inner {
            JsonInnerSolutions::Reader(reader) => loop {
                let event = self.json_parser.parse_next()?;
                if event == JsonEvent::Eof {
                    return Ok(None);
                }
                if let Some(result) = reader.parse_event(event)? {
                    return Ok(Some(result));
                }
            },
            JsonInnerSolutions::Iterator(iter) => Ok(iter.next()?),
        }
    }
}

#[cfg(feature = "async-tokio")]
pub enum TokioAsyncReaderJsonQueryResultsParserOutput<R: AsyncRead + Unpin> {
    Solutions {
        variables: Vec<Variable>,
        solutions: TokioAsyncReaderJsonSolutionsParser<R>,
    },
    Boolean(bool),
}

#[cfg(feature = "async-tokio")]
impl<R: AsyncRead + Unpin> TokioAsyncReaderJsonQueryResultsParserOutput<R> {
    pub async fn read(reader: R) -> Result<Self, QueryResultsParseError> {
        let mut json_parser = TokioAsyncReaderJsonParser::new(reader);
        let mut inner = JsonInnerReader::new();
        loop {
            if let Some(result) = inner.read_event(json_parser.parse_next().await?)? {
                return match result {
                    JsonInnerQueryResults::Solutions {
                        variables,
                        solutions,
                    } => Ok(Self::Solutions {
                        variables,
                        solutions: TokioAsyncReaderJsonSolutionsParser {
                            inner: solutions,
                            json_parser,
                        },
                    }),
                    JsonInnerQueryResults::Boolean(value) => Ok(Self::Boolean(value)),
                };
            }
        }
    }
}

#[cfg(feature = "async-tokio")]
pub struct TokioAsyncReaderJsonSolutionsParser<R: AsyncRead + Unpin> {
    inner: JsonInnerSolutions,
    json_parser: TokioAsyncReaderJsonParser<R>,
}

#[cfg(feature = "async-tokio")]
impl<R: AsyncRead + Unpin> TokioAsyncReaderJsonSolutionsParser<R> {
    pub async fn parse_next(
        &mut self,
    ) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        match &mut self.inner {
            JsonInnerSolutions::Reader(reader) => loop {
                let event = self.json_parser.parse_next().await?;
                if event == JsonEvent::Eof {
                    return Ok(None);
                }
                if let Some(result) = reader.parse_event(event)? {
                    return Ok(Some(result));
                }
            },
            JsonInnerSolutions::Iterator(iter) => Ok(iter.next()?),
        }
    }
}

pub enum SliceJsonQueryResultsParserOutput<'a> {
    Solutions {
        variables: Vec<Variable>,
        solutions: SliceJsonSolutionsParser<'a>,
    },
    Boolean(bool),
}

impl<'a> SliceJsonQueryResultsParserOutput<'a> {
    pub fn read(slice: &'a [u8]) -> Result<Self, QueryResultsSyntaxError> {
        let mut json_parser = SliceJsonParser::new(slice);
        let mut inner = JsonInnerReader::new();
        loop {
            if let Some(result) = inner.read_event(json_parser.parse_next()?)? {
                return match result {
                    JsonInnerQueryResults::Solutions {
                        variables,
                        solutions,
                    } => Ok(Self::Solutions {
                        variables,
                        solutions: SliceJsonSolutionsParser {
                            inner: solutions,
                            json_parser,
                        },
                    }),
                    JsonInnerQueryResults::Boolean(value) => Ok(Self::Boolean(value)),
                };
            }
        }
    }
}

pub struct SliceJsonSolutionsParser<'a> {
    inner: JsonInnerSolutions,
    json_parser: SliceJsonParser<'a>,
}

impl SliceJsonSolutionsParser<'_> {
    pub fn parse_next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsSyntaxError> {
        match &mut self.inner {
            JsonInnerSolutions::Reader(reader) => loop {
                let event = self.json_parser.parse_next()?;
                if event == JsonEvent::Eof {
                    return Ok(None);
                }
                if let Some(result) = reader.parse_event(event)? {
                    return Ok(Some(result));
                }
            },
            JsonInnerSolutions::Iterator(iter) => iter.next(),
        }
    }
}

pub(super) struct JsonInnerSolutionsParser {
    state: JsonInnerSolutionsParserState,
    mapping: HashMap<String, usize>,
    new_bindings: Vec<Option<Term>>,
}

enum JsonInnerSolutionsParserState {
    BeforeSolution,
    BetweenSolutionTerms,
    Term {
        reader: JsonInnerTermReader,
        key: usize,
    },
    AfterEnd,
}

impl JsonInnerSolutionsParser {
    pub(super) fn new(mapping: HashMap<String, usize>) -> Self {
        Self {
            state: JsonInnerSolutionsParserState::BeforeSolution,
            mapping,
            new_bindings: Vec::new(),
        }
    }

    fn parse_event(
        &mut self,
        event: JsonEvent<'_>,
    ) -> Result<Option<Vec<Option<Term>>>, QueryResultsSyntaxError> {
        match &mut self.state {
            JsonInnerSolutionsParserState::BeforeSolution => match event {
                JsonEvent::StartObject => {
                    self.state = JsonInnerSolutionsParserState::BetweenSolutionTerms;
                    self.new_bindings = vec![None; self.mapping.len()];
                    Ok(None)
                }
                JsonEvent::EndArray => {
                    self.state = JsonInnerSolutionsParserState::AfterEnd;
                    Ok(None)
                }
                _ => Err(QueryResultsSyntaxError::msg(
                    "Expecting a new solution object",
                )),
            },
            JsonInnerSolutionsParserState::BetweenSolutionTerms => match event {
                JsonEvent::ObjectKey(key) => {
                    let key = *self.mapping.get(key.as_ref()).ok_or_else(|| {
                        QueryResultsSyntaxError::msg(format!(
                            "The variable {key} has not been defined in the header"
                        ))
                    })?;
                    self.state = JsonInnerSolutionsParserState::Term {
                        reader: JsonInnerTermReader::default(),
                        key,
                    };
                    Ok(None)
                }
                JsonEvent::EndObject => {
                    self.state = JsonInnerSolutionsParserState::BeforeSolution;
                    Ok(Some(take(&mut self.new_bindings)))
                }
                _ => unreachable!(),
            },
            JsonInnerSolutionsParserState::Term { reader, key } => {
                let result = reader.read_event(event);
                if let Some(term) = result? {
                    self.new_bindings[*key] = Some(term);
                    self.state = JsonInnerSolutionsParserState::BetweenSolutionTerms;
                }
                Ok(None)
            }
            JsonInnerSolutionsParserState::AfterEnd => {
                if event == JsonEvent::EndObject {
                    Ok(None)
                } else {
                    Err(QueryResultsSyntaxError::msg(
                        "Unexpected JSON after the end of the bindings array",
                    ))
                }
            }
        }
    }
}

pub(super) struct JsonBufferedSolutionsIterator {
    mapping: HashMap<String, usize>,
    bindings: std::vec::IntoIter<(Vec<String>, Vec<Term>)>,
}

impl JsonBufferedSolutionsIterator {
    pub(super) fn new(
        mapping: HashMap<String, usize>,
        bindings: std::vec::IntoIter<(Vec<String>, Vec<Term>)>,
    ) -> Self {
        Self { mapping, bindings }
    }

    pub(super) fn next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsSyntaxError> {
        let Some((variables, values)) = self.bindings.next() else {
            return Ok(None);
        };
        let mut new_bindings = vec![None; self.mapping.len()];
        for (variable, value) in variables.into_iter().zip(values) {
            let k = *self.mapping.get(&variable).ok_or_else(|| {
                QueryResultsSyntaxError::msg(format!(
                    "The variable {variable} has not been defined in the header"
                ))
            })?;
            new_bindings[k] = Some(value);
        }
        Ok(Some(new_bindings))
    }
}
