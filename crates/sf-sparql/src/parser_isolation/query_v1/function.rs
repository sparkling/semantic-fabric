use spargebra::algebra::{AggregateFunction, Function};
use spargebra::term::NamedNode;

use super::error::QueryWireError;

macro_rules! function_codes {
    ($($variant:ident => $code:literal),+ $(,)?) => {
        pub(super) fn encode_function(function: &Function) -> (u32, Option<&NamedNode>) {
            match function {
                $(Function::$variant => ($code, None),)+
                Function::Custom(node) => (0, Some(node)),
            }
        }

        pub(super) fn decode_function(
            code: u32,
            custom: Option<NamedNode>,
        ) -> Result<Function, QueryWireError> {
            match (code, custom) {
                $(($code, None) => Ok(Function::$variant),)+
                (0, Some(node)) => Ok(Function::Custom(node)),
                _ => Err(QueryWireError::InvalidRecord),
            }
        }
    };
}

function_codes! {
    Str => 1,
    Lang => 2,
    LangMatches => 3,
    Datatype => 4,
    Iri => 5,
    BNode => 6,
    Rand => 7,
    Abs => 8,
    Ceil => 9,
    Floor => 10,
    Round => 11,
    Concat => 12,
    SubStr => 13,
    StrLen => 14,
    Replace => 15,
    UCase => 16,
    LCase => 17,
    EncodeForUri => 18,
    Contains => 19,
    StrStarts => 20,
    StrEnds => 21,
    StrBefore => 22,
    StrAfter => 23,
    Year => 24,
    Month => 25,
    Day => 26,
    Hours => 27,
    Minutes => 28,
    Seconds => 29,
    Timezone => 30,
    Tz => 31,
    Now => 32,
    Uuid => 33,
    StrUuid => 34,
    Md5 => 35,
    Sha1 => 36,
    Sha256 => 37,
    Sha384 => 38,
    Sha512 => 39,
    StrLang => 40,
    StrDt => 41,
    IsIri => 42,
    IsBlank => 43,
    IsLiteral => 44,
    IsNumeric => 45,
    Regex => 46,
    Triple => 47,
    Subject => 48,
    Predicate => 49,
    Object => 50,
    IsTriple => 51,
    LangDir => 52,
    HasLang => 53,
    HasLangDir => 54,
    StrLangDir => 55,
    Adjust => 56,
}

pub(super) enum AggregateFunctionRef<'a> {
    Builtin(u32),
    GroupConcat(Option<&'a str>),
    Custom(&'a NamedNode),
}

pub(super) fn encode_aggregate_function(function: &AggregateFunction) -> AggregateFunctionRef<'_> {
    match function {
        AggregateFunction::Count => AggregateFunctionRef::Builtin(1),
        AggregateFunction::Sum => AggregateFunctionRef::Builtin(2),
        AggregateFunction::Avg => AggregateFunctionRef::Builtin(3),
        AggregateFunction::Min => AggregateFunctionRef::Builtin(4),
        AggregateFunction::Max => AggregateFunctionRef::Builtin(5),
        AggregateFunction::GroupConcat { separator } => {
            AggregateFunctionRef::GroupConcat(separator.as_deref())
        }
        AggregateFunction::Sample => AggregateFunctionRef::Builtin(7),
        AggregateFunction::Custom(node) => AggregateFunctionRef::Custom(node),
    }
}

pub(super) fn decode_aggregate_function(
    code: u32,
    separator_present: bool,
    separator: Option<String>,
    custom: Option<NamedNode>,
) -> Result<AggregateFunction, QueryWireError> {
    match (code, separator_present, separator, custom) {
        (1, false, None, None) => Ok(AggregateFunction::Count),
        (2, false, None, None) => Ok(AggregateFunction::Sum),
        (3, false, None, None) => Ok(AggregateFunction::Avg),
        (4, false, None, None) => Ok(AggregateFunction::Min),
        (5, false, None, None) => Ok(AggregateFunction::Max),
        (6, false, None, None) => Ok(AggregateFunction::GroupConcat { separator: None }),
        (6, true, Some(separator), None) => Ok(AggregateFunction::GroupConcat {
            separator: Some(separator),
        }),
        (7, false, None, None) => Ok(AggregateFunction::Sample),
        (0, false, None, Some(node)) => Ok(AggregateFunction::Custom(node)),
        _ => Err(QueryWireError::InvalidRecord),
    }
}
