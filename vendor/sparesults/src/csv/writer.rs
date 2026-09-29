use oxrdf::{TermRef, Variable, VariableRef};
use std::io::{self, Write};
#[cfg(feature = "async-tokio")]
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub fn write_boolean_csv_result<W: Write>(mut writer: W, value: bool) -> io::Result<W> {
    writer.write_all(if value { b"true" } else { b"false" })?;
    Ok(writer)
}

#[cfg(feature = "async-tokio")]
pub async fn tokio_async_write_boolean_csv_result<W: AsyncWrite + Unpin>(
    mut writer: W,
    value: bool,
) -> io::Result<W> {
    writer
        .write_all(if value { b"true" } else { b"false" })
        .await?;
    Ok(writer)
}

pub struct WriterCsvSolutionsSerializer<W: Write> {
    inner: InnerCsvSolutionsSerializer,
    writer: W,
    buffer: String,
}

impl<W: Write> WriterCsvSolutionsSerializer<W> {
    pub fn start(mut writer: W, variables: Vec<Variable>) -> io::Result<Self> {
        let mut buffer = String::new();
        let inner = InnerCsvSolutionsSerializer::start(&mut buffer, variables);
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
pub struct TokioAsyncWriterCsvSolutionsSerializer<W: AsyncWrite + Unpin> {
    inner: InnerCsvSolutionsSerializer,
    writer: W,
    buffer: String,
}

#[cfg(feature = "async-tokio")]
impl<W: AsyncWrite + Unpin> TokioAsyncWriterCsvSolutionsSerializer<W> {
    pub async fn start(mut writer: W, variables: Vec<Variable>) -> io::Result<Self> {
        let mut buffer = String::new();
        let inner = InnerCsvSolutionsSerializer::start(&mut buffer, variables);
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

pub(super) struct InnerCsvSolutionsSerializer {
    variables: Vec<Variable>,
}

impl InnerCsvSolutionsSerializer {
    pub(super) fn start(output: &mut String, variables: Vec<Variable>) -> Self {
        let mut start_vars = true;
        for variable in &variables {
            if start_vars {
                start_vars = false;
            } else {
                output.push(',');
            }
            output.push_str(variable.as_str());
        }
        output.push_str("\r\n");
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
                output.push(',');
            }
            if let Some(value) = value {
                write_csv_term(output, value);
            }
        }
        output.push_str("\r\n");
    }
}

fn write_csv_term<'a>(output: &mut String, term: impl Into<TermRef<'a>>) {
    match term.into() {
        TermRef::NamedNode(uri) => output.push_str(uri.as_str()),
        TermRef::BlankNode(bnode) => {
            output.push_str("_:");
            output.push_str(bnode.as_str())
        }
        TermRef::Literal(literal) => write_escaped_csv_string(output, literal.value()),
        #[cfg(feature = "sparql-12")]
        TermRef::Triple(triple) => {
            write_csv_term(output, &triple.subject);
            output.push(' ');
            write_csv_term(output, &triple.predicate);
            output.push(' ');
            write_csv_term(output, &triple.object)
        }
    }
}

fn write_escaped_csv_string(output: &mut String, s: &str) {
    if s.bytes().any(|c| matches!(c, b'"' | b',' | b'\n' | b'\r')) {
        output.push('"');
        for c in s.chars() {
            if c == '"' {
                output.push('"');
                output.push('"');
            } else {
                output.push(c)
            };
        }
        output.push('"');
    } else {
        output.push_str(s)
    }
}
