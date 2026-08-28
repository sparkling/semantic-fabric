use std::process::Command;

pub(super) const DENIED_EXACT: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "AZURE_OPENAI_API_KEY",
    "CARGO",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_TARGET",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_INCREMENTAL",
    "CARGO_TARGET_DIR",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "DATABASE_URL",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "MSSQL_SA_PASSWORD",
    "MYSQL_DATABASE",
    "MYSQL_HOST",
    "MYSQL_PASSWORD",
    "MYSQL_PWD",
    "MYSQL_ROOT_PASSWORD",
    "MYSQL_URL",
    "MYSQL_USER",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "PGDATABASE",
    "PGHOST",
    "PGPASSWORD",
    "PGPORT",
    "PGUSER",
    "POSTGRES_DB",
    "POSTGRES_PASSWORD",
    "POSTGRES_USER",
    "REQUESTY_API_KEY",
    "RUSTC",
    "RUSTC_BOOTSTRAP",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTDOC",
    "RUSTDOCFLAGS",
    "RUSTFLAGS",
    "RUST_LOG",
    "RUST_MIN_STACK",
    "RUST_TEST_NOCAPTURE",
    "SF_MSSQL_URL",
    "SF_MYSQL_URL",
    "SF_PG_URL",
];

pub(super) const DENIED_PREFIXES: &[&str] = &["CARGO_PROFILE_"];

pub(super) fn scrub(command: &mut Command) {
    for name in DENIED_EXACT {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|name| {
            DENIED_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
        }) {
            command.env_remove(name);
        }
    }
}
