//! Borrowed, charged constant-role coverage over a validated mapping.

use sf_core::ir::{ObjectMap, TriplesMap};
use sf_core::query_control::{QueryControl, QueryControlError};
use sf_core::SourceMapping;

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
