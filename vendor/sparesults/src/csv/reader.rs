use crate::error::{QueryResultsParseError, QueryResultsSyntaxError, TextPosition};
use memchr::memchr;
use oxrdf::{Term, Variable};
use std::io::{self, Read};
use std::str::{self, FromStr};
#[cfg(feature = "async-tokio")]
use tokio::io::{AsyncRead, AsyncReadExt};

const MAX_BUFFER_SIZE: usize = 4096 * 4096;

pub enum ReaderTsvQueryResultsParserOutput<R: Read> {
    Solutions {
        variables: Vec<Variable>,
        solutions: ReaderTsvSolutionsParser<R>,
    },
    Boolean(bool),
}

impl<R: Read> ReaderTsvQueryResultsParserOutput<R> {
    pub fn read(mut reader: R) -> Result<Self, QueryResultsParseError> {
        let mut line_reader = LineReader::new();
        let mut buffer = Vec::new();
        let line = line_reader.next_line_from_reader(&mut buffer, &mut reader)?;
        Ok(match inner_read_first_line(line_reader, line)? {
            TsvInnerQueryResults::Solutions {
                variables,
                solutions,
            } => Self::Solutions {
                variables,
                solutions: ReaderTsvSolutionsParser {
                    reader,
                    inner: solutions,
                    buffer,
                },
            },
            TsvInnerQueryResults::Boolean(value) => Self::Boolean(value),
        })
    }
}

pub struct ReaderTsvSolutionsParser<R: Read> {
    reader: R,
    inner: TsvInnerSolutionsParser,
    buffer: Vec<u8>,
}

impl<R: Read> ReaderTsvSolutionsParser<R> {
    pub fn parse_next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        let line = self
            .inner
            .line_reader
            .next_line_from_reader(&mut self.buffer, &mut self.reader)?;
        Ok(self.inner.parse_next(line)?)
    }
}

#[cfg(feature = "async-tokio")]
pub enum TokioAsyncReaderTsvQueryResultsParserOutput<R: AsyncRead + Unpin> {
    Solutions {
        variables: Vec<Variable>,
        solutions: TokioAsyncReaderTsvSolutionsParser<R>,
    },
    Boolean(bool),
}

#[cfg(feature = "async-tokio")]
impl<R: AsyncRead + Unpin> TokioAsyncReaderTsvQueryResultsParserOutput<R> {
    pub async fn read(mut reader: R) -> Result<Self, QueryResultsParseError> {
        let mut line_reader = LineReader::new();
        let mut buffer = Vec::new();
        let line = line_reader
            .next_line_from_tokio_async_read(&mut buffer, &mut reader)
            .await?;
        Ok(match inner_read_first_line(line_reader, line)? {
            TsvInnerQueryResults::Solutions {
                variables,
                solutions,
            } => Self::Solutions {
                variables,
                solutions: TokioAsyncReaderTsvSolutionsParser {
                    reader,
                    inner: solutions,
                    buffer,
                },
            },
            TsvInnerQueryResults::Boolean(value) => Self::Boolean(value),
        })
    }
}

#[cfg(feature = "async-tokio")]
pub struct TokioAsyncReaderTsvSolutionsParser<R: AsyncRead + Unpin> {
    reader: R,
    inner: TsvInnerSolutionsParser,
    buffer: Vec<u8>,
}

#[cfg(feature = "async-tokio")]
impl<R: AsyncRead + Unpin> TokioAsyncReaderTsvSolutionsParser<R> {
    pub async fn parse_next(
        &mut self,
    ) -> Result<Option<Vec<Option<Term>>>, QueryResultsParseError> {
        let line = self
            .inner
            .line_reader
            .next_line_from_tokio_async_read(&mut self.buffer, &mut self.reader)
            .await?;
        Ok(self.inner.parse_next(line)?)
    }
}

pub enum SliceTsvQueryResultsParserOutput<'a> {
    Solutions {
        variables: Vec<Variable>,
        solutions: SliceTsvSolutionsParser<'a>,
    },
    Boolean(bool),
}

impl<'a> SliceTsvQueryResultsParserOutput<'a> {
    pub fn read(slice: &'a [u8]) -> Result<Self, QueryResultsSyntaxError> {
        let mut reader = LineReader::new();
        let line = reader.next_line_from_slice(slice)?;
        Ok(match inner_read_first_line(reader, line)? {
            TsvInnerQueryResults::Solutions {
                variables,
                solutions,
            } => Self::Solutions {
                variables,
                solutions: SliceTsvSolutionsParser {
                    slice,
                    inner: solutions,
                },
            },
            TsvInnerQueryResults::Boolean(value) => Self::Boolean(value),
        })
    }
}

pub struct SliceTsvSolutionsParser<'a> {
    slice: &'a [u8],
    inner: TsvInnerSolutionsParser,
}

impl SliceTsvSolutionsParser<'_> {
    pub fn parse_next(&mut self) -> Result<Option<Vec<Option<Term>>>, QueryResultsSyntaxError> {
        let line = self.inner.line_reader.next_line_from_slice(self.slice)?;
        self.inner.parse_next(line)
    }
}

enum TsvInnerQueryResults {
    Solutions {
        variables: Vec<Variable>,
        solutions: TsvInnerSolutionsParser,
    },
    Boolean(bool),
}

fn inner_read_first_line(
    reader: LineReader,
    line: &str,
) -> Result<TsvInnerQueryResults, QueryResultsSyntaxError> {
    let line = line.trim_matches(|c| matches!(c, ' ' | '\r' | '\n'));
    if line.eq_ignore_ascii_case("true") {
        return Ok(TsvInnerQueryResults::Boolean(true));
    }
    if line.eq_ignore_ascii_case("false") {
        return Ok(TsvInnerQueryResults::Boolean(false));
    }
    let mut variables = Vec::new();
    if !line.is_empty() {
        for v in line.split('\t') {
            let v = v.trim();
            if v.is_empty() {
                return Err(QueryResultsSyntaxError::msg(
                    "Empty column on the first row. The first row should be a list of variables like ?foo or $bar",
                ));
            }
            let variable = Variable::from_str(v).map_err(|e| {
                QueryResultsSyntaxError::msg(format!("Invalid variable declaration '{v}': {e}"))
            })?;
            if variables.contains(&variable) {
                return Err(QueryResultsSyntaxError::msg(format!(
                    "The variable {variable} is declared twice"
                )));
            }
            variables.push(variable);
        }
    }
    let column_len = variables.len();
    Ok(TsvInnerQueryResults::Solutions {
        variables,
        solutions: TsvInnerSolutionsParser {
            line_reader: reader,
            column_len,
        },
    })
}

struct TsvInnerSolutionsParser {
    line_reader: LineReader,
    column_len: usize,
}

impl TsvInnerSolutionsParser {
    #[expect(clippy::unwrap_in_result)]
    pub fn parse_next(
        &self,
        line: &str,
    ) -> Result<Option<Vec<Option<Term>>>, QueryResultsSyntaxError> {
        if line.is_empty() {
            return Ok(None); // EOF
        }
        let elements = line
            .split('\t')
            .enumerate()
            .map(|(i, v)| {
                let v = v.trim();
                if v.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(Term::from_str(v).map_err(|e| {
                        let start_position_char = line
                            .split('\t')
                            .take(i)
                            .map(|c| c.chars().count() + 1)
                            .sum::<usize>();
                        let start_position_bytes =
                            line.split('\t').take(i).map(|c| c.len() + 1).sum::<usize>();
                        QueryResultsSyntaxError::term(
                            e,
                            v.into(),
                            TextPosition {
                                line: self.line_reader.line_count - 1,
                                column: start_position_char.try_into().unwrap(),
                                offset: self.line_reader.last_line_start
                                    + u64::try_from(start_position_bytes).unwrap(),
                            }..TextPosition {
                                line: self.line_reader.line_count - 1,
                                column: (start_position_char + v.chars().count())
                                    .try_into()
                                    .unwrap(),
                                offset: self.line_reader.last_line_start
                                    + u64::try_from(start_position_bytes + v.len()).unwrap(),
                            },
                        )
                    })?))
                }
            })
            .collect::<Result<Vec<_>, QueryResultsSyntaxError>>()?;
        if elements.len() == self.column_len {
            Ok(Some(elements))
        } else if self.column_len == 0 && elements == [None] {
            Ok(Some(Vec::new())) // Zero columns case
        } else {
            Err(QueryResultsSyntaxError::located_message(
                format!(
                    "This TSV files has {} columns but we found a row on line {} with {} columns: {}",
                    self.column_len,
                    self.line_reader.line_count - 1,
                    elements.len(),
                    line
                ),
                TextPosition {
                    line: self.line_reader.line_count - 1,
                    column: 0,
                    offset: self.line_reader.last_line_start,
                }..TextPosition {
                    line: self.line_reader.line_count - 1,
                    column: line.chars().count().try_into().unwrap(),
                    offset: self.line_reader.last_line_end,
                },
            ))
        }
    }
}

struct LineReader {
    buffer_start: usize,
    buffer_end: usize,
    line_count: u64,
    last_line_start: u64,
    last_line_end: u64,
}

impl LineReader {
    fn new() -> Self {
        Self {
            buffer_start: 0,
            buffer_end: 0,
            line_count: 0,
            last_line_start: 0,
            last_line_end: 0,
        }
    }

    #[expect(clippy::unwrap_in_result)]
    fn next_line_from_reader<'a>(
        &mut self,
        buffer: &'a mut Vec<u8>,
        reader: &mut impl Read,
    ) -> Result<&'a str, QueryResultsParseError> {
        let line_end = loop {
            if let Some(eol) = memchr(b'\n', &buffer[self.buffer_start..self.buffer_end]) {
                break self.buffer_start + eol + 1;
            }
            if self.buffer_start > 0 {
                buffer.copy_within(self.buffer_start..self.buffer_end, 0);
                self.buffer_end -= self.buffer_start;
                self.buffer_start = 0;
            }
            if self.buffer_end + 1024 > buffer.len() {
                if self.buffer_end + 1024 > MAX_BUFFER_SIZE {
                    return Err(io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        format!("Reached the buffer maximal size of {MAX_BUFFER_SIZE}"),
                    )
                    .into());
                }
                buffer.resize(self.buffer_end + 1024, b'\0');
            }
            let read = reader.read(&mut buffer[self.buffer_end..])?;
            if read == 0 {
                break self.buffer_end;
            }
            self.buffer_end += read;
        };
        let result = str::from_utf8(&buffer[self.buffer_start..line_end]).map_err(|e| {
            QueryResultsSyntaxError::msg(format!("Invalid UTF-8 in the TSV file: {e}")).into()
        });
        self.line_count += 1;
        self.last_line_start = self.last_line_end;
        self.last_line_end += u64::try_from(line_end - self.buffer_start).unwrap();
        self.buffer_start = line_end;
        result
    }

    #[cfg(feature = "async-tokio")]
    async fn next_line_from_tokio_async_read<'a>(
        &mut self,
        buffer: &'a mut Vec<u8>,
        reader: &mut (impl AsyncRead + Unpin),
    ) -> Result<&'a str, QueryResultsParseError> {
        let line_end = loop {
            if let Some(eol) = memchr(b'\n', &buffer[self.buffer_start..self.buffer_end]) {
                break self.buffer_start + eol + 1;
            }
            if self.buffer_start > 0 {
                buffer.copy_within(self.buffer_start..self.buffer_end, 0);
                self.buffer_end -= self.buffer_start;
                self.buffer_start = 0;
            }
            if self.buffer_end + 1024 > buffer.len() {
                if self.buffer_end + 1024 > MAX_BUFFER_SIZE {
                    return Err(io::Error::new(
                        io::ErrorKind::OutOfMemory,
                        format!("Reached the buffer maximal size of {MAX_BUFFER_SIZE}"),
                    )
                    .into());
                }
                buffer.resize(self.buffer_end + 1024, b'\0');
            }
            let read = reader.read(&mut buffer[self.buffer_end..]).await?;
            if read == 0 {
                break self.buffer_end;
            }
            self.buffer_end += read;
        };
        let result = str::from_utf8(&buffer[self.buffer_start..line_end]).map_err(|e| {
            QueryResultsSyntaxError::msg(format!("Invalid UTF-8 in the TSV file: {e}")).into()
        });
        self.line_count += 1;
        self.last_line_start = self.last_line_end;
        self.last_line_end += u64::try_from(line_end - self.buffer_start).unwrap();
        self.buffer_start = line_end;
        result
    }

    #[expect(clippy::unwrap_in_result)]
    fn next_line_from_slice<'a>(
        &mut self,
        slice: &'a [u8],
    ) -> Result<&'a str, QueryResultsSyntaxError> {
        let line_end = memchr(b'\n', &slice[self.buffer_start..])
            .map_or_else(|| slice.len(), |eol| self.buffer_start + eol + 1);
        let result = str::from_utf8(&slice[self.buffer_start..line_end]).map_err(|e| {
            QueryResultsSyntaxError::msg(format!("Invalid UTF-8 in the TSV file: {e}"))
        });
        self.line_count += 1;
        self.last_line_start = self.last_line_end;
        self.last_line_end += u64::try_from(line_end - self.buffer_start).unwrap();
        self.buffer_start = line_end;
        result
    }
}
