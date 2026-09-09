use super::*;

#[test]
fn sqlite_ucschar_boundaries_match_core_with_nul_and_adjacent_scalars() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t(v TEXT)").unwrap();
    let query = format!(
        "SELECT {} FROM t",
        percent_encode_col("t.v", Dialect::Sqlite).unwrap()
    );
    let mut points = vec![
        0, 127, 128, 159, 160, 0xD7FF, 0xE000, 0xF8FF, 0xF900, 0xFDCF, 0xFDD0, 0xFDEF, 0xFDF0,
        0xFFEF, 0xFFF0, 0xFFFD, 0xFFFE, 0xFFFF, 0x10000, 0xE0000, 0xE0FFF, 0xE1000, 0xF0000,
        0x10FFFF,
    ];
    for plane in 1..=14 {
        points.extend([
            (plane << 16) + 0xFFFD,
            (plane << 16) + 0xFFFE,
            (plane << 16) + 0xFFFF,
        ]);
    }
    for point in points {
        let ch = char::from_u32(point).unwrap();
        for value in [
            ch.to_string(),
            format!("\u{e000}{ch}你好\0{ch}a\u{80}"),
            format!("{ch}{ch}"),
        ] {
            conn.execute("DELETE FROM t", []).unwrap();
            conn.execute("INSERT INTO t VALUES (?1)", [&value]).unwrap();
            let got: String = conn.query_row(&query, [], |row| row.get(0)).unwrap();
            assert_eq!(got, tests::reference_encode(&value), "{value:?}");
        }
    }
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        let query = format!(
            "SELECT {} FROM t",
            percent_encode_col("t.v", dialect).unwrap()
        );
        dialect.emit_via_ast(&query).unwrap();
    }
}

#[test]
fn sqlite_invalid_utf8_is_not_silently_repaired_to_a_valid_iri() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t(v TEXT)").unwrap();
    let query = format!(
        "SELECT {} FROM t",
        percent_encode_col("t.v", Dialect::Sqlite).unwrap()
    );
    for bad in [
        vec![0x80],
        vec![0xC2],
        vec![0xC0, 0x80],
        vec![0xE1, b'A', 0x80],
        vec![0xED, 0xA0, 0x80],
        vec![0xF4, 0x90, 0x80, 0x80],
        vec![0xE0, 0x80, 0x80],
        vec![b'a', 0, 0x80],
    ] {
        conn.execute("DELETE FROM t", []).unwrap();
        conn.execute("INSERT INTO t VALUES (CAST(?1 AS TEXT))", [&bad])
            .unwrap();
        assert!(
            conn.query_row::<String, _, _>(&query, [], |row| row.get(0))
                .is_err(),
            "{bad:?}"
        );
    }
}
