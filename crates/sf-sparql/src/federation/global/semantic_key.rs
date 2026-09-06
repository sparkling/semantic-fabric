use oxrdf::{GraphName, NamedOrBlankNode, Term};
use sf_core::query_control::{QueryBudget, ReservationError, ReservationShape, ReservationToken};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 4] = b"SFSK";
const VERSION: u8 = 1;
const HARD_MAX_VARIABLES: usize = 4_096;
const HARD_MAX_ENCODED_BYTES: usize = 16 * 1024 * 1024;
const HARD_MAX_TRIPLE_DEPTH: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BlankNodeOwner {
    Request(Box<str>),
    Dataset(Box<str>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BlankNodeScope {
    owner: BlankNodeOwner,
    graph: GraphName,
}

impl BlankNodeScope {
    pub(super) fn request(identity: impl Into<Box<str>>, graph: GraphName) -> Self {
        Self {
            owner: BlankNodeOwner::Request(identity.into()),
            graph,
        }
    }

    pub(super) fn dataset(identity: impl Into<Box<str>>, graph: GraphName) -> Self {
        Self {
            owner: BlankNodeOwner::Dataset(identity.into()),
            graph,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ScopedTerm {
    term: Term,
    blank_scope: BlankNodeScope,
}

impl ScopedTerm {
    pub(super) const fn new(term: Term, blank_scope: BlankNodeScope) -> Self {
        Self { term, blank_scope }
    }

    pub(super) const fn term(&self) -> &Term {
        &self.term
    }

    pub(super) const fn blank_scope(&self) -> &BlankNodeScope {
        &self.blank_scope
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SemanticKeyCaps {
    max_encoded_bytes: usize,
    max_variables: usize,
    max_triple_depth: usize,
}

impl SemanticKeyCaps {
    pub(super) fn new(
        max_encoded_bytes: usize,
        max_variables: usize,
        max_triple_depth: usize,
    ) -> Result<Self, SemanticKeyError> {
        if max_encoded_bytes == 0
            || max_encoded_bytes > HARD_MAX_ENCODED_BYTES
            || max_variables == 0
            || max_variables > HARD_MAX_VARIABLES
            || max_triple_depth > HARD_MAX_TRIPLE_DEPTH
        {
            return Err(SemanticKeyError::InvalidCaps);
        }
        Ok(Self {
            max_encoded_bytes,
            max_variables,
            max_triple_depth,
        })
    }
}

impl Default for SemanticKeyCaps {
    fn default() -> Self {
        Self {
            max_encoded_bytes: 64 * 1024,
            max_variables: 256,
            max_triple_depth: 16,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum SemanticKeyError {
    #[error("semantic-key caps are invalid")]
    InvalidCaps,
    #[error("semantic-key variable is malformed")]
    MalformedVariable,
    #[error("semantic-key variable is duplicated")]
    DuplicateVariable,
    #[error("semantic-key schema does not match the mapping")]
    SchemaMismatch,
    #[error("semantic-key variable limit exceeded")]
    TooManyVariables,
    #[error("semantic-key component is too large")]
    ComponentTooLarge,
    #[error("semantic-key blank-node scope is malformed")]
    MalformedBlankScope,
    #[error("semantic-key byte limit exceeded")]
    EncodedBytesExceeded,
    #[error("semantic-key triple recursion limit exceeded")]
    TripleDepthExceeded,
    #[error("semantic-key length arithmetic overflow")]
    LengthOverflow,
    #[error("semantic-key resource reservation rejected")]
    Reservation(ReservationError),
}

impl From<ReservationError> for SemanticKeyError {
    fn from(error: ReservationError) -> Self {
        Self::Reservation(error)
    }
}

#[derive(Debug)]
pub(super) struct SemanticSolutionKeyV1 {
    bytes: Vec<u8>,
    digest: [u8; 32],
    _reservation: ReservationToken,
}

impl SemanticSolutionKeyV1 {
    pub(super) fn encode(
        schema: &[&str],
        mapping: &super::mapping::SemanticMapping,
        budget: &QueryBudget,
        caps: SemanticKeyCaps,
    ) -> Result<Self, SemanticKeyError> {
        validate_schema(schema, mapping, caps)?;

        let mut measure = Encoder::measure(caps.max_encoded_bytes);
        encode_mapping(&mut measure, schema, mapping, caps.max_triple_depth)?;
        let encoded_len = measure.len();
        let retained_bytes =
            u64::try_from(encoded_len).map_err(|_| SemanticKeyError::LengthOverflow)?;
        let reservation = budget.reserve(ReservationShape::for_retained_bytes(retained_bytes))?;

        // This is the first output-buffer allocation. Any panic unwinds through
        // `reservation`, restoring the admitted capacity.
        let mut bytes = Vec::with_capacity(encoded_len);
        {
            let mut writer = Encoder::writer(&mut bytes, caps.max_encoded_bytes);
            encode_mapping(&mut writer, schema, mapping, caps.max_triple_depth)?;
            debug_assert_eq!(writer.len(), encoded_len);
        }

        let digest = Sha256::digest(&bytes).into();
        Ok(Self {
            bytes,
            digest,
            _reservation: reservation,
        })
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

impl PartialEq for SemanticSolutionKeyV1 {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest && self.bytes == other.bytes
    }
}

impl Eq for SemanticSolutionKeyV1 {}

/// Testable collision seam: even an injected equal hash cannot bypass full key
/// comparison. Both arguments are already-complete keys, so partial keys are
/// never hashable through this API.
pub(super) fn equal_after_hash_by<H: Eq>(
    left: &SemanticSolutionKeyV1,
    right: &SemanticSolutionKeyV1,
    hash: impl Fn(&SemanticSolutionKeyV1) -> H,
) -> bool {
    hash(left) == hash(right) && left.bytes == right.bytes
}

fn validate_schema(
    schema: &[&str],
    mapping: &super::mapping::SemanticMapping,
    caps: SemanticKeyCaps,
) -> Result<(), SemanticKeyError> {
    if schema.len() > caps.max_variables {
        return Err(SemanticKeyError::TooManyVariables);
    }
    if schema.len() != mapping.len() {
        return Err(SemanticKeyError::SchemaMismatch);
    }
    for (index, variable) in schema.iter().enumerate() {
        if variable.is_empty() || variable.as_bytes().contains(&0) {
            return Err(SemanticKeyError::MalformedVariable);
        }
        if schema[..index].contains(variable) {
            return Err(SemanticKeyError::DuplicateVariable);
        }
        if !mapping.contains_variable(variable) {
            return Err(SemanticKeyError::SchemaMismatch);
        }
    }
    if mapping
        .variables()
        .any(|variable| !schema.contains(&variable))
    {
        return Err(SemanticKeyError::SchemaMismatch);
    }
    Ok(())
}

pub(super) fn validate_term_depth_hard(term: &Term) -> Result<(), SemanticKeyError> {
    let mut depth = 0usize;
    let mut cursor = term;
    while let Term::Triple(triple) = cursor {
        depth = checked_len_add(depth, 1)?;
        if depth > HARD_MAX_TRIPLE_DEPTH {
            return Err(SemanticKeyError::TripleDepthExceeded);
        }
        cursor = &triple.object;
    }
    Ok(())
}

pub(super) fn checked_len_add(left: usize, right: usize) -> Result<usize, SemanticKeyError> {
    left.checked_add(right)
        .ok_or(SemanticKeyError::LengthOverflow)
}

fn encode_mapping(
    encoder: &mut Encoder<'_>,
    schema: &[&str],
    mapping: &super::mapping::SemanticMapping,
    max_depth: usize,
) -> Result<(), SemanticKeyError> {
    encoder.bytes(MAGIC)?;
    encoder.byte(VERSION)?;
    encoder.u32(schema.len())?;
    for variable in schema {
        encoder.length_prefixed(variable.as_bytes())?;
    }

    let mask_len = schema.len().div_ceil(8);
    encoder.u32(mask_len)?;
    for chunk in 0..mask_len {
        let mut mask = 0u8;
        for bit in 0..8 {
            let index = chunk * 8 + bit;
            if index < schema.len() && mapping.value(schema[index]).is_some() {
                mask |= 1 << bit;
            }
        }
        encoder.byte(mask)?;
    }
    for variable in schema {
        if let Some(term) = mapping.value(variable) {
            encode_term(encoder, term.term(), term.blank_scope(), 0, max_depth)?;
        }
    }
    Ok(())
}

fn encode_term(
    encoder: &mut Encoder<'_>,
    term: &Term,
    scope: &BlankNodeScope,
    depth: usize,
    max_depth: usize,
) -> Result<(), SemanticKeyError> {
    match term {
        Term::NamedNode(node) => encode_named_node(encoder, node.as_str()),
        Term::BlankNode(node) => encode_blank_node(encoder, node.as_str(), scope),
        Term::Literal(literal) => {
            encoder.byte(3)?;
            encoder.length_prefixed(literal.value().as_bytes())?;
            if let Some(language) = literal.language() {
                encoder.byte(1)?;
                encoder.lowercase_ascii_length_prefixed(language)?;
            } else {
                encoder.byte(0)?;
                encoder.length_prefixed(literal.datatype().as_str().as_bytes())?;
            }
            Ok(())
        }
        Term::Triple(triple) => {
            if depth >= max_depth {
                return Err(SemanticKeyError::TripleDepthExceeded);
            }
            encoder.byte(4)?;
            encode_subject(encoder, &triple.subject, scope)?;
            encode_named_node(encoder, triple.predicate.as_str())?;
            encode_term(encoder, &triple.object, scope, depth + 1, max_depth)
        }
    }
}

fn encode_subject(
    encoder: &mut Encoder<'_>,
    subject: &NamedOrBlankNode,
    scope: &BlankNodeScope,
) -> Result<(), SemanticKeyError> {
    match subject {
        NamedOrBlankNode::NamedNode(node) => encode_named_node(encoder, node.as_str()),
        NamedOrBlankNode::BlankNode(node) => encode_blank_node(encoder, node.as_str(), scope),
    }
}

fn encode_named_node(encoder: &mut Encoder<'_>, iri: &str) -> Result<(), SemanticKeyError> {
    encoder.byte(1)?;
    encoder.length_prefixed(iri.as_bytes())
}

fn encode_blank_node(
    encoder: &mut Encoder<'_>,
    identifier: &str,
    scope: &BlankNodeScope,
) -> Result<(), SemanticKeyError> {
    encoder.byte(2)?;
    match &scope.owner {
        BlankNodeOwner::Request(identity) => {
            if identity.is_empty() || identifier.is_empty() {
                return Err(SemanticKeyError::MalformedBlankScope);
            }
            encoder.byte(0)?;
            encoder.length_prefixed(identity.as_bytes())?;
        }
        BlankNodeOwner::Dataset(identity) => {
            if identity.is_empty() || identifier.is_empty() {
                return Err(SemanticKeyError::MalformedBlankScope);
            }
            encoder.byte(1)?;
            encoder.length_prefixed(identity.as_bytes())?;
        }
    }
    match &scope.graph {
        GraphName::DefaultGraph => encoder.byte(0)?,
        GraphName::NamedNode(graph) => {
            encoder.byte(1)?;
            encoder.length_prefixed(graph.as_str().as_bytes())?;
        }
        GraphName::BlankNode(graph) => {
            encoder.byte(2)?;
            encoder.length_prefixed(graph.as_str().as_bytes())?;
        }
    }
    encoder.length_prefixed(identifier.as_bytes())
}

struct Encoder<'a> {
    output: Option<&'a mut Vec<u8>>,
    len: usize,
    max: usize,
}

impl<'a> Encoder<'a> {
    const fn measure(max: usize) -> Self {
        Self {
            output: None,
            len: 0,
            max,
        }
    }

    fn writer(output: &'a mut Vec<u8>, max: usize) -> Self {
        Self {
            output: Some(output),
            len: 0,
            max,
        }
    }

    const fn len(&self) -> usize {
        self.len
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), SemanticKeyError> {
        let next = checked_len_add(self.len, bytes.len())?;
        if next > self.max {
            return Err(SemanticKeyError::EncodedBytesExceeded);
        }
        if let Some(output) = self.output.as_deref_mut() {
            output.extend_from_slice(bytes);
        }
        self.len = next;
        Ok(())
    }

    fn byte(&mut self, byte: u8) -> Result<(), SemanticKeyError> {
        self.bytes(&[byte])
    }

    fn u32(&mut self, value: usize) -> Result<(), SemanticKeyError> {
        let value = u32::try_from(value).map_err(|_| SemanticKeyError::ComponentTooLarge)?;
        self.bytes(&value.to_be_bytes())
    }

    fn length_prefixed(&mut self, bytes: &[u8]) -> Result<(), SemanticKeyError> {
        self.u32(bytes.len())?;
        self.bytes(bytes)
    }

    fn lowercase_ascii_length_prefixed(&mut self, value: &str) -> Result<(), SemanticKeyError> {
        self.u32(value.len())?;
        for byte in value.bytes() {
            self.byte(byte.to_ascii_lowercase())?;
        }
        Ok(())
    }
}
