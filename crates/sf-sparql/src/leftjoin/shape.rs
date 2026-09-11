//! Initial OPTIONAL dispatch: preserve short-circuits while paying each visit.
use std::cmp::Ordering;

use crate::build::control::BuildWork;
use crate::iq::{Branch, ScanSource, TermDef};
use crate::{CompilerWorkMode, Error, Result};

fn has_options(right: &[Branch], work: BuildWork<'_>) -> Result<bool> {
    for branch in right {
        work.charge(1)?;
        if !branch.opts.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn right_has_options(right: &[Branch], mode: CompilerWorkMode<'_>) -> Result<bool> {
    let _span = tracing::debug_span!("sf.compiler.optional_shape").entered();
    let work = BuildWork::new(mode);
    work.charge(1)?;
    has_options(right, work)
}

pub(crate) fn select_fast(
    left: &[Branch],
    right: &[Branch],
    mode: CompilerWorkMode<'_>,
) -> Result<bool> {
    let _span = tracing::debug_span!("sf.compiler.optional_shape").entered();
    let work = BuildWork::new(mode);
    work.charge(1)?;
    // Opts rejection precedes all other shape checks, as in the raw dispatch.
    if has_options(right, work)? {
        return Err(Error::Unsupported(work.string(
            "nested OPTIONAL inside an OPTIONAL right side is deferred → 501 (ADR-0007)",
        )?));
    }
    work.charge(1)?;
    if right.len() != 1 || right[0].core.len() != 1 {
        return Ok(false);
    }
    // A new mapping constant has no nullable witness for an absent optional
    // row. Keep decomposition unless every left branch also binds a Const.
    // This checks the variant, NOT equality of the constant values.
    for (name, def) in &right[0].bindings {
        work.charge(1)?;
        if matches!(def, TermDef::Const(_)) {
            for branch in left {
                work.charge(1)?;
                let mut is_constant = false;
                for (key, value) in &branch.bindings {
                    work.charge(1)?;
                    work.charge(key.len().min(name.len()))?;
                    match key.cmp(name) {
                        Ordering::Equal => {
                            is_constant = matches!(value, TermDef::Const(_));
                            break;
                        }
                        Ordering::Greater => break,
                        Ordering::Less => (),
                    }
                }
                if !is_constant {
                    return Ok(false);
                }
            }
        }
    }
    work.charge(1)?;
    // A sealed Ref represents the former two-scan relation. Preserve its
    // decomposition; an OptJoin would strand nested OPTIONALs.
    Ok(!matches!(
        right[0].core[0].source,
        ScanSource::RefAtom { .. }
    ))
}
