use spargebra::algebra::Expression;

use super::allocation::try_box;
use super::decode_record::usize_from;
use super::decode_value::{ChildCursor, Value};
use super::error::QueryWireError;
use super::model::{tag, Record};

pub(super) fn decode_expression(
    record: Record,
    mut children: ChildCursor,
) -> Result<Value, QueryWireError> {
    let expression = match record.tag {
        tag::EXPR_NAMED_NODE => Expression::NamedNode(children.named_node()?),
        tag::EXPR_LITERAL => Expression::Literal(children.literal()?),
        tag::EXPR_VARIABLE => Expression::Variable(children.variable()?),
        tag::EXPR_OR => {
            let (left, right) = binary(&mut children)?;
            Expression::Or(left, right)
        }
        tag::EXPR_AND => {
            let (left, right) = binary(&mut children)?;
            Expression::And(left, right)
        }
        tag::EXPR_EQUAL => {
            let (left, right) = binary(&mut children)?;
            Expression::Equal(left, right)
        }
        tag::EXPR_SAME_TERM => {
            let (left, right) = binary(&mut children)?;
            Expression::SameTerm(left, right)
        }
        tag::EXPR_GREATER => {
            let (left, right) = binary(&mut children)?;
            Expression::Greater(left, right)
        }
        tag::EXPR_GREATER_EQUAL => {
            let (left, right) = binary(&mut children)?;
            Expression::GreaterOrEqual(left, right)
        }
        tag::EXPR_LESS => {
            let (left, right) = binary(&mut children)?;
            Expression::Less(left, right)
        }
        tag::EXPR_LESS_EQUAL => {
            let (left, right) = binary(&mut children)?;
            Expression::LessOrEqual(left, right)
        }
        tag::EXPR_IN => Expression::In(
            try_box(children.expression()?)?,
            expressions(&mut children, usize_from(record.fields[0])?)?,
        ),
        tag::EXPR_ADD => {
            let (left, right) = binary(&mut children)?;
            Expression::Add(left, right)
        }
        tag::EXPR_SUBTRACT => {
            let (left, right) = binary(&mut children)?;
            Expression::Subtract(left, right)
        }
        tag::EXPR_MULTIPLY => {
            let (left, right) = binary(&mut children)?;
            Expression::Multiply(left, right)
        }
        tag::EXPR_DIVIDE => {
            let (left, right) = binary(&mut children)?;
            Expression::Divide(left, right)
        }
        tag::EXPR_UNARY_PLUS => Expression::UnaryPlus(unary(&mut children)?),
        tag::EXPR_UNARY_MINUS => Expression::UnaryMinus(unary(&mut children)?),
        tag::EXPR_NOT => Expression::Not(unary(&mut children)?),
        tag::EXPR_EXISTS => Expression::Exists(try_box(children.graph()?)?),
        tag::EXPR_BOUND => Expression::Bound(children.variable()?),
        tag::EXPR_IF => Expression::If(
            try_box(children.expression()?)?,
            try_box(children.expression()?)?,
            try_box(children.expression()?)?,
        ),
        tag::EXPR_COALESCE => {
            Expression::Coalesce(expressions(&mut children, usize_from(record.fields[0])?)?)
        }
        tag::EXPR_FUNCTION_CALL => Expression::FunctionCall(
            children.function()?,
            expressions(&mut children, usize_from(record.fields[0])?)?,
        ),
        _ => return Err(QueryWireError::UnknownRecordKind),
    };
    children.finish()?;
    Ok(Value::Expression(expression))
}

fn unary(children: &mut ChildCursor) -> Result<Box<Expression>, QueryWireError> {
    try_box(children.expression()?)
}

fn binary(
    children: &mut ChildCursor,
) -> Result<(Box<Expression>, Box<Expression>), QueryWireError> {
    let left = try_box(children.expression()?)?;
    let right = try_box(children.expression()?)?;
    Ok((left, right))
}

fn expressions(children: &mut ChildCursor, len: usize) -> Result<Vec<Expression>, QueryWireError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    for _ in 0..len {
        values.push(children.expression()?);
    }
    Ok(values)
}
