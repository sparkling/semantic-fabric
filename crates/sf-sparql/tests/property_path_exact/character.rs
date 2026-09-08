use super::*;
use std::collections::BTreeSet;

fn pairs(conn: &Connection, pattern: &str) -> (usize, BTreeSet<(String, String)>) {
    let plan = parse_and_translate(
        &format!("SELECT ?s ?o WHERE {{ ?s {pattern} ?o }}"),
        &edge_mapping(),
        Dialect::Sqlite,
    )
    .unwrap();
    let rows = exec::select(&plan, conn).unwrap().rows;
    (
        rows.len(),
        rows.iter()
            .map(|row| {
                (
                    row[0]
                        .as_ref()
                        .unwrap_or_else(|| panic!("unbound subject {pattern}: {row:?}"))
                        .to_string(),
                    row[1]
                        .as_ref()
                        .unwrap_or_else(|| panic!("unbound object {pattern}: {row:?}"))
                        .to_string(),
                )
            })
            .collect(),
    )
}

#[test]
fn character_paths_match_decoded_graph_for_widths_unicode_nul_and_mixed_text() {
    for (subject, object) in [
        ("CHARACTER(4)", "CHARACTER(2)"),
        ("CHARACTER(2)", "CHARACTER(4)"),
        ("CHARACTER(4)", "VARCHAR(9)"),
        ("VARCHAR(9)", "CHARACTER(4)"),
        ("CHARACTER(4)", "CHARACTER(4)"),
    ] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE edge(parent {subject}, child {object});"
        ))
        .unwrap();
        for (s, o) in [
            ("a", "b"),
            ("a ", "b "),
            ("b", "c"),
            ("b   ", "z"),
            ("é", "🐈"),
            ("🐈", "x\0y"),
            ("x\0y", ""),
            ("    ", "overlong"),
        ] {
            conn.execute("INSERT INTO edge VALUES(?,?)", params![s, o])
                .unwrap();
        }
        let (_, direct) = pairs(&conn, &format!("<{REACHES}>"));
        let mut closure = direct.clone();
        loop {
            let next: Vec<_> = closure
                .iter()
                .flat_map(|(s, m)| {
                    direct
                        .iter()
                        .filter_map(move |(n, o)| (m == n).then_some((s.clone(), o.clone())))
                })
                .collect();
            let size = closure.len();
            closure.extend(next);
            if closure.len() == size {
                break;
            }
        }
        for op in ["+", "*", "?", "|<http://ex/reaches>"] {
            let mut expected = if matches!(op, "+" | "*") {
                closure.clone()
            } else {
                direct.clone()
            };
            if matches!(op, "*" | "?") {
                expected.extend(
                    direct
                        .iter()
                        .flat_map(|(s, o)| [(s.clone(), s.clone()), (o.clone(), o.clone())]),
                );
            }
            let (count, actual) = pairs(&conn, &format!("(<{REACHES}>{op})"));
            assert_eq!(actual, expected, "{subject}/{object} {op}");
            assert_eq!(
                count,
                expected.len(),
                "decoded duplicate {subject}/{object} {op}"
            );
        }
    }
}
