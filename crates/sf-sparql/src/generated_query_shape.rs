//! Structural-only screening of an already-parsed SPARQL query.
//! A pass proves no mapping coverage, graph allowlist or complete admission.

use std::fmt;

use sf_core::query_control::{QueryControl, QueryControlError};
use spargebra::algebra::{Function, GraphPattern};
use spargebra::Query;
use terms::{ConstantOccurrence, ConstantRejection, ConstantRole};

#[path = "generated_query_constant_terms.rs"]
mod terms;
#[path = "generated_query_shape_walk.rs"]
mod walk;

/// Named refusal rule. Carries no query text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShapeRule {
    ConstructForm,
    DescribeForm,
    DatasetClause,
    ServiceInPattern,
    RdfStarUnsupported,
    CustomFunctionUnsupported,
    CustomAggregateUnsupported,
    UnclassifiedForm,
}

impl ShapeRule {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::ConstructForm => "construct-form",
            Self::DescribeForm => "describe-form",
            Self::DatasetClause => "dataset-clause",
            Self::ServiceInPattern => "service-in-pattern",
            Self::RdfStarUnsupported => "rdf-star-unsupported",
            Self::CustomFunctionUnsupported => "custom-function-unsupported",
            Self::CustomAggregateUnsupported => "custom-aggregate-unsupported",
            Self::UnclassifiedForm => "unclassified-form",
        }
    }
}

/// Redacted refusal, independent of the crate `Error`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShapeRefusal {
    Rule(ShapeRule),
    Control(QueryControlError),
    ConstantRejected(ConstantRejection),
}

impl fmt::Display for ShapeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule(rule) => write!(f, "generated-query shape refused: {}", rule.code()),
            Self::Control(cause) => write!(f, "generated-query shape stopped: {cause}"),
            Self::ConstantRejected(_) => f.write_str("generated-query constant visit rejected"),
        }
    }
}

impl std::error::Error for ShapeRefusal {}

/// Result of a structural-only screen. This is NOT an admission proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StructuralOnlyScreen {
    charged_compiler_work: u64,
}

impl StructuralOnlyScreen {
    pub(crate) const fn charged_compiler_work(&self) -> u64 {
        self.charged_compiler_work
    }
}

fn refuse(rule: ShapeRule) -> Result<(), ShapeRefusal> {
    Err(ShapeRefusal::Rule(rule))
}

fn screenable_pattern<'a>(
    query: &'a Query,
    control: &dyn QueryControl,
) -> Result<&'a GraphPattern, ShapeRefusal> {
    control.checkpoint().map_err(ShapeRefusal::Control)?;
    let pattern = match query {
        Query::Select { pattern, .. } | Query::Ask { pattern, .. } => pattern,
        Query::Construct { .. } => return Err(ShapeRefusal::Rule(ShapeRule::ConstructForm)),
        Query::Describe { .. } => return Err(ShapeRefusal::Rule(ShapeRule::DescribeForm)),
    };
    if query.dataset().is_some() {
        return Err(ShapeRefusal::Rule(ShapeRule::DatasetClause));
    }
    Ok(pattern)
}

/// Screen a parsed SELECT/ASK query. Structural-only: never proves admission.
pub(crate) fn screen_parsed_query_structure(
    query: &Query,
    control: &dyn QueryControl,
) -> Result<StructuralOnlyScreen, ShapeRefusal> {
    let pattern = screenable_pattern(query, control)?;
    let charged_compiler_work = walk::Walk::new(control).run(pattern)?;
    Ok(StructuralOnlyScreen {
        charged_compiler_work,
    })
}

/// Same screen, additionally reporting every constant IRI with its role to
/// `visit`. Each occurrence is charged and checkpointed before its callback.
/// A callback error ends the walk. The callback is a checking seam only.
pub(crate) fn visit_parsed_query_constants<'a, F>(
    query: &'a Query,
    control: &dyn QueryControl,
    mut visit: F,
) -> Result<StructuralOnlyScreen, ShapeRefusal>
where
    F: FnMut(ConstantOccurrence<'a>) -> Result<(), ConstantRejection>,
{
    let pattern = screenable_pattern(query, control)?;
    let visitor = walk::Walk::with_visitor(control, &mut visit);
    let charged_compiler_work = visitor.run(pattern)?;
    Ok(StructuralOnlyScreen {
        charged_compiler_work,
    })
}

// Only sparql-12 RDF-star functions are named: lib.rs already relies on that
// feature unguarded. Every other feature-gated variant (rest of sparql-12,
// sep-0002) is unnamed here and lands in the fail-closed wildcard arm.
#[allow(unreachable_patterns)]
fn check_function(function: &Function) -> Result<(), ShapeRefusal> {
    match function {
        Function::Custom(_) => refuse(ShapeRule::CustomFunctionUnsupported),
        Function::Triple
        | Function::Subject
        | Function::Predicate
        | Function::Object
        | Function::IsTriple => refuse(ShapeRule::RdfStarUnsupported),
        Function::Str
        | Function::Lang
        | Function::LangMatches
        | Function::Datatype
        | Function::Iri
        | Function::BNode
        | Function::Rand
        | Function::Abs
        | Function::Ceil
        | Function::Floor
        | Function::Round
        | Function::Concat
        | Function::SubStr
        | Function::StrLen
        | Function::Replace
        | Function::UCase
        | Function::LCase
        | Function::EncodeForUri
        | Function::Contains
        | Function::StrStarts
        | Function::StrEnds
        | Function::StrBefore
        | Function::StrAfter
        | Function::Year
        | Function::Month
        | Function::Day
        | Function::Hours
        | Function::Minutes
        | Function::Seconds
        | Function::Timezone
        | Function::Tz
        | Function::Now
        | Function::Uuid
        | Function::StrUuid
        | Function::Md5
        | Function::Sha1
        | Function::Sha256
        | Function::Sha384
        | Function::Sha512
        | Function::StrLang
        | Function::StrDt
        | Function::IsIri
        | Function::IsBlank
        | Function::IsLiteral
        | Function::IsNumeric
        | Function::Regex => Ok(()),
        _ => refuse(ShapeRule::UnclassifiedForm),
    }
}

#[cfg(test)]
#[path = "generated_query_shape_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "generated_query_constant_tests.rs"]
mod constant_tests;

#[cfg(test)]
#[path = "generated_query_constant_control_tests.rs"]
mod constant_control_tests;
