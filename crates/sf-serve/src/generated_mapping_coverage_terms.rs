//! Charged, borrowed term matching for mapping constant-role coverage.

use sf_core::ir::{ObjectMap, RefObjectMap, TermMap, TermType, TriplesMap};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_core::Term;

pub(super) const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub(super) const RR_DEFAULT_GRAPH: &str = "http://www.w3.org/ns/r2rml#defaultGraph";

pub(super) type Step = Result<bool, QueryControlError>;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Typed {
    No,
    Maybe,
    Yes,
}

pub(super) struct Scan<'a> {
    control: &'a dyn QueryControl,
    iri: &'a str,
    pub(super) skipped: bool,
}

impl<'a> Scan<'a> {
    pub(super) fn new(control: &'a dyn QueryControl, iri: &'a str) -> Self {
        Self {
            control,
            iri,
            skipped: false,
        }
    }

    pub(super) fn charge(&self, units: usize) -> Result<(), QueryControlError> {
        self.control.checkpoint()?;
        let units = u64::try_from(units).map_err(|_| {
            self.control
                .terminate(QueryControlError::AccountingOverflow)
        })?;
        self.control.consume(QueryCharge::CompilerWork, units)?;
        self.control.checkpoint()
    }

    fn equal(&self, left: &str, right: &str) -> Step {
        self.charge(1 + left.len().min(right.len()))?;
        Ok(left == right)
    }

    pub(super) fn same(&self, candidate: &str) -> Step {
        self.equal(candidate, self.iri)
    }

    pub(super) fn term(&mut self, map: &TermMap) -> Step {
        self.charge(1)?;
        match map {
            TermMap::Constant(Term::NamedNode(node)) => self.same(node.as_str()),
            TermMap::Constant(_) => Ok(false),
            TermMap::Column(..) | TermMap::Template(..) => {
                self.skipped = true;
                Ok(false)
            }
        }
    }

    pub(super) fn datatype(&mut self, map: &TermMap) -> Step {
        self.charge(1)?;
        match map {
            TermMap::Constant(Term::Literal(literal)) => self.same(literal.datatype().as_str()),
            TermMap::Constant(_) => Ok(false),
            TermMap::Column(_, spec) | TermMap::Template(_, spec) => {
                if spec.term_type != TermType::Literal {
                    return Ok(false);
                }
                match (&spec.datatype, &spec.language) {
                    (Some(datatype), None) => self.same(datatype.as_str()),
                    _ => {
                        self.skipped = true;
                        Ok(false)
                    }
                }
            }
        }
    }

    pub(super) fn rdf_type(&self, predicates: &[TermMap]) -> Result<Typed, QueryControlError> {
        let mut typed = Typed::No;
        for predicate in predicates {
            self.charge(1)?;
            match predicate {
                TermMap::Constant(Term::NamedNode(node)) => {
                    if self.equal(node.as_str(), RDF_TYPE)? {
                        return Ok(Typed::Yes);
                    }
                }
                TermMap::Constant(_) => {}
                TermMap::Column(..) | TermMap::Template(..) => typed = Typed::Maybe,
            }
        }
        Ok(typed)
    }

    fn parent<'m>(
        &self,
        maps: &'m [TriplesMap],
        parent_id: &str,
    ) -> Result<Option<&'m TriplesMap>, QueryControlError> {
        let mut found = None;
        for candidate in maps {
            self.charge(1 + candidate.id.len().min(parent_id.len()))?;
            if candidate.id == parent_id {
                if found.is_some() {
                    return Ok(None);
                }
                found = Some(candidate);
            }
        }
        Ok(found)
    }

    fn parent_subject(&mut self, maps: &[TriplesMap], reference: &RefObjectMap) -> Step {
        if let Some(parent) = self.parent(maps, &reference.parent_triples_map)? {
            return self.term(&parent.subject.term);
        }
        self.skipped = true;
        Ok(false)
    }

    pub(super) fn object(&mut self, maps: &[TriplesMap], object: &ObjectMap) -> Step {
        self.charge(1)?;
        match object {
            ObjectMap::Term(term) => self.term(term),
            ObjectMap::Ref(reference) => self.parent_subject(maps, reference),
        }
    }
}
