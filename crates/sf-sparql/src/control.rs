//! Cooperative controls for attacker-reachable query compilation.
//!
//! Library callers remain unrestricted unless they explicitly install a
//! [`CompileControl`]. Servers use one to stop abandoned blocking tasks and to
//! bound only plan-expansion work that could exhaust process memory.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;

use crate::{Error, Result};

/// A meaningful boundary in synchronous query compilation, retained internally
/// so direct receipts can prove that controls cover the full pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum CompileStage {
    Entry,
    Parse,
    Rewrite,
    Build,
    Resolve,
    Normalize,
    Lower,
    Cascade,
    CacheClone,
}

impl CompileStage {
    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

#[derive(Default)]
struct CompileState {
    cancelled: AtomicBool,
    reached: AtomicU16,
}

/// A clonable cancellation signal for synchronous planner code.
#[derive(Clone, Default)]
pub struct CompileCancellation {
    state: Arc<CompileState>,
}

impl CompileCancellation {
    /// Creates an active cancellation signal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cooperative cancellation. Repeated calls are harmless.
    pub fn cancel(&self) {
        self.state.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    /// Returns whether the controlled compilation crossed `stage`.
    #[cfg(test)]
    pub(crate) fn reached(&self, stage: CompileStage) -> bool {
        self.state.reached.load(Ordering::Acquire) & stage.bit() != 0
    }

    fn mark_reached(&self, stage: CompileStage) {
        self.state.reached.fetch_or(stage.bit(), Ordering::Release);
    }
}

/// Controls applied to one synchronous planner invocation.
#[derive(Clone)]
pub struct CompileControl {
    cancellation: CompileCancellation,
    max_expansion_work: usize,
}

impl CompileControl {
    /// Creates a control with an expansion-only memory-safety budget.
    ///
    /// The budget does not count input bytes, result rows, ordinary scans, or
    /// query execution. It counts nodes/branches materialized by planner
    /// rewrites and cross-products.
    pub fn new(cancellation: CompileCancellation, max_expansion_work: usize) -> Self {
        Self {
            cancellation,
            max_expansion_work,
        }
    }
}

struct ActiveControl {
    cancellation: CompileCancellation,
    remaining_expansion_work: usize,
}

thread_local! {
    static ACTIVE_CONTROL: RefCell<Option<ActiveControl>> = const { RefCell::new(None) };
}

struct RestoreControl(Option<ActiveControl>);

impl Drop for RestoreControl {
    fn drop(&mut self) {
        ACTIVE_CONTROL.with(|active| {
            *active.borrow_mut() = self.0.take();
        });
    }
}

/// Runs one planner call with cooperative cancellation and expansion controls.
///
/// Controls are thread-local so the existing recursive planner stays free of
/// plumbing on its unrestricted library path. Nested calls restore the prior
/// control on return or unwind.
pub fn with_compile_control<T>(
    control: CompileControl,
    compile: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let previous = ACTIVE_CONTROL.with(|active| {
        active.replace(Some(ActiveControl {
            cancellation: control.cancellation,
            remaining_expansion_work: control.max_expansion_work,
        }))
    });
    let _restore = RestoreControl(previous);
    checkpoint(CompileStage::Entry)?;
    compile()
}

/// Charges work that materializes planner nodes or branch products.
/// Uncontrolled library calls deliberately remain unrestricted.
pub(crate) fn charge_expansion_work(units: usize) -> Result<()> {
    ACTIVE_CONTROL.with(|active| {
        let mut active = active.borrow_mut();
        let Some(control) = active.as_mut() else {
            return Ok(());
        };
        if control.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        control.remaining_expansion_work = control
            .remaining_expansion_work
            .checked_sub(units)
            .ok_or_else(|| {
                Error::ResourceLimit(
                    "planner expansion exceeded its pre-allocation memory-safety budget".into(),
                )
            })?;
        Ok(())
    })
}

/// Returns whether the active compilation has been cancelled.
///
/// This non-failing check is used inside iterator adapters where propagating a
/// [`Result`] is not possible. Callers still checkpoint after the adapter so a
/// cancellation is returned to the user instead of being mistaken for an
/// empty result.
pub(crate) fn is_cancelled() -> bool {
    ACTIVE_CONTROL.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|control| control.cancellation.is_cancelled())
    })
}

/// Checks cancellation without consuming expansion budget.
pub(crate) fn checkpoint(stage: CompileStage) -> Result<()> {
    ACTIVE_CONTROL.with(|active| {
        if let Some(control) = active.borrow().as_ref() {
            control.cancellation.mark_reached(stage);
            if control.cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPING: &str = r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        @prefix ex: <http://example.com/> .

        <#People> a rr:TriplesMap ;
            rr:logicalTable [ rr:tableName "people" ] ;
            rr:subjectMap [ rr:template "http://example.com/person/{id}" ] ;
            rr:predicateObjectMap [
                rr:predicate ex:name ;
                rr:objectMap [ rr:column "name" ]
            ] .
    "#;

    #[test]
    fn controlled_work_stops_at_budget_and_restores_unrestricted_mode() {
        let control = CompileControl::new(CompileCancellation::new(), 2);
        let result = with_compile_control(control, || {
            charge_expansion_work(2)?;
            charge_expansion_work(1)
        });
        assert!(matches!(result, Err(Error::ResourceLimit(_))));
        assert!(charge_expansion_work(usize::MAX).is_ok());
    }

    #[test]
    fn cancellation_is_observed_at_checkpoint() {
        let cancellation = CompileCancellation::new();
        cancellation.cancel();
        let result = with_compile_control(CompileControl::new(cancellation, usize::MAX), || Ok(()));
        assert!(matches!(result, Err(Error::Cancelled)));
    }

    #[test]
    fn checkpoints_are_recorded_for_hosts() {
        let cancellation = CompileCancellation::new();
        let observation = cancellation.clone();
        with_compile_control(CompileControl::new(cancellation, usize::MAX), || {
            checkpoint(CompileStage::Build)?;
            checkpoint(CompileStage::Cascade)
        })
        .unwrap();
        assert!(observation.reached(CompileStage::Entry));
        assert!(observation.reached(CompileStage::Build));
        assert!(observation.reached(CompileStage::Cascade));
        assert!(!observation.reached(CompileStage::Parse));
    }

    #[test]
    fn values_branch_product_is_admitted_before_allocation() {
        let values = crate::iq::node::IqNode::Values {
            vars: Vec::new(),
            rows: vec![Vec::new(); 4],
        };
        let result =
            with_compile_control(CompileControl::new(CompileCancellation::new(), 1), || {
                crate::iq::lower::lower(
                    values,
                    sf_sql::Dialect::Sqlite,
                    &std::collections::HashSet::new(),
                    &crate::star::StarEnv::new(),
                )
            });
        assert!(matches!(result, Err(Error::ResourceLimit(_))));
    }

    #[test]
    fn real_compile_crosses_parse_rewrite_build_and_cascade_boundaries() {
        let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();
        let cache = crate::PlanCache::new(1);
        let cancellation = CompileCancellation::new();
        let observation = cancellation.clone();
        with_compile_control(CompileControl::new(cancellation, usize::MAX), || {
            crate::parse_and_translate_cached(
                "SELECT ?person ?name WHERE { ?person <http://example.com/name> ?name }",
                &maps,
                sf_sql::Dialect::Sqlite,
                &crate::Tbox::default(),
                &[],
                &cache,
                crate::Epoch(1),
            )
        })
        .unwrap();

        for stage in [
            CompileStage::Parse,
            CompileStage::Rewrite,
            CompileStage::Build,
            CompileStage::Resolve,
            CompileStage::Normalize,
            CompileStage::Lower,
            CompileStage::Cascade,
            CompileStage::CacheClone,
        ] {
            assert!(observation.reached(stage), "missing {stage:?} checkpoint");
        }
    }
}
