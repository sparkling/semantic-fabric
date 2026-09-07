//! Explicit trusted source settings; no application tenant vocabulary is inferred.
use crate::{problem::StartupCause, ServeError};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Transaction-local PostgreSQL custom settings bound to one service principal.
/// Database policies and this trusted configuration are operator-owned. Never
/// construct these from unverified request headers. Values are not logged.
#[derive(Clone, PartialEq, Eq)]
pub struct PostgresRlsClaims(pub(crate) BTreeMap<String, String>);

impl std::fmt::Debug for PostgresRlsClaims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PostgresRlsClaims([REDACTED])")
    }
}

impl PostgresRlsClaims {
    pub fn new(settings: BTreeMap<String, String>) -> Result<Self, ServeError> {
        if settings.is_empty()
            || settings.len() > 16
            || settings.iter().any(|(k, v)| {
                let parts: Vec<_> = k.split('.').collect();
                k.len() > 128
                    || k.bytes().any(|c| c.is_ascii_uppercase())
                    || parts.len() != 2
                    || parts[0].starts_with("pg_")
                    || parts.iter().any(|p| !identifier(p))
                    || v.is_empty()
                    || v.len() > 1024
                    || v.contains('\0')
            })
        {
            return Err(configuration_error());
        }
        Ok(Self(settings))
    }

    /// Read a bounded JSON object of custom setting names to string values.
    pub fn from_env(name: &str) -> Result<Self, ServeError> {
        if name.len() > 128 || !identifier(name) {
            return Err(configuration_error());
        }
        let value = std::env::var(name).map_err(|_| configuration_error())?;
        if value.len() > 32768 {
            return Err(configuration_error());
        }
        Self::new(serde_json::from_str(&value).map_err(|_| configuration_error())?)
    }

    pub(crate) fn identity(&self, credential: &[u8; 32], domain: &[u8]) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(domain);
        hash.update(credential);
        for (key, value) in &self.0 {
            for part in [key, value] {
                hash.update((part.len() as u64).to_be_bytes());
                hash.update(part.as_bytes());
            }
        }
        hash.finalize().into()
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

fn configuration_error() -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: "PostgreSQL row security context is missing or invalid".into(),
    })
}
