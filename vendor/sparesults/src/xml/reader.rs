use super::state::XmlInnerSolutionsParser;
use super::terms::decode_xml_entity;
use crate::error::{QueryResultsParseError, QueryResultsSyntaxError};
use oxrdf::{Term, Variable};
use quick_xml::events::Event;
use quick_xml::reader::Config;
use quick_xml::{Decoder, Reader, XmlVersion};
use std::io::{BufReader, Read};
use std::mem::take;
#[cfg(feature = "async-tokio")]
use tokio::io::{AsyncRead, BufReader as AsyncBufReader};

#[expect(clippy::large_enum_variant)]
pub enum ReaderXmlQueryResultsParserOutput<R: Read> {
    Solutions {
        variables: Vec<Variable>,
        solutions: ReaderXmlSolutionsParser<R>,
    },
    Boolean(bool),
}

impl<R: Read> ReaderXmlQueryResultsParserOutput<R> {
    pub fn read(reader: R) -> Result<Self, QueryResultsParseError> {
        let mut reader = Reader::from_reader(BufReader::new(reader));
        XmlInnerQueryResultsParser::set_options(reader.config_mut());
        let mut reader_buffer = Vec::new();
        let mut inner = XmlInnerQueryResultsParser {
            state: ResultsState::Start,
            variables: Vec::new(),
            decoder: reader.decoder(),
            text_buffer: String::new(),
            xml_version: XmlVersion::Implicit1_0,
        };
        loop {
            reader_buffer.clear();
            let event = reader.read_event_into(&mut reader_buffer)?;
            if let Some(result) = inner.read_event(event)? {
                return Ok(match result {
                    XmlInnerQueryResults::Solutions {
                        variables,
                        solutions,
                    } => Self::Solutions {
                        variables,
                        solutions: ReaderXmlSolutionsParser {
                            reader,
                            inner: solutions,
                            reader_buffer,
                        },
                    },
                    XmlInnerQueryResults::Boolean(value) => Self::Boolean(value),
                });
            }
        }
    }
}

pub struct ReaderXmlSolutionsParser<R: Read> {
    reader: Reader<BufReader<R>>,
    inner: XmlInnerSolutionsParser,
    reader_buffer: Vec<u8>,
}

impl<R: Read> ReaderXmlSolutionsParser<R> {
    pub fn parse_next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        loop {
            self.reader_buffer.clear();
            let event = self.reader.read_event_into(&mut self.reader_buffer)?;
            if event == Event::Eof {
                return Ok(None);
            }
            if let Some(solution) = self.inner.read_event(event)? {
                return Ok(Some(solution));
            }
        }
    }
}

#[cfg(feature = "async-tokio")]
#[expect(clippy::large_enum_variant)]
pub enum TokioAsyncReaderXmlQueryResultsParserOutput<R: AsyncRead + Unpin> {
    Solutions {
        variables: Vec<Variable>,
        solutions: TokioAsyncReaderXmlSolutionsParser<R>,
    },
    Boolean(bool),
}

#[cfg(feature = "async-tokio")]
impl<R: AsyncRead + Unpin> TokioAsyncReaderXmlQueryResultsParserOutput<R> {
    pub async fn read(reader: R) -> Result<Self, QueryResultsParseError> {
        let mut reader = Reader::from_reader(AsyncBufReader::new(reader));
        XmlInnerQueryResultsParser::set_options(reader.config_mut());
        let mut reader_buffer = Vec::new();
        let mut inner = XmlInnerQueryResultsParser {
            state: ResultsState::Start,
            variables: Vec::new(),
            decoder: reader.decoder(),
            text_buffer: String::new(),
            xml_version: XmlVersion::Implicit1_0,
        };
        loop {
            reader_buffer.clear();
            let event = reader.read_event_into_async(&mut reader_buffer).await?;
            if let Some(result) = inner.read_event(event)? {
                return Ok(match result {
                    XmlInnerQueryResults::Solutions {
                        variables,
                        solutions,
                    } => Self::Solutions {
                        variables,
                        solutions: TokioAsyncReaderXmlSolutionsParser {
                            reader,
                            inner: solutions,
                            reader_buffer,
                        },
                    },
                    XmlInnerQueryResults::Boolean(value) => Self::Boolean(value),
                });
            }
        }
    }
}

#[cfg(feature = "async-tokio")]
pub struct TokioAsyncReaderXmlSolutionsParser<R: AsyncRead + Unpin> {
    reader: Reader<AsyncBufReader<R>>,
    inner: XmlInnerSolutionsParser,
    reader_buffer: Vec<u8>,
}

#[cfg(feature = "async-tokio")]
impl<R: AsyncRead + Unpin> TokioAsyncReaderXmlSolutionsParser<R> {
    pub async fn parse_next(
        &mut self,
    ) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        loop {
            self.reader_buffer.clear();
            let event = self
                .reader
                .read_event_into_async(&mut self.reader_buffer)
                .await?;
            if event == Event::Eof {
                return Ok(None);
            }
            if let Some(solution) = self.inner.read_event(event)? {
                return Ok(Some(solution));
            }
        }
    }
}

#[expect(clippy::large_enum_variant)]
pub enum SliceXmlQueryResultsParserOutput<'a> {
    Solutions {
        variables: Vec<Variable>,
        solutions: SliceXmlSolutionsParser<'a>,
    },
    Boolean(bool),
}

impl<'a> SliceXmlQueryResultsParserOutput<'a> {
    pub fn read(slice: &'a [u8]) -> Result<Self, QueryResultsSyntaxError> {
        Self::do_read(slice).map_err(|e| match e {
            QueryResultsParseError::Syntax(e) => e,
            QueryResultsParseError::Io(e) => {
                unreachable!("I/O error are not possible for slice but found {e}")
            }
        })
    }

    fn do_read(slice: &'a [u8]) -> Result<Self, QueryResultsParseError> {
        let mut reader = Reader::from_reader(slice);
        XmlInnerQueryResultsParser::set_options(reader.config_mut());
        let mut reader_buffer = Vec::new();
        let mut inner = XmlInnerQueryResultsParser {
            state: ResultsState::Start,
            variables: Vec::new(),
            decoder: reader.decoder(),
            text_buffer: String::new(),
            xml_version: XmlVersion::Implicit1_0,
        };
        loop {
            reader_buffer.clear();
            let event = reader.read_event_into(&mut reader_buffer)?;
            if let Some(result) = inner.read_event(event)? {
                return Ok(match result {
                    XmlInnerQueryResults::Solutions {
                        variables,
                        solutions,
                    } => Self::Solutions {
                        variables,
                        solutions: SliceXmlSolutionsParser {
                            reader,
                            inner: solutions,
                            reader_buffer,
                        },
                    },
                    XmlInnerQueryResults::Boolean(value) => Self::Boolean(value),
                });
            }
        }
    }
}

pub struct SliceXmlSolutionsParser<'a> {
    reader: Reader<&'a [u8]>,
    inner: XmlInnerSolutionsParser,
    reader_buffer: Vec<u8>,
}

impl SliceXmlSolutionsParser<'_> {
    pub fn parse_next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsSyntaxError> {
        self.do_parse_next().map_err(|e| match e {
            QueryResultsParseError::Syntax(e) => e,
            QueryResultsParseError::Io(e) => {
                unreachable!("I/O error are not possible for slice but found {e}")
            }
        })
    }

    fn do_parse_next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        loop {
            self.reader_buffer.clear();
            let event = self.reader.read_event_into(&mut self.reader_buffer)?;
            if event == Event::Eof {
                return Ok(None);
            }
            if let Some(solution) = self.inner.read_event(event)? {
                return Ok(Some(solution));
            }
        }
    }
}

#[expect(clippy::large_enum_variant)]
enum XmlInnerQueryResults {
    Solutions {
        variables: Vec<Variable>,
        solutions: XmlInnerSolutionsParser,
    },
    Boolean(bool),
}

#[derive(Clone, Copy)]
enum ResultsState {
    Start,
    Sparql,
    Head,
    AfterHead,
    Boolean,
}

struct XmlInnerQueryResultsParser {
    state: ResultsState,
    variables: Vec<Variable>,
    decoder: Decoder,
    text_buffer: String,
    xml_version: XmlVersion,
}

impl XmlInnerQueryResultsParser {
    fn set_options(config: &mut Config) {
        config.expand_empty_elements = true;
    }

    fn read_event(
        &mut self,
        event: Event<'_>,
    ) -> Result<Option<XmlInnerQueryResults>, QueryResultsParseError> {
        match event {
            Event::Start(event) => match self.state {
                ResultsState::Start => {
                    if event.local_name().as_ref() == b"sparql" {
                        self.state = ResultsState::Sparql;
                        Ok(None)
                    } else {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting <sparql> tag, found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    }
                }
                ResultsState::Sparql => {
                    if event.local_name().as_ref() == b"head" {
                        self.state = ResultsState::Head;
                        Ok(None)
                    } else {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting <head> tag, found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    }
                }
                ResultsState::Head => {
                    if event.local_name().as_ref() == b"variable" {
                        let name = event
                            .attributes()
                            .filter_map(Result::ok)
                            .find(|attr| attr.key.local_name().as_ref() == b"name")
                            .ok_or_else(|| {
                                QueryResultsSyntaxError::msg(
                                    "No name attribute found for the <variable> tag",
                                )
                            })?;
                        let name =
                            name.decoded_and_normalized_value(self.xml_version, self.decoder)?;
                        let variable = Variable::new(name).map_err(|e| {
                            QueryResultsSyntaxError::msg(format!("Invalid variable name: {e}"))
                        })?;
                        if self.variables.contains(&variable) {
                            return Err(QueryResultsSyntaxError::msg(format!(
                                "The variable {variable} is declared twice"
                            ))
                            .into());
                        }
                        self.variables.push(variable);
                        Ok(None)
                    } else if event.local_name().as_ref() == b"link" {
                        // no op
                        Ok(None)
                    } else {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting <variable> or <link> tag, found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    }
                }
                ResultsState::AfterHead => {
                    if event.local_name().as_ref() == b"boolean" {
                        self.state = ResultsState::Boolean;
                        Ok(None)
                    } else if event.local_name().as_ref() == b"results" {
                        let solutions = XmlInnerSolutionsParser::new(
                            self.decoder,
                            &self.variables,
                            self.xml_version,
                        );
                        Ok(Some(XmlInnerQueryResults::Solutions {
                            variables: take(&mut self.variables),
                            solutions,
                        }))
                    } else if event.local_name().as_ref() != b"link"
                        && event.local_name().as_ref() != b"results"
                        && event.local_name().as_ref() != b"boolean"
                    {
                        Err(QueryResultsSyntaxError::msg(format!(
                            "Expecting sparql tag, found <{}>",
                            self.decoder.decode(event.name().as_ref())?
                        ))
                        .into())
                    } else {
                        Ok(None)
                    }
                }
                ResultsState::Boolean => Err(QueryResultsSyntaxError::msg(format!(
                    "Unexpected tag inside of <boolean> tag: <{}>",
                    self.decoder.decode(event.name().as_ref())?
                ))
                .into()),
            },
            Event::Text(event) => {
                self.text_buffer
                    .push_str(&event.xml_content(self.xml_version)?);
                Ok(None)
            }
            Event::GeneralRef(event) => {
                decode_xml_entity(&event, &mut self.text_buffer, self.xml_version)?;
                Ok(None)
            }
            Event::End(event) => {
                let value = take(&mut self.text_buffer);
                let value = value.trim_matches(|c| matches!(c, '\t' | '\n' | '\r' | ' '));
                match self.state {
                    ResultsState::Boolean => {
                        if value == "true" {
                            Ok(Some(XmlInnerQueryResults::Boolean(true)))
                        } else if value == "false" {
                            Ok(Some(XmlInnerQueryResults::Boolean(false)))
                        } else {
                            Err(QueryResultsSyntaxError::msg(format!(
                                "Unexpected boolean value. Found '{value}'"
                            ))
                            .into())
                        }
                    }
                    ResultsState::Head => {
                        if event.local_name().as_ref() == b"head" {
                            self.state = ResultsState::AfterHead;
                        }
                        Ok(None)
                    }
                    _ => {
                        if value.is_empty() {
                            Err(QueryResultsSyntaxError::msg(
                                "Unexpected early file end. All results file must have a <head> and a <result> or <boolean> tag",
                            )
                            .into())
                        } else {
                            Err(QueryResultsSyntaxError::msg(format!(
                                "Unexpected textual value found: '{value}'"
                            ))
                            .into())
                        }
                    }
                }
            }
            Event::Eof => Err(QueryResultsSyntaxError::msg(
                "Unexpected early file end. All results file must have a <head> and a <result> or <boolean> tag",
            )
            .into()),
            Event::Decl(event) => {
                self.xml_version = event.xml_version()?;
                Ok(None)
            }
            Event::Comment(_) | Event::PI(_) | Event::DocType(_) => Ok(None),
            Event::Empty(_) => unreachable!("Empty events are expended"),
            Event::CData(_) => Err(QueryResultsSyntaxError::msg(
                "<![CDATA[...]]> are not supported in SPARQL XML results",
            )
            .into()),
        }
    }
}
