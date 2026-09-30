use super::*;
use crate::serve_args::QueryShapeArg;

fn arguments() -> Vec<OsString> {
    [
        "semantic-fabric",
        "serve",
        "--source",
        "sqlite::memory:",
        "--mapping",
        "mapping.ttl",
        "--ontology",
        "ontology.ttl",
    ]
    .into_iter()
    .map(Into::into)
    .collect()
}

#[test]
fn generated_profile_defaults_and_subject_admission_stay_independent() {
    for (profile, expected) in [
        (None, QueryShapeArg::Ordinary),
        (Some("ordinary"), QueryShapeArg::Ordinary),
        (
            Some("generated-select-ask"),
            QueryShapeArg::GeneratedSelectAsk,
        ),
    ] {
        let mut argv = arguments();
        if let Some(profile) = profile {
            argv.push(format!("--query-shape-profile={profile}").into());
        }
        let args = Cli::try_parse_from(argv).unwrap().command.into_serve();
        assert_eq!(args.query_shape_profile, expected);
        assert!(!args.allow_unauthenticated);
        assert!(args.auth_token_env.is_none());
        assert!(args.auth_subjects_env.is_none());
    }
}

#[test]
fn generated_profile_layers_preserve_precedence() {
    for (file, env, cli, expected) in [
        ("ordinary", None, None, QueryShapeArg::Ordinary),
        (
            "generated-select-ask",
            None,
            None,
            QueryShapeArg::GeneratedSelectAsk,
        ),
        (
            "generated-select-ask",
            Some("ordinary"),
            None,
            QueryShapeArg::Ordinary,
        ),
        (
            "ordinary",
            Some("generated-select-ask"),
            None,
            QueryShapeArg::GeneratedSelectAsk,
        ),
        (
            "generated-select-ask",
            Some("generated-select-ask"),
            Some("ordinary"),
            QueryShapeArg::Ordinary,
        ),
        (
            "invalid-lower-layer",
            Some("ordinary"),
            None,
            QueryShapeArg::Ordinary,
        ),
        (
            "invalid-lower-layer",
            Some("invalid-lower-layer"),
            Some("generated-select-ask"),
            QueryShapeArg::GeneratedSelectAsk,
        ),
    ] {
        let path = temp_config(&format!("[serve]\nquery_shape_profile = \"{file}\"\n"));
        let mut argv = arguments();
        argv.extend(["--config".into(), path.clone().into_os_string()]);
        if let Some(cli) = cli {
            argv.push(format!("--query-shape-profile={cli}").into());
        }
        let expanded = expand_with_env(argv, |name| {
            (name == "SEMANTIC_FABRIC_QUERY_SHAPE_PROFILE")
                .then(|| env.map(Into::into))
                .flatten()
        })
        .unwrap();
        let args = Cli::try_parse_from(expanded).unwrap().command.into_serve();
        assert_eq!(args.query_shape_profile, expected);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn generated_profile_rejects_invalid_effective_values() {
    let mut argv = arguments();
    argv.push("--query-shape-profile=unsupported".into());
    assert!(Cli::try_parse_from(argv).is_err());
    assert!(expand_with_env(arguments(), |name| {
        (name == "SEMANTIC_FABRIC_QUERY_SHAPE_PROFILE").then(|| "unsupported".into())
    })
    .is_err());
    let path = temp_config("[serve]\nquery_shape_profile = \"unsupported\"\n");
    let mut argv = arguments();
    argv.extend(["--config".into(), path.clone().into_os_string()]);
    assert!(expand_with_env(argv, |_| None).is_err());
    std::fs::remove_file(path).unwrap();
}
