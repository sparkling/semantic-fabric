//! JSON text-sequence framing of ONE RDF 1.2 N-Quads dataset. Consumers concatenate
//! dataset fragments in response order and parse with one response-wide blank-
//! node scope. Metadata never enters the product/default graph.
use super::*;

pub(super) fn body<D>(drive: D, proof: Arc<crate::lineage::Lineage>, budget: RequestBudget) -> Body
where
    D: FnOnce(TripleStreamSink) -> BoxedResult + Send + 'static,
{
    let (body, producer) =
        terminal_body::spawn(CHANNEL_CAP, budget, move |tx, budget| async move {
            let mut buf = SharedBuf::new(budget.clone());
            budget.checkpoint().map_err(control_error)?;
            let mut header = proof.header.clone();
            header["profile"] = "constant-mapping-source-graph-v1".into();
            header["blankNodeScope"] = "response".into();
            header["productGraph"] = "default".into();
            header["datasetFormat"] = "application/n-quads;version=1.2".into();
            record(&mut buf, &header)?;
            let counts = Arc::new(Mutex::new((0u64, 0u64)));
            let sink: TripleStreamSink = {
                let mut buf = buf.clone();
                let tx = tx.clone();
                let budget = budget.clone();
                let counts = counts.clone();
                Box::new(move |triples| {
                    let prepared = (|| -> io::Result<Vec<u8>> {
                        budget.checkpoint().map_err(control_error)?;
                        // Invalid/unbound template terms and empty templates are not
                        // output solutions and must not manufacture activities.
                        if triples.is_empty() {
                            return Ok(Vec::new());
                        }
                        let mut counts = counts.lock().unwrap_or_else(|p| p.into_inner());
                        let next = counts.0.checked_add(1).ok_or_else(stream_failure_error)?;
                        let occurrences = counts
                            .1
                            .checked_add(triples.len() as u64)
                            .ok_or_else(stream_failure_error)?;
                        write!(
                            buf,
                            "\u{1e}{{\"type\":\"graph-solution\",\"ordinal\":{},\"dataset\":\"",
                            counts.0
                        )?;
                        proof.write_graph_dataset(
                            counts.0,
                            &triples,
                            JsonStringWriter(&mut buf),
                        )?;
                        buf.write_all(b"\"}\n")?;
                        *counts = (next, occurrences);
                        Ok(buf.take_if_full())
                    })();
                    let tx = tx.clone();
                    Box::pin(async move { send_prepared(&tx, prepared).await }) as BoxedResult
                })
            };
            drive(sink).await.map_err(sparql_stream_error)?;
            budget.checkpoint().map_err(control_error)?;
            let (solutions, occurrences) = *counts.lock().unwrap_or_else(|p| p.into_inner());
            record(
                &mut buf,
                &serde_json::json!({"type":"complete",
            "solutions":solutions, "tripleOccurrences":occurrences}),
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

/// Native RDF escaping happens first; escape the resulting UTF-8 bytes for the
/// enclosing JSON string without collecting a second copy. Writes may split a
/// UTF-8 code point, so only ASCII bytes are interpreted here. All expanded bytes
/// go through SharedBuf's existing serialized-byte charge before allocation.
struct JsonStringWriter<W>(W);

impl<W: Write> Write for JsonStringWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut start = 0;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte < 0x20 || byte == b'"' || byte == b'\\' {
                self.0.write_all(&bytes[start..index])?;
                match byte {
                    b'"' => self.0.write_all(b"\\\"")?,
                    b'\\' => self.0.write_all(b"\\\\")?,
                    _ => {
                        const HEX: &[u8] = b"0123456789abcdef";
                        self.0.write_all(&[
                            b'\\',
                            b'u',
                            b'0',
                            b'0',
                            HEX[(byte >> 4) as usize],
                            HEX[(byte & 15) as usize],
                        ])?;
                    }
                }
                start = index + 1;
            }
        }
        self.0.write_all(&bytes[start..])?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_lineage_json_escape_preserves_fragmented_utf8_and_all_control_bytes() {
        let value = format!("{}é🦀\"\\", (0..32).map(char::from).collect::<String>());
        let mut bytes = vec![b'"'];
        for byte in value.as_bytes() {
            JsonStringWriter(&mut bytes).write_all(&[*byte]).unwrap();
        }
        bytes.push(b'"');
        assert_eq!(serde_json::from_slice::<String>(&bytes).unwrap(), value);
    }
}
