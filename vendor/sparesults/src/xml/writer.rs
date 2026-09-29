use super::terms::write_xml_term;
use oxrdf::{TermRef, Variable, VariableRef};
#[cfg(feature = "async-tokio")]
use quick_xml::Error;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use std::io::{self, Write};
#[cfg(feature = "async-tokio")]
use std::sync::Arc;
#[cfg(feature = "async-tokio")]
use tokio::io::AsyncWrite;

pub fn write_boolean_xml_result<W: Write>(writer: W, value: bool) -> io::Result<W> {
    let mut writer = Writer::new(writer);
    for event in inner_write_boolean_xml_result(value) {
        writer.write_event(event)?;
    }
    Ok(writer.into_inner())
}

#[cfg(feature = "async-tokio")]
pub async fn tokio_async_write_boolean_xml_result<W: AsyncWrite + Unpin>(
    writer: W,
    value: bool,
) -> io::Result<W> {
    let mut writer = Writer::new(writer);
    for event in inner_write_boolean_xml_result(value) {
        writer
            .write_event_async(event)
            .await
            .map_err(map_xml_error)?;
    }
    Ok(writer.into_inner())
}

fn inner_write_boolean_xml_result(value: bool) -> [Event<'static>; 8] {
    [
        Event::Decl(BytesDecl::new("1.0", None, None)),
        Event::Start(
            BytesStart::new("sparql")
                .with_attributes([("xmlns", "http://www.w3.org/2005/sparql-results#")]),
        ),
        Event::Start(BytesStart::new("head")),
        Event::End(BytesEnd::new("head")),
        Event::Start(BytesStart::new("boolean")),
        Event::Text(BytesText::new(if value { "true" } else { "false" })),
        Event::End(BytesEnd::new("boolean")),
        Event::End(BytesEnd::new("sparql")),
    ]
}

pub struct WriterXmlSolutionsSerializer<W: Write> {
    inner: InnerXmlSolutionsSerializer,
    writer: Writer<W>,
}

impl<W: Write> WriterXmlSolutionsSerializer<W> {
    pub fn start(writer: W, variables: &[Variable]) -> io::Result<Self> {
        let mut writer = Writer::new(writer);
        let mut buffer = Vec::with_capacity(48);
        let inner = InnerXmlSolutionsSerializer::start(&mut buffer, variables);
        Self::do_write(&mut writer, buffer)?;
        Ok(Self { inner, writer })
    }

    pub fn serialize<'a>(
        &mut self,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) -> io::Result<()> {
        let mut buffer = Vec::with_capacity(48);
        self.inner.write(&mut buffer, solution);
        Self::do_write(&mut self.writer, buffer)
    }

    pub fn finish(mut self) -> io::Result<W> {
        let mut buffer = Vec::with_capacity(4);
        self.inner.finish(&mut buffer);
        Self::do_write(&mut self.writer, buffer)?;
        Ok(self.writer.into_inner())
    }

    fn do_write(writer: &mut Writer<W>, output: Vec<Event<'_>>) -> io::Result<()> {
        for event in output {
            writer.write_event(event)?;
        }
        Ok(())
    }
}

#[cfg(feature = "async-tokio")]
pub struct TokioAsyncWriterXmlSolutionsSerializer<W: AsyncWrite + Unpin> {
    inner: InnerXmlSolutionsSerializer,
    writer: Writer<W>,
}

#[cfg(feature = "async-tokio")]
impl<W: AsyncWrite + Unpin> TokioAsyncWriterXmlSolutionsSerializer<W> {
    pub async fn start(writer: W, variables: &[Variable]) -> io::Result<Self> {
        let mut writer = Writer::new(writer);
        let mut buffer = Vec::with_capacity(48);
        let inner = InnerXmlSolutionsSerializer::start(&mut buffer, variables);
        Self::do_write(&mut writer, buffer).await?;
        Ok(Self { inner, writer })
    }

    pub async fn serialize<'a>(
        &mut self,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) -> io::Result<()> {
        let mut buffer = Vec::with_capacity(48);
        self.inner.write(&mut buffer, solution);
        Self::do_write(&mut self.writer, buffer).await
    }

    pub async fn finish(mut self) -> io::Result<W> {
        let mut buffer = Vec::with_capacity(4);
        self.inner.finish(&mut buffer);
        Self::do_write(&mut self.writer, buffer).await?;
        Ok(self.writer.into_inner())
    }

    async fn do_write(writer: &mut Writer<W>, output: Vec<Event<'_>>) -> io::Result<()> {
        for event in output {
            writer
                .write_event_async(event)
                .await
                .map_err(map_xml_error)?;
        }
        Ok(())
    }
}

struct InnerXmlSolutionsSerializer;

impl InnerXmlSolutionsSerializer {
    fn start<'a>(output: &mut Vec<Event<'a>>, variables: &'a [Variable]) -> Self {
        output.push(Event::Decl(BytesDecl::new("1.0", None, None)));
        output.push(Event::Start(BytesStart::new("sparql").with_attributes([(
            "xmlns",
            "http://www.w3.org/2005/sparql-results#",
        )])));
        output.push(Event::Start(BytesStart::new("head")));
        for variable in variables {
            output.push(Event::Empty(
                BytesStart::new("variable").with_attributes([("name", variable.as_str())]),
            ));
        }
        output.push(Event::End(BytesEnd::new("head")));
        output.push(Event::Start(BytesStart::new("results")));
        Self {}
    }

    #[expect(clippy::unused_self)]
    fn write<'a>(
        &self,
        output: &mut Vec<Event<'a>>,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) {
        output.push(Event::Start(BytesStart::new("result")));
        for (variable, value) in solution {
            output.push(Event::Start(
                BytesStart::new("binding").with_attributes([("name", variable.as_str())]),
            ));
            write_xml_term(output, value);
            output.push(Event::End(BytesEnd::new("binding")));
        }
        output.push(Event::End(BytesEnd::new("result")));
    }

    #[expect(clippy::unused_self)]
    fn finish(self, output: &mut Vec<Event<'_>>) {
        output.push(Event::End(BytesEnd::new("results")));
        output.push(Event::End(BytesEnd::new("sparql")));
    }
}

#[cfg(feature = "async-tokio")]
fn map_xml_error(error: Error) -> io::Error {
    match error {
        Error::Io(error) => {
            Arc::try_unwrap(error).unwrap_or_else(|error| io::Error::new(error.kind(), error))
        }
        _ => io::Error::new(io::ErrorKind::InvalidData, error),
    }
}
