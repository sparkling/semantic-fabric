//! One bounded driving batch, a streamed probe, and a capped pre-200 serializer.
use super::*;
use oxrdf::Variable;
use sf_core::query_control::{QueryCharge, QueryControlError};
use sf_core::{BlankNode, SourceId, Term};
use sf_sparql::federation::BoundedJoin;
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Mutex;
mod output;

// Exact per-triple identity inside this merge, not a general DISTINCT operator.
// The cap and retained-byte admission are independent of source cardinality.
const MAX_PROBE_TRIPLES: usize = 4096;

/// Intermediate solutions are source work, not final response items. Every
/// phase still shares the very same terminal state and accounting identity.
struct FragmentControl(RequestBudget);
impl QueryControl for FragmentControl {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.0.checkpoint()
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.0.terminate(reason)
    }
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.0.consume(
            if charge == QueryCharge::ResultItems {
                QueryCharge::SourceWork
            } else {
                charge
            },
            amount,
        )
    }
}

pub(super) async fn body(
    acquired: Vec<AcquiredFragment>,
    source_ids: [SourceId; 2],
    join: BoundedJoin,
    variables: Vec<String>,
    format: stream::SelectFormat,
    budget: RequestBudget,
) -> Result<Body, Response> {
    let task_budget = budget.clone();
    let task = crate::deadline::spawn_request_task(async move {
        execute(acquired, source_ids, join, variables, format, task_budget).await
    });
    match crate::deadline::join_task(budget, task).await {
        Ok(Ok(bytes)) => Ok(Body::from(bytes)),
        Ok(Err(error)) => Err(problem::response_for_sparql(&error)),
        Err(crate::deadline::JoinedTaskError::Control(error)) => {
            Err(problem::response_for_control(error))
        }
        Err(crate::deadline::JoinedTaskError::Join(_)) => {
            Err(problem::response(ProblemCode::Internal))
        }
    }
}

async fn execute(
    acquired: Vec<AcquiredFragment>,
    source_ids: [SourceId; 2],
    join: BoundedJoin,
    variables: Vec<String>,
    format: stream::SelectFormat,
    budget: RequestBudget,
) -> sf_sparql::Result<Vec<u8>> {
    let [build, mut probe]: [AcquiredFragment; 2] = acquired
        .try_into()
        .map_err(|_| sf_sparql::Error::Mapping("bounded join source arity".into()))?;
    let control: Arc<dyn QueryControl> = Arc::new(FragmentControl(budget.clone()));
    let rows = Arc::new(Mutex::new(Vec::new()));
    let mut collector: RowSink = {
        let rows = rows.clone();
        let budget = budget.clone();
        let maximum = join.max_build_rows();
        let build_join = join.clone();
        Box::new(move |mut row| {
            let result = (|| {
                if !build_join.accepts_row(0, &row)? {
                    return Ok(());
                }
                let mut retained = rows.lock().unwrap_or_else(|p| p.into_inner());
                // Fixed row count bounds structural overhead. Charge textual
                // payload plus headroom for source scoping and reducer copies.
                budget.consume(
                    QueryCharge::RetainedBytes,
                    row_bytes(&row)?
                        .checked_mul(8)
                        .ok_or(QueryControlError::AccountingOverflow)?,
                )?;
                scope(&mut row, source_ids[0]);
                for existing in retained.iter() {
                    budget.consume(QueryCharge::SourceWork, 1)?;
                    if existing == &row {
                        return Ok(());
                    }
                }
                if retained.len() >= maximum {
                    return Err(budget
                        .terminate(QueryControlError::RetainedBytesExceeded)
                        .into());
                }
                retained.push(row);
                Ok(())
            })();
            Box::pin(std::future::ready(result))
        })
    };
    if let Err(error) = drive(build, &budget, control.clone(), &mut collector).await {
        close_acquired(vec![probe]).await;
        return Err(error);
    }
    drop(collector);
    let rows = Arc::try_unwrap(rows)
        .map_err(|_| sf_sparql::Error::Mapping("bounded join row ownership".into()))?
        .into_inner()
        .unwrap_or_else(|p| p.into_inner());
    let reduced = join.reduced_probe(probe.plan_mut(), &rows);
    match reduced {
        Ok(plan) => *probe.plan_mut() = Arc::new(plan),
        Err(error) => {
            close_acquired(vec![probe]).await;
            return Err(error);
        }
    }
    let vars: Vec<_> = variables.iter().map(Variable::new_unchecked).collect();
    let writer = match output::Output::new(format, vars.clone(), budget.clone()) {
        Ok(writer) => writer,
        Err(_) => {
            close_acquired(vec![probe]).await;
            return Err(writer_error(&budget));
        }
    };
    let writer = Arc::new(Mutex::new(Some(writer)));
    let mut sink: RowSink = {
        let writer = writer.clone();
        let budget = budget.clone();
        let mut probe_high_water = 0;
        let mut seen = HashSet::new();
        Box::new(move |mut right| {
            let result = (|| {
                budget.checkpoint()?;
                let retained = row_bytes(&right)?
                    .checked_mul(8)
                    .ok_or(QueryControlError::AccountingOverflow)?;
                if retained > probe_high_water {
                    budget.consume(QueryCharge::RetainedBytes, retained - probe_high_water)?;
                    probe_high_water = retained;
                }
                if !join.accepts_row(1, &right)? {
                    return Ok(());
                }
                scope(&mut right, source_ids[1]);
                if seen.contains(&right) {
                    return Ok(());
                }
                if seen.len() >= MAX_PROBE_TRIPLES {
                    return Err(budget
                        .terminate(QueryControlError::RetainedBytesExceeded)
                        .into());
                }
                budget.consume(QueryCharge::RetainedBytes, retained)?;
                seen.insert(right.clone());
                for left in &rows {
                    budget.consume(QueryCharge::SourceWork, 1)?;
                    if let Some(row) = join.merge(left, &right)? {
                        budget.consume(QueryCharge::ResultItems, 1)?;
                        writer
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .as_mut()
                            .unwrap()
                            .serialize(&row, &vars)
                            .map_err(|_| writer_error(&budget))?;
                    }
                }
                Ok(())
            })();
            Box::pin(std::future::ready(result))
        })
    };
    drive(probe, &budget, control, &mut sink).await?;
    drop(sink);
    let writer = writer
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .unwrap();
    let bytes = writer.finish().map_err(|_| writer_error(&budget))?.bytes;
    budget.checkpoint()?;
    Ok(bytes)
}

struct CappedWriter {
    bytes: Vec<u8>,
    budget: RequestBudget,
}
impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let amount = u64::try_from(bytes.len()).map_err(|_| io::Error::other("result capacity"))?;
        self.budget
            .consume(QueryCharge::SerializedBytes, amount)
            .map_err(|_| io::Error::other("result capacity"))?;
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("result capacity"))?;
        if needed > self.bytes.capacity() {
            let capacity = needed.max(self.bytes.capacity().saturating_mul(2)).max(8);
            self.budget
                .consume(
                    QueryCharge::RetainedBytes,
                    (capacity - self.bytes.capacity()) as u64,
                )
                .map_err(|_| io::Error::other("result capacity"))?;
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| {
                    self.budget
                        .terminate(QueryControlError::RetainedBytesExceeded);
                    io::Error::other("result capacity")
                })?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn writer_error(budget: &RequestBudget) -> sf_sparql::Error {
    budget
        .checkpoint()
        .err()
        .map(sf_sparql::Error::from)
        .unwrap_or_else(|| sf_sparql::Error::Sql("result serialization failed".into()))
}
pub(super) fn row_bytes(row: &[Option<Term>]) -> sf_sparql::Result<u64> {
    row.iter()
        .try_fold(std::mem::size_of_val(row) as u64, |n, t| {
            n.checked_add(t.as_ref().map_or(0, term_bytes))
                .ok_or_else(|| QueryControlError::AccountingOverflow.into())
        })
}
fn term_bytes(term: &Term) -> u64 {
    match term {
        Term::NamedNode(n) => n.as_str().len() as u64,
        Term::BlankNode(n) => n.as_str().len() as u64,
        Term::Literal(l) => {
            (l.value().len() + l.datatype().as_str().len() + l.language().map_or(0, str::len) + 8)
                as u64
        }
        Term::Triple(t) => {
            let subject = match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().len(),
                oxrdf::NamedOrBlankNode::BlankNode(n) => n.as_str().len(),
            };
            subject as u64 + t.predicate.as_str().len() as u64 + term_bytes(&t.object) + 64
        }
    }
}
pub(super) fn scope(row: &mut [Option<Term>], source: SourceId) {
    for term in row.iter_mut().flatten() {
        scope_term(term, source);
    }
}
fn scope_term(term: &mut Term, source: SourceId) {
    match term {
        Term::BlankNode(blank) => {
            let mut label = format!("sfj{}_", source.index());
            use std::fmt::Write as _;
            for byte in blank.as_str().bytes() {
                write!(&mut label, "{byte:02x}").unwrap();
            }
            *blank = BlankNode::new_unchecked(label);
        }
        Term::Triple(triple) => {
            let mut subject: Term = triple.subject.clone().into();
            scope_term(&mut subject, source);
            triple.subject = subject
                .try_into()
                .expect("blank scoping preserves subject type");
            scope_term(&mut triple.object, source);
        }
        Term::Literal(literal) => {
            if let Some(language) = literal.language() {
                let normalized = language.to_ascii_lowercase();
                if normalized != language {
                    *literal = match literal.direction() {
                        Some(direction) => {
                            oxrdf::Literal::new_directional_language_tagged_literal_unchecked(
                                literal.value(),
                                normalized,
                                direction,
                            )
                        }
                        None => oxrdf::Literal::new_language_tagged_literal_unchecked(
                            literal.value(),
                            normalized,
                        ),
                    };
                }
            }
        }
        _ => {}
    }
}
