use spargebra::algebra::PropertyPathExpression;

use super::allocation::try_box;
use super::decode_record::usize_from;
use super::decode_value::{ChildCursor, Value};
use super::error::QueryWireError;
use super::model::{tag, Record};

pub(super) fn decode_path(
    record: Record,
    mut children: ChildCursor,
) -> Result<Value, QueryWireError> {
    let path = match record.tag {
        tag::PATH_NAMED_NODE => PropertyPathExpression::NamedNode(children.named_node()?),
        tag::PATH_REVERSE => PropertyPathExpression::Reverse(try_box(children.path()?)?),
        tag::PATH_SEQUENCE => {
            PropertyPathExpression::Sequence(try_box(children.path()?)?, try_box(children.path()?)?)
        }
        tag::PATH_ALTERNATIVE => PropertyPathExpression::Alternative(
            try_box(children.path()?)?,
            try_box(children.path()?)?,
        ),
        tag::PATH_ZERO_OR_MORE => PropertyPathExpression::ZeroOrMore(try_box(children.path()?)?),
        tag::PATH_ONE_OR_MORE => PropertyPathExpression::OneOrMore(try_box(children.path()?)?),
        tag::PATH_ZERO_OR_ONE => PropertyPathExpression::ZeroOrOne(try_box(children.path()?)?),
        tag::PATH_NEGATED_SET => PropertyPathExpression::NegatedPropertySet(named_nodes(
            &mut children,
            usize_from(record.fields[0])?,
        )?),
        _ => return Err(QueryWireError::UnknownRecordKind),
    };
    children.finish()?;
    Ok(Value::Path(path))
}

fn named_nodes(
    children: &mut ChildCursor,
    len: usize,
) -> Result<Vec<spargebra::term::NamedNode>, QueryWireError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    for _ in 0..len {
        values.push(children.named_node()?);
    }
    Ok(values)
}
