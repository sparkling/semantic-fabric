//! Join substitutions: absent keys rebind freely, shared keys retain the raw
//! unifier's Sat/Empty/Unsupported semantics and ordered equality conditions.
use super::control::{RowVec, RowWork};
use crate::iq::node::{BindDef, IqCond, Var};
use crate::unify::{unify, Unify};
use crate::{CompilerWorkMode, Error, Result};
use std::collections::BTreeMap;

pub(super) fn merge_into(
    acc: &mut BTreeMap<Var, BindDef>,
    incoming: BTreeMap<Var, BindDef>,
    eqs: &mut RowVec<IqCond>,
    work: RowWork<'_>,
) -> Result<bool> {
    work.charge(1)?;
    for (var, rdef) in incoming {
        work.map_access(acc, &var)?;
        match acc.get(&var) {
            None => {
                work.map_entry::<BindDef>()?;
                acc.insert(var, rdef);
                work.checkpoint()?;
            }
            Some(ldef) => match (ldef, &rdef) {
                (BindDef::Resolved(l), BindDef::Resolved(r)) => {
                    let unified = match work.mode {
                        CompilerWorkMode::Uncontrolled => unify(l, r),
                        CompilerWorkMode::Metered(cx) => cx.unify_terms(l, r)?,
                    };
                    match unified {
                        Unify::Sat(conds) => {
                            for cond in conds {
                                work.push(eqs, IqCond::Sql(cond))?;
                            }
                        }
                        Unify::Empty => return Ok(true),
                        Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                    }
                }
                _ => {
                    return Err(work.unsupported(
                        "normalize: shared join variable has a symbolic BIND definition → 501",
                        || {
                            format!(
                                "normalize: shared join variable ?{var} has a symbolic BIND \
                         definition on one side → 501 (deferred, never silently dropped)"
                            )
                        },
                    )?)
                }
            },
        }
    }
    work.checkpoint()?;
    Ok(false)
}
