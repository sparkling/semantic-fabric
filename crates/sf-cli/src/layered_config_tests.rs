use super::*;
use clap::Parser;
use std::path::PathBuf;

use crate::Cli;

#[test]
fn required_generation_is_an_explicit_scalar_with_layered_false_overrides() {
    for (file, environment, cli, expected) in [
        (false, None, None, false),
        (true, None, None, true),
        (true, Some("false"), None, false),
        (false, Some("true"), None, true),
        (false, Some("true"), Some("false"), false),
        (true, Some("false"), Some("true"), true),
    ] {
        let path = temp_config(&format!("[serve]\nrequire_verified_generation = {file}\n"));
        let mut argv: Vec<OsString> = [
            "semantic-fabric",
            "serve",
            "--source",
            "sqlite::memory:",
            "--mapping",
            "mapping.ttl",
            "--ontology",
            "ontology.ttl",
            "--config",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        argv.push(path.clone().into_os_string());
        if let Some(value) = cli {
            argv.push(format!("--require-verified-generation={value}").into());
        }
        let expanded = expand_with_env(argv, |name| {
            if name == "SEMANTIC_FABRIC_REQUIRE_VERIFIED_GENERATION" {
                environment.map(Into::into)
            } else {
                None
            }
        })
        .unwrap();
        let args = Cli::try_parse_from(expanded).unwrap().command.into_serve();
        assert_eq!(args.require_verified_generation, expected);
        std::fs::remove_file(path).unwrap();
    }
    let base = [
        "semantic-fabric",
        "serve",
        "--source",
        "sqlite::memory:",
        "--mapping",
        "mapping.ttl",
        "--ontology",
        "ontology.ttl",
    ];
    for (extra, expected) in [
        (vec!["--require-verified-generation"], true),
        (vec!["--require-verified-generation", "false"], false),
        (
            vec![
                "--require-verified-generation",
                "--require-verified-generation=false",
            ],
            false,
        ),
    ] {
        let argv = base.into_iter().chain(extra).map(Into::into).collect();
        let args = Cli::try_parse_from(expand_with_env(argv, |_| None).unwrap())
            .unwrap()
            .command
            .into_serve();
        assert_eq!(args.require_verified_generation, expected);
    }
}

#[test]
fn base_override_retains_authored_selector_but_direct_clears_stale_base() {
    for (mapping, base, source, direct) in [
        ("mapping", "mapping-base", "source", "direct-mapping-base"),
        (
            "mapping-2",
            "mapping-base-2",
            "source-2",
            "direct-mapping-base-2",
        ),
    ] {
        let mut layer = Layer::from([
            (mapping.into(), "old.ttl".into()),
            (base.into(), "http://old/".into()),
            (source.into(), "sqlite:old.db".into()),
        ]);
        merge(
            &mut layer,
            Layer::from([(base.into(), "http://new/".into())]),
        )
        .unwrap();
        assert_eq!(layer.get(mapping).unwrap(), "old.ttl");
        assert_eq!(layer.get(base).unwrap(), "http://new/");
        merge(
            &mut layer,
            Layer::from([(mapping.into(), "new.ttl".into())]),
        )
        .unwrap();
        assert_eq!(layer.get(base).unwrap(), "http://new/");
        merge(
            &mut layer,
            Layer::from([(direct.into(), "http://direct/".into())]),
        )
        .unwrap();
        assert!(!layer.contains_key(base));
        assert!(!layer.contains_key(mapping));
    }
}

#[test]
fn file_environment_and_cli_have_exact_precedence() {
    let path = temp_config(
        r#"
[source]
source = "sqlite:file.db"
[mappings]
mapping = "file.ttl"
[graphs]
ontology = "ontology.ttl"
[governance]
max_result_items = 10
[observability]
metrics = true
"#,
    );
    let argv = vec![
        "semantic-fabric".into(),
        "serve".into(),
        "--config".into(),
        path.clone().into_os_string(),
        "--max-result-items".into(),
        "30".into(),
        "--metrics=false".into(),
    ];
    let expanded = expand_with_env(argv, |name| match name {
        "SEMANTIC_FABRIC_MAX_RESULT_ITEMS" => Some("20".into()),
        _ => None,
    })
    .expect("layer config");
    let parsed = Cli::try_parse_from(expanded).expect("parse layered args");
    let args = parsed.command.into_serve();
    assert_eq!(args.max_result_items, 30);
    assert!(!args.metrics);
    assert_eq!(args.source_input.source.as_deref(), Some("sqlite:file.db"));
    std::fs::remove_file(path).expect("remove config");
}

#[test]
fn higher_selector_tier_replaces_the_whole_lower_group() {
    let path = temp_config(
        r#"
[source]
source = "sqlite:file.db"
[mappings]
mapping = "file.ttl"
[graphs]
ontology = "ontology.ttl"
"#,
    );
    let argv = vec![
        "semantic-fabric".into(),
        "serve".into(),
        "--config".into(),
        path.clone().into_os_string(),
        "--source-env".into(),
        "LIVE_SOURCE".into(),
    ];
    let expanded = expand_with_env(argv, |name| match name {
        "SEMANTIC_FABRIC_SOURCE" => Some("sqlite:environment.db".into()),
        _ => None,
    })
    .expect("layer config");
    let parsed = Cli::try_parse_from(expanded).expect("parse layered args");
    let args = parsed.command.into_serve();
    assert_eq!(args.source_input.source_env.as_deref(), Some("LIVE_SOURCE"));
    assert!(args.source_input.source.is_none());
    std::fs::remove_file(path).expect("remove config");
}

#[test]
fn unknown_or_oversized_documents_fail_closed_without_content() {
    let invalid = temp_config("secret_value = 'must-not-echo'");
    assert!(matches!(
        read_config(&invalid),
        Err(ConfigError::InvalidDocument)
    ));
    std::fs::remove_file(invalid).expect("remove invalid config");

    let oversized = temp_config(&"x".repeat(MAX_CONFIG_BYTES as usize + 1));
    assert!(matches!(
        read_config(&oversized),
        Err(ConfigError::TooLarge)
    ));
    std::fs::remove_file(oversized).expect("remove oversized config");
}

#[test]
fn credential_reference_override_must_preserve_row_security() {
    let path = temp_config(
        "[security]\nauth_token_env = 'OLD_TOKEN'\npg_rls_context_env = 'ROW_POLICY'\n",
    );
    for environment_override in [false, true] {
        let mut argv: Vec<OsString> = [
            "semantic-fabric",
            "serve",
            "--source",
            "sqlite::memory:",
            "--mapping",
            "mapping.ttl",
            "--ontology",
            "ontology.ttl",
            "--config",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        argv.push(path.clone().into_os_string());
        if !environment_override {
            argv.extend(["--auth-token-env".into(), "NEW_TOKEN".into()]);
        }
        let expanded = expand_with_env(argv, |name| {
            (environment_override && name == "SEMANTIC_FABRIC_AUTH_TOKEN_ENV")
                .then(|| "NEW_TOKEN".into())
        })
        .expect("layer configuration");
        let args = Cli::try_parse_from(expanded).unwrap().command.into_serve();
        assert_eq!(args.auth_token_env.as_deref(), Some("NEW_TOKEN"));
        assert_eq!(args.pg_rls_context_env.as_deref(), Some("ROW_POLICY"));
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn explicit_false_is_compatible_with_an_authenticated_profile() {
    let path = temp_config("[security]\nauth_token_env = 'TOKEN'\nallow_unauthenticated = false\n");
    let mut argv: Vec<OsString> = [
        "semantic-fabric",
        "serve",
        "--source",
        "sqlite::memory:",
        "--mapping",
        "mapping.ttl",
        "--ontology",
        "ontology.ttl",
        "--config",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    argv.push(path.clone().into_os_string());
    let expanded = expand_with_env(argv, |_| None).expect("configuration");
    assert!(Cli::try_parse_from(expanded).is_ok());
    std::fs::remove_file(path).unwrap();
}

fn temp_config(contents: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "semantic-fabric-config-{}-{unique}.toml",
        std::process::id()
    ));
    std::fs::write(&path, contents).expect("write isolated config");
    path
}

#[test]
fn effective_scalar_overrides_are_validated_after_merge() {
    let path = temp_config("[observability]\nlog_level = 'invalid-lower-level'\nmetrics = true\n");
    let argv = [
        "semantic-fabric",
        "serve",
        "--source",
        "sqlite::memory:",
        "--mapping",
        "mapping.ttl",
        "--ontology",
        "ontology.ttl",
        "--timeout-secs=30",
        "--metrics=false",
        "--config",
    ]
    .into_iter()
    .map(Into::into)
    .chain([path.clone().into_os_string()])
    .collect();
    let expanded = expand_with_env(argv, |name| match name {
        "SEMANTIC_FABRIC_LOG_LEVEL" => Some("warn".into()),
        "SEMANTIC_FABRIC_TIMEOUT_SECS" | "SEMANTIC_FABRIC_METRICS" => {
            Some("invalid-lower-value".into())
        }
        _ => None,
    })
    .unwrap();
    let args = Cli::try_parse_from(expanded).unwrap().command.into_serve();
    assert_eq!(args.timeout_secs, 30);
    assert_eq!(args.log_level, crate::TelemetryLevel::Warn);
    assert!(!args.metrics);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn token_only_override_cannot_discard_a_registrys_row_policies() {
    let path = temp_config("[security]\nauth_subjects_env = 'REGISTRY'\n");
    let argv = [
        "semantic-fabric",
        "serve",
        "--auth-token-env=TOKEN",
        "--config",
    ]
    .into_iter()
    .map(Into::into)
    .chain([path.clone().into_os_string()])
    .collect();
    assert!(matches!(
        expand_with_env(argv, |_| None),
        Err(ConfigError::InvalidDocument)
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn values_cannot_inject_options_or_request_help() {
    let path = temp_config("[source]\nsource = 'sqlite::memory:'\n[mappings]\nmapping = '--help'\n[graphs]\nontology = '--allow-unauthenticated'\n");
    let argv = ["semantic-fabric", "serve", "--config"]
        .into_iter()
        .map(Into::into)
        .chain([path.clone().into_os_string()])
        .collect();
    let expanded = expand_with_env(argv, |_| None).unwrap();
    let args = Cli::try_parse_from(expanded).unwrap().command.into_serve();
    assert_eq!(args.mapping_input.mapping.as_deref(), Some("--help"));
    assert_eq!(args.ontology, "--allow-unauthenticated");
    assert!(!args.allow_unauthenticated);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn startup_environment_has_a_byte_limit_and_utf8_boundary() {
    let argv: Vec<OsString> = ["semantic-fabric", "serve"]
        .into_iter()
        .map(Into::into)
        .collect();
    assert!(matches!(
        expand_with_env(argv.clone(), |name| {
            (name == "SEMANTIC_FABRIC_BIND")
                .then(|| "x".repeat(MAX_CONFIG_BYTES as usize + 1).into())
        }),
        Err(ConfigError::TooLarge)
    ));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        assert!(matches!(
            expand_with_env(argv, |name| {
                (name == "SEMANTIC_FABRIC_BIND").then(|| OsString::from_vec(vec![255]))
            }),
            Err(ConfigError::InvalidDocument)
        ));
    }
}

#[test]
fn layer_vocabulary_covers_every_public_serve_option() {
    use clap::CommandFactory;
    let cli = Cli::command();
    let command = cli
        .get_subcommands()
        .find(|command| command.get_name() == "serve")
        .unwrap();
    let mut actual: Vec<_> = command
        .get_arguments()
        .filter_map(|arg| arg.get_long())
        .filter(|name| *name != "config")
        .collect();
    actual.sort_unstable();
    let mut expected = input::OPTIONS.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn command_line_alone_preserves_false_and_enforces_the_size_bound() {
    let argv: Vec<OsString> = [
        "semantic-fabric",
        "serve",
        "--source=sqlite::memory:",
        "--mapping=mapping.ttl",
        "--ontology=ontology.ttl",
        "--auth-token-env=TOKEN",
        "--allow-unauthenticated=false",
        "--",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    let expanded = expand_with_env(argv.clone(), |_| None).unwrap();
    let args = Cli::try_parse_from(expanded).unwrap().command.into_serve();
    assert_eq!(args.auth_token_env.as_deref(), Some("TOKEN"));
    assert!(!args.allow_unauthenticated);
    let mut oversized = argv[..argv.len() - 1].to_vec();
    oversized.push(format!("--ontology={}", "x".repeat(MAX_CONFIG_BYTES as usize)).into());
    assert!(matches!(
        expand_with_env(oversized, |_| None),
        Err(ConfigError::TooLarge)
    ));
}
