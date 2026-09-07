//! Bounded startup-only configuration. Documents contain environment references,
//! not inline credentials or request-provided attributes.
use super::*;
use serde::Deserialize;

const MAX_JSON_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryDocument {
    schema_version: u32,
    subjects: Vec<SubjectDocument>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubjectDocument {
    subject_ref: String,
    credential_env: String,
    postgres_rls_context_env: String,
}

impl ProvisionedBearerAdmission {
    /// Load schema-version-1 JSON from an environment reference. Every subject
    /// has `subjectRef`, `credentialEnv`, and `postgresRlsContextEnv`. All values
    /// are resolved once, before source I/O; no environment is read per request.
    /// Unknown/duplicate fields, ambiguous identities, and malformed settings deny.
    pub fn from_env(name: &str) -> Result<Self, ServeError> {
        if !valid_env_name(name) {
            return Err(invalid_registry());
        }
        let json = std::env::var(name).map_err(|_| invalid_registry())?;
        Self::from_json_with(&json, |name| std::env::var(name).map_err(|_| ()))
    }

    fn from_json_with(
        json: &str,
        mut resolve: impl FnMut(&str) -> Result<String, ()>,
    ) -> Result<Self, ServeError> {
        if json.len() > MAX_JSON_BYTES {
            return Err(invalid_registry());
        }
        let document: RegistryDocument =
            serde_json::from_str(json).map_err(|_| invalid_registry())?;
        if document.schema_version != 1
            || document.subjects.is_empty()
            || document.subjects.len() > 256
            || document.subjects.iter().any(|s| {
                !valid_env_name(&s.credential_env) || !valid_env_name(&s.postgres_rls_context_env)
            })
        {
            return Err(invalid_registry());
        }
        let subjects = document
            .subjects
            .into_iter()
            .map(|s| {
                let credential = resolve(&s.credential_env).map_err(|_| invalid_registry())?;
                let claims =
                    resolve(&s.postgres_rls_context_env).map_err(|_| invalid_registry())?;
                ProvisionedBearerSubject::postgres_rls(
                    &s.subject_ref,
                    &credential,
                    crate::PostgresRlsClaims::from_json(&claims).map_err(|_| invalid_registry())?,
                )
                .map_err(|_| invalid_registry())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(subjects)
    }
}

fn valid_env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOCUMENT: &str = r#"{"schemaVersion":1,"subjects":[{"subjectRef":"opaque-1","credentialEnv":"SF_TEST_CREDENTIAL","postgresRlsContextEnv":"SF_TEST_CLAIMS"}]}"#;
    const CREDENTIAL: &str = "test-only-env-credential-0123456789";
    fn resolve(name: &str) -> Result<String, ()> {
        match name {
            "SF_TEST_CREDENTIAL" => Ok(CREDENTIAL.into()),
            "SF_TEST_CLAIMS" => Ok(r#"{"app.tenant_id":"a"}"#.into()),
            _ => Err(()),
        }
    }
    #[test]
    fn references_resolve_once_and_create_real_admission() {
        let mut reads = 0;
        let profile = ProvisionedBearerAdmission::from_json_with(DOCUMENT, |name| {
            reads += 1;
            resolve(name)
        })
        .unwrap();
        assert_eq!(reads, 2);
        let digest = Sha256::digest(CREDENTIAL.as_bytes()).into();
        assert!(profile.match_credential(&digest).is_some());
        assert!(!format!("{profile:?}").contains("opaque-1"));
    }
    #[test]
    fn bad_schema_fields_references_and_bounds_reject_before_resolution() {
        for document in [
            DOCUMENT.replace("\"schemaVersion\":1", "\"schemaVersion\":2"),
            DOCUMENT.replace(
                "\"schemaVersion\":1",
                "\"schemaVersion\":1,\"schemaVersion\":1",
            ),
            DOCUMENT.replace(
                "\"subjectRef\":",
                "\"credential\":\"literal\",\"subjectRef\":",
            ),
            DOCUMENT.replace("SF_TEST_CREDENTIAL", "9BAD"),
            DOCUMENT.replace("SF_TEST_CLAIMS", "bad-name"),
            " ".repeat(MAX_JSON_BYTES + 1),
        ] {
            let error = ProvisionedBearerAdmission::from_json_with(&document, |_| {
                panic!("must not resolve")
            })
            .unwrap_err();
            assert!(!format!("{error:?}").contains("literal"));
        }
        assert!(ProvisionedBearerAdmission::from_json_with(DOCUMENT, |_| Err(())).is_err());
    }
    #[test]
    fn duplicate_and_non_string_rls_settings_fail_closed() {
        for value in [
            r#"{"app.tenant_id":"a","app.tenant_id":"b"}"#,
            r#"{"app.tenant_id":42}"#,
        ] {
            let error = ProvisionedBearerAdmission::from_json_with(DOCUMENT, |name| {
                if name == "SF_TEST_CLAIMS" {
                    Ok(value.into())
                } else {
                    resolve(name)
                }
            })
            .unwrap_err();
            assert!(!format!("{error:?}").contains("tenant_id"));
        }
    }
}
