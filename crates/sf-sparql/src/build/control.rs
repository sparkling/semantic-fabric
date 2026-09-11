//! Operation-local compiler work. BUILD and constant-expression materialization
//! use the depth counter; row loops reuse the allocation/copy primitives alone.
//! Neither use establishes a whole-compiler or allocator-size proof.
use sf_core::query_control::QueryControlError;
use spargebra::algebra::{Expression, PropertyPathExpression};
use spargebra::term::{
    GroundTerm, Literal, NamedNode, NamedNodePattern, TermPattern, TriplePattern,
};

use crate::iq::node::Var;
use crate::plan_measure::clone_root::CompilerCloneRootV1;
use crate::{CompilerWorkMode, Error, Result};

#[derive(Clone, Copy)]
pub(crate) struct BuildWork<'a> {
    pub mode: CompilerWorkMode<'a>,
    depth: usize,
}

/// Track paid logical slots independently of allocator over-allocation.
pub(crate) struct BuildVec<T> {
    pub values: Vec<T>,
    requested: usize,
}

impl<T> BuildVec<T> {
    pub fn new(values: Vec<T>) -> Self {
        Self {
            requested: values.len(),
            values,
        }
    }

    pub fn into_inner(self) -> Vec<T> {
        self.values
    }
}

impl<'a> BuildWork<'a> {
    pub fn new(mode: CompilerWorkMode<'a>) -> Self {
        Self { mode, depth: 0 }
    }

    pub fn enter(self) -> Result<Self> {
        self.charge(1)?;
        let next = Self {
            depth: self.depth + 1,
            ..self
        };
        if let CompilerWorkMode::Metered(cx) = self.mode {
            if next.depth > crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1 {
                return Err(cx.reject_build_resource(QueryControlError::CompilerEnvelopeExceeded));
            }
        }
        Ok(next)
    }

    pub fn checkpoint(self) -> Result<()> {
        match self.mode {
            CompilerWorkMode::Uncontrolled => Ok(()),
            CompilerWorkMode::Metered(cx) => cx.checkpoint(),
        }
    }

    pub fn charge(self, units: usize) -> Result<()> {
        if let CompilerWorkMode::Metered(cx) = self.mode {
            cx.checkpoint()?;
            cx.reserve_checked_product(&[units])?;
            cx.checkpoint()?;
        }
        Ok(())
    }

    /// Logical vector payload is prepaid before allocation; allocator overgrant
    /// and derived Clone's infallible allocations are separate remaining limits.
    pub fn vector<T>(self, slots: usize) -> Result<Vec<T>> {
        if let CompilerWorkMode::Metered(cx) = self.mode {
            self.charge(slots)?;
            cx.reserve_checked_product(&[slots, std::mem::size_of::<T>()])?;
            cx.checkpoint()?;
            let mut out = Vec::new();
            out.try_reserve_exact(slots).map_err(|_| {
                cx.reject_build_resource(QueryControlError::CompilerResourceExhausted)
            })?;
            self.checkpoint()?;
            Ok(out)
        } else {
            Ok(Vec::with_capacity(slots))
        }
    }

    /// Pay logical geometric growth and relocation before appending to a vector
    /// whose final size is not known without a second traversal.
    pub fn push<T>(self, out: &mut BuildVec<T>, value: T) -> Result<()> {
        self.charge(1)?;
        if out.values.len() == out.requested {
            if let CompilerWorkMode::Metered(cx) = self.mode {
                let target = out
                    .requested
                    .checked_mul(2)
                    .ok_or_else(|| cx.reject_build_resource(QueryControlError::AccountingOverflow))?
                    .max(1);
                cx.reserve_checked_product(&[target, std::mem::size_of::<T>()])?;
                cx.reserve_checked_product(&[out.values.len(), std::mem::size_of::<T>()])?;
                cx.checkpoint()?;
                out.values
                    .try_reserve_exact(target - out.values.len())
                    .map_err(|_| {
                        cx.reject_build_resource(QueryControlError::CompilerResourceExhausted)
                    })?;
                self.checkpoint()?;
                out.requested = target;
            }
        }
        out.values.push(value);
        self.checkpoint()?;
        Ok(())
    }

    pub fn boxed<T>(self, value: T) -> Result<Box<T>> {
        self.charge(std::mem::size_of::<T>())?;
        let out = Box::new(value);
        self.checkpoint()?;
        Ok(out)
    }

    pub fn copied<T: CopySource>(self, source: &T) -> Result<T> {
        if let CompilerWorkMode::Metered(cx) = self.mode {
            cx.reserve_ast_copy(source.root())?;
        }
        let result = source.clone();
        self.checkpoint()?;
        Ok(result)
    }

    pub fn variable(self, value: &str) -> Result<Var> {
        self.charge(1)?;
        self.charge(value.len())?;
        let out = value.into();
        self.checkpoint()?;
        Ok(out)
    }

    pub fn string(self, value: &str) -> Result<String> {
        self.charge(1)?;
        self.charge(value.len())?;
        let out = value.to_owned();
        self.checkpoint()?;
        Ok(out)
    }

    pub fn variables<'v>(self, values: impl ExactSizeIterator<Item = &'v str>) -> Result<Vec<Var>> {
        let mut out = self.vector(values.len())?;
        for value in values {
            out.push(self.variable(value)?);
        }
        Ok(out)
    }

    pub fn contains(self, values: &[Var], candidate: &str) -> Result<bool> {
        for value in values {
            self.charge(1)?;
            if value.len() == candidate.len() {
                self.charge(candidate.len())?;
                if value.as_ref() == candidate {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn unique(self, out: &mut BuildVec<Var>, value: Var) -> Result<()> {
        if !self.contains(&out.values, &value)? {
            self.push(out, value)?;
        }
        Ok(())
    }

    pub fn unsupported(self, message: &'static str, raw: impl FnOnce() -> String) -> Result<Error> {
        Ok(Error::Unsupported(match self.mode {
            CompilerWorkMode::Uncontrolled => raw(),
            CompilerWorkMode::Metered(_) => self.string(message)?,
        }))
    }
}

/// Closed typed roots: callers cannot measure one carrier and copy another.
pub(crate) trait CopySource: Clone {
    fn root(&self) -> CompilerCloneRootV1<'_>;
}
macro_rules! copy_source {
    ($($ty:ty => $variant:ident),+ $(,)?) => { $(
        impl CopySource for $ty {
            fn root(&self) -> CompilerCloneRootV1<'_> { CompilerCloneRootV1::$variant(self) }
        }
    )+ };
}
copy_source! {
    Expression => Expression, PropertyPathExpression => PropertyPath,
    TriplePattern => TriplePattern, TermPattern => TermPattern,
    NamedNodePattern => NamedNodePattern, GroundTerm => GroundTerm,
    NamedNode => NamedNode, Literal => Literal,
}
