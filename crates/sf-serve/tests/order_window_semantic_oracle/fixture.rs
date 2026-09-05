use std::fmt::Write;
use std::sync::Arc;

use oxrdf::{Dataset, GraphName, Literal, NamedNode, Quad};
use rusqlite::{params, Connection};
use sf_serve::{Backend, ServeConfig};

pub(crate) const EX: &str = "http://example.test/";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

#[derive(Clone, Copy)]
struct ScalarGroup {
    table: &'static str,
    predicate: &'static str,
    datatype: &'static str,
    values: &'static [&'static str],
}

const GROUPS: &[ScalarGroup] = &[
    ScalarGroup {
        table: "numeric_mixed_integer",
        predicate: "numeric-mixed",
        datatype: "integer",
        values: &["-2", "0", "90071992547409930"],
    },
    ScalarGroup {
        table: "numeric_mixed_decimal",
        predicate: "numeric-mixed",
        datatype: "decimal",
        values: &["-1.5", "0.0001", "8000000000000000.5"],
    },
    ScalarGroup {
        table: "numeric_mixed_float",
        predicate: "numeric-mixed",
        datatype: "float",
        values: &["-1", "1.4E-45", "16777216"],
    },
    ScalarGroup {
        table: "numeric_mixed_double",
        predicate: "numeric-mixed",
        datatype: "double",
        values: &["-0.5", "5E-324", "9.007199254740992E15"],
    },
    ScalarGroup {
        table: "numeric_precision_integer",
        predicate: "numeric-precision",
        datatype: "integer",
        values: &["9007199254740992", "9007199254740993"],
    },
    ScalarGroup {
        table: "numeric_precision_decimal",
        predicate: "numeric-precision",
        datatype: "decimal",
        values: &["9007199254740992.25", "9007199254740992.75"],
    },
    ScalarGroup {
        table: "numeric_float_ties",
        predicate: "numeric-float-ties",
        datatype: "float",
        values: &["-0", "0", "16777216", "16777217", "16777218"],
    },
    ScalarGroup {
        table: "numeric_double_ties",
        predicate: "numeric-double-ties",
        datatype: "double",
        values: &["9007199254740992", "9007199254740993", "9007199254740994"],
    },
    ScalarGroup {
        table: "numeric_infinity",
        predicate: "numeric-infinity",
        datatype: "double",
        values: &["-INF", "-1", "INF"],
    },
    ScalarGroup {
        table: "numeric_nan",
        predicate: "numeric-nan",
        datatype: "double",
        values: &["-1", "NaN", "INF"],
    },
    ScalarGroup {
        table: "boolean_aliases",
        predicate: "boolean-alias",
        datatype: "boolean",
        values: &["false", "0", "true", "1"],
    },
    ScalarGroup {
        table: "datetime_zoned",
        predicate: "datetime-zoned",
        datatype: "dateTime",
        values: &[
            "2019-12-31T23:00:00Z",
            "2020-01-01T00:30:00+01:00",
            "2020-01-01T00:00:00Z",
        ],
    },
    ScalarGroup {
        table: "datetime_local",
        predicate: "datetime-local",
        datatype: "dateTime",
        values: &[
            "2019-12-31T23:00:00",
            "2019-12-31T23:30:00",
            "2020-01-01T00:00:00",
        ],
    },
    ScalarGroup {
        table: "date_zoned",
        predicate: "date-zoned",
        datatype: "date",
        values: &["2019-12-30Z", "2020-01-01+01:00", "2020-01-02Z"],
    },
    ScalarGroup {
        table: "date_local",
        predicate: "date-local",
        datatype: "date",
        values: &["2019-12-30", "2020-01-01", "2020-01-02"],
    },
    ScalarGroup {
        table: "time_zoned",
        predicate: "time-zoned",
        datatype: "time",
        values: &["00:00:00Z", "02:00:00+01:00", "03:00:00Z"],
    },
    ScalarGroup {
        table: "time_local",
        predicate: "time-local",
        datatype: "time",
        values: &["00:00:00", "01:00:00", "03:00:00"],
    },
    ScalarGroup {
        table: "datetime_partial",
        predicate: "datetime-partial",
        datatype: "dateTime",
        values: &["2020-01-01T00:00:00Z", "2020-01-01T00:00:00"],
    },
    ScalarGroup {
        table: "duration_day_time",
        predicate: "duration-day-time",
        datatype: "dayTimeDuration",
        values: &["PT1S", "PT1.5S", "P1D"],
    },
    ScalarGroup {
        table: "duration_year_month",
        predicate: "duration-year-month",
        datatype: "yearMonthDuration",
        values: &["P1M", "P2M", "P1Y"],
    },
    ScalarGroup {
        table: "duration_defined",
        predicate: "duration-defined",
        datatype: "duration",
        values: &["P1D", "P2D", "P3D"],
    },
    ScalarGroup {
        table: "duration_partial",
        predicate: "duration-partial",
        datatype: "duration",
        values: &["P1M", "P30D"],
    },
    ScalarGroup {
        table: "cross_boolean",
        predicate: "cross-domain",
        datatype: "boolean",
        values: &["false"],
    },
    ScalarGroup {
        table: "cross_numeric",
        predicate: "cross-domain",
        datatype: "integer",
        values: &["1"],
    },
    ScalarGroup {
        table: "cross_calendar",
        predicate: "cross-domain",
        datatype: "dateTime",
        values: &["2020-01-01T00:00:00Z"],
    },
    ScalarGroup {
        table: "cross_duration",
        predicate: "cross-domain",
        datatype: "duration",
        values: &["P1D"],
    },
    ScalarGroup {
        table: "cross_string",
        predicate: "cross-domain",
        datatype: "string",
        values: &["text"],
    },
];

pub(crate) struct Fixture {
    pub(crate) config: Arc<ServeConfig>,
    pub(crate) graph: Dataset,
}

pub(crate) fn fixture() -> Fixture {
    let connection = Connection::open_in_memory().expect("open oracle fixture");
    let mut mapping = String::from("@prefix rr: <http://www.w3.org/ns/r2rml#> .\n");
    let mut quads = Vec::new();

    for group in GROUPS {
        add_scalar_group(&connection, &mut mapping, &mut quads, *group);
    }
    add_optional_group(&connection, &mut mapping, &mut quads);
    add_multiple_key_group(&connection, &mut mapping, &mut quads);

    Fixture {
        config: Arc::new(crate::support::serve_config(
            Backend::sqlite(connection),
            &mapping,
        )),
        graph: Dataset::from_iter(quads),
    }
}

fn add_scalar_group(
    connection: &Connection,
    mapping: &mut String,
    quads: &mut Vec<Quad>,
    group: ScalarGroup,
) {
    connection
        .execute_batch(&format!(
            "CREATE TABLE \"{}\" (id TEXT PRIMARY KEY, value TEXT NOT NULL)",
            group.table
        ))
        .expect("create scalar group");
    let sql = format!(
        "INSERT INTO \"{}\" (id, value) VALUES (?1, ?2)",
        group.table
    );
    for (offset, value) in group.values.iter().enumerate() {
        let id = format!("{:02}", offset + 1);
        connection
            .execute(&sql, params![id, value])
            .expect("insert scalar value");
        quads.push(typed_quad(
            group.table,
            &id,
            group.predicate,
            value,
            group.datatype,
        ));
    }
    writeln!(
        mapping,
        "<#map-{table}> a rr:TriplesMap ;\n  rr:logicalTable [ rr:sqlQuery \"SELECT id, value FROM {table} ORDER BY id\" ] ;\n  rr:subjectMap [ rr:template \"{EX}{table}/{{id}}\" ] ;\n  rr:predicateObjectMap [ rr:predicate <{EX}{predicate}> ; rr:objectMap [ rr:column \"value\" ; rr:datatype <{XSD}{datatype}> ] ] .",
        table = group.table,
        predicate = group.predicate,
        datatype = group.datatype,
    )
    .expect("write scalar mapping");
}

fn add_optional_group(connection: &Connection, mapping: &mut String, quads: &mut Vec<Quad>) {
    connection
        .execute_batch(
            "CREATE TABLE optional_values (id TEXT PRIMARY KEY, marker TEXT NOT NULL, value TEXT);\n\
             INSERT INTO optional_values VALUES\n\
               ('01', 'row', NULL), ('02', 'row', '2'),\n\
               ('03', 'row', NULL), ('04', 'row', '1');",
        )
        .expect("create optional group");
    mapping.push_str(
        "<#map-optional> a rr:TriplesMap ;\n\
           rr:logicalTable [ rr:sqlQuery \"SELECT id, marker, value FROM optional_values ORDER BY id\" ] ;\n\
           rr:subjectMap [ rr:template \"http://example.test/optional_values/{id}\" ] ;\n\
           rr:predicateObjectMap [ rr:predicate <http://example.test/row> ; rr:objectMap [ rr:column \"marker\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string> ] ] ;\n\
           rr:predicateObjectMap [ rr:predicate <http://example.test/optional> ; rr:objectMap [ rr:column \"value\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#integer> ] ] .\n",
    );
    for (id, value) in [
        ("01", None),
        ("02", Some("2")),
        ("03", None),
        ("04", Some("1")),
    ] {
        quads.push(simple_quad("optional_values", id, "row", "row"));
        if let Some(value) = value {
            quads.push(typed_quad(
                "optional_values",
                id,
                "optional",
                value,
                "integer",
            ));
        }
    }
}

fn add_multiple_key_group(connection: &Connection, mapping: &mut String, quads: &mut Vec<Quad>) {
    connection
        .execute_batch(
            "CREATE TABLE multiple_keys (id TEXT PRIMARY KEY, first_key TEXT NOT NULL, second_key TEXT NOT NULL);\n\
             INSERT INTO multiple_keys VALUES\n\
               ('01', '1', '2'), ('02', '1', '1'), ('03', '2', '2'),\n\
               ('04', '2', '1'), ('05', '2', '1');",
        )
        .expect("create multiple-key group");
    mapping.push_str(
        "<#map-multiple> a rr:TriplesMap ;\n\
           rr:logicalTable [ rr:sqlQuery \"SELECT id, first_key, second_key FROM multiple_keys ORDER BY id\" ] ;\n\
           rr:subjectMap [ rr:template \"http://example.test/multiple_keys/{id}\" ] ;\n\
           rr:predicateObjectMap [ rr:predicate <http://example.test/first> ; rr:objectMap [ rr:column \"first_key\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#integer> ] ] ;\n\
           rr:predicateObjectMap [ rr:predicate <http://example.test/second> ; rr:objectMap [ rr:column \"second_key\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#integer> ] ] .\n",
    );
    for (id, first, second) in [
        ("01", "1", "2"),
        ("02", "1", "1"),
        ("03", "2", "2"),
        ("04", "2", "1"),
        ("05", "2", "1"),
    ] {
        quads.push(typed_quad("multiple_keys", id, "first", first, "integer"));
        quads.push(typed_quad("multiple_keys", id, "second", second, "integer"));
    }
}

fn typed_quad(table: &str, id: &str, predicate: &str, value: &str, datatype: &str) -> Quad {
    Quad::new(
        subject(table, id),
        NamedNode::new_unchecked(format!("{EX}{predicate}")),
        Literal::new_typed_literal(value, NamedNode::new_unchecked(format!("{XSD}{datatype}"))),
        GraphName::DefaultGraph,
    )
}

fn simple_quad(table: &str, id: &str, predicate: &str, value: &str) -> Quad {
    Quad::new(
        subject(table, id),
        NamedNode::new_unchecked(format!("{EX}{predicate}")),
        Literal::new_simple_literal(value),
        GraphName::DefaultGraph,
    )
}

fn subject(table: &str, id: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{EX}{table}/{id}"))
}
