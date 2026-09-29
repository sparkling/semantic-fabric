use oxrdf::vocab::xsd;
use oxrdf::{TermRef, Variable, VariableRef};
use std::io::{self, Write};
#[cfg(feature = "async-tokio")]
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub struct WriterTsvSolutionsSerializer<W: Write> {
    inner: InnerTsvSolutionsSerializer,
    writer: W,
    buffer: String,
}

impl<W: Write> WriterTsvSolutionsSerializer<W> {
    pub fn start(mut writer: W, variables: Vec<Variable>) -> io::Result<Self> {
        let mut buffer = String::new();
        let inner = InnerTsvSolutionsSerializer::start(&mut buffer, variables);
        writer.write_all(buffer.as_bytes())?;
        buffer.clear();
        Ok(Self {
            inner,
            writer,
            buffer,
        })
    }

    pub fn serialize<'a>(
        &mut self,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) -> io::Result<()> {
        self.inner.write(&mut self.buffer, solution);
        self.writer.write_all(self.buffer.as_bytes())?;
        self.buffer.clear();
        Ok(())
    }

    pub fn finish(self) -> W {
        self.writer
    }
}

#[cfg(feature = "async-tokio")]
pub struct TokioAsyncWriterTsvSolutionsSerializer<W: AsyncWrite + Unpin> {
    inner: InnerTsvSolutionsSerializer,
    writer: W,
    buffer: String,
}

#[cfg(feature = "async-tokio")]
impl<W: AsyncWrite + Unpin> TokioAsyncWriterTsvSolutionsSerializer<W> {
    pub async fn start(mut writer: W, variables: Vec<Variable>) -> io::Result<Self> {
        let mut buffer = String::new();
        let inner = InnerTsvSolutionsSerializer::start(&mut buffer, variables);
        writer.write_all(buffer.as_bytes()).await?;
        buffer.clear();
        Ok(Self {
            inner,
            writer,
            buffer,
        })
    }

    pub async fn serialize<'a>(
        &mut self,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) -> io::Result<()> {
        self.inner.write(&mut self.buffer, solution);
        self.writer.write_all(self.buffer.as_bytes()).await?;
        self.buffer.clear();
        Ok(())
    }

    pub fn finish(self) -> W {
        self.writer
    }
}

pub(super) struct InnerTsvSolutionsSerializer {
    variables: Vec<Variable>,
}

impl InnerTsvSolutionsSerializer {
    pub(super) fn start(output: &mut String, variables: Vec<Variable>) -> Self {
        let mut start_vars = true;
        for variable in &variables {
            if start_vars {
                start_vars = false;
            } else {
                output.push('\t');
            }
            output.push('?');
            output.push_str(variable.as_str());
        }
        output.push('\n');
        Self { variables }
    }

    pub(super) fn write<'a>(
        &self,
        output: &mut String,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) {
        let mut values = vec![None; self.variables.len()];
        for (variable, value) in solution {
            if let Some(position) = self.variables.iter().position(|v| *v == variable) {
                values[position] = Some(value);
            }
        }
        let mut start_binding = true;
        for value in values {
            if start_binding {
                start_binding = false;
            } else {
                output.push('\t');
            }
            if let Some(value) = value {
                write_tsv_term(output, value);
            }
        }
        output.push('\n');
    }
}

fn write_tsv_term<'a>(output: &mut String, term: impl Into<TermRef<'a>>) {
    match term.into() {
        TermRef::NamedNode(node) => {
            output.push('<');
            output.push_str(node.as_str());
            output.push('>');
        }
        TermRef::BlankNode(node) => {
            output.push_str("_:");
            output.push_str(node.as_str());
        }
        TermRef::Literal(literal) => {
            let value = literal.value();
            if let Some(language) = literal.language() {
                write_tsv_quoted_str(output, value);
                output.push('@');
                output.push_str(language);
                #[cfg(feature = "sparql-12")]
                if let Some(direction) = literal.direction() {
                    output.push_str(match direction {
                        oxrdf::BaseDirection::Ltr => "--ltr",
                        oxrdf::BaseDirection::Rtl => "--rtl",
                    })
                }
            } else {
                match literal.datatype() {
                    xsd::BOOLEAN if is_turtle_boolean(value) => output.push_str(value),
                    xsd::INTEGER if is_turtle_integer(value) => output.push_str(value),
                    xsd::DECIMAL if is_turtle_decimal(value) => output.push_str(value),
                    xsd::DOUBLE if is_turtle_double(value) => output.push_str(value),
                    xsd::STRING => write_tsv_quoted_str(output, value),
                    datatype => {
                        write_tsv_quoted_str(output, value);
                        output.push_str("^^");
                        write_tsv_term(output, datatype);
                    }
                }
            }
        }
        #[cfg(feature = "sparql-12")]
        TermRef::Triple(triple) => {
            output.push_str("<<( ");
            write_tsv_term(output, &triple.subject);
            output.push(' ');
            write_tsv_term(output, &triple.predicate);
            output.push(' ');
            write_tsv_term(output, &triple.object);
            output.push_str(" )>>");
        }
    }
}

fn write_tsv_quoted_str(output: &mut String, string: &str) {
    output.push('"');
    for c in string.chars() {
        match c {
            '\t' => output.push_str("\\t"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            _ => output.push(c),
        };
    }
    output.push('"');
}

fn is_turtle_boolean(value: &str) -> bool {
    matches!(value, "true" | "false")
}

fn is_turtle_integer(value: &str) -> bool {
    // [19]  INTEGER  ::=  [+-]? [0-9]+
    let mut value = value.as_bytes();
    if let Some(v) = value.strip_prefix(b"+") {
        value = v;
    } else if let Some(v) = value.strip_prefix(b"-") {
        value = v;
    }
    !value.is_empty() && value.iter().all(u8::is_ascii_digit)
}

fn is_turtle_decimal(value: &str) -> bool {
    // [20]  DECIMAL  ::=  [+-]? [0-9]* '.' [0-9]+
    let mut value = value.as_bytes();
    if let Some(v) = value.strip_prefix(b"+") {
        value = v;
    } else if let Some(v) = value.strip_prefix(b"-") {
        value = v;
    }
    while value.first().is_some_and(u8::is_ascii_digit) {
        value = &value[1..];
    }
    let Some(value) = value.strip_prefix(b".") else {
        return false;
    };
    !value.is_empty() && value.iter().all(u8::is_ascii_digit)
}

fn is_turtle_double(value: &str) -> bool {
    // [21]    DOUBLE    ::=  [+-]? ([0-9]+ '.' [0-9]* EXPONENT | '.' [0-9]+ EXPONENT | [0-9]+ EXPONENT)
    // [154s]  EXPONENT  ::=  [eE] [+-]? [0-9]+
    let mut value = value.as_bytes();
    if let Some(v) = value.strip_prefix(b"+") {
        value = v;
    } else if let Some(v) = value.strip_prefix(b"-") {
        value = v;
    }
    let mut with_before = false;
    while value.first().is_some_and(u8::is_ascii_digit) {
        value = &value[1..];
        with_before = true;
    }
    let mut with_after = false;
    if let Some(v) = value.strip_prefix(b".") {
        value = v;
        while value.first().is_some_and(u8::is_ascii_digit) {
            value = &value[1..];
            with_after = true;
        }
    }
    if let Some(v) = value.strip_prefix(b"e") {
        value = v;
    } else if let Some(v) = value.strip_prefix(b"E") {
        value = v;
    } else {
        return false;
    }
    if let Some(v) = value.strip_prefix(b"+") {
        value = v;
    } else if let Some(v) = value.strip_prefix(b"-") {
        value = v;
    }
    (with_before || with_after) && !value.is_empty() && value.iter().all(u8::is_ascii_digit)
}
