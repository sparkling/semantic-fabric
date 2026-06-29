//! MySQL integration tests (WS-D, ADR-0006): run the emitted MySQL SQL over a
//! live MySQL server via `exec_mysql`, comparing results to the expected values.
//!
//! **Skips gracefully** when no MySQL server is reachable — controlled by the
//! `SF_MYSQL_URL` environment variable (or the default `mysql://root:sftest@127.0.0.1:13306/sftest`).
//! Set this to target a different server.
//!
//! Requires the `sf-mysql-test` Docker container (or any MySQL 8.0+ instance) to
//! be running. The tests create and drop isolated tables so they are self-cleaning.

use sf_core::ir::{
    LogicalSource, ObjectMap, PredicateObjectMap, SubjectMap, Template, TermMap, TermSpec,
    TriplesMap,
};
use sf_core::NamedNode;
use sf_sparql::{exec_mysql, parse_and_translate};
use sf_sql::Dialect;

const EMP_NAME: &str = "http://ex/empName";
const EMP_DEPT: &str = "http://ex/empDept";
const DEPT_NAME: &str = "http://ex/deptName";

fn iri(s: &str) -> NamedNode {
    NamedNode::new_unchecked(s)
}

fn template_iri(t: &str) -> TermMap {
    TermMap::Template(Template::parse(t).unwrap(), TermSpec::iri())
}

fn column_literal(c: &str) -> TermMap {
    TermMap::Column(c.into(), TermSpec::plain_literal())
}

fn pom(predicate: &str, object: TermMap) -> PredicateObjectMap {
    PredicateObjectMap {
        predicates: vec![TermMap::Constant(iri(predicate).into())],
        objects: vec![ObjectMap::Term(object)],
        graphs: vec![],
    }
}

/// The default MySQL connection URL.  Override with `SF_MYSQL_URL`.
fn mysql_url() -> String {
    std::env::var("SF_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:sftest@127.0.0.1:13306/sftest".to_owned())
}

/// Try to open a MySQL connection; return `None` to skip if the server is unreachable.
async fn try_connect() -> Option<mysql_async::Conn> {
    let opts = mysql_async::Opts::from_url(&mysql_url()).ok()?;
    mysql_async::Conn::new(opts).await.ok()
}

/// The R2RML mapping for the MySQL integration test:
///   emp(id, name, dept_id) and dept(id, dname) → EMP_NAME, EMP_DEPT, DEPT_NAME.
fn mapping() -> Vec<TriplesMap> {
    let emp = TriplesMap {
        id: "EMP".to_owned(),
        source: LogicalSource::Table("sf_mysql_emp".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/emp/{id}"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![
            pom(EMP_NAME, column_literal("name")),
            pom(EMP_DEPT, template_iri("http://ex/dept/{dept_id}")),
        ],
    };
    let dept = TriplesMap {
        id: "DEPT".to_owned(),
        source: LogicalSource::Table("sf_mysql_dept".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/dept/{id}"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![pom(DEPT_NAME, column_literal("dname"))],
    };
    vec![emp, dept]
}

/// Create the test tables and load fixture rows. Uses `INSERT IGNORE` to make
/// parallel test runs idempotent (the schema and data are fixed across tests).
async fn setup_tables(conn: &mut mysql_async::Conn) {
    use mysql_async::prelude::Queryable;
    conn.query_drop(
        "CREATE TABLE IF NOT EXISTS sf_mysql_emp \
         (id INT PRIMARY KEY, name VARCHAR(80), dept_id INT NOT NULL)",
    )
    .await
    .unwrap();
    conn.query_drop(
        "CREATE TABLE IF NOT EXISTS sf_mysql_dept (id INT PRIMARY KEY, dname VARCHAR(80))",
    )
    .await
    .unwrap();
    conn.query_drop("INSERT IGNORE INTO sf_mysql_dept VALUES (10,'R&D'),(20,'Ops')")
        .await
        .unwrap();
    conn.query_drop("INSERT IGNORE INTO sf_mysql_emp VALUES (1,'Ada',10),(2,'Grace',20)")
        .await
        .unwrap();
}

#[tokio::test]
async fn mysql_select_emp_names() {
    let Some(mut conn) = try_connect().await else {
        eprintln!(
            "SKIP mysql_select_emp_names: no MySQL at {} — set SF_MYSQL_URL to run",
            mysql_url()
        );
        return;
    };
    setup_tables(&mut conn).await;
    let maps = mapping();
    let q = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n }}");
    let plan = parse_and_translate(&q, &maps, Dialect::MySql).unwrap();
    let result = exec_mysql::select_mysql(&plan, &mut conn).await.unwrap();
    let mut got: Vec<String> = result
        .rows
        .iter()
        .filter_map(|r| r[0].as_ref())
        .map(|t| t.to_string())
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["\"Ada\"".to_owned(), "\"Grace\"".to_owned()],
        "MySQL SELECT must return both employee names"
    );
}

#[tokio::test]
async fn mysql_select_cross_table_join() {
    let Some(mut conn) = try_connect().await else {
        eprintln!(
            "SKIP mysql_select_cross_table_join: no MySQL at {} — set SF_MYSQL_URL to run",
            mysql_url()
        );
        return;
    };
    setup_tables(&mut conn).await;
    let maps = mapping();
    let q = format!(
        "SELECT ?n ?dn WHERE {{ ?e <{EMP_NAME}> ?n . ?e <{EMP_DEPT}> ?d . ?d <{DEPT_NAME}> ?dn }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::MySql).unwrap();
    let result = exec_mysql::select_mysql(&plan, &mut conn).await.unwrap();
    assert_eq!(result.rows.len(), 2, "cross-table join must return 2 rows");
    // Verify Ada is in R&D and Grace is in Ops.
    let pairs: Vec<(String, String)> = result
        .rows
        .iter()
        .map(|r| {
            let n = r[0].as_ref().map(|t| t.to_string()).unwrap_or_default();
            let dn = r[1].as_ref().map(|t| t.to_string()).unwrap_or_default();
            (n, dn)
        })
        .collect();
    assert!(
        pairs.contains(&("\"Ada\"".to_owned(), "\"R&D\"".to_owned())),
        "Ada must be in R&D; got: {pairs:?}"
    );
    assert!(
        pairs.contains(&("\"Grace\"".to_owned(), "\"Ops\"".to_owned())),
        "Grace must be in Ops; got: {pairs:?}"
    );
}

#[tokio::test]
async fn mysql_construct_triples() {
    let Some(mut conn) = try_connect().await else {
        eprintln!(
            "SKIP mysql_construct_triples: no MySQL at {} — set SF_MYSQL_URL to run",
            mysql_url()
        );
        return;
    };
    setup_tables(&mut conn).await;
    let maps = mapping();
    let q = format!("CONSTRUCT {{ ?e <{EMP_NAME}> ?n }} WHERE {{ ?e <{EMP_NAME}> ?n }}");
    let plan = parse_and_translate(&q, &maps, Dialect::MySql).unwrap();
    let triples = exec_mysql::construct_triples_mysql(&plan, &mut conn)
        .await
        .unwrap();
    assert_eq!(
        triples.len(),
        2,
        "CONSTRUCT must produce 2 emp:name triples"
    );
}
