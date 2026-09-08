//! RFC 7464 framing for the explicit lineage extension. Each solution contains
//! one unchanged SPARQL JSON result plus its PROV-O JSON-LD bundle. A terminal
//! completion record is written only after successful source/cleanup completion.
use super::*;

pub(super) fn body<D>(
    drive: D,
    proof: Arc<crate::lineage::Lineage>,
    vars: Vec<String>,
    budget: RequestBudget,
) -> Body
where
    D: FnOnce(RowSink) -> BoxedResult + Send + 'static,
{
    let (body, producer) =
        terminal_body::spawn(CHANNEL_CAP, budget, move |tx, budget| async move {
            let variables = variables(&vars);
            let mut buf = SharedBuf::new(budget.clone());
            budget.checkpoint().map_err(control_error)?;
            let mut header = proof.header.clone();
            header["variables"] = serde_json::to_value(&vars).map_err(io::Error::other)?;
            record(&mut buf, &header)?;
            let count = Arc::new(Mutex::new(0u64));
            let sink: RowSink = {
                let mut buf = buf.clone();
                let tx = tx.clone();
                let budget = budget.clone();
                let count = count.clone();
                Box::new(move |row| {
                    let prepared = (|| -> io::Result<Vec<u8>> {
                        budget.checkpoint().map_err(control_error)?;
                        let mut ordinal = count.lock().unwrap_or_else(|p| p.into_inner());
                        let next = ordinal.checked_add(1).ok_or_else(stream_failure_error)?;
                        // Serialize directly to the charged writer: no collecting
                        // result rows, re-querying sources or embedding source values
                        // in the provenance object.
                        write!(
                            buf,
                            "\u{1e}{{\"type\":\"solution\",\"ordinal\":{},\"result\":",
                            *ordinal
                        )?;
                        let mut writer =
                            QueryResultsSerializer::from_format(QueryResultsFormat::Json)
                                .serialize_solutions_to_writer(buf.clone(), variables.clone())?;
                        writer.serialize(solution_pairs(&row, &variables))?;
                        writer.finish()?;
                        buf.write_all(b",\"provenance\":")?;
                        serde_json::to_writer(&mut buf, &proof.bundle(*ordinal))
                            .map_err(io::Error::other)?;
                        buf.write_all(b"}\n")?;
                        *ordinal = next;
                        Ok(buf.take_if_full())
                    })();
                    let tx = tx.clone();
                    Box::pin(async move { send_prepared(&tx, prepared).await }) as BoxedResult
                })
            };
            drive(sink).await.map_err(sparql_stream_error)?;
            budget.checkpoint().map_err(control_error)?;
            let solutions = *count.lock().unwrap_or_else(|p| p.into_inner());
            record(
                &mut buf,
                &serde_json::json!({"type":"complete", "solutions":solutions}),
            )?;
            budget.checkpoint().map_err(control_error)?;
            send_chunk(&tx, buf.take_all()).await
        });
    drop(producer);
    body
}

fn record(buf: &mut SharedBuf, value: &serde_json::Value) -> io::Result<()> {
    buf.write_all(b"\x1e")?;
    serde_json::to_writer(&mut *buf, value).map_err(io::Error::other)?;
    buf.write_all(b"\n")
}
