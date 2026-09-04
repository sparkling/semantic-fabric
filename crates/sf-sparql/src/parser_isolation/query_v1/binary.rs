use super::error::{QueryWireError, QueryWireLimit};
use super::model::{
    checked_wire_len, Arena, Record, HEADER_LEN, MAGIC, MAX_WIRE_BYTES_V1, RECORD_LEN, VERSION,
};

pub(super) fn encode_arena(arena: &Arena) -> Result<Vec<u8>, QueryWireError> {
    let record_count = arena.records.len();
    if record_count == 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    let total = checked_wire_len(record_count, arena.edges.len(), arena.scalar_bytes.len())?;
    let mut output = Vec::new();
    output.try_reserve_exact(total)?;
    output.extend_from_slice(&MAGIC);
    push_u16(&mut output, VERSION);
    push_u16(&mut output, HEADER_LEN as u16);
    push_u16(&mut output, RECORD_LEN as u16);
    push_u16(&mut output, 0);
    push_u32(&mut output, usize_to_u32(record_count - 1)?);
    push_u32(&mut output, usize_to_u32(record_count)?);
    push_u32(&mut output, usize_to_u32(arena.edges.len())?);
    push_u32(&mut output, usize_to_u32(arena.scalar_bytes.len())?);
    for record in &arena.records {
        push_u16(&mut output, record.tag);
        push_u16(&mut output, record.flags);
        for value in record.fields {
            push_u32(&mut output, value);
        }
        push_u32(&mut output, record.edge_start);
        push_u32(&mut output, record.edge_len);
    }
    for edge in &arena.edges {
        push_u32(&mut output, *edge);
    }
    output.extend_from_slice(&arena.scalar_bytes);
    if output.len() != total {
        return Err(QueryWireError::AccountingOverflow);
    }
    Ok(output)
}

/// Allocation-free view over an untrusted QueryV1 payload.
///
/// The parent validates every count, range, index, scalar, and record shape
/// through this borrowed view before allocating decode storage.
pub(super) struct BorrowedWire<'wire> {
    input: &'wire [u8],
    record_count: usize,
    edge_count: usize,
    edge_offset: usize,
    scalar_offset: usize,
}

impl BorrowedWire<'_> {
    pub fn record_count(&self) -> usize {
        self.record_count
    }

    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    pub fn record(&self, index: usize) -> Result<Record, QueryWireError> {
        if index >= self.record_count {
            return Err(QueryWireError::InvalidIndex);
        }
        let offset = HEADER_LEN
            .checked_add(
                index
                    .checked_mul(RECORD_LEN)
                    .ok_or(QueryWireError::AccountingOverflow)?,
            )
            .ok_or(QueryWireError::AccountingOverflow)?;
        let mut fields = [0_u32; 5];
        for (field_index, value) in fields.iter_mut().enumerate() {
            *value = read_u32(self.input, offset + 4 + field_index * 4)?;
        }
        Ok(Record {
            tag: read_u16(self.input, offset)?,
            flags: read_u16(self.input, offset + 2)?,
            fields,
            edge_start: read_u32(self.input, offset + 24)?,
            edge_len: read_u32(self.input, offset + 28)?,
        })
    }

    pub fn edge(&self, index: usize) -> Result<u32, QueryWireError> {
        if index >= self.edge_count {
            return Err(QueryWireError::InvalidIndex);
        }
        let offset = self
            .edge_offset
            .checked_add(
                index
                    .checked_mul(std::mem::size_of::<u32>())
                    .ok_or(QueryWireError::AccountingOverflow)?,
            )
            .ok_or(QueryWireError::AccountingOverflow)?;
        read_u32(self.input, offset)
    }

    pub fn scalar_bytes(&self) -> &[u8] {
        &self.input[self.scalar_offset..]
    }
}

pub(super) fn inspect_wire(input: &[u8]) -> Result<BorrowedWire<'_>, QueryWireError> {
    // Reject an oversized, otherwise arbitrary child response before reading
    // even its fixed header. Component limits bound canonical encodings more
    // tightly today; this independent envelope also closes oversized garbage
    // and remains authoritative if a later schema revision changes that mix.
    if input.len() > MAX_WIRE_BYTES_V1 {
        return Err(QueryWireError::LimitExceeded(QueryWireLimit::WireBytes));
    }
    if input.len() < HEADER_LEN
        || input[..MAGIC.len()] != MAGIC
        || read_u16(input, 8)? != VERSION
        || usize::from(read_u16(input, 10)?) != HEADER_LEN
        || usize::from(read_u16(input, 12)?) != RECORD_LEN
        || read_u16(input, 14)? != 0
    {
        return Err(QueryWireError::InvalidHeader);
    }
    let root = u32_to_usize(read_u32(input, 16)?)?;
    let record_count = u32_to_usize(read_u32(input, 20)?)?;
    let edge_count = u32_to_usize(read_u32(input, 24)?)?;
    let scalar_len = u32_to_usize(read_u32(input, 28)?)?;
    let total = checked_wire_len(record_count, edge_count, scalar_len)?;
    if record_count == 0 || root != record_count - 1 {
        return Err(QueryWireError::InvalidHeader);
    }
    if input.len() != total {
        return Err(QueryWireError::InvalidLength);
    }

    let edge_offset = HEADER_LEN
        .checked_add(
            record_count
                .checked_mul(RECORD_LEN)
                .ok_or(QueryWireError::AccountingOverflow)?,
        )
        .ok_or(QueryWireError::AccountingOverflow)?;
    let scalar_offset = edge_offset
        .checked_add(
            edge_count
                .checked_mul(std::mem::size_of::<u32>())
                .ok_or(QueryWireError::AccountingOverflow)?,
        )
        .ok_or(QueryWireError::AccountingOverflow)?;
    Ok(BorrowedWire {
        input,
        record_count,
        edge_count,
        edge_offset,
        scalar_offset,
    })
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn read_u16(input: &[u8], offset: usize) -> Result<u16, QueryWireError> {
    let bytes = input
        .get(offset..offset + 2)
        .ok_or(QueryWireError::InvalidLength)?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u32(input: &[u8], offset: usize) -> Result<u32, QueryWireError> {
    let bytes = input
        .get(offset..offset + 4)
        .ok_or(QueryWireError::InvalidLength)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub(super) fn usize_to_u32(value: usize) -> Result<u32, QueryWireError> {
    u32::try_from(value).map_err(|_| QueryWireError::AccountingOverflow)
}

pub(super) fn u32_to_usize(value: u32) -> Result<usize, QueryWireError> {
    usize::try_from(value).map_err(|_| QueryWireError::AccountingOverflow)
}
