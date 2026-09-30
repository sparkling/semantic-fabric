//! Variable unification and FILTER lowering — the raw-column half of the base
//! translation (ADR-0007 *Term-construction lifting*).
//!
//! When a SPARQL variable occurs in two patterns, its two [`TermDef`]s must be
//! made equal. Unification reduces that to **raw-column equalities** whenever the
//! two term definitions are structurally compatible (the lifting that keeps joins
//! on indexed key columns); proves the join **empty** when the IRI/literal shapes
//! are disjoint (the seed the IRI-template-mismatch cascade pass formalises); and
//! otherwise reports the case **unsupported** (deferred, never silently wrong).

use sf_core::ir::{Segment, TermMap, TermSpec, TermType};
use sf_core::Term;

use sf_sql::Dialect;
mod iri_cmp;
mod literal_cmp;
use crate::iq::literal_cmp::LiteralOperand;

use crate::iq::{plain_term_def, CmpOp, ColRef, R2rmlGraphScope, SqlCond, StrMatchOp, TermDef};

/// The result of unifying two term definitions for the same variable.
pub enum Unify {
    /// Satisfiable under these (raw-column / constant) conditions.
    Sat(Vec<SqlCond>),
    /// Provably disjoint — the containing branch is pruned (0 rows).
    Empty,
    /// A correct reduction is not yet implemented (deferred; ADR-0007 v1).
    Unsupported(String),
}

/// Unify two definitions of the same variable.
pub fn unify(a: &TermDef, b: &TermDef) -> Unify {
    match (a, b) {
        (
            TermDef::R2rmlBlank {
                term_map: t1,
                alias: a1,
                graph: g1,
            },
            TermDef::R2rmlBlank {
                term_map: t2,
                alias: a2,
                graph: g2,
            },
        ) => combine_unify(
            unify(&plain_term_def(t1, *a1), &plain_term_def(t2, *a2)),
            unify_graph_scope(g1, g2),
        ),
        (TermDef::Const(x), TermDef::Const(y)) => {
            if x == y {
                Unify::Sat(vec![])
            } else {
                Unify::Empty
            }
        }
        (TermDef::Const(c), TermDef::Derived { term_map, alias })
        | (TermDef::Derived { term_map, alias }, TermDef::Const(c)) => {
            unify_const_derived(c, term_map, *alias)
        }
        (
            TermDef::Derived {
                term_map: t1,
                alias: a1,
            },
            TermDef::Derived {
                term_map: t2,
                alias: a2,
            },
        ) => unify_derived(t1, *a1, t2, *a2),
        (TermDef::R2rmlBlank { .. }, TermDef::Const(term))
        | (TermDef::Const(term), TermDef::R2rmlBlank { .. }) => {
            if matches!(term, Term::BlankNode(_)) {
                Unify::Unsupported(
                    "unification of a graph-scoped R2RML blank node with an unscoped blank-node constant"
                        .to_owned(),
                )
            } else {
                Unify::Empty
            }
        }
        (TermDef::R2rmlBlank { .. }, TermDef::Derived { term_map, .. })
        | (TermDef::Derived { term_map, .. }, TermDef::R2rmlBlank { .. }) => {
            if term_map_type(term_map) == Some(TermType::BlankNode) {
                Unify::Unsupported(
                    "unification of a graph-scoped R2RML blank node with an unscoped blank-node recipe"
                        .to_owned(),
                )
            } else {
                Unify::Empty
            }
        }
        // A COALESCE'd (multi-OPTIONAL shared var) or CONCAT'd (BIND-computed)
        // binding is a multi-source constructed term; reducing it to raw-column
        // equalities is deferred (ADR-0007 v1 — never silently wrong). So sharing a
        // BIND variable with a later pattern (a join/filter on it) defers to 501.
        // An aggregate result is produced post-grouping and is never re-unified into
        // a join/filter (the group is the outermost pattern in v1). A `ComposedTriple`
        // (ADR-0032 D2) is likewise multi-source (three component defs) and is, in
        // practice, only ever installed at the very end of translation (`lib.rs`'s
        // env-composed projection override, AFTER unify has already run for every
        // real join/filter) — never actually reached here, but the same conservative
        // "not reducible to one raw-column equality" verdict applies if it ever were.
        (
            TermDef::Coalesce(..)
            | TermDef::Concat(..)
            | TermDef::Agg { .. }
            | TermDef::ComposedTriple { .. },
            _,
        )
        | (
            _,
            TermDef::Coalesce(..)
            | TermDef::Concat(..)
            | TermDef::Agg { .. }
            | TermDef::ComposedTriple { .. },
        ) => Unify::Unsupported(
            "unification of a COALESCE'd / CONCAT'd / aggregate / composed-triple \
             (multi-source / computed) binding"
                .to_owned(),
        ),
    }
}

use spargebra::algebra::{Expression, Function};
use spargebra::term::{Literal, Variable};
use std::collections::BTreeMap;

mod filter_condition;
mod str_comparison;
mod template_unification;
mod term_unification;
pub use filter_condition::{bind_term_def, filter_cond};
pub(crate) use filter_condition::{filter_branch, filter_scopes};
pub(crate) use template_unification::templates_provably_disjoint;
use template_unification::*;
use term_unification::*;

#[cfg(test)]
mod tests;
