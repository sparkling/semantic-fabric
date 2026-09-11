//! R2 output carriers. Callers reserve copies from the exact borrowed right
//! branch before these helpers; map/box/vector growth is paid separately here.
use std::collections::BTreeMap;

use super::work;
use crate::build::control::{BuildVec, BuildWork};
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Result};

pub(crate) fn insert_copied(
    bindings: &mut BTreeMap<String, TermDef>,
    name: &str,
    right: &TermDef,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let _span = tracing::debug_span!("sf.compiler.optional_bindings").entered();
    work::binding_edit(mode, bindings, name)?;
    bindings.insert(name.to_owned(), right.clone());
    BuildWork::new(mode).checkpoint()
}

/// Fast/matched branches already own their left definition. Move it only after
/// the FILTER has consumed the original left-preferred view (ADR-0007 R5).
pub(crate) fn coalesce_owned(
    bindings: &mut BTreeMap<String, TermDef>,
    name: &str,
    right: &TermDef,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let _span = tracing::debug_span!("sf.compiler.optional_bindings").entered();
    let build = BuildWork::new(mode);
    work::binding_edit(mode, bindings, name)?;
    let (name, left) = bindings
        .remove_entry(name)
        .expect("nullable shared binding came from the left branch");
    let value = TermDef::Coalesce(build.boxed(left)?, build.boxed(right.clone())?);
    work::binding_edit(mode, bindings, &name)?;
    bindings.insert(name, value);
    build.checkpoint()
}

/// Preserve the pure-SubPlan helper's existing, source-bound scalar copy
/// admission. No unrelated whole-left-branch reservation is borrowed here.
pub(crate) fn coalesce_copied(
    bindings: &mut BTreeMap<String, TermDef>,
    name: &str,
    right: &TermDef,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let _span = tracing::debug_span!("sf.compiler.optional_bindings").entered();
    let build = BuildWork::new(mode);
    work::binding_edit(mode, bindings, name)?;
    let left = bindings
        .get(name)
        .expect("nullable shared binding came from the left branch");
    let left = match mode {
        CompilerWorkMode::Uncontrolled => left.clone(),
        CompilerWorkMode::Metered(cx) => cx.clone_optional_term_def(left)?,
    };
    let value = TermDef::Coalesce(build.boxed(left)?, build.boxed(right.clone())?);
    work::binding_edit(mode, bindings, name)?;
    bindings.insert(name.to_owned(), value);
    build.checkpoint()
}

/// The enclosing exact-source reservation pays each source payload clone once.
/// This pays output visits, logical growth and relocation without borrowing
/// spare allocator capacity; it does not claim a physical-heap byte bound.
pub(crate) fn extend_copied<T: Clone>(
    values: &mut Vec<T>,
    source: &[T],
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let work = BuildWork::new(mode);
    let mut out = BuildVec::new(std::mem::take(values));
    for value in source {
        work.push(&mut out, value.clone())?;
    }
    *values = out.into_inner();
    work.checkpoint()
}
