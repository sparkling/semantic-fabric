//! Deterministic first-occurrence order, with paid scans and fresh names.
use spargebra::term::Variable;

use crate::build::control::{BuildVec, BuildWork};
use crate::Result;

#[derive(Clone, Copy)]
pub(super) enum Role {
    Preserved,
    Use,
    Aggregate,
    DescribeBind,
    DescribeProject,
}

struct Name {
    original: String,
    aggregate: usize,
    describe_bind: usize,
    describe_project: usize,
    uses: usize,
    preserved: bool,
    replacement: Option<String>,
}

pub(super) struct Names<'a> {
    work: BuildWork<'a>,
    names: BuildVec<Name>,
}

impl<'a> Names<'a> {
    pub fn new(work: BuildWork<'a>) -> Self {
        Self {
            work,
            names: BuildVec::new(Vec::new()),
        }
    }

    pub fn observe(&mut self, variable: &Variable, role: Role) -> Result<()> {
        let mut found = None;
        for (index, name) in self.names.values.iter().enumerate() {
            if equal(self.work, &name.original, variable.as_str())? {
                found = Some(index);
                break;
            }
        }
        let index = if let Some(index) = found {
            index
        } else {
            let index = self.names.values.len();
            let original = self.work.string(variable.as_str())?;
            self.work.push(
                &mut self.names,
                Name {
                    original,
                    aggregate: 0,
                    describe_bind: 0,
                    describe_project: 0,
                    uses: 0,
                    preserved: false,
                    replacement: None,
                },
            )?;
            index
        };
        // All occurrence counters are bounded by the validated AST node count.
        let name = &mut self.names.values[index];
        match role {
            Role::Preserved => name.preserved = true,
            Role::Use => name.uses += 1,
            Role::Aggregate => name.aggregate += 1,
            Role::DescribeBind => name.describe_bind += 1,
            Role::DescribeProject => name.describe_project += 1,
        }
        Ok(())
    }

    pub fn assign(&mut self) -> Result<bool> {
        let mut next = 0usize;
        let mut changed = false;
        for index in 0..self.names.values.len() {
            self.work.charge(1)?;
            let name = &self.names.values[index];
            let aggregate =
                name.aggregate == 1 && name.describe_bind == 0 && name.describe_project == 0;
            let describe = name.aggregate == 0
                && name.describe_bind == 1
                && name.describe_project == 1
                && name.uses == 0;
            if name.preserved || !(aggregate || describe) {
                continue;
            }
            let replacement = loop {
                // At most (original names + eligible names) candidates are
                // needed. Decimal rendering uses fixed stack storage.
                self.work.charge(32)?;
                let mut bytes = [0u8; 32];
                let mut cursor = bytes.len();
                let mut digits = next;
                loop {
                    cursor -= 1;
                    bytes[cursor] = b'0' + (digits % 10) as u8;
                    digits /= 10;
                    if digits == 0 {
                        break;
                    }
                }
                cursor -= b"__sf_cache".len();
                bytes[cursor..cursor + b"__sf_cache".len()].copy_from_slice(b"__sf_cache");
                let candidate = self
                    .work
                    .string(std::str::from_utf8(&bytes[cursor..]).expect("ASCII name"))?;
                next += 1;
                self.work.checkpoint()?;
                let mut collision = false;
                for name in &self.names.values {
                    if equal(self.work, &name.original, &candidate)? {
                        collision = true;
                        break;
                    }
                }
                if !collision {
                    break candidate;
                }
            };
            self.names.values[index].replacement = Some(replacement);
            changed = true;
        }
        Ok(changed)
    }

    pub fn rename(&self, variable: &mut Variable) -> Result<()> {
        for name in &self.names.values {
            if equal(self.work, &name.original, variable.as_str())? {
                if let Some(replacement) = &name.replacement {
                    let value = self.work.string(replacement)?;
                    *variable = Variable::new_unchecked(value);
                    self.work.checkpoint()?;
                }
                break;
            }
        }
        Ok(())
    }
}

fn equal(work: BuildWork<'_>, left: &str, right: &str) -> Result<bool> {
    work.charge(1)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    work.charge(left.len())?;
    let equal = left == right;
    work.checkpoint()?;
    Ok(equal)
}
