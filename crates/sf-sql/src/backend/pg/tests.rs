use super::timetz::decode_pg_timetz;
use super::*;
use crate::source_work::SourceWork;
use tokio_postgres::types::FromSql;
use tokio_postgres::{Client, NoTls};

#[derive(Debug)]
struct AlwaysMarshal;

impl<'a> FromSql<'a> for AlwaysMarshal {
    fn from_sql(
        _ty: &Type,
        _raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Err(Box::new(Error::Marshal(
            "synthetic decoder failure".to_owned(),
        )))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, Type::INT4)
    }
}

fn configured_conn() -> Option<String> {
    match std::env::var("SF_PG_URL") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("SF_PG_URL must be valid Unicode when configured")
        }
    }
}

async fn disposable_database(prefix: &str) -> Option<(Client, Client, String)> {
    let Some(base_conn) = configured_conn() else {
        eprintln!("skipping live PostgreSQL test: SF_PG_URL is not configured");
        return None;
    };
    let conn_str = format!("{base_conn} dbname=postgres");
    let (admin, connection) = tokio_postgres::connect(&conn_str, NoTls)
        .await
        .expect("SF_PG_URL is configured but its PostgreSQL server is unreachable");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let db = format!("{prefix}_{}", std::process::id());
    admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .await
        .expect("reset disposable test database");
    admin
        .batch_execute(&format!("CREATE DATABASE {db}"))
        .await
        .expect("create test database");

    let (client, connection) = tokio_postgres::connect(&format!("{base_conn} dbname={db}"), NoTls)
        .await
        .expect("connect to test database");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Some((admin, client, db))
}

async fn drop_database(admin: &Client, client: Client, db: &str) {
    drop(client);
    admin
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .await
        .expect("drop disposable test database");
}

#[test]
fn pg_value_reads_date_time_timestamp_columns() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let Some((admin, client, db)) = disposable_database("sf_sql_pgdt_test").await else {
            return;
        };
        client
            .batch_execute(
                "CREATE TABLE t (
                    d DATE, tm TIME, tmz TIMETZ, ts TIMESTAMP, tstz TIMESTAMPTZ
                 );
                 INSERT INTO t VALUES (
                    '2024-03-15', '13:45:30', '13:45:30.1234+05:30',
                    '2024-03-15 13:45:30', '2024-03-15 13:45:30+00'
                 );
                 INSERT INTO t VALUES (NULL, NULL, NULL, NULL, NULL);
                 CREATE TABLE tz_cases (id INTEGER PRIMARY KEY, tmz TIMETZ);
                 INSERT INTO tz_cases VALUES
                    (1, '13:45:30.1234+05:30'),
                    (2, '23:00:00-03:00'),
                    (3, '00:15:00+05:30'),
                    (4, '24:00:00+14:00');
                 CREATE TABLE tz_unsupported (id INTEGER PRIMARY KEY, tmz TIMETZ);
                 INSERT INTO tz_unsupported VALUES
                    (1, '00:00:00+00:00:01'),
                    (2, '00:00:00+14:01'),
                    (3, '00:00:00+15:59:59');",
            )
            .await
            .expect("seed table");

        let mut backend = PgBackend::new(&client);
        let mut stream = backend
            .open_branch(
                "SELECT d, tm, tmz, ts, tstz FROM t ORDER BY d NULLS LAST",
                &[],
            )
            .await
            .expect("open branch");
        let row = stream.next_row().await.unwrap().expect("first row");
        assert_eq!(
            row.codes,
            vec![
                Some(XsdTypeCode::Date),
                Some(XsdTypeCode::Time),
                Some(XsdTypeCode::Time),
                Some(XsdTypeCode::DateTime),
                Some(XsdTypeCode::DateTime),
            ]
        );
        assert_eq!(row.values[0].as_deref(), Some("2024-03-15"));
        assert_eq!(row.values[1].as_deref(), Some("13:45:30"));
        assert_eq!(row.values[2].as_deref(), Some("13:45:30.1234+05:30"));
        assert_eq!(row.values[3].as_deref(), Some("2024-03-15 13:45:30"));
        assert!(row.values[4]
            .as_deref()
            .unwrap()
            .starts_with("2024-03-15T13:45:30"));
        assert_eq!(
            stream.next_row().await.unwrap().unwrap().values,
            vec![None, None, None, None, None]
        );
        drop(stream);

        let mut backend = PgBackend::new(&client);
        let mut stream = backend
            .open_branch("SELECT tmz FROM tz_cases ORDER BY id", &[])
            .await
            .expect("open TIMETZ cases");
        for expected in [
            "13:45:30.1234+05:30",
            "23:00:00-03:00",
            "00:15:00+05:30",
            "00:00:00+14:00",
        ] {
            let row = stream.next_row().await.unwrap().expect("TIMETZ row");
            assert_eq!(row.codes, vec![Some(XsdTypeCode::Time)]);
            assert_eq!(row.values[0].as_deref(), Some(expected));
        }
        assert!(stream.next_row().await.unwrap().is_none());
        drop(stream);

        for id in 1..=3 {
            let mut backend = PgBackend::new(&client);
            let mut stream = backend
                .open_branch(
                    &format!("SELECT tmz FROM tz_unsupported WHERE id = {id}"),
                    &[],
                )
                .await
                .expect("open unsupported TIMETZ case");
            assert!(
                matches!(stream.next_row().await, Err(Error::Unsupported(_))),
                "valid PostgreSQL TIMETZ case {id} outside the XSD domain must fail closed"
            );
        }
        drop_database(&admin, client, &db).await;
    });
}

#[test]
fn pg_value_numeric_nan_and_infinity_surface_as_unsupported() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let Some((admin, client, db)) = disposable_database("sf_sql_pgnan_test").await else {
            return;
        };
        client
            .batch_execute(
                "CREATE TABLE t (id INTEGER, p NUMERIC);
                 INSERT INTO t VALUES (1, 'NaN'), (2, 'Infinity'), (3, '-Infinity');",
            )
            .await
            .expect("seed table");

        let decoder_row = client
            .query_one("SELECT 1::int4", &[])
            .await
            .expect("obtain decoder test row");
        let wrapped = decoder_row
            .try_get::<_, AlwaysMarshal>(0)
            .expect_err("synthetic decoder must fail");
        assert!(matches!(
            recover_pg_from_sql_error(wrapped),
            Error::Marshal(message) if message == "synthetic decoder failure"
        ));

        for (id, class) in [(1, "NaN"), (2, "+Infinity"), (3, "-Infinity")] {
            let mut backend = PgBackend::new(&client);
            let mut stream = backend
                .open_branch(&format!("SELECT p FROM t WHERE id = {id}"), &[])
                .await
                .expect("open branch");
            match stream.next_row().await {
                Err(Error::Unsupported(_)) => {}
                Err(error) => panic!("PG NUMERIC {class} must be unsupported, got {error:?}"),
                Ok(_) => panic!("PG NUMERIC {class} must not decode"),
            }
        }
        drop_database(&admin, client, &db).await;
    });
}

fn numeric_wire(ndigits: u16, weight: i16, sign: u16, dscale: u16, digits: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + digits.len() * 2);
    bytes.extend_from_slice(&ndigits.to_be_bytes());
    bytes.extend_from_slice(&weight.to_be_bytes());
    bytes.extend_from_slice(&sign.to_be_bytes());
    bytes.extend_from_slice(&dscale.to_be_bytes());
    for digit in digits {
        bytes.extend_from_slice(&digit.to_be_bytes());
    }
    bytes
}

#[test]
fn decode_pg_numeric_unsigned_digit_count_preserves_large_finite_values() {
    let digits = vec![1111; 32768];
    let wire = numeric_wire(32768, 32767, 0x0000, 2, &digits);
    let expected = format!("{}.00", "1111".repeat(32768));
    assert_eq!(
        decode::decode_numeric(&wire, SourceWork::new(None)).unwrap(),
        expected
    );
    assert!(matches!(
        decode::decode_numeric(&wire[..wire.len() - 2], SourceWork::new(None)),
        Err(Error::Marshal(message)) if message.contains("digit array truncated")
    ));
}

#[test]
fn decode_pg_numeric_zero() {
    assert_eq!(
        decode::decode_numeric(&numeric_wire(0, 0, 0x0000, 0, &[]), SourceWork::new(None)).unwrap(),
        "0"
    );
}

#[test]
fn decode_pg_numeric_one() {
    assert_eq!(
        decode::decode_numeric(&numeric_wire(1, 0, 0x0000, 0, &[1]), SourceWork::new(None))
            .unwrap(),
        "1"
    );
}

#[test]
fn decode_pg_numeric_negative_one() {
    assert_eq!(
        decode::decode_numeric(&numeric_wire(1, 0, 0x4000, 0, &[1]), SourceWork::new(None))
            .unwrap(),
        "-1"
    );
}

#[test]
fn decode_pg_numeric_12345_678() {
    assert_eq!(
        decode::decode_numeric(
            &numeric_wire(3, 1, 0x0000, 3, &[1, 2345, 6780]),
            SourceWork::new(None)
        )
        .unwrap(),
        "12345.678"
    );
}

#[test]
fn decode_pg_numeric_0_0001() {
    assert_eq!(
        decode::decode_numeric(&numeric_wire(1, -1, 0x0000, 4, &[1]), SourceWork::new(None))
            .unwrap(),
        "0.0001"
    );
}

#[test]
fn decode_pg_numeric_weight_exceeds_stored_digits_trailing_zeros() {
    assert_eq!(
        decode::decode_numeric(&numeric_wire(1, 2, 0x0000, 0, &[1]), SourceWork::new(None))
            .unwrap(),
        "100000000"
    );
}

#[test]
fn decode_pg_numeric_nan_is_unsupported_not_a_wrong_value() {
    assert!(matches!(
        decode::decode_numeric(&numeric_wire(0, 0, 0xC000, 0, &[]), SourceWork::new(None))
            .unwrap_err(),
        Error::Unsupported(_)
    ));
}

#[test]
fn decode_pg_numeric_positive_infinity_is_unsupported() {
    assert!(matches!(
        decode::decode_numeric(&numeric_wire(0, 0, 0xD000, 0, &[]), SourceWork::new(None))
            .unwrap_err(),
        Error::Unsupported(_)
    ));
}

#[test]
fn decode_pg_numeric_negative_infinity_is_unsupported() {
    assert!(matches!(
        decode::decode_numeric(&numeric_wire(0, 0, 0xF000, 0, &[]), SourceWork::new(None))
            .unwrap_err(),
        Error::Unsupported(_)
    ));
}

fn timetz_wire(micros: i64, seconds_west: i32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(12);
    bytes.extend_from_slice(&micros.to_be_bytes());
    bytes.extend_from_slice(&seconds_west.to_be_bytes());
    bytes
}

#[test]
fn timetz_is_admitted_as_xsd_time() {
    assert_eq!(pg_xsd_code(&Type::TIMETZ), Some(XsdTypeCode::Time));
}

#[test]
fn decode_pg_timetz_preserves_offset_sign_local_time_and_fraction() {
    let micros = 49_530_123_400;
    assert_eq!(
        decode_pg_timetz(&timetz_wire(micros, -19_800), SourceWork::new(None)).unwrap(),
        "13:45:30.1234+05:30"
    );
    assert_eq!(
        decode_pg_timetz(&timetz_wire(micros, 14_400), SourceWork::new(None)).unwrap(),
        "13:45:30.1234-04:00"
    );
}

#[test]
fn decode_pg_timetz_emits_canonical_zero_offset_and_midnight() {
    assert_eq!(
        decode_pg_timetz(&timetz_wire(0, 0), SourceWork::new(None)).unwrap(),
        "00:00:00Z"
    );
    assert_eq!(
        decode_pg_timetz(&timetz_wire(86_400_000_000, 0), SourceWork::new(None)).unwrap(),
        "00:00:00Z"
    );
    assert_eq!(
        decode_pg_timetz(&timetz_wire(86_400_000_000, -19_800), SourceWork::new(None)).unwrap(),
        "00:00:00+05:30"
    );
}

#[test]
fn decode_pg_timetz_accepts_exact_xsd_offset_boundaries() {
    assert_eq!(
        decode_pg_timetz(&timetz_wire(0, -50_400), SourceWork::new(None)).unwrap(),
        "00:00:00+14:00"
    );
    assert_eq!(
        decode_pg_timetz(&timetz_wire(0, 50_400), SourceWork::new(None)).unwrap(),
        "00:00:00-14:00"
    );
}

#[test]
fn decode_pg_timetz_does_not_shift_across_the_arbitrary_day_boundary() {
    assert_eq!(
        decode_pg_timetz(&timetz_wire(82_800_000_000, 10_800), SourceWork::new(None)).unwrap(),
        "23:00:00-03:00"
    );
}

#[test]
fn decode_pg_timetz_rejects_valid_postgres_offsets_outside_xsd() {
    for seconds_west in [1, -1, 50_460, -50_460, 57_599, -57_599] {
        assert!(matches!(
            decode_pg_timetz(&timetz_wire(0, seconds_west), SourceWork::new(None)).unwrap_err(),
            Error::Unsupported(_)
        ));
    }
}

#[test]
fn decode_pg_timetz_rejects_malformed_wire_values() {
    assert!(matches!(
        decode_pg_timetz(&[0; 11], SourceWork::new(None)).unwrap_err(),
        Error::Marshal(_)
    ));
    assert!(matches!(
        decode_pg_timetz(&timetz_wire(-1, 0), SourceWork::new(None)).unwrap_err(),
        Error::Marshal(_)
    ));
    assert!(matches!(
        decode_pg_timetz(&timetz_wire(86_400_000_001, 0), SourceWork::new(None)).unwrap_err(),
        Error::Marshal(_)
    ));
    for seconds_west in [57_600, -57_600] {
        assert!(matches!(
            decode_pg_timetz(&timetz_wire(0, seconds_west), SourceWork::new(None)).unwrap_err(),
            Error::Marshal(_)
        ));
    }
}
