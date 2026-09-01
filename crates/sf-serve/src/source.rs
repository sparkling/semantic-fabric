//! Typed source-reference resolution and pre-I/O source admission.

use std::env;
use std::fmt;

use crate::problem::StartupCause;
use crate::ServeError;

/// Maximum admitted source specification size, in UTF-8 bytes.
pub const MAX_SOURCE_INPUT_BYTES: usize = 16 * 1024;

/// Maximum admitted environment-variable name size, in ASCII bytes.
pub const MAX_SOURCE_ENV_NAME_BYTES: usize = 128;

/// Exact PostgreSQL startup option that binds unqualified runtime relations to
/// the catalogue scope introspected by `sf-sql`. `pg_temp` is explicit and last
/// so a temporary same-name relation cannot shadow `public`.
pub(crate) const POSTGRES_RELATION_SCOPE_OPTIONS: &str = "-csearch_path=pg_catalog,public,pg_temp";

/// Exact server-reported value corresponding to
/// [`POSTGRES_RELATION_SCOPE_OPTIONS`].
pub(crate) const POSTGRES_RELATION_SCOPE_SETTING: &str = "pg_catalog,public,pg_temp";

/// Reapply the invariant whenever a pooled session is recycled.
pub(crate) const POSTGRES_RELATION_SCOPE_RECYCLE_SQL: &str =
    "SELECT pg_catalog.set_config('search_path', 'pg_catalog,public,pg_temp', false)";

/// A source supplied directly on the command line or referenced through the
/// process environment. Inline values must be credential-free; environment
/// values are the only admitted secret-injection path in this narrow boundary.
pub enum SourceRef {
    Inline(String),
    Environment(String),
}

impl SourceRef {
    pub fn inline(value: impl Into<String>) -> Self {
        Self::Inline(value.into())
    }

    pub fn environment(variable: impl Into<String>) -> Self {
        Self::Environment(variable.into())
    }

    /// Resolve and validate this reference without exposing the source value in
    /// any public error representation.
    pub fn resolve(&self) -> Result<SourceInput, ServeError> {
        match self {
            Self::Inline(value) => SourceInput::new(value.clone(), SourceOrigin::Inline),
            Self::Environment(variable) => {
                validate_environment_name(variable)?;
                let value = env::var(variable).map_err(|error| match error {
                    env::VarError::NotPresent => source_error(
                        SourceOrigin::Environment,
                        "source environment variable is not set",
                    ),
                    env::VarError::NotUnicode(_) => source_error(
                        SourceOrigin::Environment,
                        "source environment variable is not valid UTF-8",
                    ),
                })?;
                SourceInput::new(value, SourceOrigin::Environment)
            }
        }
    }
}

impl fmt::Debug for SourceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Inline(_) => "inline",
            Self::Environment(_) => "environment",
        };
        formatter
            .debug_struct("SourceRef")
            .field("kind", &kind)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// A validated, resolved source value. The inner value intentionally has no
/// public accessor or derived formatter: only `sf-serve` may pass it to a
/// source-family parser and connector.
pub struct SourceInput {
    value: String,
    origin: SourceOrigin,
}

impl SourceInput {
    fn new(value: String, origin: SourceOrigin) -> Result<Self, ServeError> {
        validate_source_value(&value, origin)?;
        Ok(Self { value, origin })
    }

    pub fn is_environment_provided(&self) -> bool {
        self.origin == SourceOrigin::Environment
    }

    #[cfg(test)]
    pub(crate) fn injected(value: String) -> Result<Self, ServeError> {
        Self::new(value, SourceOrigin::Environment)
    }

    pub(crate) fn prepare(self) -> Result<PreparedSource, ServeError> {
        let label = self.origin.label();
        let admits_credentials = self.is_environment_provided();

        if let Some(path) = self.value.strip_prefix("sqlite:") {
            return Ok(PreparedSource::Sqlite {
                path: path.to_owned(),
                label,
            });
        }

        if let Some(conninfo) = self.value.strip_prefix("pg:") {
            let mut config = conninfo
                .parse::<tokio_postgres::Config>()
                .map_err(|error| source_error_with_label(label, error.to_string()))?;
            if !admits_credentials && config.get_password().is_some() {
                return Err(source_error_with_label(
                    label,
                    "inline PostgreSQL credentials are not admitted; use environment injection",
                ));
            }
            if config.get_options().is_some() {
                return Err(source_error_with_label(
                    label,
                    "PostgreSQL server options are reserved by the runtime relation-scope invariant",
                ));
            }
            config.options(POSTGRES_RELATION_SCOPE_OPTIONS);
            return Ok(PreparedSource::Postgres {
                config: Box::new(config),
                label,
            });
        }

        if self.value.starts_with("mysql://") {
            let options = mysql_async::Opts::from_url(&self.value)
                .map_err(|error| source_error_with_label(label, error.to_string()))?;
            if !admits_credentials && options.pass().is_some() {
                return Err(source_error_with_label(
                    label,
                    "inline MySQL credentials are not admitted; use environment injection",
                ));
            }
            return Ok(PreparedSource::Mysql { options, label });
        }

        Err(source_error_with_label(label, "unrecognised source family"))
    }
}

impl fmt::Debug for SourceInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceInput")
            .field("origin", &self.origin.label())
            .field("value", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SourceOrigin {
    Inline,
    Environment,
}

impl SourceOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::Inline => "<inline-source>",
            Self::Environment => "<environment-source>",
        }
    }
}

pub(crate) enum PreparedSource {
    Sqlite {
        path: String,
        label: &'static str,
    },
    Postgres {
        config: Box<tokio_postgres::Config>,
        label: &'static str,
    },
    Mysql {
        options: mysql_async::Opts,
        label: &'static str,
    },
}

fn validate_environment_name(variable: &str) -> Result<(), ServeError> {
    let mut bytes = variable.bytes();
    let valid_first = matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'));
    let valid_rest = bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if variable.len() > MAX_SOURCE_ENV_NAME_BYTES || !valid_first || !valid_rest {
        return Err(source_error(
            SourceOrigin::Environment,
            "invalid source environment variable name",
        ));
    }
    Ok(())
}

fn validate_source_value(value: &str, origin: SourceOrigin) -> Result<(), ServeError> {
    if value.is_empty() {
        return Err(source_error(origin, "source value is empty"));
    }
    if value.len() > MAX_SOURCE_INPUT_BYTES {
        return Err(source_error(origin, "source value exceeds the size limit"));
    }
    if value.contains('\0') {
        return Err(source_error(origin, "source value contains a NUL byte"));
    }
    Ok(())
}

fn source_error(origin: SourceOrigin, error: impl Into<String>) -> ServeError {
    source_error_with_label(origin.label(), error)
}

fn source_error_with_label(label: &'static str, error: impl Into<String>) -> ServeError {
    ServeError::new(StartupCause::SourceSpec {
        spec: label.to_owned(),
        error: error.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: &str = "sf_secret_NEVER_EXPOSE_source_model_71d2";

    #[test]
    fn debug_representations_redact_values_and_environment_names() {
        let reference = SourceRef::environment(SENTINEL);
        let input = SourceInput::new(SENTINEL.to_owned(), SourceOrigin::Environment)
            .expect("bounded UTF-8 source");

        assert!(!format!("{reference:?}").contains(SENTINEL));
        assert!(!format!("{input:?}").contains(SENTINEL));
    }

    #[test]
    fn inline_values_are_bounded_and_marked_as_non_environmental() {
        let input = SourceRef::inline("sqlite::memory:")
            .resolve()
            .expect("valid inline source");
        assert!(!input.is_environment_provided());

        let empty = SourceRef::inline("").resolve().unwrap_err();
        assert_eq!(empty.code(), "startup-source");
        let oversized = SourceRef::inline("x".repeat(MAX_SOURCE_INPUT_BYTES + 1))
            .resolve()
            .unwrap_err();
        assert_eq!(oversized.code(), "startup-source");
    }

    #[test]
    fn environment_names_use_a_bounded_portable_identifier_grammar() {
        for invalid in ["", "9SOURCE", "SOURCE-NAME", "SOURCE.NAME", "SØURCE"] {
            let error = SourceRef::environment(invalid).resolve().unwrap_err();
            assert_eq!(error.code(), "startup-source", "name={invalid:?}");
        }
        let too_long = format!("S{}", "A".repeat(MAX_SOURCE_ENV_NAME_BYTES));
        assert_eq!(
            SourceRef::environment(too_long)
                .resolve()
                .unwrap_err()
                .code(),
            "startup-source"
        );
    }

    #[test]
    fn inline_postgres_and_mysql_passwords_are_rejected_before_connectors() {
        for spec in [
            format!("pg:host=database.invalid user=test password={SENTINEL}"),
            format!("mysql://user:{SENTINEL}@database.invalid/db"),
        ] {
            let error = SourceRef::inline(spec)
                .resolve()
                .expect("bounded source")
                .prepare()
                .err()
                .expect("inline credentials must fail");
            assert_eq!(error.code(), "startup-source");
            assert!(!error.to_string().contains(SENTINEL));
            assert!(!format!("{error:?}").contains(SENTINEL));
        }
    }

    #[test]
    fn environment_inputs_may_prepare_driver_native_password_configuration() {
        let postgres = SourceInput::injected(format!(
            "pg:host=database.invalid user=test password={SENTINEL}"
        ))
        .expect("valid injected value")
        .prepare()
        .expect("environment PostgreSQL credential");
        let PreparedSource::Postgres { config, .. } = postgres else {
            panic!("expected PostgreSQL")
        };
        assert_eq!(config.get_password(), Some(SENTINEL.as_bytes()));
        assert_eq!(config.get_options(), Some(POSTGRES_RELATION_SCOPE_OPTIONS));

        let mysql = SourceInput::injected(format!("mysql://user:{SENTINEL}@database.invalid/db"))
            .expect("valid injected value")
            .prepare()
            .expect("environment MySQL credential");
        let PreparedSource::Mysql { options, .. } = mysql else {
            panic!("expected MySQL")
        };
        assert_eq!(options.pass(), Some(SENTINEL));
    }

    #[test]
    fn supported_secret_free_inline_families_prepare_without_connecting() {
        let specs = [
            "sqlite::memory:",
            "pg:host=database.invalid user=test",
            "mysql://user@database.invalid/db",
        ];
        for spec in specs {
            SourceRef::inline(spec)
                .resolve()
                .expect("valid source")
                .prepare()
                .unwrap_or_else(|error| panic!("{spec:?} should prepare: {error}"));
        }
    }

    #[test]
    fn caller_supplied_postgres_server_options_are_rejected() {
        let error = SourceInput::injected(
            "pg:host=database.invalid user=test options=-csearch_path=private".to_owned(),
        )
        .expect("bounded injected source")
        .prepare()
        .err()
        .expect("runtime-reserved server options must fail");
        assert_eq!(error.code(), "startup-source");
        assert!(!error.to_string().contains("private"));
    }
}
