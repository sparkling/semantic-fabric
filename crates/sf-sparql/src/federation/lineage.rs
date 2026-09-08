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
    let query = crate::parse_query(sparql)?;
    if join::is_join(&query) {
        charge_join_inputs(sparql, bindings, control)?;
        return join::compile_lineage(query, bindings, control);
    }
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

/// Admit finite query/catalog traversal before the legacy join arm compiler.
/// This is request work accounting, not total parser/optimizer CPU governance.
fn charge_join_inputs(
    query: &str,
    bindings: [&CompilerBinding; 2],
    control: &dyn QueryControl,
) -> Result<()> {
    let meter = crate::compiler_control::CompileMeter::new(control);
    meter.reserve_work(meter.checked_usize(query.len())?)?;
    for binding in bindings {
        let tbox = binding.tbox();
        for edges in tbox
            .sub_classes
            .values()
            .chain(tbox.sub_properties.values())
        {
            meter.precharge_product(&[2, edges.len().saturating_add(1)])?;
        }
        for edges in tbox.inverses.values() {
            meter.precharge_product(&[2, edges.len().saturating_add(1)])?;
        }
        meter.reserve_work(meter.checked_usize(tbox.symmetric.len())?)?;
        for map in binding.triples_maps() {
            meter.reserve_work(2)?;
            meter.reserve_work(meter.checked_usize(map.subject.classes.len())?)?;
            for pom in &map.predicate_object_maps {
                meter.precharge_product(&[
                    2,
                    pom.predicates.len().saturating_add(1),
                    pom.objects.len().saturating_add(1),
                ])?;
            }
        }
    }
    Ok(())
}
