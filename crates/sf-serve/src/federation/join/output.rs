//! The existing pre-200 join buffer owns both ordinary and provenance bytes.
use super::*;
use serde_json::{json, Value};
use sparesults::{QueryResultsSerializer, WriterSolutionsSerializer};

pub(super) enum Output {
    Ordinary(WriterSolutionsSerializer<CappedWriter>),
    Lineage {
        writer: CappedWriter,
        proof: Arc<crate::lineage::Lineage>,
        ordinal: u64,
    },
}
impl Output {
    pub(super) fn new(
        format: stream::SelectFormat,
        variables: Vec<Variable>,
        budget: RequestBudget,
    ) -> io::Result<Self> {
        let mut writer = CappedWriter {
            bytes: Vec::new(),
            budget,
        };
        match format {
            stream::SelectFormat::Standard(format) => Ok(Self::Ordinary(
                QueryResultsSerializer::from_format(format)
                    .serialize_solutions_to_writer(writer, variables)?,
            )),
            stream::SelectFormat::Lineage(proof) => {
                let mut header = proof.header.clone();
                header["variables"] =
                    json!(variables.iter().map(Variable::as_str).collect::<Vec<_>>());
                header["maxProbeTriples"] = MAX_PROBE_TRIPLES.into();
                record(&mut writer, &header)?;
                Ok(Self::Lineage {
                    writer,
                    proof,
                    ordinal: 0,
                })
            }
        }
    }

    pub(super) fn serialize(&mut self, row: &[Option<Term>], vars: &[Variable]) -> io::Result<()> {
        let pairs = || {
            row.iter()
                .zip(vars)
                .filter_map(|(t, v)| t.as_ref().map(|t| (v.as_ref(), t.as_ref())))
        };
        match self {
            Self::Ordinary(writer) => writer.serialize(pairs()),
            Self::Lineage {
                writer,
                proof,
                ordinal,
            } => {
                write!(
                    writer,
                    "\u{1e}{{\"type\":\"solution\",\"ordinal\":{ordinal},\"result\":"
                )?;
                let mut result = QueryResultsSerializer::from_format(QueryResultsFormat::Json)
                    .serialize_solutions_to_writer(&mut *writer, vars.to_vec())?;
                result.serialize(pairs())?;
                result.finish()?;
                writer.write_all(b",\"provenance\":")?;
                serde_json::to_writer(&mut *writer, &proof.bundle_join(*ordinal)?)
                    .map_err(io::Error::other)?;
                writer.write_all(b"}\n")?;
                *ordinal = ordinal
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("result ordinal overflow"))?;
                Ok(())
            }
        }
    }

    /// Called only after both fragment drivers have acknowledged their cleanup.
    pub(super) fn finish(self) -> io::Result<CappedWriter> {
        match self {
            Self::Ordinary(writer) => writer.finish(),
            Self::Lineage {
                mut writer,
                ordinal,
                ..
            } => {
                record(&mut writer, &json!({"type":"complete","solutions":ordinal}))?;
                Ok(writer)
            }
        }
    }
}
fn record(writer: &mut CappedWriter, value: &Value) -> io::Result<()> {
    writer.write_all(b"\x1e")?;
    serde_json::to_writer(&mut *writer, value).map_err(io::Error::other)?;
    writer.write_all(b"\n")
}
