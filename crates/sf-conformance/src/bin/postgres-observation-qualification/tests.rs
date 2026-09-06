use super::*;
use tokio_postgres::config::Host;

#[test]
fn connection_authority_is_fixed_and_has_no_password() {
    for role in [OWNER, OBSERVER] {
        let config = fixed_config(role);
        assert_eq!(config.get_dbname(), Some(DATABASE));
        assert_eq!(config.get_user(), Some(role));
        assert_eq!(config.get_password(), None);
        assert_eq!(config.get_ports(), [5432]);
        assert!(matches!(
            config.get_hosts(),
            [Host::Unix(path)] if path == Path::new(SOCKET)
        ));
        assert_eq!(config.get_application_name(), Some(APPLICATION));
        assert_eq!(
            config.get_options(),
            Some("-c client_encoding=UTF8 -c session_replication_role=origin")
        );
    }
}

#[test]
fn version_validation_is_exact_bounded_and_printable() {
    assert!(validate_server_version("PostgreSQL 16.9", 160_009).is_ok());
    assert!(validate_server_version("PostgreSQL 16.15 (Debian)", 160_015).is_ok());
    assert!(validate_server_version("PostgreSQL 16.150", 160_015).is_err());
    assert!(validate_server_version("PostgreSQL 16.15\nleak", 160_015).is_err());
    assert!(validate_server_version("PostgreSQL 17.0", 170_000).is_err());
}

#[test]
fn stream_and_phase_inventories_are_closed_and_complete() {
    assert_eq!(ALL_STREAMS.len(), 10);
    assert_eq!(ALL_PHASES.len(), 10);
    assert_eq!(ALL_STREAMS[0].as_str(), "legacy-tables");
    assert_eq!(ALL_STREAMS[9].as_str(), "rich-catalog-constraints");
    assert_eq!(ALL_PHASES[0].as_str(), "guard");
    assert_eq!(ALL_PHASES[8].as_str(), "legacy-comparison");
    assert_eq!(ALL_PHASES[9].as_str(), "identity-build");
    assert_eq!(POSTGRES_PROFILE_PREQUALIFICATION_QUERY_COUNT_V1, 1);
    assert_eq!(POSTGRES_RICH_CAPTURE_CATALOGUE_QUERY_COUNT_V1, 4);
}

#[test]
fn role_admission_requires_every_negative_and_positive_fact() {
    let mut role = ComparisonRole {
        superuser: false,
        database_owner: false,
        inherit: false,
        bypass_rls: false,
        can_set_role: false,
        can_ddl: false,
        privileges_exact: true,
    };
    assert!(role.admitted());
    role.can_set_role = true;
    assert!(!role.admitted());
    role.can_set_role = false;
    role.privileges_exact = false;
    assert!(!role.admitted());
}
