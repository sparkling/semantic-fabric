//! NULL-safe condition construction under the same request work owner.
use crate::build::control::{BuildVec, BuildWork};
use crate::iq::iri_cmp::{IriOperand, IriPart};
use crate::iq::literal_cmp::LiteralOperand;
use crate::iq::{CmpOp, ColRef, SqlCond};
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::{CompilerWorkMode, Result};
use sf_core::ir::Segment;

fn copy_column(column: &ColRef, work: BuildWork<'_>) -> Result<ColRef> {
    if let CompilerWorkMode::Metered(cx) = work.mode {
        cx.reserve_ast_copy(CompilerCloneRootV1::ColRef(column))?;
    }
    let out = column.clone();
    work.checkpoint()?;
    Ok(out)
}

fn push_column(out: &mut BuildVec<SqlCond>, column: &ColRef, work: BuildWork<'_>) -> Result<()> {
    work.push(out, SqlCond::IsNull(copy_column(column, work)?))
}

fn iri_columns(
    operand: &IriOperand,
    out: &mut BuildVec<SqlCond>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    match operand {
        IriOperand::Column { column, .. } => push_column(out, column, work)?,
        IriOperand::Template { parts, .. } => {
            // Do not use columns(): it skips literal segments without yielding
            // control, so a wide constant-only template could evade the charge.
            for part in parts {
                work.charge(1)?;
                if let IriPart::Column(column) = part {
                    push_column(out, column, work)?;
                }
            }
        }
        IriOperand::Constant(_) => (),
    }
    Ok(())
}

pub(crate) fn null_safe(
    condition: SqlCond,
    nullable: bool,
    mode: CompilerWorkMode<'_>,
) -> Result<SqlCond> {
    if matches!(mode, CompilerWorkMode::Uncontrolled) {
        return Ok(super::null_safe(condition, nullable));
    }
    let work = BuildWork::new(mode);
    work.charge(1)?;
    if !nullable {
        return Ok(condition);
    }
    let mut out = BuildVec::new(Vec::new());
    // Reserve a first slot before collecting guards. Replace this trivial
    // placeholder with the owned condition afterward: no shift or shadow copy.
    match &condition {
        SqlCond::IriCmp(cmp) => {
            work.push(&mut out, SqlCond::ExpressionError)?;
            iri_columns(&cmp.left, &mut out, work)?;
            iri_columns(&cmp.right, &mut out, work)?;
        }
        SqlCond::LiteralCmp(cmp) if cmp.value_op.is_none() => {
            work.push(&mut out, SqlCond::ExpressionError)?;
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                if let LiteralOperand::Column { column, .. } = operand {
                    push_column(&mut out, column, work)?;
                }
            }
        }
        SqlCond::ColEq(..) => {
            let SqlCond::ColEq(left, right) = condition else {
                unreachable!()
            };
            return Ok(SqlCond::NullSafeEq(left, right));
        }
        SqlCond::Cmp(column, CmpOp::Eq, _) => {
            work.push(&mut out, SqlCond::ExpressionError)?;
            push_column(&mut out, column, work)?;
        }
        SqlCond::TemplateEq(left, a, right, b, _) => {
            work.push(&mut out, SqlCond::ExpressionError)?;
            for (parts, alias) in [(left, a), (right, b)] {
                for part in parts {
                    work.charge(1)?;
                    if let Segment::Column(name) = part {
                        work.charge(std::mem::size_of::<ColRef>())?;
                        work.charge(name.len())?;
                        let column = ColRef::new(*alias, name.clone());
                        work.checkpoint()?;
                        work.push(&mut out, SqlCond::IsNull(column))?;
                    }
                }
            }
        }
        _ => return Ok(condition),
    }
    out.values[0] = condition;
    Ok(SqlCond::Or(out.into_inner()))
}
