//! Typed multi-origin records use the same charged, fail-terminal body owner.
use super::*;
use sf_sparql::exec_core::{LineageOutput, LineageSolution};
pub(crate) type OriginSink = Box<dyn FnMut(LineageSolution) -> BoxedResult + Send>;
pub(crate) type TaggedOriginSink =
    Box<dyn FnMut(Option<sf_core::SourceId>, LineageSolution) -> BoxedResult + Send>;

pub(crate) fn body<D>(
    drive: D,
    proof: Arc<crate::lineage::Lineage>,
    form: sf_sparql::PlanForm,
    budget: RequestBudget,
) -> Body
where
    D: FnOnce(OriginSink) -> BoxedResult + Send + 'static,
{
    tagged_body(
        move |mut sink| drive(Box::new(move |solution| sink(None, solution))),
        proof,
        form,
        budget,
    )
}

pub(crate) fn tagged_body<D>(
    drive: D,
    proof: Arc<crate::lineage::Lineage>,
    form: sf_sparql::PlanForm,
    budget: RequestBudget,
) -> Body
where
    D: FnOnce(TaggedOriginSink) -> BoxedResult + Send + 'static,
{
    let (body, producer) = terminal_body::spawn(
        CHANNEL_CAP,
        budget,
        move |tx, budget| async move {
            let mut buf = SharedBuf::new(budget.clone());
            let mut header = proof.header.clone();
            let vars = match form {
                sf_sparql::PlanForm::Select { vars } => {
                    header["variables"] = serde_json::json!(vars);
                    vars
                }
                sf_sparql::PlanForm::Construct { .. } => {
                    header["profile"] = "bounded-mapping-source-graph-v1".into();
                    header["blankNodeScope"] = "response".into();
                    header["productGraph"] = "default".into();
                    header["datasetFormat"] = "application/n-quads;version=1.2".into();
                    vec![]
                }
                _ => return Err(stream_failure_error()),
            };
            let graph = header["profile"] == "bounded-mapping-source-graph-v1";
            record(&mut buf, &header)?;
            let variables = variables(&vars);
            let counts = Arc::new(Mutex::new((0u64, 0u64)));
            let sink: TaggedOriginSink = {
                let mut buf = buf.clone();
                let tx = tx.clone();
                let budget = budget.clone();
                let counts = counts.clone();
                Box::new(move |source, solution| {
                    let prepared = (|| -> io::Result<Vec<u8>> {
                        budget.checkpoint().map_err(control_error)?;
                        let proof = proof.for_source(source)?;
                        let mut count = counts.lock().unwrap_or_else(|p| p.into_inner());
                        match solution.output {
                            LineageOutput::Row(row) if !graph => {
                                write!(
                                    buf,
                                    "\u{1e}{{\"type\":\"solution\",\"ordinal\":{},\"result\":",
                                    count.0
                                )?;
                                let mut writer =
                                    QueryResultsSerializer::from_format(QueryResultsFormat::Json)
                                        .serialize_solutions_to_writer(
                                        buf.clone(),
                                        variables.clone(),
                                    )?;
                                writer.serialize(solution_pairs(&row, &variables))?;
                                writer.finish()?;
                                buf.write_all(b",\"provenance\":")?;
                                serde_json::to_writer(
                                    &mut buf,
                                    &proof.bundle_for(count.0, solution.mappings)?,
                                )
                                .map_err(io::Error::other)?;
                                buf.write_all(b"}\n")?;
                            }
                            LineageOutput::Graph(triples) if graph => {
                                if triples.is_empty() {
                                    return Ok(Vec::new());
                                }
                                write!(buf,"\u{1e}{{\"type\":\"graph-solution\",\"ordinal\":{},\"dataset\":\"",count.0)?;
                                proof.write_graph_for(
                                    count.0,
                                    &triples,
                                    solution.mappings,
                                    graph_lineage::JsonStringWriter(&mut buf),
                                )?;
                                buf.write_all(b"\"}\n")?;
                                count.1 = count
                                    .1
                                    .checked_add(triples.len() as u64)
                                    .ok_or_else(stream_failure_error)?;
                            }
                            _ => return Err(stream_failure_error()),
                        }
                        count.0 = count.0.checked_add(1).ok_or_else(stream_failure_error)?;
                        Ok(buf.take_if_full())
                    })();
                    let tx = tx.clone();
                    Box::pin(async move { send_prepared(&tx, prepared).await }) as BoxedResult
                })
            };
            drive(sink).await.map_err(sparql_stream_error)?;
            budget.checkpoint().map_err(control_error)?;
            let count = *counts.lock().unwrap_or_else(|p| p.into_inner());
            let mut complete = serde_json::json!({"type":"complete","solutions":count.0});
            if graph {
                complete["tripleOccurrences"] = count.1.into();
            }
            record(&mut buf, &complete)?;
            send_chunk(&tx, buf.take_all()).await
        },
    );
    drop(producer);
    body
}
fn record(buf: &mut SharedBuf, value: &serde_json::Value) -> io::Result<()> {
    buf.write_all(b"\x1e")?;
    serde_json::to_writer(&mut *buf, value).map_err(io::Error::other)?;
    buf.write_all(b"\n")
}
