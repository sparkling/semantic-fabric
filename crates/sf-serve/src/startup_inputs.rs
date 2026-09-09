//! Bounded, exact semantic bytes for initial startup and off-path reload.

use std::io::Read;

use crate::problem::StartupCause;
use crate::{MappingRef, ServeError, ServeOptions};

#[derive(Eq, PartialEq)]
pub(crate) struct SemanticInputs {
    pub(crate) ontology: String,
    pub(crate) primary: Option<String>,
    pub(crate) additional: Option<String>,
}

impl SemanticInputs {
    pub(crate) fn capture(opts: &ServeOptions) -> Result<Self, ServeError> {
        Ok(Self {
            ontology: read_regular(&opts.ontology_path).map_err(|error| {
                ServeError::new(StartupCause::OntologyRead {
                    path: opts.ontology_path.clone(),
                    error: error.to_string(),
                })
            })?,
            primary: mapping(&opts.mapping)?,
            additional: opts
                .additional_source
                .as_ref()
                .map(|source| mapping(&source.mapping))
                .transpose()?
                .flatten(),
        })
    }
}

fn mapping(mapping: &MappingRef) -> Result<Option<String>, ServeError> {
    match mapping {
        MappingRef::Direct { .. } => Ok(None),
        MappingRef::R2rmlFile(path) | MappingRef::R2rmlFileWithBase { path, .. } => {
            read_regular(path).map(Some).map_err(|error| {
                ServeError::new(StartupCause::MappingRead {
                    path: path.clone(),
                    error: error.to_string(),
                })
            })
        }
    }
}

fn read_regular(path: &str) -> std::io::Result<String> {
    let maximum = sf_validation::DEFAULT_GRAPH_LIMITS.max_utf8_bytes;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Opening a replaced FIFO must not park the sole candidate worker.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "semantic input must be a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "semantic input exceeds its byte limit",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "semantic input is not valid UTF-8",
        )
    })
}
