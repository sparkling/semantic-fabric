//! Borrowed, charged constant-role and static IRI-template coverage over a validated mapping.

use sf_core::ir::{ObjectMap, TriplesMap};
use sf_core::query_control::{QueryControl, QueryControlError};
use sf_core::SourceMapping;

#[cfg(test)]
#[path = "generated_mapping_coverage_template_tests.rs"]
mod template_tests;
#[path = "generated_mapping_coverage_templates.rs"]
mod templates;
#[path = "generated_mapping_coverage_terms.rs"]
mod terms;
#[cfg(test)]
#[path = "generated_mapping_coverage_tests.rs"]
mod tests;

use terms::{Scan, Step, Typed};

/// Distinct occurrence roles of one constant IRI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConstantRole {
    Subject,
    Predicate,
    Object,
    Class,
    NamedGraph,
    LiteralDatatype,
    Unresolved,
}

const CONCRETE: [ConstantRole; 6] = [
    ConstantRole::Subject,
    ConstantRole::Predicate,
    ConstantRole::Object,
    ConstantRole::Class,
    ConstantRole::NamedGraph,
    ConstantRole::LiteralDatatype,
];

/// Structural possibility only: never source rows, satisfiability, profile
/// issuance or live admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Coverage {
    Covered,
    Uncovered,
    Indeterminate,
}

/// Borrowed view of a validated mapping; constructible only from its receipt.
pub(crate) struct MappingCoverage<'a> {
    maps: &'a [TriplesMap],
}

impl std::fmt::Debug for MappingCoverage<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MappingCoverage")
            .finish_non_exhaustive()
    }
}

impl<'a> MappingCoverage<'a> {
    pub(super) fn new(mapping: &'a SourceMapping) -> Self {
        Self {
            maps: mapping.triples_maps(),
        }
    }

    pub(crate) fn classify(
        &self,
        role: ConstantRole,
        iri: &str,
        control: &dyn QueryControl,
    ) -> Result<Coverage, QueryControlError> {
        let mut scan = Scan::new(control, iri);
        scan.charge(1)?;
        let matched = if role == ConstantRole::Unresolved {
            self.any_role(&mut scan)?
        } else {
            self.role_hit(&mut scan, role)?
        };
        let coverage = if matched {
            Coverage::Covered
        } else if scan.skipped || role == ConstantRole::Unresolved {
            Coverage::Indeterminate
        } else {
            Coverage::Uncovered
        };
        Ok(coverage)
    }

    fn any_role(&self, scan: &mut Scan<'_>) -> Step {
        for role in CONCRETE {
            if self.role_hit(scan, role)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn role_hit(&self, scan: &mut Scan<'_>, role: ConstantRole) -> Step {
        if role == ConstantRole::NamedGraph && scan.same(terms::RR_DEFAULT_GRAPH)? {
            return Ok(false);
        }
        for map in self.maps {
            scan.charge(1)?;
            let hit = match role {
                ConstantRole::Subject => scan.term(&map.subject.term)?,
                ConstantRole::Predicate => Self::predicates(scan, map)?,
                ConstantRole::Object => self.objects(scan, map)?,
                ConstantRole::Class => self.classes(scan, map)?,
                ConstantRole::NamedGraph => Self::graphs(scan, map)?,
                ConstantRole::LiteralDatatype => Self::datatypes(scan, map)?,
                ConstantRole::Unresolved => false,
            };
            if hit {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn predicates(scan: &mut Scan<'_>, map: &TriplesMap) -> Step {
        if !map.subject.classes.is_empty() && scan.same(terms::RDF_TYPE)? {
            return Ok(true);
        }
        for pom in &map.predicate_object_maps {
            scan.charge(1)?;
            for predicate in &pom.predicates {
                if scan.term(predicate)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn objects(&self, scan: &mut Scan<'_>, map: &TriplesMap) -> Step {
        for class in &map.subject.classes {
            if scan.same(class.as_str())? {
                return Ok(true);
            }
        }
        for pom in &map.predicate_object_maps {
            scan.charge(1)?;
            for object in &pom.objects {
                if scan.object(self.maps, object)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn classes(&self, scan: &mut Scan<'_>, map: &TriplesMap) -> Step {
        for class in &map.subject.classes {
            if scan.same(class.as_str())? {
                return Ok(true);
            }
        }
        for pom in &map.predicate_object_maps {
            scan.charge(1)?;
            let typed = scan.rdf_type(&pom.predicates)?;
            if typed == Typed::No {
                continue;
            }
            for object in &pom.objects {
                if scan.object(self.maps, object)? {
                    if typed == Typed::Yes {
                        return Ok(true);
                    }
                    scan.skipped = true;
                }
            }
        }
        Ok(false)
    }

    fn graphs(scan: &mut Scan<'_>, map: &TriplesMap) -> Step {
        for graph in &map.subject.graphs {
            if scan.term(graph)? {
                return Ok(true);
            }
        }
        for pom in &map.predicate_object_maps {
            scan.charge(1)?;
            for graph in &pom.graphs {
                if scan.term(graph)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn datatypes(scan: &mut Scan<'_>, map: &TriplesMap) -> Step {
        for pom in &map.predicate_object_maps {
            scan.charge(1)?;
            for object in &pom.objects {
                scan.charge(1)?;
                if let ObjectMap::Term(term) = object {
                    if scan.datatype(term)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod template_cases {
    use super::{ConstantRole as Role, Coverage};
    use sf_core::query_control::QueryControlError;

    pub(super) const EX: &str = "http://example.test/";
    pub(super) const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    pub(super) const DEFAULT_GRAPH: &str = "http://www.w3.org/ns/r2rml#defaultGraph";
    pub(super) const YES: Coverage = Coverage::Covered;
    pub(super) const NO: Coverage = Coverage::Uncovered;
    pub(super) const MAYBE: Coverage = Coverage::Indeterminate;

    pub(super) type Table = &'static [(&'static str, Coverage)];
    pub(super) type RoleTable = &'static [(Role, Coverage)];

    pub(super) const REASONS: [QueryControlError; 2] = [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ];

    pub(super) const OTHER_HOSTS: &[&str] = &[
        "http://other.test/abc",
        "http://example.test",
        "https://example.test/abc",
        "urn:x",
    ];

    pub(super) const BARE: Table = &[
        ("abc", YES),
        ("", YES),
        ("a%2Fb", YES),
        ("A0-._~z", YES),
        ("a/b", MAYBE),
        ("a b", MAYBE),
    ];

    pub(super) const PREFIX_SUFFIX: Table = &[
        ("abc/end", YES),
        ("/end", YES),
        ("a%2Fb/end", YES),
        ("abc/other", NO),
        ("abc/en", NO),
        ("abc/end/", NO),
        ("end", NO),
        ("a/end/end", MAYBE),
    ];

    pub(super) const UNICODE_LITERAL: Table = &[("\u{e9}/x", YES), ("e/x", NO), ("\u{e9}/", YES)];

    pub(super) const ENCODING: Table = &[
        ("caf\u{e9}", YES),
        ("caf%C3%A9", MAYBE),
        ("\u{65e5}\u{672c}", YES),
        ("%E6%97%A5", MAYBE),
        ("\u{1f600}", YES),
        ("%F0%9F%98%80", MAYBE),
        ("\u{e000}", MAYBE),
        ("%EE%80%80", YES),
        ("\u{fffe}", MAYBE),
        ("%EF%BF%BE", YES),
        ("\u{80}", MAYBE),
        ("%C2%80", YES),
        ("%20", YES),
        ("%25", YES),
        ("%", MAYBE),
        ("100%", MAYBE),
        ("100%2", MAYBE),
        ("%zz", MAYBE),
        ("%2f", MAYBE),
        ("%41", MAYBE),
        ("%C3", MAYBE),
        ("%C3%28", MAYBE),
        ("%C0%80", MAYBE),
        ("%ED%A0%80", MAYBE),
        ("%F8%88%80%80%80", MAYBE),
        ("caf%C3%A9%2F", MAYBE),
    ];

    pub(super) const SPLIT: Table = &[
        ("p-7-s", YES),
        ("p-7-x", NO),
        ("p--s", YES),
        ("p-s", NO),
        ("p-a/b-s", MAYBE),
    ];

    pub(super) const FIXED: Table = &[("fixed", YES), ("fixe", NO), ("fixed2", NO), ("Fixed", NO)];

    pub(super) const MULTI: Table = &[("1/2", MAYBE), ("12", MAYBE), ("", NO)];

    pub(super) const GATED: Table = &[
        ("item/42", YES),
        ("item/a%2Fb", YES),
        ("item/a/b", MAYBE),
        ("other/42", NO),
        ("item", NO),
    ];

    pub(super) const SUBJECT_ROLES: RoleTable = &[
        (Role::Subject, YES),
        (Role::Predicate, NO),
        (Role::Object, NO),
        (Role::Class, NO),
        (Role::NamedGraph, NO),
        (Role::LiteralDatatype, NO),
        (Role::Unresolved, YES),
    ];

    pub(super) const PREDICATE_ROLES: RoleTable = &[
        (Role::Subject, NO),
        (Role::Predicate, YES),
        (Role::Object, NO),
        (Role::Class, NO),
        (Role::NamedGraph, NO),
        (Role::LiteralDatatype, NO),
        (Role::Unresolved, YES),
    ];

    pub(super) const CHARGED: &[(&str, &str)] = &[
        ("http://example.test/fixed", "http://example.test/fixed"),
        ("http://example.test/{id}", "http://example.test/abc%2Fdef"),
        ("http://example.test/{id}", "http://other.test/x"),
        ("http://example.test/{a}/{b}", "http://example.test/1/2"),
        (
            "http://example.test/{id}/end",
            "http://example.test/caf\u{e9}/end",
        ),
        ("{id}", "http://example.test/x"),
    ];

    pub(super) const ALPHABET: &str = "a%2FC3A\u{e9}\u{e000}/ ";
    pub(super) const SAMPLES_A: &str = "|abc|a b|a/b|100%|\u{e9}|\u{65e5}\u{672c}|\u{1f600}";
    pub(super) const SAMPLES_B: &str = "\u{e000}|\u{fffe}|\u{1fffe}|\u{80}|\0|~-._|x%25y";

    pub(super) const ONTOLOGY: &str = r#"
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
    @prefix ex: <http://example.test/> .
    ex:Item a owl:Class .
    ex:value a rdf:Property .
    "#;
}
