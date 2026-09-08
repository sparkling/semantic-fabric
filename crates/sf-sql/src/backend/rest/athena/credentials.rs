//! Static AWS credentials for the Athena backend.
//!
//! [`AthenaCredentials`] deliberately does **not** derive `Debug`: the secret
//! access key and session token must never reach a log line, so the manual impl
//! prints placeholders only and [`AthenaCredentials::redact`] scrubs any
//! diagnostic string built from a remote (attacker-influenced) response body.

use std::fmt;

use crate::error::Result;

use super::config::cfg_err;

/// Replacement text substituted for any credential material in diagnostics.
const REDACTED: &str = "***REDACTED***";

/// Static AWS credentials used to sign Athena requests.
///
/// No `Debug` derive by design — see the module docs.
#[derive(Clone)]
pub struct AthenaCredentials {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: Option<String>,
}

impl fmt::Debug for AthenaCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AthenaCredentials")
            .field("access_key_id", &REDACTED)
            .field("secret_access_key", &REDACTED)
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| REDACTED),
            )
            .finish()
    }
}

impl AthenaCredentials {
    /// Long-lived key pair. Both halves must be non-empty.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Result<Self> {
        let access_key_id = access_key_id.into();
        let secret_access_key = secret_access_key.into();
        if access_key_id.trim().is_empty() {
            return Err(cfg_err("access key id must not be empty"));
        }
        if secret_access_key.is_empty() {
            return Err(cfg_err("secret access key must not be empty"));
        }
        Ok(Self {
            access_key_id,
            secret_access_key,
            session_token: None,
        })
    }

    /// Attach an STS session token (temporary credentials).
    pub fn with_session_token(mut self, token: impl Into<String>) -> Result<Self> {
        let token = token.into();
        if token.is_empty() {
            return Err(cfg_err("session token must not be empty"));
        }
        self.session_token = Some(token);
        Ok(self)
    }

    /// Read the dedicated semantic-fabric Athena environment variables:
    /// `SF_ATHENA_ACCESS_KEY_ID`, `SF_ATHENA_SECRET_ACCESS_KEY`, and the
    /// optional `SF_ATHENA_SESSION_TOKEN`.
    pub fn from_env() -> Result<Self> {
        Self::from_env_with(|name| std::env::var(name).ok())
    }

    fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let access_key_id = get("SF_ATHENA_ACCESS_KEY_ID")
            .ok_or_else(|| cfg_err("SF_ATHENA_ACCESS_KEY_ID not set"))?;
        let secret_access_key = get("SF_ATHENA_SECRET_ACCESS_KEY")
            .ok_or_else(|| cfg_err("SF_ATHENA_SECRET_ACCESS_KEY not set"))?;
        let creds = Self::new(access_key_id, secret_access_key)?;
        match get("SF_ATHENA_SESSION_TOKEN") {
            Some(t) if !t.is_empty() => creds.with_session_token(t),
            _ => Ok(creds),
        }
    }

    /// Scrub every credential component out of `text`.
    ///
    /// Applied to EVERY diagnostic this backend constructs, because an AWS
    /// (or hostile look-alike) error body is free to echo request material
    /// back at us, and those diagnostics end up in `Error` strings and logs.
    pub(crate) fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        let secrets = [
            Some(&self.access_key_id),
            Some(&self.secret_access_key),
            self.session_token.as_ref(),
        ];
        for secret in secrets.into_iter().flatten() {
            if !secret.is_empty() && out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), REDACTED);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::AthenaCredentials;

    #[test]
    fn credentials_reject_empty_halves() {
        assert!(AthenaCredentials::new("", "secret").is_err());
        assert!(AthenaCredentials::new("AKID", "").is_err());
        let creds = AthenaCredentials::new("AKID", "secret").unwrap();
        assert!(creds.clone().with_session_token("").is_err());
        assert!(creds.with_session_token("tok").is_ok());
    }

    #[test]
    fn environment_credentials_are_scoped_to_sf_athena_variables() {
        let creds = AthenaCredentials::from_env_with(|name| match name {
            "SF_ATHENA_ACCESS_KEY_ID" => Some("AKID".to_owned()),
            "SF_ATHENA_SECRET_ACCESS_KEY" => Some("secret".to_owned()),
            "SF_ATHENA_SESSION_TOKEN" => Some("token".to_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(creds.access_key_id, "AKID");
        assert_eq!(creds.secret_access_key, "secret");
        assert_eq!(creds.session_token.as_deref(), Some("token"));

        let aws_only = AthenaCredentials::from_env_with(|name| match name {
            "AWS_ACCESS_KEY_ID" => Some("AKID".to_owned()),
            "AWS_SECRET_ACCESS_KEY" => Some("secret".to_owned()),
            _ => None,
        });
        assert!(aws_only.is_err());
    }

    #[test]
    fn debug_and_redact_never_leak_credential_material() {
        let creds = AthenaCredentials::new("AKIDEXAMPLE", "supersecretkey")
            .unwrap()
            .with_session_token("sessiontoken123")
            .unwrap();
        let debug = format!("{creds:?}");
        assert!(!debug.contains("supersecretkey"), "{debug}");
        assert!(!debug.contains("sessiontoken123"), "{debug}");
        assert!(!debug.contains("AKIDEXAMPLE"), "{debug}");

        let hostile = "denied for AKIDEXAMPLE using supersecretkey / sessiontoken123";
        let clean = creds.redact(hostile);
        assert!(!clean.contains("supersecretkey"), "{clean}");
        assert!(!clean.contains("sessiontoken123"), "{clean}");
        assert!(!clean.contains("AKIDEXAMPLE"), "{clean}");
    }
}
