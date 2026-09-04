use std::collections::TryReserveError;
use std::fmt;

/// Independent V1 input ceiling. Changing any profile constant requires a new
/// compile-profile identity when this scanner is integrated with the cache.
pub(crate) const MAX_SCANNED_BYTES_V1: usize = 256 * 1024;
pub(crate) const MAX_TOKENS_V1: usize = 32 * 1024;
pub(crate) const MAX_LEXEME_BYTES_V1: usize = 16 * 1024;
pub(crate) const MAX_NESTING_DEPTH_V1: usize = 64;
pub(crate) const MAX_RDF_STAR_DEPTH_V1: usize = 32;
pub(crate) const MAX_OPERATORS_PER_SCOPE_V1: usize = 128;
/// Provisional active-path ceiling pending corpus and isolated crash-probe
/// calibration. It combines open delimiters with operators not released by a
/// separator or matching close.
pub(crate) const MAX_RECURSION_POTENTIAL_V1: usize = 256;
/// Provisional prologue ceilings pending corpus calibration. Prefix state is
/// separately bounded because a short prefixed name can expand to a long IRI.
pub(crate) const MAX_PREFIX_DECLARATIONS_V1: usize = 1_024;
pub(crate) const MAX_PREFIX_BINDINGS_V1: usize = 512;
pub(crate) const MAX_DIRECT_IRI_MATERIALIZATION_BYTES_V1: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompileEnvelopeLimit {
    InputBytes,
    Tokens,
    LexemeBytes,
    NestingDepth,
    RdfStarDepth,
    OperatorsPerScope,
    RecursionPotential,
    PrefixDeclarations,
    PrefixBindings,
    DirectIriMaterializationBytes,
    AlgebraNodes,
    AlgebraDepth,
    CollectionSlots,
    RetainedPayloadBytes,
}

impl fmt::Display for CompileEnvelopeLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InputBytes => "input-bytes",
            Self::Tokens => "tokens",
            Self::LexemeBytes => "lexeme-bytes",
            Self::NestingDepth => "nesting-depth",
            Self::RdfStarDepth => "rdf-star-depth",
            Self::OperatorsPerScope => "operators-per-scope",
            Self::RecursionPotential => "recursion-potential",
            Self::PrefixDeclarations => "prefix-declarations",
            Self::PrefixBindings => "prefix-bindings",
            Self::DirectIriMaterializationBytes => "direct-iri-materialization-bytes",
            Self::AlgebraNodes => "algebra-nodes",
            Self::AlgebraDepth => "algebra-depth",
            Self::CollectionSlots => "collection-slots",
            Self::RetainedPayloadBytes => "retained-payload-bytes",
        })
    }
}

/// Internal admission failure. It intentionally carries bounded numeric data,
/// never submitted query text or an upstream parser diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CompileEnvelopeError {
    #[error("compile envelope V1 {dimension} limit exceeded ({observed}>{maximum})")]
    LimitExceeded {
        dimension: CompileEnvelopeLimit,
        observed: usize,
        maximum: usize,
    },
    #[error("compile envelope V1 arithmetic overflow")]
    AccountingOverflow,
    #[error("compile envelope V1 work-stack allocation failed")]
    AllocationFailed,
}

impl From<TryReserveError> for CompileEnvelopeError {
    fn from(_: TryReserveError) -> Self {
        Self::AllocationFailed
    }
}

/// Measurements retained by the V1 scan for later work accounting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CompileEnvelopeV1 {
    pub(crate) input_bytes: usize,
    pub(crate) tokens: usize,
    pub(crate) max_lexeme_bytes: usize,
    pub(crate) max_nesting_depth: usize,
    pub(crate) max_rdf_star_depth: usize,
    /// Diagnostic upper estimate within syntax recognized by the scanner;
    /// contextual PEG ambiguities can still choose a different parser view.
    pub(crate) max_operators_per_scope: usize,
    /// Conservative active-path proxy for recursive parser work. This is a
    /// scanner measurement, not a claim about the upstream parser's stack.
    pub(crate) max_recursion_potential: usize,
}
