use oxrdf::{Literal, NamedOrBlankNode, Term};

use super::semantic_key::{validate_term_depth_hard, BlankNodeScope, ScopedTerm, SemanticKeyError};

#[derive(Clone, Debug)]
pub(super) struct SemanticBinding {
    variable: Box<str>,
    value: Option<ScopedTerm>,
}

#[derive(Clone, Debug)]
pub(super) struct SemanticMapping {
    bindings: Vec<SemanticBinding>,
}

impl SemanticMapping {
    pub(super) fn new(
        bindings: Vec<(Box<str>, Option<ScopedTerm>)>,
    ) -> Result<Self, SemanticKeyError> {
        for (index, (variable, value)) in bindings.iter().enumerate() {
            if variable.is_empty() || variable.as_bytes().contains(&0) {
                return Err(SemanticKeyError::MalformedVariable);
            }
            if bindings[..index]
                .iter()
                .any(|(earlier, _)| earlier == variable)
            {
                return Err(SemanticKeyError::DuplicateVariable);
            }
            if let Some(value) = value {
                validate_term_depth_hard(value.term())?;
            }
        }
        Ok(Self {
            bindings: bindings
                .into_iter()
                .map(|(variable, value)| SemanticBinding { variable, value })
                .collect(),
        })
    }

    pub(super) fn value(&self, variable: &str) -> Option<&ScopedTerm> {
        self.bindings
            .iter()
            .find(|binding| binding.variable.as_ref() == variable)
            .and_then(|binding| binding.value.as_ref())
    }

    pub(super) fn contains_variable(&self, variable: &str) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.variable.as_ref() == variable)
    }

    pub(super) fn len(&self) -> usize {
        self.bindings.len()
    }

    pub(super) fn variables(&self) -> impl Iterator<Item = &str> {
        self.bindings
            .iter()
            .map(|binding| binding.variable.as_ref())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Compatibility {
    compatible: bool,
    shared_bound: usize,
}

fn compare(left: &SemanticMapping, right: &SemanticMapping) -> Compatibility {
    let mut shared_bound = 0usize;
    for binding in &left.bindings {
        let Some(left_value) = binding.value.as_ref() else {
            continue;
        };
        let Some(right_value) = right.value(&binding.variable) else {
            continue;
        };
        shared_bound += 1;
        if !semantic_term_eq(left_value, right_value) {
            return Compatibility {
                compatible: false,
                shared_bound,
            };
        }
    }
    Compatibility {
        compatible: true,
        shared_bound,
    }
}

/// SPARQL solution mappings are compatible exactly when every variable bound
/// in both has the same RDF term. An empty shared domain is vacuously compatible.
pub(super) fn compatible(left: &SemanticMapping, right: &SemanticMapping) -> bool {
    compare(left, right).compatible
}

/// SPARQL MINUS additionally requires at least one variable bound in both.
pub(super) fn minus_matches(left: &SemanticMapping, right: &SemanticMapping) -> bool {
    let result = compare(left, right);
    result.compatible && result.shared_bound != 0
}

/// Lazy exact bag pairing; no deduplication or multiplicity cap is applied.
pub(super) fn compatible_bag_pairs<'a>(
    left: &'a [SemanticMapping],
    right: &'a [SemanticMapping],
) -> impl Iterator<Item = (usize, usize)> + 'a {
    left.iter().enumerate().flat_map(move |(left_index, l)| {
        right
            .iter()
            .enumerate()
            .filter_map(move |(right_index, r)| {
                compatible(l, r).then_some((left_index, right_index))
            })
    })
}

fn semantic_term_eq(left: &ScopedTerm, right: &ScopedTerm) -> bool {
    let mut left_term = left.term();
    let mut right_term = right.term();
    loop {
        match (left_term, right_term) {
            (Term::NamedNode(left), Term::NamedNode(right)) if left == right => return true,
            (Term::BlankNode(left_node), Term::BlankNode(right_node)) => {
                return left_node == right_node && left.blank_scope() == right.blank_scope();
            }
            (Term::Literal(left), Term::Literal(right)) => return literals_equal(left, right),
            (Term::Triple(left_triple), Term::Triple(right_triple)) => {
                if left_triple.predicate != right_triple.predicate
                    || !subjects_equal(
                        &left_triple.subject,
                        left.blank_scope(),
                        &right_triple.subject,
                        right.blank_scope(),
                    )
                {
                    return false;
                }
                left_term = &left_triple.object;
                right_term = &right_triple.object;
            }
            _ => return false,
        }
    }
}

fn subjects_equal(
    left: &NamedOrBlankNode,
    left_scope: &BlankNodeScope,
    right: &NamedOrBlankNode,
    right_scope: &BlankNodeScope,
) -> bool {
    match (left, right) {
        (NamedOrBlankNode::NamedNode(left), NamedOrBlankNode::NamedNode(right)) => left == right,
        (NamedOrBlankNode::BlankNode(left), NamedOrBlankNode::BlankNode(right)) => {
            left == right && left_scope == right_scope
        }
        _ => false,
    }
}

fn literals_equal(left: &Literal, right: &Literal) -> bool {
    if left.value() != right.value() {
        return false;
    }
    match (left.language(), right.language()) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        (None, None) => left.datatype() == right.datatype(),
        _ => false,
    }
}
