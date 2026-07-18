//! SQL Server `SqlBackend` adapter (ADR-0024 M8).
//!
//! Uses the `tiberius` crate (TDS protocol, async, `@Pn`-style placeholders).
//! Requires the `sqlserver-backend` feature which pulls in `tiberius` and
//! `tokio-util` (for the `Compat` wrapper that bridges tokio's `TcpStream`
//! into the `futures_io` interface `tiberius` expects).
//!
//! Connection string format (passed to `SqlServerBackend::connect`):
//!   `"server=tcp:localhost,11433;user id=SA;password=SfTest123!"`
//!
//! Verification tier: live Docker (SQL Server 2022) when `sqlserver-backend`
//! feature is enabled and `SF_MSSQL_URL` is set; unit marshalling tests always.

#[cfg(feature = "sqlserver-backend")]
pub use real::SqlServerBackend;
#[cfg(feature = "sqlserver-backend")]
pub use real::SqlServerStream;

#[cfg(not(feature = "sqlserver-backend"))]
pub use stub::SqlServerBackend;
#[cfg(not(feature = "sqlserver-backend"))]
pub use stub::SqlServerStream;

// ─── stub (no tiberius dep) ──────────────────────────────────────────────────

#[cfg(not(feature = "sqlserver-backend"))]
mod stub {
    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// SQL Server stub backend. Enable `sqlserver-backend` feature to activate.
    pub struct SqlServerBackend;

    /// Stub stream for SQL Server — never yields rows.
    pub struct SqlServerStream;

    impl BranchStream for SqlServerStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Err(Error::Unsupported(
                "SqlServerBackend: enable the `sqlserver-backend` feature".to_owned(),
            ))
        }
    }

    impl SqlBackend for SqlServerBackend {
        type Stream<'s>
            = SqlServerStream
        where
            Self: 's;

        async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
            Err(Error::Unsupported(
                "SqlServerBackend: enable the `sqlserver-backend` feature".to_owned(),
            ))
        }

        async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<SqlServerStream> {
            Err(Error::Unsupported(
                "SqlServerBackend: enable the `sqlserver-backend` feature".to_owned(),
            ))
        }
    }
}

// ─── real implementation (requires `sqlserver-backend` feature) ──────────────

#[cfg(feature = "sqlserver-backend")]
pub mod real {
    use std::collections::VecDeque;

    use tiberius::{AuthMethod, Client, ColumnData, Config, Query};
    use tokio::net::TcpStream;
    use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

    use crate::backend::{BranchStream, RawTuple, SqlBackend};
    use crate::error::{Error, Result};

    /// SQL Server backend wrapping a live `tiberius::Client<Compat<TcpStream>>`.
    pub struct SqlServerBackend {
        client: Client<Compat<TcpStream>>,
    }

    impl SqlServerBackend {
        /// Connect using an ADO.NET–style connection string.
        /// Example: `"server=tcp:localhost,11433;user id=SA;password=SfTest123!"`
        pub async fn connect(conn_str: &str) -> Result<Self> {
            let config = parse_conn_str(conn_str)?;
            let tcp = TcpStream::connect(config.get_addr())
                .await
                .map_err(|e| Error::Marshal(format!("sql server tcp connect: {e}")))?;
            tcp.set_nodelay(true)
                .map_err(|e| Error::Marshal(format!("sql server tcp nodelay: {e}")))?;
            let client = Client::connect(config, tcp.compat_write())
                .await
                .map_err(|e| Error::Marshal(format!("sql server tiberius connect: {e}")))?;
            Ok(Self { client })
        }
    }

    /// Parse an ADO.NET connection string into a `tiberius::Config`.
    pub fn parse_conn_str(s: &str) -> Result<Config> {
        let mut config = Config::new();
        let mut host = "localhost".to_owned();
        let mut port: u16 = 1433;
        let mut user = String::new();
        let mut pwd = String::new();

        for part in s.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(eq) = part.find('=') {
                let key = part[..eq].trim().to_lowercase();
                let val = part[eq + 1..].trim();
                match key.as_str() {
                    "server" | "data source" => {
                        let addr = val.trim_start_matches("tcp:");
                        if let Some((h, p)) = addr.rsplit_once(',') {
                            host = h.to_owned();
                            port = p.parse::<u16>().unwrap_or(1433);
                        } else {
                            host = addr.to_owned();
                        }
                    }
                    "user id" | "uid" | "user" => user = val.to_owned(),
                    "password" | "pwd" => pwd = val.to_owned(),
                    "database" | "initial catalog" => {
                        config.database(val);
                    }
                    _ => {}
                }
            }
        }

        if !user.is_empty() {
            config.authentication(AuthMethod::sql_server(&user, &pwd));
        }
        config.host(&host);
        config.port(port);
        // Trust the server certificate — required for test containers without
        // a signed cert.
        config.trust_cert();
        Ok(config)
    }

    /// A collected row stream: rows are drained from the tiberius QueryStream
    /// upfront so the connection is free for the next query. Rows are freed
    /// after delivery (VecDeque pop_front), keeping peak memory proportional
    /// to the result set shape, not cardinality of held rows.
    pub struct SqlServerStream {
        rows: VecDeque<RawTuple>,
    }

    impl BranchStream for SqlServerStream {
        async fn next_row(&mut self) -> Result<Option<RawTuple>> {
            Ok(self.rows.pop_front())
        }
    }

    impl SqlBackend for SqlServerBackend {
        type Stream<'s>
            = SqlServerStream
        where
            Self: 's;

        async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
            // Use simple_query for metadata — it returns column names even for
            // LIMIT 0 / zero-row results via ResultSetStream metadata items.
            let stream = self
                .client
                .simple_query(probe_sql)
                .await
                .map_err(|e| Error::Marshal(format!("sql server column_names: {e}")))?;
            // into_results collects Vec<Vec<Row>>; we want column metadata from
            // the first result set's columns() — available on any row, or from
            // the metadata if there are no rows.
            let result_sets = stream
                .into_results()
                .await
                .map_err(|e| Error::Marshal(format!("sql server column_names drain: {e}")))?;
            // If rows came back, column names are on the first row.
            // If no rows, we fall through to the empty default.
            let names: Vec<String> = result_sets
                .into_iter()
                .next()
                .and_then(|rows| rows.into_iter().next())
                .map(|row| row.columns().iter().map(|c| c.name().to_owned()).collect())
                .unwrap_or_default();
            Ok(names)
        }

        async fn open_branch(
            &mut self,
            sql: &str,
            lexical_params: &[String],
        ) -> Result<SqlServerStream> {
            let mut q = Query::new(sql);
            for p in lexical_params {
                // Bind each param as a string — tiberius accepts &str for @Pn.
                q.bind(p.as_str());
            }
            let stream = q
                .query(&mut self.client)
                .await
                .map_err(|e| Error::Marshal(format!("sql server open_branch: {e}")))?;
            let result_sets = stream
                .into_results()
                .await
                .map_err(|e| Error::Marshal(format!("sql server drain results: {e}")))?;

            let mut rows = VecDeque::new();
            if let Some(result) = result_sets.into_iter().next() {
                for tib_row in result {
                    let ncols = tib_row.len();
                    let mut values = Vec::with_capacity(ncols);
                    for (_col, data) in tib_row.cells() {
                        values.push(marshal_column_data(Some(data))?);
                    }
                    let codes = vec![None; ncols];
                    rows.push_back(RawTuple { values, codes });
                }
            }
            Ok(SqlServerStream { rows })
        }
    }

    /// Marshal a tiberius `ColumnData` reference to a lexical `Option<String>`.
    /// NULL (inner `None`) → `None`; every live value → `Some(lexical_string)`.
    pub fn marshal_column_data(data: Option<&ColumnData<'_>>) -> Result<Option<String>> {
        let Some(data) = data else {
            return Ok(None);
        };
        let s: Option<String> = match data {
            ColumnData::U8(v) => v.map(|x| x.to_string()),
            ColumnData::I16(v) => v.map(|x| x.to_string()),
            ColumnData::I32(v) => v.map(|x| x.to_string()),
            ColumnData::I64(v) => v.map(|x| x.to_string()),
            ColumnData::F32(v) => v.map(|x| x.to_string()),
            ColumnData::F64(v) => v.map(|x| x.to_string()),
            ColumnData::Bit(v) => v.map(|x| if x { "true" } else { "false" }.to_owned()),
            ColumnData::String(v) => v.as_ref().map(|s| s.to_string()),
            ColumnData::Guid(v) => v.map(|g| {
                let b = g.to_bytes_le();
                format!(
                    "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                    b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
                )
            }),
            ColumnData::Binary(v) => v.as_ref().map(|b| {
                let mut out = String::with_capacity(b.len() * 2);
                for byte in b.iter() {
                    use std::fmt::Write;
                    let _ = write!(out, "{byte:02X}");
                }
                out
            }),
            ColumnData::Numeric(v) => v.map(|n| n.to_string()),
            ColumnData::Xml(_) => {
                return Err(Error::Unsupported(
                    "SQL Server XML type not supported".to_owned(),
                ))
            }
            ColumnData::DateTime(v) => v.map(|dt| {
                // days since 1900-01-01; seconds_fragments are 1/300-second units.
                let (y, mo, d) = date_from_days_1900(dt.days());
                let total_ms = dt.seconds_fragments() as u64 * 1000 / 300;
                let ss = total_ms / 1000;
                let ms = total_ms % 1000;
                let hh = ss / 3600;
                let mm = (ss % 3600) / 60;
                let sec = ss % 60;
                if ms == 0 {
                    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{sec:02}")
                } else {
                    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{sec:02}.{ms:03}")
                }
            }),
            ColumnData::SmallDateTime(v) => v.map(|dt| {
                let (y, mo, d) = date_from_days_1900(dt.days() as i32);
                // seconds_fragments for SmallDateTime are whole minutes.
                let mins = dt.seconds_fragments() as u32;
                let hh = mins / 60;
                let mm = mins % 60;
                format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:00")
            }),
            ColumnData::Date(v) => v.map(|d| {
                // days since 0001-01-01.
                let (y, mo, day) = date_from_days_year1(d.days());
                format!("{y:04}-{mo:02}-{day:02}")
            }),
            ColumnData::Time(v) => v.map(|t| {
                let ns = t.increments() * 10u64.pow(9 - t.scale() as u32);
                let secs = ns / 1_000_000_000;
                let frac = ns % 1_000_000_000;
                let hh = secs / 3600;
                let mm = (secs % 3600) / 60;
                let ss = secs % 60;
                if frac == 0 {
                    format!("{hh:02}:{mm:02}:{ss:02}")
                } else {
                    format!("{hh:02}:{mm:02}:{ss:02}.{frac:09}")
                }
            }),
            ColumnData::DateTime2(v) => v.map(|dt2| {
                let (y, mo, d) = date_from_days_year1(dt2.date().days());
                let t = dt2.time();
                let ns = t.increments() * 10u64.pow(9 - t.scale() as u32);
                let secs = ns / 1_000_000_000;
                let frac = ns % 1_000_000_000;
                let hh = secs / 3600;
                let mm = (secs % 3600) / 60;
                let ss = secs % 60;
                if frac == 0 {
                    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")
                } else {
                    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{frac:09}")
                }
            }),
            ColumnData::DateTimeOffset(_) => {
                return Err(Error::Unsupported(
                    "SQL Server DateTimeOffset not yet supported".to_owned(),
                ))
            }
        };
        Ok(s)
    }

    /// Convert days since 1900-01-01 (SQL Server DateTime/SmallDateTime baseline)
    /// to (year, month, day).
    fn date_from_days_1900(days: i32) -> (i32, u8, u8) {
        // 1900-01-01 in proleptic Gregorian day count (day 1 = 0001-01-01): 693596.
        let proleptic = days as i64 + 693_596;
        date_from_proleptic(proleptic)
    }

    /// Convert days since 0001-01-01 (tiberius Date baseline) to (year, month, day).
    fn date_from_days_year1(days: u32) -> (i32, u8, u8) {
        date_from_proleptic(days as i64 + 1) // day 0 == 0001-01-01 in tiberius
    }

    /// Convert a 1-based proleptic Gregorian day number (Rata Die: day 1 =
    /// 0001-01-01) to (year, month, day).
    /// Uses the algorithm from https://howardhinnant.github.io/date_algorithms.html,
    /// whose `civil_from_days(z)` expects `z` = days since 1970-01-01 (Unix epoch,
    /// 0-based) — shift our Rata Die input into that frame first (1970-01-01 is
    /// Rata Die 719_163) before applying Hinnant's own +719_468 internal shift.
    fn date_from_proleptic(d: i64) -> (i32, u8, u8) {
        let z = d - 719_163 + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let mo = if mp < 10 { mp + 3 } else { mp - 9 };
        let yr = y + if mo <= 2 { 1 } else { 0 };
        (yr as i32, mo as u8, day as u8)
    }
}

// ─── unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(feature = "sqlserver-backend")]
    use super::real::{marshal_column_data, parse_conn_str};
    #[cfg(feature = "sqlserver-backend")]
    use tiberius::ColumnData;

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_defaults_host_and_port_when_unspecified() {
        let config = parse_conn_str("user id=SA;password=SfTest123!").unwrap();
        assert_eq!(config.get_addr(), "localhost:1433");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_parses_server_host_and_port() {
        let config = parse_conn_str("server=tcp:myhost,5555;user id=SA;password=x").unwrap();
        assert_eq!(config.get_addr(), "myhost:5555");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_defaults_port_when_server_has_no_port() {
        let config = parse_conn_str("server=tcp:myhost;user id=SA;password=x").unwrap();
        assert_eq!(config.get_addr(), "myhost:1433");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_accepts_data_source_as_a_server_alias() {
        let config = parse_conn_str("data source=tcp:otherhost,7777").unwrap();
        assert_eq!(config.get_addr(), "otherhost:7777");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_keys_are_case_insensitive() {
        let config = parse_conn_str("SERVER=tcp:myhost,123;USER ID=sa;PASSWORD=x").unwrap();
        assert_eq!(config.get_addr(), "myhost:123");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_falls_back_to_default_port_on_non_numeric_port() {
        let config = parse_conn_str("server=tcp:myhost,notaport").unwrap();
        assert_eq!(config.get_addr(), "myhost:1433");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_tolerates_blank_segments_and_whitespace() {
        let config = parse_conn_str("  ; server = tcp:myhost,42 ; ; password = x ; ").unwrap();
        assert_eq!(config.get_addr(), "myhost:42");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn parse_conn_str_ignores_unknown_keys() {
        let config = parse_conn_str("server=tcp:myhost,42;some_unknown_key=whatever").unwrap();
        assert_eq!(config.get_addr(), "myhost:42");
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_integers() {
        assert_eq!(
            marshal_column_data(Some(&ColumnData::I32(Some(42)))).unwrap(),
            Some("42".to_owned())
        );
        assert_eq!(
            marshal_column_data(Some(&ColumnData::I32(None))).unwrap(),
            None
        );
        assert_eq!(
            marshal_column_data(Some(&ColumnData::I64(Some(-1)))).unwrap(),
            Some("-1".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_bit() {
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Bit(Some(true)))).unwrap(),
            Some("true".to_owned())
        );
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Bit(Some(false)))).unwrap(),
            Some("false".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_string() {
        assert_eq!(
            marshal_column_data(Some(&ColumnData::String(Some("hello".into())))).unwrap(),
            Some("hello".to_owned())
        );
        assert_eq!(
            marshal_column_data(Some(&ColumnData::String(None))).unwrap(),
            None
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_binary() {
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Binary(Some(vec![0xAB_u8, 0xCD].into()))))
                .unwrap(),
            Some("ABCD".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_numeric() {
        use tiberius::numeric::Numeric;
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Numeric(Some(Numeric::new_with_scale(
                12345, 2
            )))))
            .unwrap(),
            Some("123.45".to_owned())
        );
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Numeric(None))).unwrap(),
            None
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_xml_is_unsupported() {
        let err = marshal_column_data(Some(&ColumnData::Xml(None))).unwrap_err();
        assert!(matches!(err, crate::error::Error::Unsupported(_)));
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_datetimeoffset_is_unsupported() {
        let err = marshal_column_data(Some(&ColumnData::DateTimeOffset(None))).unwrap_err();
        assert!(matches!(err, crate::error::Error::Unsupported(_)));
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_date_at_year_one_epoch() {
        use tiberius::time::Date;
        // tiberius Date day 0 == 0001-01-01 (see date_from_days_year1's own doc).
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Date(Some(Date::new(0))))).unwrap(),
            Some("0001-01-01".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_time_with_whole_seconds() {
        use tiberius::time::Time;
        // scale 0 => 1 increment == 1 second (10^(9-0) ns per increment).
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Time(Some(Time::new(3661, 0))))).unwrap(),
            Some("01:01:01".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_time_with_fractional_seconds() {
        use tiberius::time::Time;
        // scale 7 => 100ns per increment; 15_000_000 * 100ns = 1.5s.
        assert_eq!(
            marshal_column_data(Some(&ColumnData::Time(Some(Time::new(15_000_000, 7))))).unwrap(),
            Some("00:00:01.500000000".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_datetime_at_1900_epoch() {
        use tiberius::time::DateTime;
        // tiberius DateTime day 0 == 1900-01-01 (see date_from_days_1900's own doc).
        assert_eq!(
            marshal_column_data(Some(&ColumnData::DateTime(Some(DateTime::new(0, 0))))).unwrap(),
            Some("1900-01-01T00:00:00".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_small_datetime_rounds_to_the_minute() {
        use tiberius::time::SmallDateTime;
        // seconds_fragments for SmallDateTime are whole minutes: 61 == 1h01m.
        assert_eq!(
            marshal_column_data(Some(&ColumnData::SmallDateTime(Some(SmallDateTime::new(
                0, 61
            )))))
            .unwrap(),
            Some("1900-01-01T01:01:00".to_owned())
        );
    }

    #[cfg(feature = "sqlserver-backend")]
    #[test]
    fn marshal_datetime2_combines_date_and_time() {
        use tiberius::time::{Date, DateTime2, Time};
        assert_eq!(
            marshal_column_data(Some(&ColumnData::DateTime2(Some(DateTime2::new(
                Date::new(0),
                Time::new(3661, 0)
            )))))
            .unwrap(),
            Some("0001-01-01T01:01:01".to_owned())
        );
    }
}
