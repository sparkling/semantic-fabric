//! Shared vocabulary constants and small stateless primitives used across the
//! `star` module tree: the RDF 1.2 `reifies`/basic-encoding IRIs and the
//! error-marker IRI (ADR-0029 §B.2 — MUST match `crates/sf-mapping/src/r2rml.rs`'s
//! consts of the same name exactly, a different crate so not shared by import),
//! whole-query fresh-variable minting (one counter threaded through the entire
//! rewrite, see [`super::top_level::rewrite_query`]), and the handful of tiny
//! recursive predicates ([`has_subject_position_triple_term`]) and converters
//! ([`named_node_pattern_to_term_pattern`]) with no state of their own that
//! [`super::walk`] and [`super::env`] both need.

use std::collections::BTreeSet;

use super::control::StarWork;
use crate::{CompilerWorkMode, Result};
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};

/// RDF 1.2's native "reifies" predicate (`oxrdf::vocab::rdf::REIFIES`, cited
/// verbatim in ADR-0031's Context) — `oxrdf` itself is only a dev-dependency
/// of this crate (everything else reaches these vocab IRIs through spargebra's
/// re-exported types), so this is hand-declared like every other vocabulary
/// constant in this crate (`unfold::RDF_TYPE`, `sf-mapping`'s r2rml.rs consts).
pub(super) const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

// --- RDF 1.2 Interoperability "basic encoding" vocabulary (ADR-0029 §B.2) —
// MUST match `crates/sf-mapping/src/r2rml.rs`'s consts of the same name
// exactly (a different crate, so not shared by import): `sf-mapping` compiles
// `rml:StarMap` mappings onto these same predicates, so a query asking for a
// different IRI would never match a single mapped triple. `rdf:type` reuses
// `unfold::RDF_TYPE` rather than a third copy of the same string.
pub(super) const RDF_PROPOSITION_FORM: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#PropositionForm";
pub(super) const RDF_PROPOSITION_FORM_SUBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormSubject";
pub(super) const RDF_PROPOSITION_FORM_PREDICATE: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormPredicate";
pub(super) const RDF_PROPOSITION_FORM_OBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormObject";

pub(super) const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

/// The [`spargebra::algebra::Function::Concat`] marker `rewrite_expr` produces
/// for a provably-non-composed SUBJECT/PREDICATE/OBJECT argument (ADR-0032 D3
/// / SPARQL §17.4.6) — see [`super::expr::error_marker_expr`]'s doc comment
/// for the full rationale. Any IRI works (the marker's SOUNDNESS rests on
/// being a `NamedNode`, never its specific value); this one is
/// self-documenting if it ever surfaces in a debug print or error message.
pub(super) const ERROR_MARKER_IRI: &str = "urn:sf-star:error-marker";

/// Whole-query generated-variable state. Parsed SPARQL variables may legally
/// start with `__`, so a prefix alone is not hygienic. Every pre-existing query
/// variable is reserved up front, and each generated name is inserted into the
/// same set before it is returned. One ordinal sequence still spans BGPs,
/// UNION arms, expressions, VALUES decomposition, and empty markers.
pub(super) struct FreshVars<'a> {
    next: usize,
    reserved: BTreeSet<Variable>,
    pub work: StarWork<'a>,
}

impl Default for FreshVars<'_> {
    fn default() -> Self {
        Self::with_work(
            BTreeSet::new(),
            StarWork::new(CompilerWorkMode::Uncontrolled),
        )
    }
}

impl<'a> FreshVars<'a> {
    #[cfg(test)]
    pub(super) fn new(vars: impl IntoIterator<Item = Variable>) -> Self {
        Self::with_work(
            vars.into_iter().collect(),
            StarWork::new(CompilerWorkMode::Uncontrolled),
        )
    }

    pub(super) fn with_work(reserved: BTreeSet<Variable>, work: StarWork<'a>) -> Self {
        Self {
            next: 0,
            reserved,
            work,
        }
    }

    fn mint(&mut self, prefix: &str) -> Result<Variable> {
        loop {
            let ordinal = self.next;
            let next = self
                .next
                .checked_add(1)
                .ok_or_else(|| self.work.overflow())?;
            self.work.fresh(self.reserved.len(), prefix, ordinal)?;
            let candidate = Variable::new_unchecked(format!("{prefix}{ordinal}"));
            self.next = next;
            if self.reserved.insert(candidate.clone()) {
                self.work.checkpoint()?;
                return Ok(candidate);
            }
            self.work.checkpoint()?;
        }
    }

    #[cfg(test)]
    pub(super) const fn next_ordinal(&self) -> usize {
        self.next
    }
}

/// A fresh whole-query identity variable (`__sf_star_{n}`), minted through
/// [`FreshVars`] so it cannot capture any variable already present in the
/// parsed query.
pub(super) fn fresh_var(fresh: &mut FreshVars) -> Result<TermPattern> {
    Ok(TermPattern::Variable(fresh_component_var(fresh)?))
}

/// Like [`fresh_var`], but returns the bare [`Variable`] (not wrapped in a
/// `TermPattern`) — for minting [`super::env::ComposedInfo`]'s
/// `s_var`/`p_var`/`o_var`.
pub(super) fn fresh_component_var(fresh: &mut FreshVars) -> Result<Variable> {
    fresh.mint("__sf_star_")
}

/// Rule R1's zero-solution replacement: a fresh, collision-checked
/// `__sf_star_empty_{n}` variable naming an empty `VALUES` clause (see
/// [`empty_pattern`]) — kept textually distinct from [`fresh_var`]'s identity
/// variables (which DO bind real terms) purely for readability when a
/// rewritten query is inspected; both draw from the same whole-query state.
fn fresh_empty_var(fresh: &mut FreshVars) -> Result<Variable> {
    fresh.mint("__sf_star_empty_")
}

/// Rule R1 (SPARQL 1.2 §18.1.3): the statically-empty replacement for a
/// pattern containing a subject-position triple term — a `VALUES` clause
/// with zero rows, the standard zero-solution element on both translation
/// paths: `unfold::translate_pattern`'s `Values` arm folds zero binding rows
/// into zero branches (`TransPattern::plain(Vec::new())`), and the tree
/// path's `IqNode::Values` lowering does the same (`iq/lower.rs`) — indeed
/// `iq/normalize.rs` already treats an empty result as the `Join`/`Union`
/// absorbing/identity element via its own purpose-built `IqNode::Empty`.
/// Never an error, never a match.
pub(super) fn empty_pattern(fresh: &mut FreshVars) -> Result<GraphPattern> {
    fresh.work.charge(3)?;
    Ok(GraphPattern::Values {
        variables: vec![fresh_empty_var(fresh)?],
        bindings: Vec::new(),
    })
}

/// Whether `pred` is the constant `rdf:reifies` (rule R2's trigger). A
/// variable predicate can never BE the constant `rdf:reifies`, so only the
/// `NamedNode` arm can match.
pub(super) fn is_reifies(pred: &NamedNodePattern) -> bool {
    matches!(pred, NamedNodePattern::NamedNode(p) if p.as_str() == RDF_REIFIES)
}

/// A quoted triple's predicate (`NamedNodePattern` — structurally never a
/// `Triple`) copied verbatim into the `rdf:propositionFormPredicate` object
/// position (a `TermPattern`).
pub(super) fn named_node_pattern_to_term_pattern(p: &NamedNodePattern) -> TermPattern {
    match p {
        NamedNodePattern::NamedNode(n) => TermPattern::NamedNode(n.clone()),
        NamedNodePattern::Variable(v) => TermPattern::Variable(v.clone()),
    }
}

/// R1's recursive trigger: does `tp` — or (R4) any quoted triple reached
/// through its own OBJECT chain — have a Triple-typed SUBJECT? Subject-side
/// nesting is spec-impossible at any depth (RDF 1.2 Concepts §3.1: triple
/// terms are object-position-only), so it is never checked on the object
/// side of the recursion (there is no other legal place for it to hide).
pub(super) fn has_subject_position_triple_term(tp: &TriplePattern) -> bool {
    matches!(tp.subject, TermPattern::Triple(_))
        || matches!(&tp.object, TermPattern::Triple(inner) if has_subject_position_triple_term(inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_control::CompileContext;
    use crate::Error;
    use sf_core::query_control::{
        QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    };

    #[test]
    fn paid_mint_reserves_each_collision_and_refuses_overflow() {
        // Three 11-byte candidates, two already-reserved names per attempt:
        // K*(C+1) comparisons + 2*C copied bytes + one logical set slot.
        let exact_work = 3 * (2 * 12 + 2 * 11 + 1);
        for limit in [exact_work, exact_work - 1] {
            let control = QueryBudget::new(QueryLimits::new(limit, u64::MAX, u64::MAX, u64::MAX));
            let work = StarWork::new(CompilerWorkMode::Metered(CompileContext::new(&control)));
            let mut fresh = FreshVars::with_work(
                [authored("__sf_star_0"), authored("__sf_star_1")]
                    .into_iter()
                    .collect(),
                work,
            );
            if limit == exact_work {
                assert_eq!(
                    fresh_component_var(&mut fresh).unwrap().as_str(),
                    "__sf_star_2"
                );
                assert_eq!(control.consumed(QueryCharge::CompilerWork), exact_work);
            } else {
                assert!(matches!(
                    fresh_component_var(&mut fresh),
                    Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
                ));
                assert_eq!(fresh.next_ordinal(), 2);
                assert_eq!(fresh.reserved.len(), 2);
            }
        }
        let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
        let mut fresh = FreshVars::with_work(
            BTreeSet::new(),
            StarWork::new(CompilerWorkMode::Metered(CompileContext::new(&control))),
        );
        fresh.next = usize::MAX;
        assert!(matches!(
            fresh_component_var(&mut fresh),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
        assert!(fresh.reserved.is_empty());
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
    }

    fn authored(name: &str) -> Variable {
        Variable::new_unchecked(name)
    }

    #[test]
    fn mint_skips_multiple_consecutive_authored_candidates() {
        let mut fresh = FreshVars::new([authored("__sf_star_0"), authored("__sf_star_1")]);
        assert_eq!(
            fresh_component_var(&mut fresh).unwrap().as_str(),
            "__sf_star_2"
        );
        assert_eq!(fresh.next_ordinal(), 3);
    }

    #[test]
    fn prefixes_share_an_ordinal_without_skipping_collision_checks() {
        let mut fresh = FreshVars::new([authored("__sf_star_1")]);
        let GraphPattern::Values { variables, .. } = empty_pattern(&mut fresh).unwrap() else {
            panic!("empty marker must be a VALUES pattern");
        };
        assert_eq!(variables[0].as_str(), "__sf_star_empty_0");
        assert_eq!(
            fresh_component_var(&mut fresh).unwrap().as_str(),
            "__sf_star_2"
        );
        assert_eq!(fresh.next_ordinal(), 3);
    }
}
