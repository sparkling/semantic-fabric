//! Hygienic lowering for the admitted one-target DESCRIBE profile.

use std::collections::BTreeSet;

use spargebra::algebra::{Expression, GraphPattern};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};

use crate::Error;

/// Lower one DESCRIBE target to its one-hop outgoing-description graph.
///
/// SPARQL leaves DESCRIBE graph construction implementation-defined. This
/// profile deliberately emits only outgoing triples for one target. Multiple
/// targets remain unsupported until their union and graph-set dedup have a
/// source-independent memory proof.
pub(crate) fn rewrite(pattern: &GraphPattern) -> Result<(GraphPattern, Vec<TriplePattern>), Error> {
    let (targets, inner) = match pattern {
        GraphPattern::Project { variables, inner } => (variables.clone(), inner.as_ref().clone()),
        other => (Vec::new(), other.clone()),
    };
    if targets.len() != 1 {
        return Err(Error::Unsupported(
            "DESCRIBE currently admits exactly one target".to_owned(),
        ));
    }

    let target = targets.into_iter().next().expect("exactly one target");
    let (subject, inner) = description_subject(target, inner)?;
    let mut names = crate::star::collect_pattern_vars(&inner)
        .into_iter()
        .map(|variable| variable.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    if let TermPattern::Variable(variable) = &subject {
        names.insert(variable.as_str().to_owned());
    }
    let predicate = fresh(&mut names, "predicate");
    let object = fresh(&mut names, "object");
    let triple = TriplePattern {
        subject,
        predicate: NamedNodePattern::Variable(predicate),
        object: TermPattern::Variable(object),
    };
    Ok((
        GraphPattern::Join {
            left: Box::new(inner),
            right: Box::new(GraphPattern::Bgp {
                patterns: vec![triple.clone()],
            }),
        },
        vec![triple],
    ))
}

/// Recover spargebra's generated constant-target BIND without joining through
/// its symbolic variable. Variable targets must already be bound by the WHERE
/// pattern; treating an unbound projected variable as the outgoing-triple
/// subject would incorrectly turn `DESCRIBE ?s` into an all-subject scan.
fn description_subject(
    target: Variable,
    inner: GraphPattern,
) -> Result<(TermPattern, GraphPattern), Error> {
    if let GraphPattern::Extend {
        inner: base,
        variable,
        expression: Expression::NamedNode(node),
    } = &inner
    {
        if variable == &target {
            return Ok((TermPattern::NamedNode(node.clone()), base.as_ref().clone()));
        }
    }

    let mut target_is_bound = false;
    inner.on_in_scope_variable(|variable| {
        target_is_bound |= variable == &target;
    });
    if target_is_bound {
        Ok((TermPattern::Variable(target), inner))
    } else {
        Err(Error::Unsupported(
            "DESCRIBE variable target is not bound by its WHERE pattern".to_owned(),
        ))
    }
}

fn fresh(names: &mut BTreeSet<String>, role: &str) -> Variable {
    for ordinal in 0usize.. {
        let candidate = format!("__sf_describe_{role}_{ordinal}");
        if names.insert(candidate.clone()) {
            return Variable::new_unchecked(candidate);
        }
    }
    unreachable!("unbounded ordinal sequence contains a fresh variable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::{Query, SparqlParser};

    fn describe_pattern(source: &str) -> GraphPattern {
        match SparqlParser::new().parse_query(source).unwrap() {
            Query::Describe { pattern, .. } => pattern,
            _ => unreachable!(),
        }
    }

    #[test]
    fn legal_user_names_do_not_capture_internal_bindings() {
        let pattern = describe_pattern(
            "DESCRIBE ?s WHERE { ?s ?__sf_describe_predicate_0 ?__sf_describe_object_0 }",
        );
        let (_, template) = rewrite(&pattern).unwrap();
        let NamedNodePattern::Variable(predicate) = &template[0].predicate else {
            unreachable!()
        };
        let TermPattern::Variable(object) = &template[0].object else {
            unreachable!()
        };
        assert_eq!(predicate.as_str(), "__sf_describe_predicate_1");
        assert_eq!(object.as_str(), "__sf_describe_object_1");
    }

    #[test]
    fn names_mentioned_only_in_expressions_are_still_reserved() {
        let pattern = describe_pattern(
            "DESCRIBE ?s WHERE { VALUES ?s { <http://ex/a> } FILTER(!BOUND(?__sf_describe_predicate_0) && !BOUND(?__sf_describe_object_0)) }",
        );
        let (_, template) = rewrite(&pattern).unwrap();
        let NamedNodePattern::Variable(predicate) = &template[0].predicate else {
            unreachable!()
        };
        let TermPattern::Variable(object) = &template[0].object else {
            unreachable!()
        };
        assert_eq!(predicate.as_str(), "__sf_describe_predicate_1");
        assert_eq!(object.as_str(), "__sf_describe_object_1");
    }

    #[test]
    fn multiple_targets_reject_instead_of_becoming_a_conjunctive_under_answer() {
        let pattern = describe_pattern("DESCRIBE <http://ex/a> <http://ex/b>");
        assert!(matches!(rewrite(&pattern), Err(Error::Unsupported(_))));
    }

    #[test]
    fn constant_target_is_lowered_without_a_symbolic_shared_bind() {
        let pattern = describe_pattern("DESCRIBE <http://ex/a>");
        let (rewritten, template) = rewrite(&pattern).unwrap();
        assert!(matches!(
            &template[0].subject,
            TermPattern::NamedNode(node) if node.as_str() == "http://ex/a"
        ));
        assert!(matches!(
            rewritten,
            GraphPattern::Join { left, .. }
                if matches!(left.as_ref(), GraphPattern::Bgp { patterns } if patterns.is_empty())
        ));
    }

    #[test]
    fn unbound_variable_target_rejects_instead_of_scanning_every_subject() {
        let pattern = describe_pattern("DESCRIBE ?s WHERE { ?x ?p ?o }");
        assert!(matches!(rewrite(&pattern), Err(Error::Unsupported(_))));
    }
}
