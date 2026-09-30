//! Sealed proof tying gate-validated digests to one exact compiler binding.

use sf_core::SourceId;
use sf_sparql::{CompilerBinding, MappingDigest, OntologyDigest, SemanticAdmissionDigest};

use super::generated_mapping_coverage::MappingCoverage;
use super::ValidatedMapping;
use crate::generated_profile_identity::{
    mint, GeneratedProfileIdentity, PinnedGraphAllowlist, ProfileDescriptor,
};

/// The receipt does not belong to the compiler binding presented with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("generated admission receipt does not match this compiler binding")]
pub(crate) struct ReceiptMismatch;

/// Copyable proof minted only from a gate-validated mapping. Fields are private
/// and no constructor accepts caller-supplied digests.
#[derive(Clone, Copy)]
pub(crate) struct GeneratedAdmissionReceipt {
    source_id: SourceId,
    mapping: MappingDigest,
    ontology: OntologyDigest,
    admission: SemanticAdmissionDigest,
    identity: GeneratedProfileIdentity,
}

impl ValidatedMapping {
    pub(crate) fn generated_receipt(&self) -> GeneratedAdmissionReceipt {
        let allowlist = PinnedGraphAllowlist::new(std::iter::empty::<&str>())
            .expect("an empty pinned graph allowlist is always valid");
        let identity = mint(
            self.mapping_digest,
            self.ontology_digest,
            self.admission_digest,
            &ProfileDescriptor::v1(),
            &allowlist,
        );
        GeneratedAdmissionReceipt {
            source_id: self.mapping.source_id(),
            mapping: self.mapping_digest,
            ontology: self.ontology_digest,
            admission: self.admission_digest,
            identity,
        }
    }
}

impl GeneratedAdmissionReceipt {
    /// Off-path policy preparation, not query admission or an issued response.
    /// The future consumer must still enforce graph coverage and all request
    /// admission before using this identity. The existing receipt is unchanged.
    pub(crate) fn for_single_default_dataset(
        &self,
        compiler: &CompilerBinding,
        allowlist: &PinnedGraphAllowlist,
    ) -> Result<Self, ReceiptMismatch> {
        self.coverage(compiler)?;
        Ok(Self {
            identity: mint(
                self.mapping,
                self.ontology,
                self.admission,
                &ProfileDescriptor::single_default_dataset(),
                allowlist,
            ),
            ..*self
        })
    }

    /// Borrow coverage over the compiler's own mapping, only when every sealed
    /// identity equals the compiler's stored scope. Fixed-width compares only.
    pub(crate) fn coverage<'a>(
        &self,
        compiler: &'a CompilerBinding,
    ) -> Result<MappingCoverage<'a>, ReceiptMismatch> {
        let digests = compiler.digests();
        if compiler.source_id() != self.source_id
            || digests.mapping() != self.mapping
            || digests.ontology() != self.ontology
            || digests.semantic_admission() != self.admission
        {
            return Err(ReceiptMismatch);
        }
        Ok(MappingCoverage::new(compiler.source_mapping()))
    }

    /// Comparison-only attestation; never authorization.
    pub(crate) const fn identity(&self) -> GeneratedProfileIdentity {
        self.identity
    }
}

impl std::fmt::Debug for GeneratedAdmissionReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GeneratedAdmissionReceipt")
            .field("source_id", &self.source_id)
            .finish_non_exhaustive()
    }
}
