//! Keep source affinity and actual origin matching on the same bounded compiler.
use super::*;

pub(super) fn compile(
    sparql: &str,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
) -> Result<FederatedPlan> {
    if bindings[0].source_id() == bindings[1].source_id() {
        return unsupported();
    }
    control.checkpoint()?;
    let query = SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::Parse(e.to_string()))?;
    let parsed = SourceAffineUnion::from_query(query)?;
    let mut fragments = Vec::with_capacity(2);
    for arm in parsed.arms() {
        let text = arm.query().to_string();
        let mut selected = None;
        for binding in bindings {
            let (plan, spec) = binding.compile_lineage(&text, control)?;
            // This includes the same subproperty/inverse and default-graph
            // semantics as witness execution, not raw predicate equality.
            if plan.branches.is_empty() {
                continue;
            }
            if selected.is_some() {
                return unsupported();
            }
            let mut fragment = SourceFragment::new(binding.source_id(), Arc::new(plan))?;
            fragment.lineage = Some(Arc::new(spec));
            selected = Some(fragment);
        }
        fragments
            .push(selected.ok_or_else(|| Error::Unsupported("lineage arm has no source".into()))?);
    }
    let right = fragments.pop().unwrap();
    let left = fragments.pop().unwrap();
    FederatedPlan::union_all(parsed.variables().to_vec(), [left, right])
}
