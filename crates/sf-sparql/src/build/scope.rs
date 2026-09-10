//! Meter the same stable output-variable scope BUILD already used.
use super::control::{BuildVec, BuildWork};
use crate::iq::node::{IqNode, Var};
use crate::{CompilerWorkMode, Result};
use spargebra::term::{NamedNodePattern, TermPattern};

pub(super) fn output_vars(node: &IqNode, work: BuildWork<'_>) -> Result<Vec<Var>> {
    if matches!(work.mode, CompilerWorkMode::Uncontrolled) {
        return Ok(node.output_vars());
    }
    let work = work.enter()?;
    match node {
        IqNode::Construction { project, .. }
        | IqNode::Union { project, .. }
        | IqNode::Values { vars: project, .. }
        | IqNode::Empty { vars: project } => work.variables(project.iter().map(|s| s.as_ref())),
        IqNode::Filter { child, .. }
        | IqNode::Distinct { child }
        | IqNode::Slice { child, .. }
        | IqNode::OrderBy { child, .. } => output_vars(child, work),
        IqNode::InnerJoin { children, .. } => {
            let mut out = BuildVec::new(Vec::new());
            for child in children {
                work.charge(1)?;
                for v in output_vars(child, work)? {
                    work.unique(&mut out, v)?;
                }
            }
            Ok(out.into_inner())
        }
        IqNode::LeftJoin { left, right, .. } => {
            let mut out = BuildVec::new(output_vars(left, work)?);
            for v in output_vars(right, work)? {
                work.unique(&mut out, v)?;
            }
            Ok(out.into_inner())
        }
        IqNode::Aggregation { grouping, aggs, .. } => {
            let mut out = BuildVec::new(Vec::new());
            for v in grouping {
                work.charge(1)?;
                work.unique(&mut out, work.variable(v)?)?;
            }
            for a in aggs {
                work.charge(1)?;
                work.unique(&mut out, work.variable(&a.var)?)?;
            }
            Ok(out.into_inner())
        }
        IqNode::Extensional { bind, .. } => work.variables(bind.keys().map(|s| s.as_ref())),
        IqNode::Intensional { pattern, graph } => {
            let mut out = BuildVec::new(Vec::new());
            term(&mut out, &pattern.subject, work)?;
            named(&mut out, Some(&pattern.predicate), work)?;
            term(&mut out, &pattern.object, work)?;
            named(&mut out, graph.as_ref(), work)?;
            Ok(out.into_inner())
        }
        IqNode::UnresolvedPath {
            subject,
            object,
            graph,
            ..
        } => {
            let mut out = BuildVec::new(Vec::new());
            term(&mut out, subject, work)?;
            term(&mut out, object, work)?;
            named(&mut out, graph.as_ref(), work)?;
            Ok(out.into_inner())
        }
        IqNode::True | IqNode::Path { .. } => Ok(Vec::new()),
    }
}

fn term(out: &mut BuildVec<Var>, term: &TermPattern, work: BuildWork<'_>) -> Result<()> {
    work.charge(1)?;
    if let TermPattern::Variable(v) = term {
        work.unique(out, work.variable(v.as_str())?)?;
    }
    Ok(())
}
fn named(
    out: &mut BuildVec<Var>,
    term: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    if let Some(NamedNodePattern::Variable(v)) = term {
        work.unique(out, work.variable(v.as_str())?)?;
    }
    Ok(())
}
