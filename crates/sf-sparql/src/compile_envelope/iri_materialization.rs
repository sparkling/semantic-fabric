//! Bounded direct-IRI materialization measurement for the pinned parser.
//!
//! This dormant V1 primitive examines spargebra's parser view of the query,
//! including its feature-gated global Unicode decoding, before the real parse.
//! It follows sequential `BASE`/`PREFIX` declarations and counts logical UTF-8
//! bytes for source-direct IRIREF and prefixed-name materializations.
//!
//! It is deliberately **not** a parser-memory or retained-payload bound.
//! Spargebra may deep-clone terms while expanding property lists, collections,
//! reifiers, and `CONSTRUCT WHERE`; allocator capacity and container overhead
//! are also excluded. Contextual PEG choices for `<...>` remain a differential-
//! calibration requirement. Do not use this primitive for admission until
//! those gaps and externally configured parser state have an accepted design.

use std::collections::HashMap;

use oxiri::Iri;

use super::{
    enforce, CompileEnvelopeError, CompileEnvelopeLimit, MAX_DIRECT_IRI_MATERIALIZATION_BYTES_V1,
    MAX_PREFIX_BINDINGS_V1, MAX_PREFIX_DECLARATIONS_V1, MAX_SCANNED_BYTES_V1,
};

mod parser_view;
mod scanner;

use parser_view::parser_view;
use scanner::{BodyToken, Scanner};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Limits {
    prefix_declarations: usize,
    prefix_bindings: usize,
    direct_iri_materialization_bytes: usize,
}

impl Limits {
    const V1: Self = Self {
        prefix_declarations: MAX_PREFIX_DECLARATIONS_V1,
        prefix_bindings: MAX_PREFIX_BINDINGS_V1,
        direct_iri_materialization_bytes: MAX_DIRECT_IRI_MATERIALIZATION_BYTES_V1,
    };
}

/// Logical direct-IRI measurements; none of these fields is total heap usage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DirectIriMaterializationEnvelopeV1 {
    pub(crate) prefix_declarations: usize,
    pub(crate) unique_prefix_bindings: usize,
    /// Final resolved prefix IRI values. Map keys and allocation overhead are
    /// excluded and remain bounded indirectly by the input and binding limits.
    pub(crate) prefix_state_bytes: usize,
    /// Source-direct IRI payload represented by the returned query, including
    /// the final `BASE` clone, but excluding parser-generated term clones.
    pub(crate) direct_ast_iri_bytes: usize,
    /// High-water logical IRI bytes held by prefix/base state, direct AST terms,
    /// and the materialization currently being constructed.
    pub(crate) peak_direct_parser_iri_bytes: usize,
    pub(crate) max_resolved_iri_bytes: usize,
}

impl DirectIriMaterializationEnvelopeV1 {
    pub(crate) fn measure(input: &str) -> Result<Self, CompileEnvelopeError> {
        Self::measure_with_limits(input, Limits::V1)
    }

    fn measure_with_limits(input: &str, limits: Limits) -> Result<Self, CompileEnvelopeError> {
        // Raw input is bounded before the optional parser-view allocation.
        enforce(
            CompileEnvelopeLimit::InputBytes,
            input.len(),
            MAX_SCANNED_BYTES_V1,
        )?;
        let view = parser_view(input);
        debug_assert!(view.len() <= input.len());
        Analyzer::new(&view, limits).run()
    }
}

struct Analyzer<'input> {
    scanner: Scanner<'input>,
    limits: Limits,
    base: Option<Iri<String>>,
    prefixes: HashMap<String, String>,
    envelope: DirectIriMaterializationEnvelopeV1,
}

struct ResolvedIri {
    value: Iri<String>,
    /// Logical IRI strings simultaneously alive while spargebra resolves this
    /// token. With a base, both the unescaped reference and resolved output
    /// exist until `Iri::resolve` returns.
    temporary_bytes: usize,
}

impl<'input> Analyzer<'input> {
    fn new(input: &'input str, limits: Limits) -> Self {
        Self {
            scanner: Scanner::new(input),
            limits,
            base: None,
            prefixes: HashMap::new(),
            envelope: DirectIriMaterializationEnvelopeV1::default(),
        }
    }

    fn run(mut self) -> Result<DirectIriMaterializationEnvelopeV1, CompileEnvelopeError> {
        while let Some(declaration) = self.scanner.next_prologue() {
            match declaration {
                scanner::Prologue::Base { iri } => self.base_declaration(iri)?,
                scanner::Prologue::Prefix { name, iri } => {
                    self.prefix_declaration(name, iri)?;
                }
                scanner::Prologue::Version => {}
            }
        }

        while let Some(token) = self.scanner.next_body() {
            match token {
                BodyToken::IriRef(iri) => {
                    if let Some(resolved) = self.resolve_iriref(iri)? {
                        self.record_direct_ast_iri(
                            resolved.value.as_str().len(),
                            resolved.temporary_bytes,
                        )?;
                    }
                }
                BodyToken::PrefixedName { prefix, local } => {
                    if let Some(prefix_iri) = self.prefixes.get(prefix) {
                        let raw_capacity = checked_sum(prefix_iri.len(), local.len());
                        enforce(
                            CompileEnvelopeLimit::DirectIriMaterializationBytes,
                            raw_capacity,
                            self.limits.direct_iri_materialization_bytes,
                        )?;
                        let resolved = checked_sum(prefix_iri.len(), unescaped_local_len(local));
                        self.record_direct_ast_iri(resolved, resolved)?;
                    }
                }
            }
        }

        // spargebra clones the final base into the returned root Query while
        // ParserState still owns its copy.
        if let Some(base_bytes) = self.base.as_ref().map(|base| base.as_str().len()) {
            self.record_direct_ast_iri(base_bytes, base_bytes)?;
        }
        Ok(self.envelope)
    }

    fn base_declaration(&mut self, iri: &str) -> Result<(), CompileEnvelopeError> {
        let Some(resolved) = self.resolve_iriref(iri)? else {
            return Ok(());
        };
        let resolved_bytes = resolved.value.as_str().len();
        self.note_temporary_iri(resolved.temporary_bytes, resolved_bytes)?;
        self.base = Some(resolved.value);
        self.note_live_iri_bytes()
    }

    fn prefix_declaration(&mut self, name: &str, iri: &str) -> Result<(), CompileEnvelopeError> {
        let declarations = checked_sum(self.envelope.prefix_declarations, 1);
        enforce(
            CompileEnvelopeLimit::PrefixDeclarations,
            declarations,
            self.limits.prefix_declarations,
        )?;
        self.envelope.prefix_declarations = declarations;

        let is_new_binding = !self.prefixes.contains_key(name);
        if is_new_binding {
            let bindings = checked_sum(self.prefixes.len(), 1);
            enforce(
                CompileEnvelopeLimit::PrefixBindings,
                bindings,
                self.limits.prefix_bindings,
            )?;
        }
        let Some(resolved) = self.resolve_iriref(iri)? else {
            return Ok(());
        };
        let resolved_bytes = resolved.value.as_str().len();
        self.note_temporary_iri(resolved.temporary_bytes, resolved_bytes)?;

        let previous_bytes = self.prefixes.get(name).map_or(0, String::len);
        let state_without_previous = self
            .envelope
            .prefix_state_bytes
            .checked_sub(previous_bytes)
            .unwrap_or(usize::MAX);
        let next_state = checked_sum(state_without_previous, resolved_bytes);
        enforce(
            CompileEnvelopeLimit::DirectIriMaterializationBytes,
            next_state,
            self.limits.direct_iri_materialization_bytes,
        )?;

        self.prefixes
            .insert(name.to_owned(), resolved.value.into_inner());
        self.envelope.unique_prefix_bindings = self.prefixes.len();
        self.envelope.prefix_state_bytes = next_state;
        self.note_live_iri_bytes()
    }

    fn resolve_iriref(&mut self, iri: &str) -> Result<Option<ResolvedIri>, CompileEnvelopeError> {
        let Some(unescaped) = scanner::unescape_iriref(iri) else {
            return Ok(None);
        };
        let unescaped_bytes = unescaped.len();
        let (resolved, temporary_bytes) = if let Some(base) = &self.base {
            // oxiri reserves base + reference bytes before resolution. Bound
            // that prospective allocation even though this envelope reports
            // logical string lengths rather than allocator capacity.
            let prospective_capacity = checked_sum(base.as_str().len(), unescaped.len());
            enforce(
                CompileEnvelopeLimit::DirectIriMaterializationBytes,
                prospective_capacity,
                self.limits.direct_iri_materialization_bytes,
            )?;
            let Some(resolved) = base.resolve(&unescaped).ok() else {
                return Ok(None);
            };
            let temporary = checked_sum(unescaped_bytes, resolved.as_str().len());
            (resolved, temporary)
        } else {
            let Some(resolved) = Iri::parse(unescaped).ok() else {
                return Ok(None);
            };
            (resolved, unescaped_bytes)
        };
        Ok(Some(ResolvedIri {
            value: resolved,
            temporary_bytes,
        }))
    }

    fn record_direct_ast_iri(
        &mut self,
        bytes: usize,
        temporary_bytes: usize,
    ) -> Result<(), CompileEnvelopeError> {
        self.note_temporary_iri(temporary_bytes, bytes)?;
        let direct = checked_sum(self.envelope.direct_ast_iri_bytes, bytes);
        enforce(
            CompileEnvelopeLimit::DirectIriMaterializationBytes,
            direct,
            self.limits.direct_iri_materialization_bytes,
        )?;
        self.envelope.direct_ast_iri_bytes = direct;
        self.note_live_iri_bytes()
    }

    fn note_temporary_iri(
        &mut self,
        temporary_bytes: usize,
        resolved_bytes: usize,
    ) -> Result<(), CompileEnvelopeError> {
        let observed = checked_sum(self.live_iri_bytes(), temporary_bytes);
        enforce(
            CompileEnvelopeLimit::DirectIriMaterializationBytes,
            observed,
            self.limits.direct_iri_materialization_bytes,
        )?;
        self.envelope.max_resolved_iri_bytes =
            self.envelope.max_resolved_iri_bytes.max(resolved_bytes);
        self.envelope.peak_direct_parser_iri_bytes =
            self.envelope.peak_direct_parser_iri_bytes.max(observed);
        Ok(())
    }

    fn note_live_iri_bytes(&mut self) -> Result<(), CompileEnvelopeError> {
        let observed = self.live_iri_bytes();
        enforce(
            CompileEnvelopeLimit::DirectIriMaterializationBytes,
            observed,
            self.limits.direct_iri_materialization_bytes,
        )?;
        self.envelope.peak_direct_parser_iri_bytes =
            self.envelope.peak_direct_parser_iri_bytes.max(observed);
        Ok(())
    }

    fn live_iri_bytes(&self) -> usize {
        checked_sum(
            checked_sum(
                self.envelope.prefix_state_bytes,
                self.base.as_ref().map_or(0, |base| base.as_str().len()),
            ),
            self.envelope.direct_ast_iri_bytes,
        )
    }
}

fn checked_sum(left: usize, right: usize) -> usize {
    // Saturation is the overflow sentinel consumed by the immediately
    // following finite-limit check, so overflow fails closed before mutation.
    left.saturating_add(right)
}

fn unescaped_local_len(local: &str) -> usize {
    local.len()
        - local
            .as_bytes()
            .iter()
            .filter(|byte| **byte == b'\\')
            .count()
}

#[cfg(test)]
mod tests;
