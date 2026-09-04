use super::error::QueryWireError;
use super::model::{tag, Record};

pub(super) fn validate_scalar_values(
    record: Record,
    scalar_bytes: &[u8],
) -> Result<(), QueryWireError> {
    match record.tag {
        tag::NAMED_NODE => {
            oxrdf::NamedNodeRef::new(scalar(record, scalar_bytes, 0, 1)?)
                .map_err(|_| QueryWireError::InvalidScalar)?;
        }
        tag::VARIABLE => {
            oxrdf::VariableRef::new(scalar(record, scalar_bytes, 0, 1)?)
                .map_err(|_| QueryWireError::InvalidScalar)?;
        }
        tag::BLANK_NODE => {
            oxrdf::BlankNodeRef::new(scalar(record, scalar_bytes, 0, 1)?)
                .map_err(|_| QueryWireError::InvalidScalar)?;
        }
        tag::BASE_IRI => {
            oxiri::Iri::parse(scalar(record, scalar_bytes, 0, 1)?)
                .map_err(|_| QueryWireError::InvalidScalar)?;
        }
        tag::LITERAL => validate_literal_values(record, scalar_bytes)?,
        _ => {}
    }
    Ok(())
}

fn validate_literal_values(record: Record, scalar_bytes: &[u8]) -> Result<(), QueryWireError> {
    if record.fields[4] == 1 {
        let datatype = oxrdf::NamedNodeRef::new(scalar(record, scalar_bytes, 2, 3)?)
            .map_err(|_| QueryWireError::InvalidScalar)?;
        if datatype == oxrdf::vocab::xsd::STRING {
            return Err(QueryWireError::NonCanonical);
        }
    } else if matches!(record.fields[4], 2..=4) {
        let language = scalar(record, scalar_bytes, 2, 3)?;
        oxilangtag::LanguageTag::parse(language).map_err(|_| QueryWireError::InvalidScalar)?;
        if language.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(QueryWireError::NonCanonical);
        }
    }
    Ok(())
}

fn scalar(
    record: Record,
    scalar_bytes: &[u8],
    offset_field: usize,
    len_field: usize,
) -> Result<&str, QueryWireError> {
    let offset = usize::try_from(record.fields[offset_field])
        .map_err(|_| QueryWireError::AccountingOverflow)?;
    let len = usize::try_from(record.fields[len_field])
        .map_err(|_| QueryWireError::AccountingOverflow)?;
    let end = offset
        .checked_add(len)
        .ok_or(QueryWireError::AccountingOverflow)?;
    std::str::from_utf8(
        scalar_bytes
            .get(offset..end)
            .ok_or(QueryWireError::InvalidScalar)?,
    )
    .map_err(|_| QueryWireError::InvalidScalar)
}

pub(super) fn validate_record_shape(record: Record) -> Result<(), QueryWireError> {
    let expected_edges = match record.tag {
        tag::QUERY_SELECT | tag::QUERY_DESCRIBE | tag::QUERY_ASK => {
            flags_mask(record, 0b11)?;
            zero_fields(record, 0)?;
            1 + u64::from(record.flags & 1 != 0) + u64::from(record.flags & 2 != 0)
        }
        tag::QUERY_CONSTRUCT => {
            flags_mask(record, 0b11)?;
            zero_fields(record, 1)?;
            u64::from(record.fields[0])
                + 1
                + u64::from(record.flags & 1 != 0)
                + u64::from(record.flags & 2 != 0)
        }
        tag::DATASET => {
            bool_flags(record)?;
            zero_fields(record, 2)?;
            if record.flags == 0 && record.fields[1] != 0 {
                return Err(QueryWireError::InvalidRecord);
            }
            u64::from(record.fields[0]) + u64::from(record.fields[1])
        }
        tag::GRAPH_BGP => counted(record, 0, 1)?,
        tag::GRAPH_PATH => fixed(record, 3)?,
        tag::GRAPH_JOIN
        | tag::GRAPH_LATERAL
        | tag::GRAPH_FILTER
        | tag::GRAPH_UNION
        | tag::GRAPH_NAMED
        | tag::GRAPH_MINUS => fixed(record, 2)?,
        tag::GRAPH_LEFT_JOIN => {
            bool_flags(record)?;
            zero_fields(record, 0)?;
            2 + u64::from(record.flags)
        }
        tag::GRAPH_EXTEND => fixed(record, 3)?,
        tag::GRAPH_VALUES => two_counts(record, 0)?,
        tag::GRAPH_ORDER_BY | tag::GRAPH_PROJECT => counted_plus_one(record)?,
        tag::GRAPH_DISTINCT | tag::GRAPH_REDUCED => fixed(record, 1)?,
        tag::GRAPH_SLICE => {
            bool_flags(record)?;
            if record.fields[4] != 0
                || (record.flags == 0 && (record.fields[2] != 0 || record.fields[3] != 0))
            {
                return Err(QueryWireError::InvalidRecord);
            }
            1
        }
        tag::GRAPH_GROUP => two_counts_plus_one(record)?,
        tag::GRAPH_SERVICE => {
            bool_flags(record)?;
            zero_fields(record, 0)?;
            2
        }
        tag::EXPR_NAMED_NODE | tag::EXPR_LITERAL | tag::EXPR_VARIABLE | tag::EXPR_BOUND => {
            fixed(record, 1)?
        }
        tag::EXPR_OR
        | tag::EXPR_AND
        | tag::EXPR_EQUAL
        | tag::EXPR_SAME_TERM
        | tag::EXPR_GREATER
        | tag::EXPR_GREATER_EQUAL
        | tag::EXPR_LESS
        | tag::EXPR_LESS_EQUAL
        | tag::EXPR_ADD
        | tag::EXPR_SUBTRACT
        | tag::EXPR_MULTIPLY
        | tag::EXPR_DIVIDE => fixed(record, 2)?,
        tag::EXPR_IN | tag::EXPR_FUNCTION_CALL => counted_plus_one(record)?,
        tag::EXPR_UNARY_PLUS | tag::EXPR_UNARY_MINUS | tag::EXPR_NOT | tag::EXPR_EXISTS => {
            fixed(record, 1)?
        }
        tag::EXPR_IF => fixed(record, 3)?,
        tag::EXPR_COALESCE => counted(record, 0, 1)?,
        tag::FUNCTION => function_shape(record)?,
        tag::PATH_NAMED_NODE
        | tag::PATH_REVERSE
        | tag::PATH_ZERO_OR_MORE
        | tag::PATH_ONE_OR_MORE
        | tag::PATH_ZERO_OR_ONE => fixed(record, 1)?,
        tag::PATH_SEQUENCE | tag::PATH_ALTERNATIVE => fixed(record, 2)?,
        tag::PATH_NEGATED_SET => counted(record, 0, 1)?,
        tag::AGGREGATE_COUNT_SOLUTIONS => {
            bool_flags(record)?;
            zero_fields(record, 0)?;
            0
        }
        tag::AGGREGATE_FUNCTION_CALL => {
            bool_flags(record)?;
            zero_fields(record, 0)?;
            2
        }
        tag::AGGREGATE_FUNCTION => aggregate_function_shape(record)?,
        tag::ORDER_ASC | tag::ORDER_DESC => fixed(record, 1)?,
        tag::TRIPLE_PATTERN | tag::GROUND_TRIPLE => fixed(record, 3)?,
        tag::TERM_NAMED_NODE
        | tag::TERM_BLANK_NODE
        | tag::TERM_LITERAL
        | tag::TERM_TRIPLE
        | tag::TERM_VARIABLE
        | tag::NAMED_PATTERN_NODE
        | tag::NAMED_PATTERN_VARIABLE
        | tag::GROUND_TERM_NAMED_NODE
        | tag::GROUND_TERM_LITERAL
        | tag::GROUND_TERM_TRIPLE => fixed(record, 1)?,
        tag::NAMED_NODE | tag::VARIABLE | tag::BLANK_NODE | tag::BASE_IRI => {
            scalar_shape(record)?;
            0
        }
        tag::LITERAL => {
            literal_shape(record)?;
            0
        }
        tag::VALUES_ROW => counted(record, 0, 1)?,
        tag::AGGREGATE_BINDING => fixed(record, 2)?,
        _ => return Err(QueryWireError::UnknownRecordKind),
    };
    if u64::from(record.edge_len) != expected_edges {
        return Err(QueryWireError::InvalidRecord);
    }
    Ok(())
}

pub(super) fn is_query_tag(tag: u16) -> bool {
    matches!(
        tag,
        tag::QUERY_SELECT | tag::QUERY_CONSTRUCT | tag::QUERY_DESCRIBE | tag::QUERY_ASK
    )
}

pub(super) fn scalar_spans(record: Record) -> ([(u32, u32); 2], usize) {
    let first = (record.fields[0], record.fields[1]);
    match record.tag {
        tag::NAMED_NODE | tag::VARIABLE | tag::BLANK_NODE | tag::BASE_IRI => ([first, (0, 0)], 1),
        tag::LITERAL if record.fields[4] == 0 => ([first, (0, 0)], 1),
        tag::LITERAL => ([first, (record.fields[2], record.fields[3])], 2),
        tag::AGGREGATE_FUNCTION if record.fields[0] == 6 && record.flags == 1 => {
            ([(record.fields[1], record.fields[2]), (0, 0)], 1)
        }
        _ => ([(0, 0), (0, 0)], 0),
    }
}

fn fixed(record: Record, edges: u64) -> Result<u64, QueryWireError> {
    if record.flags != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    zero_fields(record, 0)?;
    Ok(edges)
}

fn counted(record: Record, field: usize, zero_from: usize) -> Result<u64, QueryWireError> {
    if record.flags != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    zero_fields(record, zero_from)?;
    Ok(u64::from(record.fields[field]))
}

fn counted_plus_one(record: Record) -> Result<u64, QueryWireError> {
    counted(record, 0, 1).map(|count| count + 1)
}

fn two_counts(record: Record, extra: u64) -> Result<u64, QueryWireError> {
    if record.flags != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    zero_fields(record, 2)?;
    Ok(u64::from(record.fields[0]) + u64::from(record.fields[1]) + extra)
}

fn two_counts_plus_one(record: Record) -> Result<u64, QueryWireError> {
    two_counts(record, 1)
}

fn scalar_shape(record: Record) -> Result<(), QueryWireError> {
    if record.flags != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    zero_fields(record, 2)
}

fn literal_shape(record: Record) -> Result<(), QueryWireError> {
    if record.flags != 0 || record.fields[4] > 4 {
        return Err(QueryWireError::InvalidRecord);
    }
    if record.fields[4] == 0 && (record.fields[2] != 0 || record.fields[3] != 0) {
        return Err(QueryWireError::InvalidRecord);
    }
    Ok(())
}

fn function_shape(record: Record) -> Result<u64, QueryWireError> {
    if record.flags != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    zero_fields(record, 1)?;
    match record.fields[0] {
        0 => Ok(1),
        1..=56 => Ok(0),
        _ => Err(QueryWireError::InvalidRecord),
    }
}

fn aggregate_function_shape(record: Record) -> Result<u64, QueryWireError> {
    if record.fields[3] != 0 || record.fields[4] != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    match record.fields[0] {
        0 if record.flags == 0 && record.fields[1] == 0 && record.fields[2] == 0 => Ok(1),
        1..=5 | 7 if record.flags == 0 && record.fields[1] == 0 && record.fields[2] == 0 => Ok(0),
        6 if record.flags == 0 && record.fields[1] == 0 && record.fields[2] == 0 => Ok(0),
        6 if record.flags == 1 => Ok(0),
        _ => Err(QueryWireError::InvalidRecord),
    }
}

fn flags_mask(record: Record, mask: u16) -> Result<(), QueryWireError> {
    if record.flags & !mask == 0 {
        Ok(())
    } else {
        Err(QueryWireError::InvalidRecord)
    }
}

fn bool_flags(record: Record) -> Result<(), QueryWireError> {
    if record.flags <= 1 {
        Ok(())
    } else {
        Err(QueryWireError::InvalidRecord)
    }
}

fn zero_fields(record: Record, from: usize) -> Result<(), QueryWireError> {
    if record.fields[from..].iter().all(|field| *field == 0) {
        Ok(())
    } else {
        Err(QueryWireError::InvalidRecord)
    }
}
