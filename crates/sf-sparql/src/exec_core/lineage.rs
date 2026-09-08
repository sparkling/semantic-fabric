//! Exact bounded RDF witness evaluation for the opt-in positive lineage plan.
use super::row::Bindings;
use crate::lineage::{LineageSpec, Modifier, Node, MAX_WITNESSES};
use crate::{Error, Plan, PlanForm, Result};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_core::{Term, Triple};
use sf_sql::SqlBackend;
use spargebra::term::TermPattern;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub enum LineageOutput {
    Row(Vec<Option<Term>>),
    Graph(Vec<Triple>),
}
pub struct LineageSolution {
    pub output: LineageOutput,
    pub mappings: u64,
}

#[derive(Clone, Default)]
struct Witness {
    values: BTreeMap<String, Term>,
    arm: Vec<u8>,
    origins: u64,
}
#[derive(Default)]
struct Relation {
    rows: Vec<Witness>,
    keys: HashMap<u64, Vec<usize>>,
}

fn row_bytes(row: &Witness) -> Option<u64> {
    row.values
        .iter()
        .try_fold(512 + row.arm.len() as u64, |total, (key, term)| {
            total
                .checked_add(key.len() as u64)?
                .checked_add(super::row::term_payload_bytes(term)?)?
                .checked_add(256)
        })
}
fn scratch(bytes: Option<u64>, peak: &mut u64, control: &dyn QueryControl) -> Result<()> {
    let bytes = bytes.ok_or_else(|| stop(control))?;
    if bytes > *peak {
        control.consume(QueryCharge::RetainedBytes, bytes - *peak)?;
        *peak = bytes;
    }
    Ok(())
}

fn stop(control: &dyn QueryControl) -> Error {
    Error::QueryControl(control.terminate(QueryControlError::RetainedBytesExceeded))
}
fn insert(relation: &mut Relation, row: Witness, control: &dyn QueryControl) -> Result<()> {
    control.checkpoint()?;
    // Hash borrowed terms. Hash collisions use exact borrowed RDF equality;
    // neither lookup nor a duplicate witness allocates a second source-sized key.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    row.arm.hash(&mut hasher);
    row.values.hash(&mut hasher);
    let key = hasher.finish();
    if let Some(indices) = relation.keys.get(&key) {
        for &index in indices {
            control.checkpoint()?;
            if relation.rows[index].arm == row.arm && relation.rows[index].values == row.values {
                relation.rows[index].origins |= row.origins;
                return Ok(());
            }
        }
    }
    if relation.rows.len() >= MAX_WITNESSES {
        return Err(stop(control));
    }
    let bytes = row_bytes(&row).ok_or_else(|| stop(control))?;
    control.consume(QueryCharge::RetainedBytes, bytes)?;
    relation
        .keys
        .entry(key)
        .or_default()
        .push(relation.rows.len());
    relation.rows.push(row);
    Ok(())
}

fn bind(pattern: &TermPattern, term: &Term, values: &mut BTreeMap<String, Term>) -> bool {
    let key = match pattern {
        TermPattern::NamedNode(node) => {
            return matches!(term,Term::NamedNode(value) if value == node)
        }
        TermPattern::Literal(value) => return matches!(term,Term::Literal(other) if value == other),
        TermPattern::Variable(variable) => format!("v:{}", variable.as_str()),
        TermPattern::BlankNode(node) => format!("b:{}", node.as_str()),
        TermPattern::Triple(_) => return false,
    };
    match values.get(&key) {
        Some(existing) => existing == term,
        None => {
            values.insert(key, term.clone());
            true
        }
    }
}

fn evaluate(node: &Node, atoms: &mut [Relation], control: &dyn QueryControl) -> Result<Relation> {
    control.checkpoint()?;
    match node {
        Node::Atom(index) => Ok(std::mem::take(&mut atoms[*index])),
        Node::Union(left, right) => {
            let mut out = Relation::default();
            for (side, node) in [(0, left), (1, right)] {
                for mut row in evaluate(node, atoms, control)?.rows {
                    row.arm.insert(0, side);
                    insert(&mut out, row, control)?;
                }
            }
            Ok(out)
        }
        Node::Join(left, right) => {
            let left = evaluate(left, atoms, control)?;
            let right = evaluate(right, atoms, control)?;
            let mut out = Relation::default();
            let mut candidate_peak = 0;
            for a in &left.rows {
                for b in &right.rows {
                    // Finite shared work budget also bounds local witness products.
                    control.consume(QueryCharge::SourceWork, 1)?;
                    if a.values
                        .iter()
                        .any(|(key, value)| b.values.get(key).is_some_and(|other| other != value))
                    {
                        continue;
                    }
                    scratch(
                        row_bytes(a).and_then(|n| n.checked_add(row_bytes(b)?)),
                        &mut candidate_peak,
                        control,
                    )?;
                    let mut row = a.clone();
                    row.values.extend(b.values.clone());
                    row.arm.extend(&b.arm);
                    row.origins |= b.origins;
                    insert(&mut out, row, control)?;
                }
            }
            Ok(out)
        }
    }
}

/// The raw plan and spec must be paired by their compiler/runtime owner. All
/// source rows are read once for their compiled atoms; there is no provenance
/// re-query. Duplicate witnesses are resolved before emitting final metadata.
pub async fn lineage_each_async_controlled<B, F, Fut>(
    plan: &Plan,
    spec: &LineageSpec,
    backend: &mut B,
    control: &dyn QueryControl,
    mut sink: F,
) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(LineageSolution) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if plan.branches.len() != spec.branches.len()
        || plan.distinct
        || plan.limit.is_some()
        || plan.offset != 0
        || !plan.order.is_empty()
        || plan.rust_group.is_some()
        || !plan.dedup_scopes.is_empty()
    {
        return Err(crate::lineage::unsupported());
    }
    // Validate all branch identities before any metadata/source operation.
    for (branch, (alias, _, _)) in plan.branches.iter().zip(&spec.branches) {
        if branch.core.len() != 1
            || branch.core[0].alias != *alias
            || !branch.opts.is_empty()
            || !branch.subplan_joins.is_empty()
            || branch.path.is_some()
            || branch.agg.is_some()
            || branch.distinct
        {
            return Err(crate::lineage::unsupported());
        }
    }
    if spec
        .modifiers
        .iter()
        .any(|m| matches!(m, Modifier::Slice(_, Some(0))))
    {
        return Ok(());
    }
    let mut atoms: Vec<Relation> = (0..spec.atoms.len()).map(|_| Relation::default()).collect();
    let mut candidate_peak = 0;
    super::driver::for_each_solution_controlled(plan, backend, control, |branch, bindings| {
        let &(_, atom, origins) = spec
            .branches
            .iter()
            .find(|(alias, _, _)| *alias == branch.core[0].alias)
            .ok_or_else(crate::lineage::unsupported)?;
        let pattern = &spec.atoms[atom];
        if let (Some(subject), Some(object)) = (bindings.get("s"), bindings.get("o")) {
            if matches!(subject, Term::NamedNode(_) | Term::BlankNode(_)) {
                scratch(
                    bindings
                        .retained_payload_bytes()
                        .and_then(|n| n.checked_add(spec.query_bytes))
                        .and_then(|n| n.checked_add(4096)),
                    &mut candidate_peak,
                    control,
                )?;
                let mut row = Witness {
                    origins,
                    ..Witness::default()
                };
                if bind(&pattern.subject, subject, &mut row.values)
                    && bind(&pattern.object, object, &mut row.values)
                {
                    insert(&mut atoms[atom], row, control)?;
                }
            }
        }
        Ok(std::future::ready(Ok(())))
    })
    .await?;
    let mut rows = evaluate(&spec.root, &mut atoms, control)?.rows;
    for modifier in spec.modifiers.iter().rev() {
        control.checkpoint()?;
        match modifier {
            Modifier::Project(vars) => {
                for row in &mut rows {
                    row.values.retain(|key, _| {
                        key.strip_prefix("v:")
                            .is_some_and(|v| vars.iter().any(|var| var == v))
                    });
                }
            }
            Modifier::Distinct => {
                let mut distinct = Relation::default();
                for mut row in rows {
                    row.arm.clear();
                    insert(&mut distinct, row, control)?;
                }
                rows = distinct.rows;
            }
            Modifier::Slice(start, length) => {
                rows = rows
                    .into_iter()
                    .skip(*start)
                    .take(length.unwrap_or(usize::MAX))
                    .collect()
            }
        }
    }
    let mut output_peak = 0;
    for (ordinal, row) in rows.into_iter().enumerate() {
        control.checkpoint()?;
        let output = match &plan.form {
            PlanForm::Select { vars } => {
                scratch(
                    row_bytes(&row)
                        .and_then(|n| n.checked_add(spec.query_bytes))
                        .and_then(|n| n.checked_mul(vars.len() as u64 + 1)),
                    &mut output_peak,
                    control,
                )?;
                control.consume(QueryCharge::ResultItems, 1)?;
                LineageOutput::Row(
                    vars.iter()
                        .map(|var| row.values.get(&format!("v:{var}")).cloned())
                        .collect(),
                )
            }
            PlanForm::Construct { template } => {
                // Reserve a conservative expansion before any template term is
                // cloned. It includes repeated/nested variables and constants.
                scratch(
                    row_bytes(&row)
                        .and_then(|n| n.checked_add(spec.query_bytes))
                        .and_then(|n| n.checked_mul(spec.output_nodes + 1)),
                    &mut output_peak,
                    control,
                )?;
                let mut bindings = Bindings::new();
                for (key, value) in row.values {
                    if let Some(var) = key.strip_prefix("v:") {
                        bindings.insert(Arc::from(var), value);
                    }
                }
                let triples: Vec<_> = template
                    .iter()
                    .filter_map(|pattern| {
                        super::template::instantiate(pattern, &bindings, ordinal as u64)
                    })
                    .collect();
                control.consume(QueryCharge::ResultItems, triples.len() as u64)?;
                LineageOutput::Graph(triples)
            }
            _ => return Err(crate::lineage::unsupported()),
        };
        sink(LineageSolution {
            output,
            mappings: row.origins,
        })
        .await?;
    }
    control.checkpoint()?;
    Ok(())
}
