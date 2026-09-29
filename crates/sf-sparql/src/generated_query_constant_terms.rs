//! Role-aware constant-IRI occurrences reported by the structural walker.
//! Reporting an occurrence is a checking seam and never an admission proof.

use spargebra::term::{Literal, NamedNodePattern, TermPattern};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// Syntactic position of a constant IRI. `Unresolved` claims no role.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ConstantRole {
    Subject,
    Predicate,
    Object,
    /// Object of a triple whose predicate is exactly `rdf:type`.
    Class,
    NamedGraph,
    LiteralDatatype,
    /// Expression, VALUES or path-endpoint position with no proven role.
    Unresolved,
}

impl ConstantRole {
    pub(super) fn for_object(predicate: &NamedNodePattern, object: &TermPattern) -> Self {
        let NamedNodePattern::NamedNode(node) = predicate else {
            return Self::Object;
        };
        let is_type = node.as_str() == RDF_TYPE;
        if is_type && matches!(object, TermPattern::NamedNode(_)) {
            Self::Class
        } else {
            Self::Object
        }
    }
}

/// One borrowed constant IRI with its role. Duplicates are never merged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConstantOccurrence<'a> {
    iri: &'a str,
    role: ConstantRole,
}

impl<'a> ConstantOccurrence<'a> {
    pub(super) fn new(iri: &'a str, role: ConstantRole) -> Self {
        Self { iri, role }
    }

    pub(super) fn datatype_of(literal: &'a Literal) -> Option<Self> {
        if literal.language().is_some() {
            return None;
        }
        let iri = literal.datatype().as_str();
        if iri == XSD_STRING {
            return None;
        }
        Some(Self::new(iri, ConstantRole::LiteralDatatype))
    }

    pub(crate) fn iri(&self) -> &'a str {
        self.iri
    }

    pub(crate) fn role(&self) -> ConstantRole {
        self.role
    }
}

/// Callback refusal. Carries no query text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConstantRejection;
