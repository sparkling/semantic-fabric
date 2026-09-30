#[cfg(feature = "async-tokio")]
use json_event_parser::TokioAsyncWriterJsonSerializer;
use json_event_parser::{JsonEvent, WriterJsonSerializer};
use oxrdf::vocab::xsd;
use oxrdf::*;
use std::io::{self, Write};
#[cfg(feature = "async-tokio")]
use tokio::io::AsyncWrite;

pub fn write_boolean_json_result<W: Write>(writer: W, value: bool) -> io::Result<W> {
    let mut serializer = WriterJsonSerializer::new(writer);
    for event in inner_write_boolean_json_result(value) {
        serializer.serialize_event(event)?;
    }
    serializer.finish()
}

#[cfg(feature = "async-tokio")]
pub async fn tokio_async_write_boolean_json_result<W: AsyncWrite + Unpin>(
    writer: W,
    value: bool,
) -> io::Result<W> {
    let mut serializer = TokioAsyncWriterJsonSerializer::new(writer);
    for event in inner_write_boolean_json_result(value) {
        serializer.serialize_event(event).await?;
    }
    serializer.finish()
}

fn inner_write_boolean_json_result(value: bool) -> [JsonEvent<'static>; 7] {
    [
        JsonEvent::StartObject,
        JsonEvent::ObjectKey("head".into()),
        JsonEvent::StartObject,
        JsonEvent::EndObject,
        JsonEvent::ObjectKey("boolean".into()),
        JsonEvent::Boolean(value),
        JsonEvent::EndObject,
    ]
}

pub struct WriterJsonSolutionsSerializer<W: Write> {
    inner: InnerJsonSolutionsSerializer,
    serializer: WriterJsonSerializer<W>,
}

impl<W: Write> WriterJsonSolutionsSerializer<W> {
    pub fn start(writer: W, variables: &[Variable]) -> io::Result<Self> {
        let mut serializer = WriterJsonSerializer::new(writer);
        let mut buffer = Vec::with_capacity(48);
        let inner = InnerJsonSolutionsSerializer::start(&mut buffer, variables);
        Self::do_write(&mut serializer, buffer)?;
        Ok(Self { inner, serializer })
    }

    pub fn serialize<'a>(
        &mut self,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) -> io::Result<()> {
        let mut buffer = Vec::with_capacity(48);
        self.inner.write(&mut buffer, solution);
        Self::do_write(&mut self.serializer, buffer)
    }

    pub fn finish(mut self) -> io::Result<W> {
        let mut buffer = Vec::with_capacity(4);
        self.inner.finish(&mut buffer);
        Self::do_write(&mut self.serializer, buffer)?;
        self.serializer.finish()
    }

    fn do_write(
        serializer: &mut WriterJsonSerializer<W>,
        output: Vec<JsonEvent<'_>>,
    ) -> io::Result<()> {
        for event in output {
            serializer.serialize_event(event)?;
        }
        Ok(())
    }
}

#[cfg(feature = "async-tokio")]
pub struct TokioAsyncWriterJsonSolutionsSerializer<W: AsyncWrite + Unpin> {
    inner: InnerJsonSolutionsSerializer,
    serializer: TokioAsyncWriterJsonSerializer<W>,
}

#[cfg(feature = "async-tokio")]
impl<W: AsyncWrite + Unpin> TokioAsyncWriterJsonSolutionsSerializer<W> {
    pub async fn start(writer: W, variables: &[Variable]) -> io::Result<Self> {
        let mut serializer = TokioAsyncWriterJsonSerializer::new(writer);
        let mut buffer = Vec::with_capacity(48);
        let inner = InnerJsonSolutionsSerializer::start(&mut buffer, variables);
        Self::do_write(&mut serializer, buffer).await?;
        Ok(Self { inner, serializer })
    }

    pub async fn serialize<'a>(
        &mut self,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) -> io::Result<()> {
        let mut buffer = Vec::with_capacity(48);
        self.inner.write(&mut buffer, solution);
        Self::do_write(&mut self.serializer, buffer).await
    }

    pub async fn finish(mut self) -> io::Result<W> {
        let mut buffer = Vec::with_capacity(4);
        self.inner.finish(&mut buffer);
        Self::do_write(&mut self.serializer, buffer).await?;
        self.serializer.finish()
    }

    async fn do_write(
        serializer: &mut TokioAsyncWriterJsonSerializer<W>,
        output: Vec<JsonEvent<'_>>,
    ) -> io::Result<()> {
        for event in output {
            serializer.serialize_event(event).await?;
        }
        Ok(())
    }
}

struct InnerJsonSolutionsSerializer;

impl InnerJsonSolutionsSerializer {
    fn start<'a>(output: &mut Vec<JsonEvent<'a>>, variables: &'a [Variable]) -> Self {
        output.push(JsonEvent::StartObject);
        output.push(JsonEvent::ObjectKey("head".into()));
        output.push(JsonEvent::StartObject);
        output.push(JsonEvent::ObjectKey("vars".into()));
        output.push(JsonEvent::StartArray);
        for variable in variables {
            output.push(JsonEvent::String(variable.as_str().into()));
        }
        output.push(JsonEvent::EndArray);
        output.push(JsonEvent::EndObject);
        output.push(JsonEvent::ObjectKey("results".into()));
        output.push(JsonEvent::StartObject);
        output.push(JsonEvent::ObjectKey("bindings".into()));
        output.push(JsonEvent::StartArray);
        Self {}
    }

    #[expect(clippy::unused_self)]
    fn write<'a>(
        &self,
        output: &mut Vec<JsonEvent<'a>>,
        solution: impl IntoIterator<Item = (VariableRef<'a>, TermRef<'a>)>,
    ) {
        output.push(JsonEvent::StartObject);
        for (variable, value) in solution {
            output.push(JsonEvent::ObjectKey(variable.as_str().into()));
            write_json_term(output, value);
        }
        output.push(JsonEvent::EndObject);
    }

    #[expect(clippy::unused_self)]
    fn finish(self, output: &mut Vec<JsonEvent<'_>>) {
        output.push(JsonEvent::EndArray);
        output.push(JsonEvent::EndObject);
        output.push(JsonEvent::EndObject);
    }
}

fn write_json_term<'a>(output: &mut Vec<JsonEvent<'a>>, term: TermRef<'a>) {
    match term {
        TermRef::NamedNode(uri) => {
            output.push(JsonEvent::StartObject);
            output.push(JsonEvent::ObjectKey("type".into()));
            output.push(JsonEvent::String("uri".into()));
            output.push(JsonEvent::ObjectKey("value".into()));
            output.push(JsonEvent::String(uri.as_str().into()));
            output.push(JsonEvent::EndObject);
        }
        TermRef::BlankNode(bnode) => {
            output.push(JsonEvent::StartObject);
            output.push(JsonEvent::ObjectKey("type".into()));
            output.push(JsonEvent::String("bnode".into()));
            output.push(JsonEvent::ObjectKey("value".into()));
            output.push(JsonEvent::String(bnode.as_str().into()));
            output.push(JsonEvent::EndObject);
        }
        TermRef::Literal(literal) => {
            output.push(JsonEvent::StartObject);
            output.push(JsonEvent::ObjectKey("type".into()));
            output.push(JsonEvent::String("literal".into()));
            output.push(JsonEvent::ObjectKey("value".into()));
            output.push(JsonEvent::String(literal.value().into()));
            if let Some(language) = literal.language() {
                output.push(JsonEvent::ObjectKey("xml:lang".into()));
                output.push(JsonEvent::String(language.into()));
                #[cfg(feature = "sparql-12")]
                if let Some(direction) = literal.direction() {
                    output.push(JsonEvent::ObjectKey("its:dir".into()));
                    output.push(JsonEvent::String(
                        match direction {
                            BaseDirection::Ltr => "ltr",
                            BaseDirection::Rtl => "rtl",
                        }
                        .into(),
                    ));
                }
            } else if literal.datatype() != xsd::STRING {
                output.push(JsonEvent::ObjectKey("datatype".into()));
                output.push(JsonEvent::String(literal.datatype().as_str().into()));
            }
            output.push(JsonEvent::EndObject);
        }
        #[cfg(feature = "sparql-12")]
        TermRef::Triple(triple) => {
            output.push(JsonEvent::StartObject);
            output.push(JsonEvent::ObjectKey("type".into()));
            output.push(JsonEvent::String("triple".into()));
            output.push(JsonEvent::ObjectKey("value".into()));
            output.push(JsonEvent::StartObject);
            output.push(JsonEvent::ObjectKey("subject".into()));
            write_json_term(output, triple.subject.as_ref().into());
            output.push(JsonEvent::ObjectKey("predicate".into()));
            write_json_term(output, triple.predicate.as_ref().into());
            output.push(JsonEvent::ObjectKey("object".into()));
            write_json_term(output, triple.object.as_ref());
            output.push(JsonEvent::EndObject);
            output.push(JsonEvent::EndObject);
        }
    }
}
