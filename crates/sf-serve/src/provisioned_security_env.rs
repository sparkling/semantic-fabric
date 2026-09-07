//! Bounded startup-only configuration. Documents contain environment references,
//! not inline credentials, policy values, or request-provided attributes.
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
    #[serde(default)]
    postgres_rls_context_env: Option<String>,
    #[serde(default)]
    portable_rows: Vec<PortableRuleDocument>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableRuleDocument {
    source_index: usize,
    table: String,
    column: String,
    value_env: String,
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
        if !(document.schema_version == 1 || document.schema_version == 2)
            || document.subjects.is_empty()
            || document.subjects.len() > 256
            || document.subjects.iter().any(|s| {
                !valid_env_name(&s.credential_env)
                    || (document.schema_version == 1
                        && (s.postgres_rls_context_env.is_none() || !s.portable_rows.is_empty()))
                    || (document.schema_version == 2
                        && (s.postgres_rls_context_env.is_some() != s.portable_rows.is_empty()))
                    || s.postgres_rls_context_env
                        .as_deref()
                        .is_some_and(|name| !valid_env_name(name))
                    || s.portable_rows
                        .iter()
                        .any(|rule| !valid_env_name(&rule.value_env))
            })
        {
            return Err(invalid_registry());
        }
        let subjects = document
            .subjects
            .into_iter()
            .map(|s| {
                let credential = resolve(&s.credential_env).map_err(|_| invalid_registry())?;
                if let Some(reference) = s.postgres_rls_context_env {
                    let claims = resolve(&reference).map_err(|_| invalid_registry())?;
                    ProvisionedBearerSubject::postgres_rls(
                        &s.subject_ref,
                        &credential,
                        crate::PostgresRlsClaims::from_json(&claims)
                            .map_err(|_| invalid_registry())?,
                    )
                    .map_err(|_| invalid_registry())
                } else {
                    let rules = s
                        .portable_rows
                        .into_iter()
                        .map(|rule| {
                            let value = resolve(&rule.value_env).map_err(|_| invalid_registry())?;
                            crate::PortableRowRule::new(
                                rule.source_index,
                                rule.table,
                                rule.column,
                                value,
                            )
                            .map_err(|_| invalid_registry())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    ProvisionedBearerSubject::portable_rows(
                        &s.subject_ref,
                        &credential,
                        crate::PortableRowPolicy::new(rules).map_err(|_| invalid_registry())?,
                    )
                    .map_err(|_| invalid_registry())
                }
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
    const PORTABLE_DOCUMENT: &str = r#"{"schemaVersion":2,"subjects":[{"subjectRef":"opaque-1","credentialEnv":"SF_TEST_CREDENTIAL","portableRows":[{"sourceIndex":0,"table":"people","column":"tenant","valueEnv":"SF_TEST_TENANT"}]}]}"#;
    const CREDENTIAL: &str = "test-only-env-credential-0123456789";
    fn resolve(name: &str) -> Result<String, ()> {
        match name {
            "SF_TEST_CREDENTIAL" => Ok(CREDENTIAL.into()),
            "SF_TEST_CLAIMS" => Ok(r#"{"app.tenant_id":"a"}"#.into()),
            "SF_TEST_TENANT" => Ok("a".into()),
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
    fn version_two_resolves_portable_values_once() {
        let mut reads = 0;
        let profile = ProvisionedBearerAdmission::from_json_with(PORTABLE_DOCUMENT, |name| {
            reads += 1;
            resolve(name)
        })
        .unwrap();
        assert_eq!(reads, 2);
        let digest = Sha256::digest(CREDENTIAL.as_bytes()).into();
        assert!(profile.match_credential(&digest).is_some());
        assert!(!format!("{profile:?}").contains("people"));
    }
    #[test]
    fn bad_schema_fields_references_and_bounds_reject_before_resolution() {
        for document in [
            DOCUMENT.replace("\"schemaVersion\":1", "\"schemaVersion\":3"),
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
        for document in [
            PORTABLE_DOCUMENT.replace("\"schemaVersion\":2", "\"schemaVersion\":1"),
            PORTABLE_DOCUMENT.replace(
                "\"portableRows\":",
                "\"postgresRlsContextEnv\":\"SF_TEST_CLAIMS\",\"portableRows\":",
            ),
            PORTABLE_DOCUMENT.replace("SF_TEST_TENANT", "bad-name"),
        ] {
            assert!(ProvisionedBearerAdmission::from_json_with(&document, |_| {
                panic!("must not resolve")
            })
            .is_err());
        }
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
