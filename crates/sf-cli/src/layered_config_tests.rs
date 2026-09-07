use super::*;
use clap::Parser;

use crate::{Cli, Command};

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
    let Command::Serve(args) = parsed.command else {
        panic!("serve command")
    };
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
    let Command::Serve(args) = parsed.command else {
        panic!("serve command")
    };
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
