use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueryWireLimit {
    WireBytes,
    Records,
    Edges,
    ScalarBytes,
    TreeDepth,
    Algebra,
}

impl fmt::Display for QueryWireLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WireBytes => "wire-byte",
            Self::Records => "record-count",
            Self::Edges => "edge-count",
            Self::ScalarBytes => "scalar-byte",
            Self::TreeDepth => "tree-depth",
            Self::Algebra => "algebra-envelope",
        })
    }
}

/// Closed failures for the untrusted worker-to-parent query boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueryWireError {
    AllocationFailed,
    AccountingOverflow,
    InvalidHeader,
    InvalidLength,
    LimitExceeded(QueryWireLimit),
    UnknownRecordKind,
    InvalidRecord,
    InvalidIndex,
    InvalidScalar,
    TypeMismatch,
    NonCanonical,
}

impl fmt::Display for QueryWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => formatter.write_str("parser query wire allocation failed"),
            Self::AccountingOverflow => {
                formatter.write_str("parser query wire accounting overflowed")
            }
            Self::InvalidHeader => formatter.write_str("invalid parser query wire header"),
            Self::InvalidLength => formatter.write_str("invalid parser query wire length"),
            Self::LimitExceeded(limit) => {
                write!(formatter, "parser query wire {limit} limit exceeded")
            }
            Self::UnknownRecordKind => formatter.write_str("unknown parser query wire record kind"),
            Self::InvalidRecord => formatter.write_str("invalid parser query wire record"),
            Self::InvalidIndex => formatter.write_str("invalid parser query wire index"),
            Self::InvalidScalar => formatter.write_str("invalid parser query wire scalar"),
            Self::TypeMismatch => formatter.write_str("parser query wire type mismatch"),
            Self::NonCanonical => formatter.write_str("non-canonical parser query wire"),
        }
    }
}

impl std::error::Error for QueryWireError {}

impl From<std::collections::TryReserveError> for QueryWireError {
    fn from(_: std::collections::TryReserveError) -> Self {
        Self::AllocationFailed
    }
}

impl From<crate::compile_envelope::CompileEnvelopeError> for QueryWireError {
    fn from(error: crate::compile_envelope::CompileEnvelopeError) -> Self {
        use crate::compile_envelope::CompileEnvelopeError;
        match error {
            CompileEnvelopeError::LimitExceeded { .. } => {
                Self::LimitExceeded(QueryWireLimit::Algebra)
            }
            CompileEnvelopeError::AccountingOverflow => Self::AccountingOverflow,
            CompileEnvelopeError::AllocationFailed => Self::AllocationFailed,
        }
    }
}
