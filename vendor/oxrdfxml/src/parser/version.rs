use super::events::InternalRdfXmlParser;
use super::state::RdfXmlState;
use crate::error::RdfXmlSyntaxError;

#[derive(Copy, Clone)]
pub(super) enum RdfVersion {
    V11,
    V12,
    V12Basic,
}

impl RdfVersion {
    pub(super) fn supports_its_dir(self) -> bool {
        matches!(self, Self::V12 | Self::V12Basic)
    }

    pub(super) fn supports_triple_term(self) -> bool {
        matches!(self, Self::V12)
    }

    pub(super) fn from_str(value: &str) -> Result<RdfVersion, RdfXmlSyntaxError> {
        match value {
            "1.1" => Ok(RdfVersion::V11),
            "1.2" => Ok(RdfVersion::V12),
            "1.2-basic" => Ok(RdfVersion::V12Basic),
            _ => Err(RdfXmlSyntaxError::msg(format!(
                "The rdf:version value '{value}' is not supported, allowed values are '1.1' and '1.2'"
            ))),
        }
    }
}

impl<R> InternalRdfXmlParser<R> {
    pub(super) fn current_rdf_version(&self) -> RdfVersion {
        for state in self.state.iter().rev() {
            match state {
                RdfXmlState::Doc { .. } => (),
                RdfXmlState::Rdf { rdf_version, .. }
                | RdfXmlState::NodeElt { rdf_version, .. }
                | RdfXmlState::PropertyElt { rdf_version, .. }
                | RdfXmlState::ParseTypeCollectionPropertyElt { rdf_version, .. }
                | RdfXmlState::ParseTypeLiteralPropertyElt { rdf_version, .. } => {
                    if let Some(rdf_version) = rdf_version {
                        return *rdf_version;
                    }
                }
                #[cfg(feature = "rdf-12")]
                RdfXmlState::ParseTypeTriplePropertyElt { rdf_version, .. } => {
                    if let Some(rdf_version) = rdf_version {
                        return *rdf_version;
                    }
                }
            }
        }
        RdfVersion::V11
    }
}
