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

use sf_core::ir::TriplesMap;
use sf_core::NamedNode;

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

    /// Whether the Tbox contains no hierarchy edges (the default / empty case).
    /// Used by [`crate::translate_inner`] to skip the offline expansion when no
    /// T-Box is supplied (the plain R2RML conformance path; ADR-0023 M6).
    pub fn is_empty(&self) -> bool {
        self.sub_classes.is_empty()
            && self.sub_properties.is_empty()
            && self.inverses.is_empty()
            && self.symmetric.is_empty()
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

    /// All super-classes of `sub` (the classes C such that `sub ⊑ C`, i.e. for
    /// which `sub_classes[C]` contains `sub`). Used by [`saturate_maps`] for the
    /// offline T-mapping expansion (ADR-0023 M6).
    pub fn super_classes(&self, sub: &str) -> Vec<&str> {
        self.sub_classes
            .iter()
            .filter_map(|(sup, subs)| {
                if subs.iter().any(|s| s.as_str() == sub) {
                    Some(sup.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    /// All super-properties of `sub` (the properties P such that `sub ⊑ P`).
    /// Used by [`saturate_maps`] for the offline T-mapping expansion (ADR-0023 M6).
    pub fn super_properties(&self, sub: &str) -> Vec<&str> {
        self.sub_properties
            .iter()
            .filter_map(|(sup, subs)| {
                if subs.iter().any(|s| s.as_str() == sub) {
                    Some(sup.as_str())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Pre-expand a slice of [`TriplesMap`]s by folding a [`Tbox`] into the mapping
/// set (ADR-0023 M6 offline T-mapping stage). When `tbox` is empty this is a
/// zero-cost identity (returns the input slice, no allocation). Otherwise returns
/// an expanded `Vec` that contains:
///
/// - every original map unchanged (the direct-match case), plus
/// - for each map whose subject carries a class `sub`, one NEW cloned map per
///   super-class `sup ⊑ sub` (i.e. where `sub ⊑ sup`) that is NOT already
///   covered directly by an existing map in `maps`. The new map has a single
///   class `[sup]` in its subject, but otherwise the SAME source / template /
///   predicate-object maps as the original.
///
/// After this expansion the per-query Tbox lookup in [`crate::unfold`] is no
/// longer needed for the subClassOf axis: a query for `:Person` will directly
/// find the cloned `:Person`-branded copies of any `:Manager ⊑ :Employee ⊑
/// :Person` maps. The Unfolder is then called with `&Tbox::default()` so no
/// runtime lookup is done.
///
/// **=_bag proof**: the expanded maps are semantically equivalent to running
/// `saturate_class` per query — the same branches that would be emitted at
/// query time are instead present from the start. The cascade passes are
/// unaffected (no new structural patterns are introduced).
pub fn saturate_maps<'a>(
    maps: &'a [TriplesMap],
    tbox: &Tbox,
) -> std::borrow::Cow<'a, [TriplesMap]> {
    if tbox.sub_classes.is_empty() && tbox.sub_properties.is_empty() {
        return std::borrow::Cow::Borrowed(maps);
    }

    let mut out: Vec<TriplesMap> = maps.to_vec();
    let mut additions: Vec<TriplesMap> = Vec::new();

    for tm in maps {
        for cls in &tm.subject.classes {
            for sup in tbox.super_classes(cls.as_str()) {
                // Skip if this super-class is already covered by any existing map
                // with the same id (same source). We use (tm.id, sup) as the key
                // so that a user-supplied :Person map isn't shadowed.
                if maps
                    .iter()
                    .any(|m| m.id == tm.id && m.subject.classes.iter().any(|c| c.as_str() == sup))
                {
                    continue;
                }
                // Also skip if we already queued this (tm.id, sup) addition.
                if additions
                    .iter()
                    .any(|m| m.id == tm.id && m.subject.classes.iter().any(|c| c.as_str() == sup))
                {
                    continue;
                }
                // Clone the map with a single super-class entry.
                let mut new_tm = tm.clone();
                new_tm.subject.classes = vec![NamedNode::new_unchecked(sup)];
                additions.push(new_tm);
            }
        }
    }

    if additions.is_empty() {
        return std::borrow::Cow::Borrowed(maps);
    }
    out.extend(additions);
    std::borrow::Cow::Owned(out)
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

    // --- M6: super_classes / super_properties / saturate_maps ---------------

    #[test]
    fn super_classes_reverse_lookup() {
        let mut t = Tbox::new();
        t.add_subclass("http://ex/Manager", "http://ex/Employee");
        t.add_subclass("http://ex/Engineer", "http://ex/Employee");
        t.add_subclass("http://ex/Employee", "http://ex/Person");

        // Manager ⊑ Employee: super_classes("Manager") = ["Employee"]
        let mut sc = t.super_classes("http://ex/Manager");
        sc.sort();
        assert_eq!(sc, vec!["http://ex/Employee"]);

        // Engineer ⊑ Employee: super_classes("Engineer") = ["Employee"]
        let mut sc = t.super_classes("http://ex/Engineer");
        sc.sort();
        assert_eq!(sc, vec!["http://ex/Employee"]);

        // Employee ⊑ Person: super_classes("Employee") = ["Person"]
        let mut sc = t.super_classes("http://ex/Employee");
        sc.sort();
        assert_eq!(sc, vec!["http://ex/Person"]);

        // A class with no super-class entry returns empty.
        assert!(t.super_classes("http://ex/Person").is_empty());
    }

    #[test]
    fn super_properties_reverse_lookup() {
        let mut t = Tbox::new();
        t.add_subproperty("http://ex/worksFor", "http://ex/relatedTo");
        let sp = t.super_properties("http://ex/worksFor");
        assert_eq!(sp, vec!["http://ex/relatedTo"]);
        assert!(t.super_properties("http://ex/relatedTo").is_empty());
    }

    // A minimal TriplesMap factory for tests (no POMs, single class, table source).
    fn make_tm(id: &str, class: &str) -> sf_core::ir::TriplesMap {
        use sf_core::ir::{LogicalSource, SubjectMap, TermMap, TermSpec};
        sf_core::ir::TriplesMap {
            id: id.to_owned(),
            source: LogicalSource::Table("t".to_owned()),
            subject: SubjectMap {
                term: TermMap::Column("id".into(), TermSpec::iri()),
                classes: vec![NamedNode::new_unchecked(class)],
                graphs: vec![],
            },
            predicate_object_maps: vec![],
        }
    }

    #[test]
    fn saturate_maps_empty_tbox_is_noop() {
        let maps = vec![make_tm("T", "http://ex/Employee")];
        let result = saturate_maps(&maps, &Tbox::default());
        // Borrowed: no allocation for an empty Tbox.
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn saturate_maps_adds_superclass_entry() {
        // Manager ⊑ Employee ⊑ Person.
        let mut t = Tbox::new();
        t.add_subclass("http://ex/Manager", "http://ex/Employee");
        t.add_subclass("http://ex/Employee", "http://ex/Person");

        let maps = vec![make_tm("T", "http://ex/Manager")];
        let result = saturate_maps(&maps, &t);

        // Should have 3 entries: original Manager + synthetic Employee + synthetic Person.
        // (super_classes("Manager") = ["Employee"], super_classes("Employee") = ["Person"]
        //  but since we only call super_classes on each original map's class, we only add
        //  direct super-classes of "Manager", which is just "Employee").
        // Note: super_classes is a direct-level lookup (non-recursive), since the Tbox already
        // holds the transitive closure (add_subclass records direct edges; for a two-hop chain
        // the caller adds both edges explicitly).
        let classes: Vec<_> = result
            .iter()
            .flat_map(|m| m.subject.classes.iter().map(|c| c.as_str().to_owned()))
            .collect();
        // Manager is in the original.
        assert!(classes.contains(&"http://ex/Manager".to_owned()));
        // Employee must be added as the super of Manager.
        assert!(classes.contains(&"http://ex/Employee".to_owned()));
        // Person is NOT added here because super_classes("Manager") only returns "Employee"
        // (the Tbox has "Employee" → ["Manager"] and "Person" → ["Employee"]; the transitive
        // closure for Person/Manager would require add_subclass("Manager","Person") to be recorded
        // in addition — this is the CALLER's responsibility per the Tbox contract).
        assert_eq!(result.len(), 2, "one original + one synthetic super-class");
    }

    #[test]
    fn saturate_maps_no_duplicate_when_already_covered() {
        // Both Employee and Manager maps present — saturating Manager should NOT
        // add another Employee entry (it's already there under the same map id).
        let mut t = Tbox::new();
        t.add_subclass("http://ex/Manager", "http://ex/Employee");

        // Existing Employee-branded map with same id "T".
        let mut emp_map = make_tm("T", "http://ex/Employee");
        emp_map.subject.classes = vec![
            NamedNode::new_unchecked("http://ex/Employee"),
            NamedNode::new_unchecked("http://ex/Manager"),
        ];
        let maps = vec![emp_map];
        let result = saturate_maps(&maps, &t);
        // No additions needed: the map already covers the super-class Employee.
        assert_eq!(
            result.len(),
            1,
            "no synthetic entry added when super already present"
        );
    }

    #[test]
    fn saturate_maps_round_trip_equivalent_to_per_query_tbox() {
        use sf_core::ir::{LogicalSource, SubjectMap, TermMap, TermSpec};

        // A Manager-branded TriplesMap.
        let mgr_map = sf_core::ir::TriplesMap {
            id: "T".to_owned(),
            source: LogicalSource::Table("employees".to_owned()),
            subject: SubjectMap {
                term: TermMap::Template(
                    sf_core::ir::Template::parse("http://ex/emp/{id}").unwrap(),
                    TermSpec::iri(),
                ),
                classes: vec![NamedNode::new_unchecked("http://ex/Manager")],
                graphs: vec![],
            },
            predicate_object_maps: vec![],
        };

        let mut tbox = Tbox::new();
        tbox.add_subclass("http://ex/Manager", "http://ex/Employee");

        // Path A: expand maps offline then use empty Tbox.
        let expanded = saturate_maps(std::slice::from_ref(&mgr_map), &tbox);
        let expanded_classes: std::collections::BTreeSet<String> = expanded
            .iter()
            .flat_map(|m| m.subject.classes.iter().map(|c| c.as_str().to_owned()))
            .collect();

        // Path B: per-query expansion of what we'd match (saturate_class("Employee")).
        let per_query: std::collections::BTreeSet<String> = tbox
            .saturate_class("http://ex/Employee")
            .into_iter()
            .collect();

        // The offline expansion should contain all per-query classes (Manager + Employee).
        assert_eq!(
            expanded_classes, per_query,
            "offline expansion covers exactly the per-query saturate_class set"
        );
    }
}
