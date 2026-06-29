//! T-saturation — tier-1 entailment folded into the rewrite (ADR-0008).
//!
//! The fabric **consumes a pre-classified T** (it never classifies an ontology —
//! that is upstream DL work, ADR-0008); this module is the documented rewriter
//! stage that uses T's already-built hierarchy to saturate a query before it is
//! unfolded against the mappings:
//!
//! * **subClassOf** — a class query absorbs its subclasses (UNION-folded): asking
//!   for `:C` also matches subjects typed as any `:D ⊑ :C`.
//! * **subPropertyOf** — a predicate absorbs its sub-properties (UNION-folded).
//! * **owl:inverseOf / owl:SymmetricProperty** — fold into the rewriter: a
//!   predicate also matches its inverse with subject/object swapped.
//!
//! The [`Tbox`] holds the **transitive closure** of the hierarchy edges (the
//! caller pre-computes it from T — this is "transitive closure over T's already
//! built hierarchy edges", not a reasoner). Domain/range is documentation-only
//! (not inferred). `owl:TransitiveProperty` is **not** tier-1 — it is served live
//! as a `P+`/`P*` recursive CTE (ADR-0007), deferred here.
//!
//! **Tier-2 is closed (depth-0 by construction):** the consumed T-Box excludes
//! right-hand-side existentials (the OWL-as-documentation policy), so tier-1 is
//! provably complete and no tree-witness rewriting is built (ADR-0008).

use std::collections::{HashMap, HashSet};

/// A pre-classified T-Box hierarchy (transitive closure already applied), the
/// input to tier-1 saturation. An empty `Tbox` (the default) means "no
/// entailment" — the dump path and plain R2RML conformance need no T.
#[derive(Debug, Clone, Default)]
pub struct Tbox {
    /// class IRI → all its sub-classes (reflexive-transitive closure, **excluding**
    /// the class itself; the class is always matched directly).
    sub_classes: HashMap<String, Vec<String>>,
    /// property IRI → all its sub-properties (transitive, excluding itself).
    sub_properties: HashMap<String, Vec<String>>,
    /// property IRI → its inverse property IRI (`owl:inverseOf`, both directions).
    inverses: HashMap<String, String>,
    /// symmetric properties (`owl:SymmetricProperty`).
    symmetric: HashSet<String>,
}

impl Tbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare `sub ⊑ super` (already-transitive edge). The caller supplies the
    /// closure; this just records it.
    pub fn add_subclass(&mut self, sub: impl Into<String>, super_: impl Into<String>) {
        self.sub_classes
            .entry(super_.into())
            .or_default()
            .push(sub.into());
    }

    /// Declare `sub rdfs:subPropertyOf super` (already-transitive edge).
    pub fn add_subproperty(&mut self, sub: impl Into<String>, super_: impl Into<String>) {
        self.sub_properties
            .entry(super_.into())
            .or_default()
            .push(sub.into());
    }

    /// Declare `p owl:inverseOf q` (recorded both ways).
    pub fn add_inverse(&mut self, p: impl Into<String>, q: impl Into<String>) {
        let (p, q) = (p.into(), q.into());
        self.inverses.insert(p.clone(), q.clone());
        self.inverses.insert(q, p);
    }

    /// Declare `p a owl:SymmetricProperty`.
    pub fn add_symmetric(&mut self, p: impl Into<String>) {
        self.symmetric.insert(p.into());
    }

    /// Every class IRI to match for a query asking for `class` — the class itself
    /// plus all its sub-classes (UNION-folding; ADR-0008 tier-1).
    pub fn saturate_class(&self, class: &str) -> Vec<String> {
        let mut out = vec![class.to_owned()];
        if let Some(subs) = self.sub_classes.get(class) {
            out.extend(subs.iter().cloned());
        }
        out
    }

    /// Allocation-free fast check: can a mapping POM with constant predicate
    /// `mapping_pred` ever match a query triple pattern on `query_pred`?
    ///
    /// Semantically equivalent to `saturate_predicate(query_pred).contains(mapping_pred)
    /// || inverse_predicates(query_pred).contains(mapping_pred)` but avoids the
    /// `Vec` allocations those functions produce. Used by `unfold::atom` to fast-
    /// reject non-matching POMs before any `Branch` is allocated (ADR-0013 Path-B).
    pub fn predicate_can_match(&self, mapping_pred: &str, query_pred: &str) -> bool {
        // Direct: the mapping predicate is the query predicate itself, or one of its
        // sub-properties (saturate_predicate expansion).
        if mapping_pred == query_pred {
            return true;
        }
        if self
            .sub_properties
            .get(query_pred)
            .is_some_and(|subs| subs.iter().any(|s| s == mapping_pred))
        {
            return true;
        }
        // Inverse: the mapping predicate is the owl:inverseOf of the query predicate,
        // or the query predicate is symmetric and the mapping predicate equals it
        // (already caught by the direct check above, so only the inverseOf case
        // remains here).
        if self
            .inverses
            .get(query_pred)
            .is_some_and(|inv| inv == mapping_pred)
        {
            return true;
        }
        false
    }

    /// Every predicate IRI to match directly for a query on `predicate` — the
    /// predicate itself plus all its sub-properties (UNION-folding). The inverse
    /// directions are reported separately by [`Tbox::inverse_predicates`].
    pub fn saturate_predicate(&self, predicate: &str) -> Vec<String> {
        let mut out = vec![predicate.to_owned()];
        if let Some(subs) = self.sub_properties.get(predicate) {
            out.extend(subs.iter().cloned());
        }
        out
    }

    /// Predicates that, matched with subject/object **swapped**, also satisfy a
    /// query on `predicate`: its `owl:inverseOf` partner, and `predicate` itself
    /// if it is symmetric (ADR-0008 inverse/symmetric folding).
    pub fn inverse_predicates(&self, predicate: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(inv) = self.inverses.get(predicate) {
            out.push(inv.clone());
        }
        if self.symmetric.contains(predicate) {
            out.push(predicate.to_owned());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subclass_union_folds() {
        let mut t = Tbox::new();
        t.add_subclass("http://ex/Manager", "http://ex/Employee");
        t.add_subclass("http://ex/Engineer", "http://ex/Employee");
        let mut classes = t.saturate_class("http://ex/Employee");
        classes.sort();
        assert_eq!(
            classes,
            vec![
                "http://ex/Employee".to_owned(),
                "http://ex/Engineer".to_owned(),
                "http://ex/Manager".to_owned(),
            ]
        );
        // A class with no sub-classes still matches itself.
        assert_eq!(
            t.saturate_class("http://ex/Manager"),
            vec!["http://ex/Manager"]
        );
    }

    #[test]
    fn subproperty_union_folds() {
        let mut t = Tbox::new();
        t.add_subproperty("http://ex/worksFor", "http://ex/relatedTo");
        let preds = t.saturate_predicate("http://ex/relatedTo");
        assert!(preds.contains(&"http://ex/worksFor".to_owned()));
        assert!(preds.contains(&"http://ex/relatedTo".to_owned()));
    }

    #[test]
    fn inverse_and_symmetric_fold() {
        let mut t = Tbox::new();
        t.add_inverse("http://ex/parentOf", "http://ex/childOf");
        t.add_symmetric("http://ex/marriedTo");
        assert_eq!(
            t.inverse_predicates("http://ex/parentOf"),
            vec!["http://ex/childOf"]
        );
        assert_eq!(
            t.inverse_predicates("http://ex/childOf"),
            vec!["http://ex/parentOf"]
        );
        assert_eq!(
            t.inverse_predicates("http://ex/marriedTo"),
            vec!["http://ex/marriedTo"]
        );
        assert!(t.inverse_predicates("http://ex/unrelated").is_empty());
    }
}
