//! Normalization reuses the existing logical allocation/copy schedule.
//! Bounds here are operation-local: derived drop and physical allocator behavior
//! remain separate from the prepaid row/slot/byte work.
use std::cmp::Ordering;

pub(super) use crate::build::control::{BuildVec as RowVec, BuildWork as RowWork};
use crate::iq::node::Var;
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Result};

impl RowWork<'_> {
    pub(super) fn insert<T>(self, out: &mut RowVec<T>, index: usize, value: T) -> Result<()> {
        self.moved::<T>(out.values.len() - index)?;
        self.push(out, value)?;
        out.values[index..].rotate_right(1);
        self.checkpoint()
    }

    pub(super) fn map_access<T>(
        self,
        map: &std::collections::BTreeMap<Var, T>,
        key: &str,
    ) -> Result<()> {
        // Bound a get followed by an insert, independently of private BTree
        // search geometry. Both may compare each existing key at most once.
        self.charge(1)?;
        for existing in map.keys() {
            self.charge(2)?;
            self.charge(existing.len())?;
            self.charge(existing.len())?;
            self.charge(key.len())?;
            self.charge(key.len())?;
        }
        self.checkpoint()
    }

    pub(super) fn map_entry<T>(self) -> Result<()> {
        self.charge(1)?;
        self.charge(std::mem::size_of::<(Var, T)>())?;
        self.checkpoint()
    }

    pub(super) fn vars_equal(self, left: &[Var], right: &[Var]) -> Result<bool> {
        self.charge(1)?;
        if left.len() != right.len() {
            return Ok(false);
        }
        for (a, b) in left.iter().zip(right) {
            if !self.contains(std::slice::from_ref(a), b)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn name_order(self, left: &str, right: &str) -> Result<Ordering> {
        self.charge(1)?;
        self.charge(left.len())?;
        self.charge(right.len())?;
        let order = left.cmp(right);
        self.checkpoint()?;
        Ok(order)
    }

    pub(super) fn moved<T>(self, slots: usize) -> Result<()> {
        self.charge(slots)?;
        if let CompilerWorkMode::Metered(cx) = self.mode {
            cx.reserve_checked_product(&[slots, std::mem::size_of::<T>()])?;
        }
        self.checkpoint()
    }

    pub(super) fn append<T>(self, out: &mut RowVec<T>, values: Vec<T>) -> Result<()> {
        for value in values {
            self.push(out, value)?;
        }
        self.checkpoint()
    }

    pub(super) fn term_equal(self, left: &TermDef, right: &TermDef) -> Result<bool> {
        match self.mode {
            CompilerWorkMode::Metered(cx) => cx.constant_terms_equal(left, right),
            CompilerWorkMode::Uncontrolled => Ok(matches!((left, right),
                (TermDef::Const(a), TermDef::Const(b)) if a == b)),
        }
    }
}
