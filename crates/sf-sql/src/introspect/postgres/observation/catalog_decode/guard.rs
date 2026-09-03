//! PostgreSQL 16 profile-guard catalogue decoding.

use super::super::{PostgresSchemaIdentityGuardCodeV1, PostgresSchemaIdentityUnavailableV1};
use super::{one_char, BoundedCatalogTextV1};
use tokio_postgres::Row;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::introspect::postgres::observation) struct CatalogGuardRowV1 {
    pub(in crate::introspect::postgres::observation) server_version_num: i32,
    pub(in crate::introspect::postgres::observation) server_encoding: String,
    pub(in crate::introspect::postgres::observation) client_encoding: String,
    pub(in crate::introspect::postgres::observation) max_identifier_length: i32,
    pub(in crate::introspect::postgres::observation) max_index_keys: i32,
    pub(in crate::introspect::postgres::observation) integer_datetimes: String,
    pub(in crate::introspect::postgres::observation) session_replication_role: String,
    pub(in crate::introspect::postgres::observation) search_path: String,
    pub(in crate::introspect::postgres::observation) database_oid: u32,
    pub(in crate::introspect::postgres::observation) database_provider: String,
    pub(in crate::introspect::postgres::observation) database_collate: String,
    pub(in crate::introspect::postgres::observation) database_ctype: String,
    pub(in crate::introspect::postgres::observation) database_icu_locale: Option<String>,
    pub(in crate::introspect::postgres::observation) database_icu_rules: Option<String>,
    pub(in crate::introspect::postgres::observation) database_recorded_version: Option<String>,
    pub(in crate::introspect::postgres::observation) database_actual_version: Option<String>,
    pub(in crate::introspect::postgres::observation) public_namespace_count: i64,
    pub(in crate::introspect::postgres::observation) current_database_count: i64,
}

impl CatalogGuardRowV1 {
    pub(in crate::introspect::postgres::observation) fn validate(
        &self,
    ) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
        match self.server_version_num {
            160_009 | 160_015 => {}
            160_000..=169_999 => {
                return Err(PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch)
            }
            _ => return Err(PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented),
        }
        let exact = [
            (
                self.server_encoding.as_str(),
                "UTF8",
                PostgresSchemaIdentityGuardCodeV1::ServerEncoding,
            ),
            (
                self.client_encoding.as_str(),
                "UTF8",
                PostgresSchemaIdentityGuardCodeV1::ServerEncoding,
            ),
            (
                self.integer_datetimes.as_str(),
                "on",
                PostgresSchemaIdentityGuardCodeV1::IntegerDatetimes,
            ),
            (
                self.session_replication_role.as_str(),
                "origin",
                PostgresSchemaIdentityGuardCodeV1::ReplicationRole,
            ),
            (
                self.search_path.as_str(),
                "pg_catalog,public,pg_temp",
                PostgresSchemaIdentityGuardCodeV1::SearchPath,
            ),
        ];
        if let Some((_, _, code)) = exact
            .into_iter()
            .find(|(actual, expected, _)| actual != expected)
        {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(code));
        }
        if self.max_identifier_length != 63 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::IdentifierLength,
            ));
        }
        if self.max_index_keys != 32 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit,
            ));
        }
        if self.public_namespace_count != 1 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::PublicNamespace,
            ));
        }
        if self.current_database_count != 1 {
            return Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                PostgresSchemaIdentityGuardCodeV1::CurrentDatabase,
            ));
        }
        if self.database_oid == 0
            || self
                .database_recorded_version
                .as_ref()
                .zip(self.database_actual_version.as_ref())
                .is_some_and(|(recorded, actual)| recorded != actual)
        {
            return Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected);
        }
        BoundedCatalogTextV1::new(self.database_collate.clone())?;
        BoundedCatalogTextV1::new(self.database_ctype.clone())?;
        one_char(self.database_provider.clone())?;
        for value in [
            self.database_icu_locale.as_deref(),
            self.database_icu_rules.as_deref(),
            self.database_recorded_version.as_deref(),
            self.database_actual_version.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            BoundedCatalogTextV1::new(value.to_owned())?;
        }
        Ok(())
    }
}

pub(in crate::introspect::postgres::observation) fn decode_guard_row_v1(
    row: &Row,
) -> Result<CatalogGuardRowV1, PostgresSchemaIdentityUnavailableV1> {
    macro_rules! get {
        ($name:literal, $ty:ty) => {
            row.try_get::<_, $ty>($name)
                .map_err(|_| PostgresSchemaIdentityUnavailableV1::CatalogDecode)?
        };
    }
    let value = CatalogGuardRowV1 {
        server_version_num: get!("server_version_num", i32),
        server_encoding: get!("server_encoding", String),
        client_encoding: get!("client_encoding", String),
        max_identifier_length: get!("max_identifier_length", i32),
        max_index_keys: get!("max_index_keys", i32),
        integer_datetimes: get!("integer_datetimes", String),
        session_replication_role: get!("session_replication_role", String),
        search_path: get!("search_path", String),
        database_oid: get!("database_oid", u32),
        database_provider: get!("database_provider", String),
        database_collate: get!("database_collate", String),
        database_ctype: get!("database_ctype", String),
        database_icu_locale: get!("database_icu_locale", Option<String>),
        database_icu_rules: get!("database_icu_rules", Option<String>),
        database_recorded_version: get!("database_recorded_version", Option<String>),
        database_actual_version: get!("database_actual_version", Option<String>),
        public_namespace_count: get!("public_namespace_count", i64),
        current_database_count: get!("current_database_count", i64),
    };
    value.validate()?;
    Ok(value)
}
