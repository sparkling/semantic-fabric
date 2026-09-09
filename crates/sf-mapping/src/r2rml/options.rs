//! Independent execution and document bases (R2RML §4; Turtle syntax).
use sf_core::{Error, NamedNodeRef, Result};

/// Legacy default for each independent base; Turtle directives affect only syntax.
pub const DEFAULT_R2RML_BASE_IRI: &str = "http://example.com/base/";
/// Inclusive UTF-8 ceiling checked before parsing either base.
pub const MAX_R2RML_BASE_IRI_BYTES: usize = 8 * 1024;

/// Explicit processor/output and Turtle/document environments.
#[derive(Clone, Copy, Debug)]
pub struct R2rmlOptions<'a> {
    /// Prefix for generated relative IRIs. Turtle directives never change it.
    pub processor_base_iri: &'a str,
    /// Initial base for RDF syntax; in-document BASE/@base directives may change it.
    pub document_base_iri: &'a str,
}

impl Default for R2rmlOptions<'_> {
    fn default() -> Self {
        Self {
            processor_base_iri: DEFAULT_R2RML_BASE_IRI,
            document_base_iri: DEFAULT_R2RML_BASE_IRI,
        }
    }
}

impl R2rmlOptions<'_> {
    pub(super) fn validate(&self) -> Result<()> {
        validate_r2rml_base(self.processor_base_iri)?;
        validate_r2rml_base(self.document_base_iri)
    }
}

/// Validate before parsing or source I/O; never echo invalid configuration.
pub fn validate_r2rml_base(base: &str) -> Result<()> {
    if base.len() > MAX_R2RML_BASE_IRI_BYTES {
        return Err(Error::Mapping(
            "R2RML base IRI exceeds its UTF-8 byte limit".into(),
        ));
    }
    NamedNodeRef::new(base)
        .map(|_| ())
        .map_err(|_| Error::Mapping("R2RML base must be a valid absolute IRI".into()))
}
