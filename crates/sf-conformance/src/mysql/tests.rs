use super::*;

#[test]
fn malformed_url_is_redacted() {
    let marker = "must-not-leak";
    let error = url_opts(&format!("mysql://root:{marker}@[")).expect_err("malformed URL must fail");
    assert_eq!(error, "invalid MySQL connection configuration");
    assert!(!error.contains(marker));
}

#[test]
fn socket_boundary_rejects_relative_controlled_and_oversized_paths() {
    for value in ["relative.sock", "/tmp/bad\nsock"] {
        assert_eq!(
            socket_opts(value, None, None).unwrap_err(),
            "invalid MySQL socket configuration"
        );
    }
    let oversized = format!("/{}", "s".repeat(101));
    assert_eq!(
        socket_opts(&oversized, None, None).unwrap_err(),
        "invalid MySQL socket configuration"
    );
}

#[test]
fn no_endpoint_configuration_produces_no_network_options() {
    let marker = "credential-must-not-trigger-ambient-contact";
    let opts = connection_opts(None, None, Some("root".to_owned()), Some(marker.to_owned()))
        .expect("absence is typed, not an error");
    assert!(
        opts.is_none(),
        "no endpoint must create no connection options"
    );
}

#[test]
fn seeded_credentials_never_appear_in_configuration_errors() {
    let marker = "seeded-password-must-not-appear";
    let error = connection_opts(
        Some("relative.sock"),
        None,
        Some("root".to_owned()),
        Some(marker.to_owned()),
    )
    .expect_err("relative socket must fail closed");
    assert_eq!(error, "invalid MySQL socket configuration");
    assert!(!error.contains(marker));
}

#[test]
fn scratch_database_names_are_bounded_identifiers() {
    let first = scratch_database_name().expect("scratch name");
    let second = scratch_database_name().expect("scratch name");
    assert_ne!(first, second);
    for name in [first, second] {
        assert!(name.len() <= 64, "{name}");
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
            "{name}"
        );
    }
}

#[test]
fn required_and_optional_provider_absence_are_disjoint() {
    let optional = unavailable::<()>(LiveMode::LocalOptional, "MySQL connection failed")
        .expect("typed untested");
    assert!(matches!(optional, LiveRun::Untested(_)));
    let required = unavailable::<()>(LiveMode::CiRequired, "MySQL connection failed").unwrap_err();
    assert_eq!(
        required,
        "required MySQL provider is unavailable: MySQL connection failed"
    );
}

#[test]
fn cleanup_failure_takes_precedence_over_case_failure() {
    let error = prefer_cleanup_error::<()>(
        Err("case execution failed".to_owned()),
        Err("drop MySQL scratch database failed".to_owned()),
    )
    .unwrap_err();
    assert_eq!(error, "drop MySQL scratch database failed");
    assert_eq!(
        prefer_cleanup_error::<()>(Err("case execution failed".to_owned()), Ok(())).unwrap_err(),
        "case execution failed"
    );
}

#[test]
#[ignore = "requires a purpose-created isolated MySQL provider"]
fn live_cancelled_run_removes_only_its_owned_scratch_database() {
    let opts = base_opts()
        .expect("valid required-live MySQL configuration")
        .expect("required-live MySQL provider must be configured");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(async move {
        use mysql_async::prelude::Queryable;

        let owned = scratch_database_name().expect("owned name");
        let sibling = scratch_database_name().expect("sibling name");
        let mut control = Conn::new(opts.clone()).await.expect("isolated provider");
        control
            .query_drop(format!("CREATE DATABASE `{owned}`"))
            .await
            .expect("create owned scratch database");
        control
            .query_drop(format!("CREATE DATABASE `{sibling}`"))
            .await
            .expect("create sibling scratch database");

        let admin = Conn::new(opts).await.expect("cleanup admin");
        let guard = cleanup::ScratchDatabase::new(admin, control.opts().clone(), owned.clone());
        let task = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        task.abort();
        assert!(
            task.await
                .expect_err("task must be cancelled")
                .is_cancelled(),
            "test must exercise cancellation-driven Drop"
        );

        let remaining: Vec<String> = control
            .exec(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name IN (?, ?) ORDER BY schema_name",
                (owned.as_str(), sibling.as_str()),
            )
            .await
            .expect("inspect exact scratch names");
        let sibling_present = remaining.iter().any(|name| name == &sibling);
        let owned_present = remaining.iter().any(|name| name == &owned);
        control
            .query_drop(format!("DROP DATABASE IF EXISTS `{sibling}`"))
            .await
            .expect("remove sibling fixture");
        control
            .disconnect()
            .await
            .expect("close control connection");

        assert!(
            !owned_present,
            "Drop fallback must remove its owned database"
        );
        assert!(
            sibling_present,
            "Drop fallback must preserve the sibling database"
        );
    });
}
