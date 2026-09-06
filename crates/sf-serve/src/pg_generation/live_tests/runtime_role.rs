//! Required-live proof for the PostgreSQL runtime-role boundary.

use super::*;

pub(super) async fn exercise(fixture: &Fixture) {
    let client = fixture
        .pool
        .get()
        .await
        .expect("acquire restricted runtime-role probe");
    capture_runtime_role(&client)
        .await
        .expect("exact restricted runtime-role context");
    assert_context_read_failure_classification(&client).await;

    let label: String = client
        .query_one("SELECT label FROM public.parent WHERE id = 1", &[])
        .await
        .expect("runtime role may SELECT each mapped table")
        .get(0);
    assert_eq!(label, "parent");
    let _: Option<String> = client
        .query_one(
            "SELECT pg_catalog.pg_database_collation_actual_version(oid) \
             FROM pg_catalog.pg_database \
             WHERE datname = pg_catalog.current_database()",
            &[],
        )
        .await
        .expect("runtime role may call the required collation probe")
        .get(0);

    for (operation, sql) in [
        ("set-role", "SET ROLE pg_read_all_data"),
        (
            "schema-ddl",
            "CREATE TABLE public.denied_create (id integer)",
        ),
        (
            "temporary-ddl",
            "CREATE TEMP TABLE denied_temp (id integer)",
        ),
        ("insert", "INSERT INTO public.parent VALUES (2, 'denied')"),
        (
            "update",
            "UPDATE public.parent SET label = 'denied' WHERE id = 1",
        ),
        ("delete", "DELETE FROM public.parent WHERE id = 1"),
        ("truncate", "TRUNCATE TABLE public.child"),
        (
            "alter",
            "ALTER TABLE public.parent ADD COLUMN denied integer",
        ),
        ("drop", "DROP TABLE public.parent"),
    ] {
        assert_insufficient_privilege(&client, operation, sql).await;
    }
    drop(client);

    for (label, granted, revoked) in [
        ("superuser", "SUPERUSER", "NOSUPERUSER"),
        ("inherit", "INHERIT", "NOINHERIT"),
        ("create-role", "CREATEROLE", "NOCREATEROLE"),
        ("create-database", "CREATEDB", "NOCREATEDB"),
        ("replication", "REPLICATION", "NOREPLICATION"),
        ("bypass-rls", "BYPASSRLS", "NOBYPASSRLS"),
    ] {
        assert_context_drift(
            fixture,
            label,
            &format!("ALTER ROLE {} WITH {granted}", fixture.role),
            &format!("ALTER ROLE {} WITH {revoked}", fixture.role),
        )
        .await;
    }
    assert_context_drift(
        fixture,
        "role membership",
        &format!("GRANT pg_read_all_data TO {}", fixture.role),
        &format!("REVOKE pg_read_all_data FROM {}", fixture.role),
    )
    .await;
    assert_context_drift(
        fixture,
        "database ownership",
        &format!(
            "ALTER DATABASE {} OWNER TO {}",
            fixture.database, fixture.role
        ),
        &format!(
            "ALTER DATABASE {} OWNER TO CURRENT_USER; \
             REVOKE ALL ON DATABASE {} FROM PUBLIC; \
             REVOKE ALL ON DATABASE {} FROM {}; \
             GRANT CONNECT ON DATABASE {} TO {}",
            fixture.database,
            fixture.database,
            fixture.database,
            fixture.role,
            fixture.database,
            fixture.role
        ),
    )
    .await;
    assert_context_drift(
        fixture,
        "schema ownership",
        &format!("ALTER SCHEMA public OWNER TO {}", fixture.role),
        &format!(
            "ALTER SCHEMA public OWNER TO pg_database_owner; \
             REVOKE ALL ON SCHEMA public FROM PUBLIC; \
             REVOKE ALL ON SCHEMA public FROM {}; \
             GRANT USAGE ON SCHEMA public TO {}",
            fixture.role, fixture.role
        ),
    )
    .await;
    assert_context_drift(
        fixture,
        "table ownership",
        &format!("ALTER TABLE public.parent OWNER TO {}", fixture.role),
        &format!(
            "ALTER TABLE public.parent OWNER TO CURRENT_USER; \
             REVOKE ALL ON TABLE public.parent FROM PUBLIC; \
             REVOKE ALL ON TABLE public.parent FROM {}; \
             GRANT SELECT ON TABLE public.parent TO {}",
            fixture.role, fixture.role
        ),
    )
    .await;
    assert_extra_privileges_fail_closed(fixture).await;
    assert_missing_privileges_fail_closed(fixture).await;
}

async fn assert_extra_privileges_fail_closed(fixture: &Fixture) {
    for (label, grant, revoke) in [
        (
            "database-create",
            format!(
                "GRANT CREATE ON DATABASE {} TO {}",
                fixture.database, fixture.role
            ),
            format!(
                "REVOKE CREATE ON DATABASE {} FROM {}",
                fixture.database, fixture.role
            ),
        ),
        (
            "database-temporary",
            format!(
                "GRANT TEMP ON DATABASE {} TO {}",
                fixture.database, fixture.role
            ),
            format!(
                "REVOKE TEMP ON DATABASE {} FROM {}",
                fixture.database, fixture.role
            ),
        ),
        (
            "schema-create",
            format!("GRANT CREATE ON SCHEMA public TO {}", fixture.role),
            format!("REVOKE CREATE ON SCHEMA public FROM {}", fixture.role),
        ),
    ] {
        assert_context_drift(fixture, label, &grant, &revoke).await;
    }

    for privilege in [
        "INSERT",
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "REFERENCES",
        "TRIGGER",
    ] {
        assert_context_drift(
            fixture,
            &format!("table-{privilege}"),
            &format!(
                "GRANT {privilege} ON TABLE public.parent TO {}",
                fixture.role
            ),
            &format!(
                "REVOKE {privilege} ON TABLE public.parent FROM {}",
                fixture.role
            ),
        )
        .await;
    }
    for privilege in ["INSERT", "UPDATE", "REFERENCES"] {
        assert_context_drift(
            fixture,
            &format!("column-{privilege}"),
            &format!(
                "GRANT {privilege} (label) ON TABLE public.parent TO {}",
                fixture.role
            ),
            &format!(
                "REVOKE {privilege} (label) ON TABLE public.parent FROM {}",
                fixture.role
            ),
        )
        .await;
    }
}

async fn assert_missing_privileges_fail_closed(fixture: &Fixture) {
    for (label, revoke, grant) in [
        (
            "missing-connect",
            format!(
                "REVOKE CONNECT ON DATABASE {} FROM {}",
                fixture.database, fixture.role
            ),
            format!(
                "GRANT CONNECT ON DATABASE {} TO {}",
                fixture.database, fixture.role
            ),
        ),
        (
            "missing-usage",
            format!("REVOKE USAGE ON SCHEMA public FROM {}", fixture.role),
            format!("GRANT USAGE ON SCHEMA public TO {}", fixture.role),
        ),
        (
            "missing-select",
            format!("REVOKE SELECT ON TABLE public.parent FROM {}", fixture.role),
            format!("GRANT SELECT ON TABLE public.parent TO {}", fixture.role),
        ),
        (
            "missing-collation-probe",
            format!(
                "REVOKE EXECUTE ON FUNCTION \
                 pg_catalog.pg_database_collation_actual_version(oid) FROM PUBLIC, {}",
                fixture.role
            ),
            format!(
                "GRANT EXECUTE ON FUNCTION \
                 pg_catalog.pg_database_collation_actual_version(oid) TO PUBLIC, {}",
                fixture.role
            ),
        ),
    ] {
        assert_context_drift(fixture, label, &revoke, &grant).await;
    }
}

async fn assert_context_drift(fixture: &Fixture, label: &str, apply: &str, restore: &str) {
    fixture
        .admin
        .batch_execute(apply)
        .await
        .unwrap_or_else(|_| panic!("apply isolated {label} mutation"));
    let client = fixture
        .pool
        .get()
        .await
        .expect("acquire mutated runtime-role probe");
    let observed = capture_runtime_role(&client).await;
    drop(client);
    fixture
        .admin
        .batch_execute(restore)
        .await
        .unwrap_or_else(|_| panic!("restore isolated {label} mutation"));
    assert!(
        matches!(observed, Err(PgGenerationError::CapabilityDrift)),
        "isolated {label} mutation must fail closed"
    );
    let restored = fixture
        .pool
        .get()
        .await
        .expect("acquire restored runtime-role probe");
    capture_runtime_role(&restored)
        .await
        .unwrap_or_else(|_| panic!("restore exact isolated {label} boundary"));
}

async fn capture_runtime_role(client: &Client) -> Result<(), PgGenerationError> {
    capture_runtime_role_query(client, None).await
}

async fn capture_runtime_role_query(
    client: &Client,
    query: Option<&str>,
) -> Result<(), PgGenerationError> {
    let setup = transaction_setup_sql(&budget()).expect("build runtime-role probe transaction");
    client
        .batch_execute(&setup)
        .await
        .expect("begin restricted runtime-role probe");
    let captured = match query {
        Some(query) => super::super::context::capture_session_context_query_for_test(client, query)
            .await
            .map(|_| ()),
        None => capture_session_context(client).await.map(|_| ()),
    };
    client
        .batch_execute("ROLLBACK")
        .await
        .expect("rollback restricted runtime-role probe");
    captured
}

async fn assert_context_read_failure_classification(client: &Client) {
    for (label, query) in [
        (
            "query failure",
            "SELECT * FROM pg_catalog.sf_missing_context_relation",
        ),
        ("row decode failure", "SELECT 1 AS unrelated_context_field"),
    ] {
        assert!(
            matches!(
                capture_runtime_role_query(client, Some(query)).await,
                Err(PgGenerationError::SourceUnavailable)
            ),
            "{label} before a context row is decoded must report source unavailable"
        );
    }
}

async fn assert_insufficient_privilege(client: &Client, operation: &str, sql: &str) {
    let error = match client.batch_execute(sql).await {
        Ok(()) => panic!("restricted runtime role {operation} must fail"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        Some(&SqlState::INSUFFICIENT_PRIVILEGE),
        "unexpected {operation} denial for fixed runtime-role probe"
    );
}
