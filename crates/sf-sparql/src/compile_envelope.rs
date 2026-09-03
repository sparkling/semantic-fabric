//! Fixed pre-parser lexical envelope for staged governed compilation.
//!
//! V1 deliberately has its own limits; a caller's HTTP/body limit cannot raise
//! them. The scanner is allocation-free and linear; an IRI candidate receives
//! one bounded lookahead before its bytes are consumed. It only classifies
//! enough syntax to avoid charging punctuation inside lexical payloads. It is
//! not a SPARQL parser and does not make `spargebra` parsing cancellable or
//! pre-emptible.
//!
//! Lexeme byte counts include their quotes, angle brackets, or comment marker;
//! comments count as conservative tokens. Operator accounting counts every
//! recognized operator in a delimiter scope, so it upper-bounds rather than
//! attempts to reconstruct exact grammar-level chains.

use std::fmt;

pub(crate) mod algebra;

/// Independent V1 input ceiling. Changing any profile constant requires a new
/// compile-profile identity when this scanner is integrated with the cache.
pub(crate) const MAX_SCANNED_BYTES_V1: usize = 256 * 1024;
pub(crate) const MAX_TOKENS_V1: usize = 32 * 1024;
pub(crate) const MAX_LEXEME_BYTES_V1: usize = 16 * 1024;
pub(crate) const MAX_NESTING_DEPTH_V1: usize = 64;
pub(crate) const MAX_RDF_STAR_DEPTH_V1: usize = 32;
pub(crate) const MAX_OPERATORS_PER_SCOPE_V1: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompileEnvelopeLimit {
    InputBytes,
    Tokens,
    LexemeBytes,
    NestingDepth,
    RdfStarDepth,
    OperatorsPerScope,
    AlgebraNodes,
    AlgebraDepth,
    CollectionSlots,
    RetainedPayloadBytes,
}

impl fmt::Display for CompileEnvelopeLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InputBytes => "input-bytes",
            Self::Tokens => "tokens",
            Self::LexemeBytes => "lexeme-bytes",
            Self::NestingDepth => "nesting-depth",
            Self::RdfStarDepth => "rdf-star-depth",
            Self::OperatorsPerScope => "operators-per-scope",
            Self::AlgebraNodes => "algebra-nodes",
            Self::AlgebraDepth => "algebra-depth",
            Self::CollectionSlots => "collection-slots",
            Self::RetainedPayloadBytes => "retained-payload-bytes",
        })
    }
}

/// Internal admission failure. It intentionally carries bounded numeric data,
/// never submitted query text or an upstream parser diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CompileEnvelopeError {
    #[error("compile envelope V1 {dimension} limit exceeded ({observed}>{maximum})")]
    LimitExceeded {
        dimension: CompileEnvelopeLimit,
        observed: usize,
        maximum: usize,
    },
}

/// Measurements retained by the V1 scan for later work accounting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CompileEnvelopeV1 {
    pub(crate) input_bytes: usize,
    pub(crate) tokens: usize,
    pub(crate) max_lexeme_bytes: usize,
    pub(crate) max_nesting_depth: usize,
    pub(crate) max_rdf_star_depth: usize,
    /// Conservative upper bound for any operator chain: every operator in one
    /// delimiter scope is counted, even when separate operands break the chain.
    pub(crate) max_operators_per_scope: usize,
}

impl CompileEnvelopeV1 {
    pub(crate) fn scan(input: &str) -> Result<Self, CompileEnvelopeError> {
        enforce(
            CompileEnvelopeLimit::InputBytes,
            input.len(),
            MAX_SCANNED_BYTES_V1,
        )?;
        Scanner::new(input).scan()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Delimiter {
    Round,
    Square,
    Curly,
    RdfStar,
}

struct Scanner<'input> {
    bytes: &'input [u8],
    cursor: usize,
    depth: usize,
    rdf_star_depth: usize,
    delimiters: [Delimiter; MAX_NESTING_DEPTH_V1],
    scope_operators: [usize; MAX_NESTING_DEPTH_V1 + 1],
    envelope: CompileEnvelopeV1,
}

impl<'input> Scanner<'input> {
    fn new(input: &'input str) -> Self {
        Self {
            bytes: input.as_bytes(),
            cursor: 0,
            depth: 0,
            rdf_star_depth: 0,
            delimiters: [Delimiter::Round; MAX_NESTING_DEPTH_V1],
            scope_operators: [0; MAX_NESTING_DEPTH_V1 + 1],
            envelope: CompileEnvelopeV1 {
                input_bytes: input.len(),
                ..CompileEnvelopeV1::default()
            },
        }
    }

    fn scan(mut self) -> Result<CompileEnvelopeV1, CompileEnvelopeError> {
        while let Some(&byte) = self.bytes.get(self.cursor) {
            match byte {
                byte if byte.is_ascii_whitespace() => self.cursor += 1,
                b'#' => self.scan_comment()?,
                b'\'' | b'"' => self.scan_string(byte)?,
                b'<' if self.peek(1) == Some(b'<') => self.scan_double_less_than()?,
                b'<' if self.starts_iri() => self.scan_iri()?,
                b'<' => self.operator(if self.peek(1) == Some(b'=') { 2 } else { 1 })?,
                b'>' if self.peek(1) == Some(b'>') && self.top() == Some(Delimiter::RdfStar) => {
                    self.close(Delimiter::RdfStar, 2)?;
                }
                b'>' => self.operator(if self.peek(1) == Some(b'=') { 2 } else { 1 })?,
                b'(' => self.open(Delimiter::Round, 1)?,
                b'[' => self.open(Delimiter::Square, 1)?,
                b'{' => self.open(Delimiter::Curly, 1)?,
                b')' => self.close(Delimiter::Round, 1)?,
                b']' => self.close(Delimiter::Square, 1)?,
                b'}' => self.close(Delimiter::Curly, 1)?,
                b'.' => self.punctuation()?,
                b';' | b',' => self.separator()?,
                b'?' | b'$' if self.peek(1).is_some_and(is_variable_byte) => {
                    self.scan_variable()?;
                }
                b'!' | b'=' | b'+' | b'-' | b'*' | b'/' | b'|' | b'&' | b'^' | b'?' => {
                    let width = self.operator_width(byte);
                    self.operator(width)?;
                }
                _ => self.scan_word()?,
            }
        }
        Ok(self.envelope)
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.cursor + offset).copied()
    }

    fn top(&self) -> Option<Delimiter> {
        self.depth
            .checked_sub(1)
            .map(|index| self.delimiters[index])
    }

    fn starts_iri(&self) -> bool {
        self.iri_candidate_len(self.cursor).is_some()
    }

    fn iri_candidate_len(&self, start: usize) -> Option<usize> {
        let mut candidate = start.checked_add(1)?;
        while let Some(&byte) = self.bytes.get(candidate) {
            match byte {
                b'>' => return candidate.checked_add(1)?.checked_sub(start),
                byte if is_iri_boundary(byte) => return None,
                b'\\' => {
                    candidate += 1;
                    if candidate >= self.bytes.len() {
                        return None;
                    }
                }
                _ => {}
            }
            candidate += 1;
        }
        None
    }

    fn scan_double_less_than(&mut self) -> Result<(), CompileEnvelopeError> {
        // `<<` is context-sensitive in the pinned parser: it can be the quoted
        // triple opener, or relational `<` immediately followed by `<iri>`. In
        // the latter case consume only the operator; the next scan step sees and
        // measures the complete IRI. A real compact RDF-star opener followed by
        // an IRI is `<<<iri>`, so the candidate beginning at the second byte is
        // stopped by its own following `<` and cannot enter this branch.
        if self.iri_candidate_len(self.cursor + 1).is_some() {
            self.operator(1)
        } else {
            self.open(Delimiter::RdfStar, 2)
        }
    }

    fn scan_comment(&mut self) -> Result<(), CompileEnvelopeError> {
        let start = self.cursor;
        while let Some(&byte) = self.bytes.get(self.cursor) {
            if matches!(byte, b'\n' | b'\r') {
                break;
            }
            self.advance_lexeme(start, 1)?;
        }
        self.finish_token(start, false)
    }

    fn scan_string(&mut self, quote: u8) -> Result<(), CompileEnvelopeError> {
        let start = self.cursor;
        let long = self.peek(1) == Some(quote) && self.peek(2) == Some(quote);
        self.advance_lexeme(start, if long { 3 } else { 1 })?;
        while let Some(&byte) = self.bytes.get(self.cursor) {
            if !long && matches!(byte, b'\n' | b'\r') {
                break;
            } else if byte == b'\\' {
                self.advance_lexeme(start, 1)?;
                if !long && matches!(self.peek(0), Some(b'\n' | b'\r')) {
                    break;
                } else if self.cursor < self.bytes.len() {
                    self.advance_lexeme(start, 1)?;
                }
            } else if long
                && byte == quote
                && self.peek(1) == Some(quote)
                && self.peek(2) == Some(quote)
            {
                self.advance_lexeme(start, 3)?;
                break;
            } else if !long && byte == quote {
                self.advance_lexeme(start, 1)?;
                break;
            } else {
                self.advance_lexeme(start, 1)?;
            }
        }
        self.finish_token(start, false)
    }

    fn scan_iri(&mut self) -> Result<(), CompileEnvelopeError> {
        let start = self.cursor;
        self.advance_lexeme(start, 1)?;
        while let Some(&byte) = self.bytes.get(self.cursor) {
            if byte != b'>' && is_iri_boundary(byte) {
                break;
            } else if byte == b'\\' {
                self.advance_lexeme(start, 1)?;
                if self.cursor < self.bytes.len() {
                    self.advance_lexeme(start, 1)?;
                }
            } else {
                self.advance_lexeme(start, 1)?;
                if byte == b'>' {
                    break;
                }
            }
        }
        self.finish_token(start, false)
    }

    fn scan_variable(&mut self) -> Result<(), CompileEnvelopeError> {
        let start = self.cursor;
        self.advance_lexeme(start, 1)?;
        while self.peek(0).is_some_and(is_variable_byte) {
            self.advance_lexeme(start, 1)?;
        }
        self.finish_token(start, false)
    }

    fn scan_word(&mut self) -> Result<(), CompileEnvelopeError> {
        let start = self.cursor;
        while let Some(&byte) = self.bytes.get(self.cursor) {
            if is_boundary(byte) {
                break;
            }
            if byte == b'\\' {
                self.advance_lexeme(start, 1)?;
                if self.cursor < self.bytes.len() {
                    self.advance_lexeme(start, 1)?;
                }
            } else {
                self.advance_lexeme(start, 1)?;
            }
        }
        if self.cursor == start {
            self.advance_lexeme(start, 1)?;
        }
        let operator_keyword = is_operator_keyword(&self.bytes[start..self.cursor]);
        self.finish_token(start, operator_keyword)
    }

    fn advance_lexeme(&mut self, start: usize, width: usize) -> Result<(), CompileEnvelopeError> {
        self.cursor += width;
        enforce(
            CompileEnvelopeLimit::LexemeBytes,
            self.cursor - start,
            MAX_LEXEME_BYTES_V1,
        )
    }

    fn finish_token(&mut self, start: usize, operator: bool) -> Result<(), CompileEnvelopeError> {
        let width = self.cursor - start;
        self.record_token(width)?;
        if operator {
            self.record_scope_operator()?;
        }
        Ok(())
    }

    fn operator_width(&self, first: u8) -> usize {
        match (first, self.peek(1)) {
            (b'!', Some(b'=')) | (b'|', Some(b'|')) | (b'&', Some(b'&')) | (b'^', Some(b'^')) => 2,
            _ => 1,
        }
    }

    fn operator(&mut self, width: usize) -> Result<(), CompileEnvelopeError> {
        self.record_token(width)?;
        self.record_scope_operator()?;
        self.cursor += width;
        Ok(())
    }

    fn separator(&mut self) -> Result<(), CompileEnvelopeError> {
        self.record_token(1)?;
        self.scope_operators[self.depth] = 0;
        self.cursor += 1;
        Ok(())
    }

    fn punctuation(&mut self) -> Result<(), CompileEnvelopeError> {
        self.record_token(1)?;
        self.cursor += 1;
        Ok(())
    }

    fn open(&mut self, delimiter: Delimiter, width: usize) -> Result<(), CompileEnvelopeError> {
        self.record_token(width)?;
        let next_depth = self.depth + 1;
        enforce(
            CompileEnvelopeLimit::NestingDepth,
            next_depth,
            MAX_NESTING_DEPTH_V1,
        )?;
        let next_rdf_depth = self.rdf_star_depth + usize::from(delimiter == Delimiter::RdfStar);
        enforce(
            CompileEnvelopeLimit::RdfStarDepth,
            next_rdf_depth,
            MAX_RDF_STAR_DEPTH_V1,
        )?;
        self.delimiters[self.depth] = delimiter;
        self.depth = next_depth;
        self.rdf_star_depth = next_rdf_depth;
        self.scope_operators[self.depth] = 0;
        self.envelope.max_nesting_depth = self.envelope.max_nesting_depth.max(self.depth);
        self.envelope.max_rdf_star_depth =
            self.envelope.max_rdf_star_depth.max(self.rdf_star_depth);
        self.cursor += width;
        Ok(())
    }

    fn close(&mut self, delimiter: Delimiter, width: usize) -> Result<(), CompileEnvelopeError> {
        self.record_token(width)?;
        if self.top() == Some(delimiter) {
            if delimiter == Delimiter::RdfStar {
                self.rdf_star_depth -= 1;
            }
            self.scope_operators[self.depth] = 0;
            self.depth -= 1;
        }
        self.cursor += width;
        Ok(())
    }

    fn record_token(&mut self, width: usize) -> Result<(), CompileEnvelopeError> {
        let observed = self.envelope.tokens + 1;
        enforce(CompileEnvelopeLimit::Tokens, observed, MAX_TOKENS_V1)?;
        self.envelope.tokens = observed;
        self.envelope.max_lexeme_bytes = self.envelope.max_lexeme_bytes.max(width);
        Ok(())
    }

    fn record_scope_operator(&mut self) -> Result<(), CompileEnvelopeError> {
        let observed = self.scope_operators[self.depth] + 1;
        enforce(
            CompileEnvelopeLimit::OperatorsPerScope,
            observed,
            MAX_OPERATORS_PER_SCOPE_V1,
        )?;
        self.scope_operators[self.depth] = observed;
        self.envelope.max_operators_per_scope = self.envelope.max_operators_per_scope.max(observed);
        Ok(())
    }
}

fn enforce(
    dimension: CompileEnvelopeLimit,
    observed: usize,
    maximum: usize,
) -> Result<(), CompileEnvelopeError> {
    if observed > maximum {
        Err(CompileEnvelopeError::LimitExceeded {
            dimension,
            observed,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn is_variable_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn is_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'#' | b'\''
                | b'"'
                | b'<'
                | b'>'
                | b'('
                | b')'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'.'
                | b';'
                | b','
                | b'!'
                | b'='
                | b'+'
                | b'-'
                | b'*'
                | b'/'
                | b'|'
                | b'&'
                | b'^'
                | b'?'
        )
}

fn is_iri_boundary(byte: u8) -> bool {
    byte.is_ascii_control()
        || byte.is_ascii_whitespace()
        || matches!(byte, b'<' | b'"' | b'{' | b'}' | b'|' | b'^' | b'`')
}

fn is_operator_keyword(word: &[u8]) -> bool {
    [
        b"UNION".as_slice(),
        b"MINUS",
        b"OPTIONAL",
        b"IN",
        b"NOT",
        b"AS",
    ]
    .iter()
    .any(|candidate| word.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests;
