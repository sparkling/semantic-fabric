use super::*;

#[test]
fn non_serve_commands_bypass_structured_subscriber_initialization() {
    for command in [Command::Conformance, Command::Bench] {
        let mut calls = 0;
        initialize_telemetry_for(&command, |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 0);
    }
}

#[test]
fn suite_root_points_at_the_vendored_w3c_suite_relative_to_the_crate() {
    let root = suite_root();
    assert!(
        root.ends_with("tests/w3c/rdb2rdf"),
        "expected the path to end in tests/w3c/rdb2rdf, got {root:?}"
    );
    assert!(
        root.is_absolute(),
        "CARGO_MANIFEST_DIR-based path should be absolute, got {root:?}"
    );
}

#[test]
fn suite_root_cases_dir_and_earl_report_paths_exist_under_the_workspace() {
    let root = suite_root();
    assert!(
        root.join("cases").is_dir(),
        "expected {:?} to exist (the vendored W3C RDB2RDF cases)",
        root.join("cases")
    );
}

#[test]
fn serve_source_selector_requires_exactly_one_transport() {
    let base = [
        "semantic-fabric",
        "serve",
        "--mapping",
        "mapping.ttl",
        "--ontology",
        "ontology.ttl",
    ];
    assert!(Cli::try_parse_from(base).is_err());

    let both = [
        "semantic-fabric",
        "serve",
        "--mapping",
        "mapping.ttl",
        "--ontology",
        "ontology.ttl",
        "--source",
        "sqlite::memory:",
        "--source-env",
        "SF_SOURCE",
    ];
    assert!(Cli::try_parse_from(both).is_err());

    for selector in [
        ["--source", "sqlite::memory:"],
        ["--source-env", "SF_SOURCE"],
    ] {
        let args = base.into_iter().chain(selector);
        assert!(Cli::try_parse_from(args).is_ok());
    }
}

#[test]
fn serve_mapping_selector_requires_exactly_one_mapping_mode() {
    let source = [
        "semantic-fabric",
        "serve",
        "--source",
        "sqlite::memory:",
        "--ontology",
        "ontology.ttl",
    ];
    assert!(Cli::try_parse_from(source).is_err());
    assert!(Cli::try_parse_from(source.into_iter().chain([
        "--mapping",
        "mapping.ttl",
        "--direct-mapping-base",
        "http://example.com/base/",
    ]))
    .is_err());

    assert!(Cli::try_parse_from(
        source
            .into_iter()
            .chain(["--direct-mapping-base", "http://example.com/base/"])
    )
    .is_ok());
}

#[test]
fn serve_requires_ontology_and_documents_it_in_help() {
    let error = Cli::try_parse_from([
        "semantic-fabric",
        "serve",
        "--source",
        "sqlite::memory:",
        "--mapping",
        "mapping.ttl",
    ])
    .err()
    .expect("serve without an ontology must fail parsing");
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    let help = Cli::try_parse_from(["semantic-fabric", "serve", "--help"])
        .err()
        .expect("help exits through clap's display-help error");
    assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
    let help = help.to_string();
    assert!(help.contains("--ontology <ONTOLOGY>"));
    assert!(help.contains("Required ontology (Turtle)"));
}

#[test]
fn serve_second_source_requires_one_selector_and_its_mapping() {
    let base = [
        "semantic-fabric",
        "serve",
        "--mapping",
        "first.ttl",
        "--source",
        "sqlite:first.db",
        "--ontology",
        "ontology.ttl",
    ];
    assert!(
        Cli::try_parse_from(base.into_iter().chain(["--source-2", "sqlite:second.db"])).is_err()
    );
    assert!(Cli::try_parse_from(base.into_iter().chain(["--mapping-2", "second.ttl"])).is_err());
    assert!(Cli::try_parse_from(base.into_iter().chain([
        "--source-2",
        "sqlite:second.db",
        "--source-env-2",
        "SF_SOURCE_2",
        "--mapping-2",
        "second.ttl",
    ]))
    .is_err());
    assert!(Cli::try_parse_from(base.into_iter().chain([
        "--source-2",
        "sqlite:second.db",
        "--mapping-2",
        "second.ttl",
        "--direct-mapping-base-2",
        "http://example.com/second/",
    ]))
    .is_err());

    let parsed = Cli::try_parse_from(base.into_iter().chain([
        "--source-2",
        "sqlite:second.db",
        "--mapping-2",
        "second.ttl",
    ]))
    .expect("complete two-source startup arguments");
    let Command::Serve(parsed) = parsed.command else {
        panic!("serve command")
    };
    assert_eq!(
        parsed
            .additional_source_input
            .mapping_input
            .mapping
            .as_deref(),
        Some("second.ttl")
    );
    assert_eq!(
        parsed
            .additional_source_input
            .source_input
            .source
            .as_deref(),
        Some("sqlite:second.db")
    );

    let direct = Cli::try_parse_from(base.into_iter().chain([
        "--source-2",
        "sqlite:second.db",
        "--direct-mapping-base-2",
        "http://example.com/second/",
    ]))
    .expect("complete two-source Direct Mapping startup arguments");
    let Command::Serve(direct) = direct.command else {
        panic!("serve command")
    };
    assert_eq!(
        direct
            .additional_source_input
            .mapping_input
            .direct_mapping_base
            .as_deref(),
        Some("http://example.com/second/")
    );
}

#[test]
fn serve_request_admission_limit_has_a_finite_default_and_accepts_an_override() {
    let base = [
        "semantic-fabric",
        "serve",
        "--mapping",
        "mapping.ttl",
        "--source",
        "sqlite::memory:",
        "--ontology",
        "ontology.ttl",
    ];
    let defaults = Cli::try_parse_from(base).expect("default serve arguments");
    let Command::Serve(defaults) = defaults.command else {
        panic!("serve command")
    };
    assert_eq!(
        defaults.max_concurrent_requests,
        DEFAULT_MAX_CONCURRENT_REQUESTS
    );
    assert_eq!(
        Duration::from_secs(defaults.shutdown_timeout_secs),
        DEFAULT_SHUTDOWN_TIMEOUT
    );

    let explicit = Cli::try_parse_from(base.into_iter().chain(["--max-concurrent-requests", "7"]))
        .expect("explicit request-admission limit");
    let Command::Serve(explicit) = explicit.command else {
        panic!("serve command")
    };
    assert_eq!(explicit.max_concurrent_requests, 7);

    let explicit_shutdown =
        Cli::try_parse_from(base.into_iter().chain(["--shutdown-timeout-secs", "9"]))
            .expect("explicit shutdown timeout");
    let Command::Serve(explicit_shutdown) = explicit_shutdown.command else {
        panic!("serve command")
    };
    assert_eq!(explicit_shutdown.shutdown_timeout_secs, 9);
}

#[test]
fn serve_log_level_is_closed_bounded_and_defaults_to_info() {
    let base = [
        "semantic-fabric",
        "serve",
        "--mapping",
        "mapping.ttl",
        "--source",
        "sqlite::memory:",
        "--ontology",
        "ontology.ttl",
    ];
    let parsed = Cli::try_parse_from(base).unwrap();
    let Command::Serve(args) = parsed.command else {
        panic!("serve command")
    };
    assert_eq!(args.log_level, TelemetryLevel::Info);

    for (value, expected) in [
        ("off", TelemetryLevel::Off),
        ("error", TelemetryLevel::Error),
        ("warn", TelemetryLevel::Warn),
        ("info", TelemetryLevel::Info),
    ] {
        let parsed = Cli::try_parse_from(base.into_iter().chain(["--log-level", value])).unwrap();
        let Command::Serve(args) = parsed.command else {
            panic!("serve command")
        };
        assert_eq!(args.log_level, expected);
    }
    for invalid in ["debug", "trace", "sf_sql=debug", "info,hyper=trace"] {
        assert!(Cli::try_parse_from(base.into_iter().chain(["--log-level", invalid])).is_err());
    }
}

#[test]
fn serve_returns_failure_exit_code_not_panic_on_missing_mapping_file() {
    let opts = ServeArgs {
        source_input: SourceArgs {
            source: Some("sqlite::memory:".to_owned()),
            source_env: None,
        },
        mapping_input: MappingArgs {
            mapping: Some("/nonexistent/path/does-not-exist.ttl".to_owned()),
            direct_mapping_base: None,
        },
        additional_source_input: AdditionalSourceArgs {
            source_input: AdditionalSourceSelector {
                source: None,
                source_env: None,
            },
            mapping_input: AdditionalMappingSelector {
                mapping: None,
                direct_mapping_base: None,
            },
        },
        ontology: suite_root()
            .join("manifest-evaluation.ttl")
            .to_string_lossy()
            .into_owned(),
        bind: "127.0.0.1:0".to_owned(),
        log_level: TelemetryLevel::Info,
        timeout_secs: 1,
        max_query_len: 1024,
        max_concurrent_requests: DEFAULT_MAX_CONCURRENT_REQUESTS,
        max_source_work: 1_000,
        max_result_items: 1_000,
        max_order_rows: DEFAULT_MAX_ORDER_ROWS,
        max_order_bytes: DEFAULT_MAX_ORDER_BYTES,
        max_serialized_bytes: 1 << 20,
        pg_pool_size: 16,
        pg_pool_wait_secs: 5,
        sqlite_pool_size: 4,
        shutdown_timeout_secs: DEFAULT_SHUTDOWN_TIMEOUT.as_secs(),
    };
    assert_eq!(serve(opts), ExitCode::FAILURE);
}

#[test]
fn conformance_returns_success_exit_code_on_the_real_suite() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let out_dir = std::env::temp_dir().join(format!(
        "sf_cli_conformance_{}_{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&out_dir).expect("create isolated evidence directory");
    assert_eq!(conformance_to(&out_dir), ExitCode::SUCCESS);
    assert!(out_dir.join("earl-semantic-fabric-r2rml.ttl").is_file());
    assert!(out_dir.join("earl-semantic-fabric-direct.ttl").is_file());
    std::fs::remove_dir_all(out_dir).expect("remove isolated evidence directory");
}
