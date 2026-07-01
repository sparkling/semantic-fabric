//! The GTFS-Madrid-Bench OBDA workload (ADR-0005): a faithful, self-contained
//! subset of the official benchmark — six core GTFS tables, an R2RML mapping over
//! the GTFS vocabulary, and the representative OBDA queries the virtualizer
//! supports (BGP / JOIN / FILTER / OPTIONAL across tables).
//!
//! ## Provenance
//!
//! Derived from the **official GTFS-Madrid-Bench** (oeg-upm/gtfs-bench,
//! commit `7fcdaa7`, Apache-2.0): the table/column schema mirrors
//! `utils/postgresql.sql` and the mapping mirrors `mappings/gtfs-rdb.r2rml.ttl`
//! (GTFS vocab `http://vocab.gtfs.org/terms#`, subject IRIs under
//! `http://transport.linkeddata.es/madrid/metro/`). The official artifacts are
//! vendored verbatim under `vendor/gtfs-madrid-bench/` for reference; this module
//! drives the engine with a **self-contained, cross-reference-consistent subset**
//! (six tables, every `rr:parentTriplesMap` resolvable) so every query is valid
//! at any scale. Full provenance: `crates/sf-bench/README.md`.
//!
//! ## Scale (ADR-0006)
//!
//! The dataset is emitted into a **file-backed** SQLite database (a temp file) at
//! a chosen scale factor, so engine memory is separable from the source data:
//! an in-memory SQLite would hold the rows in-process and confound the
//! constant-memory measurement. `STOP_TIMES` — the dominant table — grows
//! linearly with the scale factor, so result-row counts grow ~linearly while the
//! engine's streaming working set stays bounded.
//!
//! ## PostgreSQL fixture (ADR-0023 shootout)
//!
//! [`generate_pg`] populates the same six tables (identical column names and data)
//! in a live PostgreSQL database. Table names are **double-quoted uppercase**
//! (`"AGENCY"`, `"ROUTES"`, …) so they match the R2RML mapping's
//! `rr:tableName "AGENCY"` / `rr:tableName "ROUTES"` exactly — PostgreSQL folds
//! unquoted identifiers to lowercase, so the bench uses quoted DDL + quoted
//! `DROP TABLE IF EXISTS "AGENCY" CASCADE` cleanup. The caller owns the scratch
//! database and must clean up after the bench run.

use std::path::Path;

use rusqlite::Connection;

/// The derived GTFS R2RML mapping (Turtle), parsed once by `sf-mapping` into the
/// `sf-core` IR. Types use `rr:class`; cross-table links use `rr:RefObjectMap`
/// with a join condition (the OBDA join path, ADR-0007).
pub const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix gtfs: <http://vocab.gtfs.org/terms#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix geo: <http://www.w3.org/2003/01/geo/wgs84_pos#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<#agency_0> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "AGENCY" ] ;
    rr:subjectMap [
        rr:template "http://transport.linkeddata.es/madrid/metro/agency/{agency_id}" ;
        rr:class gtfs:Agency
    ] ;
    rr:predicateObjectMap [ rr:predicate foaf:name ; rr:objectMap [ rr:column "agency_name" ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:timezone ; rr:objectMap [ rr:column "agency_timezone" ] ] ;
    rr:predicateObjectMap [ rr:predicate foaf:page ;
        rr:objectMap [ rr:column "agency_url" ; rr:termType rr:IRI ] ] .

<#calendar_0> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "CALENDAR" ] ;
    rr:subjectMap [
        rr:template "http://transport.linkeddata.es/madrid/metro/services/{service_id}" ;
        rr:class gtfs:Service
    ] ;
    rr:predicateObjectMap [ rr:predicate dct:date ; rr:objectMap [ rr:column "start_date" ] ] .

<#routes_0> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "ROUTES" ] ;
    rr:subjectMap [
        rr:template "http://transport.linkeddata.es/madrid/metro/routes/{route_id}" ;
        rr:class gtfs:Route
    ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:shortName ; rr:objectMap [ rr:column "route_short_name" ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:longName ; rr:objectMap [ rr:column "route_long_name" ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:routeType ;
        rr:objectMap [ rr:template "http://transport.linkeddata.es/resource/RouteType/{route_type}" ; rr:termType rr:IRI ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:agency ;
        rr:objectMap [ rr:parentTriplesMap <#agency_0> ;
            rr:joinCondition [ rr:child "agency_id" ; rr:parent "agency_id" ] ] ] .

<#trips_0> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "TRIPS" ] ;
    rr:subjectMap [
        rr:template "http://transport.linkeddata.es/madrid/metro/trips/{trip_id}" ;
        rr:class gtfs:Trip
    ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:headsign ; rr:objectMap [ rr:column "trip_headsign" ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:direction ;
        rr:objectMap [ rr:column "direction_id" ; rr:datatype xsd:integer ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:route ;
        rr:objectMap [ rr:parentTriplesMap <#routes_0> ;
            rr:joinCondition [ rr:child "route_id" ; rr:parent "route_id" ] ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:service ;
        rr:objectMap [ rr:parentTriplesMap <#calendar_0> ;
            rr:joinCondition [ rr:child "service_id" ; rr:parent "service_id" ] ] ] .

<#stops_0> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "STOPS" ] ;
    rr:subjectMap [
        rr:template "http://transport.linkeddata.es/madrid/metro/stops/{stop_id}" ;
        rr:class gtfs:Stop
    ] ;
    rr:predicateObjectMap [ rr:predicate foaf:name ; rr:objectMap [ rr:column "stop_name" ] ] ;
    rr:predicateObjectMap [ rr:predicate geo:lat ; rr:objectMap [ rr:column "stop_lat" ] ] ;
    rr:predicateObjectMap [ rr:predicate geo:long ; rr:objectMap [ rr:column "stop_lon" ] ] .

<#stoptimes_0> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "STOP_TIMES" ] ;
    rr:subjectMap [
        rr:template "http://transport.linkeddata.es/madrid/metro/stoptimes/{trip_id}-{stop_id}-{arrival_time}" ;
        rr:class gtfs:StopTime
    ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:arrivalTime ; rr:objectMap [ rr:column "arrival_time" ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:departureTime ; rr:objectMap [ rr:column "departure_time" ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:stopSequence ;
        rr:objectMap [ rr:column "stop_sequence" ; rr:datatype xsd:integer ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:trip ;
        rr:objectMap [ rr:parentTriplesMap <#trips_0> ;
            rr:joinCondition [ rr:child "trip_id" ; rr:parent "trip_id" ] ] ] ;
    rr:predicateObjectMap [ rr:predicate gtfs:stop ;
        rr:objectMap [ rr:parentTriplesMap <#stops_0> ;
            rr:joinCondition [ rr:child "stop_id" ; rr:parent "stop_id" ] ] ] .
"#;

/// The five representative OBDA queries (name, SPARQL), each within the v1
/// support surface (ADR-0007): `Q1` single-table BGP, `Q2` 2-way cross-table
/// join, `Q3` 3-way join (`stop_times → trip → route`), `Q4` pushed-down FILTER,
/// `Q5` OPTIONAL (NULL-safe left join). All are SELECT (latency track).
pub fn queries() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "Q1_routes_bgp",
            "PREFIX gtfs: <http://vocab.gtfs.org/terms#>
             SELECT ?route ?short ?long WHERE {
               ?route a gtfs:Route ; gtfs:shortName ?short ; gtfs:longName ?long }",
        ),
        (
            "Q2_route_agency_join",
            "PREFIX gtfs: <http://vocab.gtfs.org/terms#>
             PREFIX foaf: <http://xmlns.com/foaf/0.1/>
             SELECT ?route ?agencyName WHERE {
               ?route gtfs:agency ?a . ?a foaf:name ?agencyName }",
        ),
        (
            "Q3_stoptime_trip_route_join",
            "PREFIX gtfs: <http://vocab.gtfs.org/terms#>
             SELECT ?st ?short WHERE {
               ?st gtfs:trip ?t . ?t gtfs:route ?r . ?r gtfs:shortName ?short }",
        ),
        (
            "Q4_route_filter",
            "PREFIX gtfs: <http://vocab.gtfs.org/terms#>
             SELECT ?route ?long WHERE {
               ?route gtfs:shortName ?short ; gtfs:longName ?long . FILTER(?short = \"R0\") }",
        ),
        (
            "Q5_trip_optional_headsign",
            "PREFIX gtfs: <http://vocab.gtfs.org/terms#>
             SELECT ?t ?hs WHERE {
               ?t a gtfs:Trip . OPTIONAL { ?t gtfs:headsign ?hs } }",
        ),
    ]
}

/// The CONSTRUCT dump — the result-producing, linearly-growing OBDA query used
/// for the streaming constant-memory demonstration (ADR-0006). Output triples
/// grow ~linearly with the scale factor (dominated by `STOP_TIMES`).
pub const DUMP_QUERY: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";

/// Per-table row counts for one generated dataset.
#[derive(Debug, Clone, Copy, Default)]
pub struct RowCounts {
    pub agency: u64,
    pub calendar: u64,
    pub routes: u64,
    pub stops: u64,
    pub trips: u64,
    pub stop_times: u64,
}

impl RowCounts {
    /// Total source rows across all six tables (a proxy for source-data size).
    pub fn total(&self) -> u64 {
        self.agency + self.calendar + self.routes + self.stops + self.trips + self.stop_times
    }
}

/// Stops visited per trip — the `STOP_TIMES` fan-out (kept below `stops` so each
/// trip's stop ids are distinct, honouring the `(trip_id, stop_id, arrival_time)`
/// primary key).
const STOPS_PER_TRIP: u64 = 20;

/// The SQLite DDL — the six core GTFS tables, column names/affinities mirroring
/// the official `utils/postgresql.sql` (TEXT keys, REAL lat/lon → `xsd:double`,
/// INTEGER sequence → `xsd:integer` under R2RML §10; ADR-0015).
const SCHEMA_SQL: &str = "
CREATE TABLE AGENCY (
  agency_id TEXT PRIMARY KEY, agency_name TEXT, agency_url TEXT, agency_timezone TEXT);
CREATE TABLE CALENDAR (
  service_id TEXT PRIMARY KEY, monday INTEGER, start_date TEXT);
CREATE TABLE ROUTES (
  route_id TEXT PRIMARY KEY, agency_id TEXT, route_short_name TEXT,
  route_long_name TEXT, route_type INTEGER);
CREATE TABLE STOPS (
  stop_id TEXT PRIMARY KEY, stop_name TEXT, stop_lat REAL, stop_lon REAL, location_type INTEGER);
CREATE TABLE TRIPS (
  trip_id TEXT PRIMARY KEY, route_id TEXT, service_id TEXT, trip_headsign TEXT, direction_id INTEGER);
CREATE TABLE STOP_TIMES (
  trip_id TEXT, stop_id TEXT, arrival_time TEXT, departure_time TEXT,
  stop_sequence INTEGER, stop_headsign TEXT,
  PRIMARY KEY (trip_id, stop_id, arrival_time));
";

/// Generate the GTFS dataset at `scale` into an already-empty `conn`, returning
/// the per-table row counts. Deterministic: a given scale always yields the same
/// rows (so latency/memory runs are comparable). Inserts run inside one
/// transaction with prepared statements for speed.
pub fn generate(conn: &Connection, scale: u32) -> rusqlite::Result<RowCounts> {
    let sf = scale.max(1) as u64;
    conn.execute_batch(SCHEMA_SQL)?;
    conn.execute_batch("PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;")?;

    let n_agency = 2u64;
    let n_calendar = 3u64;
    let n_routes = 8 * sf;
    let n_stops = 40 * sf;
    let n_trips = 40 * sf;

    let tx = conn.unchecked_transaction()?;

    for i in 0..n_agency {
        tx.execute(
            "INSERT INTO AGENCY VALUES (?1,?2,?3,?4)",
            rusqlite::params![
                format!("A{i}"),
                format!("Agency {i}"),
                format!("http://transport.linkeddata.es/madrid/agency/{i}"),
                "Europe/Madrid",
            ],
        )?;
    }
    for i in 0..n_calendar {
        tx.execute(
            "INSERT INTO CALENDAR VALUES (?1,?2,?3)",
            rusqlite::params![format!("S{i}"), 1, "20260101"],
        )?;
    }
    for i in 0..n_routes {
        tx.execute(
            "INSERT INTO ROUTES VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                format!("r{i}"),
                format!("A{}", i % n_agency),
                format!("R{i}"),
                format!("Route {i}"),
                (i % 4) as i64,
            ],
        )?;
    }
    for i in 0..n_stops {
        tx.execute(
            "INSERT INTO STOPS VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                format!("s{i}"),
                format!("Stop {i}"),
                40.0 + (i as f64) * 0.0001,
                -3.7 + (i as f64) * 0.0001,
                0i64,
            ],
        )?;
    }
    for i in 0..n_trips {
        // Every third trip has a NULL headsign so Q5's OPTIONAL exercises both
        // the matched and the unbound branch.
        let headsign: Option<String> = if i % 3 == 0 {
            None
        } else {
            Some(format!("Trip headsign {i}"))
        };
        tx.execute(
            "INSERT INTO TRIPS VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                format!("t{i}"),
                format!("r{}", i % n_routes),
                format!("S{}", i % n_calendar),
                headsign,
                (i % 2) as i64,
            ],
        )?;
    }
    let mut stop_times = 0u64;
    {
        let mut stmt = tx.prepare("INSERT INTO STOP_TIMES VALUES (?1,?2,?3,?4,?5,?6)")?;
        for i in 0..n_trips {
            let trip = format!("t{i}");
            for k in 0..STOPS_PER_TRIP {
                let stop = format!("s{}", (i + k) % n_stops);
                let arr = format!("{:02}:{:02}:00", (k * 3) / 60 + 5, (k * 3) % 60);
                let dep = format!("{:02}:{:02}:30", (k * 3) / 60 + 5, (k * 3) % 60);
                stmt.execute(rusqlite::params![
                    trip,
                    stop,
                    arr,
                    dep,
                    k as i64,
                    format!("To stop {}", (i + k + 1) % n_stops),
                ])?;
                stop_times += 1;
            }
        }
    }
    tx.commit()?;

    Ok(RowCounts {
        agency: n_agency,
        calendar: n_calendar,
        routes: n_routes,
        stops: n_stops,
        trips: n_trips,
        stop_times,
    })
}

/// Open a **file-backed** SQLite source at `path`, generate the dataset at
/// `scale`, and return the live connection (ADR-0006: source data lives on disk,
/// off the engine heap). The file is created fresh; callers own its lifetime
/// (e.g. via a `tempfile::TempDir`).
pub fn open_source_db(path: &Path, scale: u32) -> rusqlite::Result<(Connection, RowCounts)> {
    let conn = Connection::open(path)?;
    let counts = generate(&conn, scale)?;
    Ok((conn, counts))
}

/// PostgreSQL DDL for the six GTFS bench tables. Table names are double-quoted
/// uppercase so they match the `rr:tableName "AGENCY"` / `"ROUTES"` etc. in the
/// R2RML mapping verbatim. Column names likewise preserved as-is (TEXT / REAL /
/// INTEGER mirror the SQLite schema so the same mapping applies to both).
pub const PG_SCHEMA_SQL: &str = r#"
DROP TABLE IF EXISTS "STOP_TIMES" CASCADE;
DROP TABLE IF EXISTS "TRIPS"      CASCADE;
DROP TABLE IF EXISTS "STOPS"      CASCADE;
DROP TABLE IF EXISTS "ROUTES"     CASCADE;
DROP TABLE IF EXISTS "CALENDAR"   CASCADE;
DROP TABLE IF EXISTS "AGENCY"     CASCADE;

CREATE TABLE "AGENCY" (
  agency_id TEXT PRIMARY KEY, agency_name TEXT, agency_url TEXT, agency_timezone TEXT);
CREATE TABLE "CALENDAR" (
  service_id TEXT PRIMARY KEY, monday INTEGER, start_date TEXT);
CREATE TABLE "ROUTES" (
  route_id TEXT PRIMARY KEY, agency_id TEXT, route_short_name TEXT,
  route_long_name TEXT, route_type INTEGER);
CREATE TABLE "STOPS" (
  stop_id TEXT PRIMARY KEY, stop_name TEXT, stop_lat DOUBLE PRECISION, stop_lon DOUBLE PRECISION, location_type INTEGER);
CREATE TABLE "TRIPS" (
  trip_id TEXT PRIMARY KEY, route_id TEXT, service_id TEXT, trip_headsign TEXT, direction_id INTEGER);
CREATE TABLE "STOP_TIMES" (
  trip_id TEXT, stop_id TEXT, arrival_time TEXT, departure_time TEXT,
  stop_sequence INTEGER, stop_headsign TEXT,
  PRIMARY KEY (trip_id, stop_id, arrival_time));
"#;

/// DDL to tear down the bench tables (idempotent; call after the bench run).
pub const PG_DROP_SQL: &str = r#"
DROP TABLE IF EXISTS "STOP_TIMES" CASCADE;
DROP TABLE IF EXISTS "TRIPS"      CASCADE;
DROP TABLE IF EXISTS "STOPS"      CASCADE;
DROP TABLE IF EXISTS "ROUTES"     CASCADE;
DROP TABLE IF EXISTS "CALENDAR"   CASCADE;
DROP TABLE IF EXISTS "AGENCY"     CASCADE;
"#;

/// Populate the six GTFS tables in a live **PostgreSQL** database at `scale`.
/// The caller must have already run `PG_SCHEMA_SQL` on the client. Returns row
/// counts identical to [`generate`] so the two fixtures are directly comparable.
///
/// Table names are double-quoted uppercase to match the R2RML mapping's
/// `rr:tableName` values. Inserts are batched in a single transaction for speed.
pub async fn generate_pg(
    client: &tokio_postgres::Client,
    scale: u32,
) -> Result<RowCounts, tokio_postgres::Error> {
    let sf = scale.max(1) as u64;
    let n_agency = 2u64;
    let n_calendar = 3u64;
    let n_routes = 8 * sf;
    let n_stops = 40 * sf;
    let n_trips = 40 * sf;

    client.batch_execute("BEGIN").await?;

    for i in 0..n_agency {
        client
            .execute(
                r#"INSERT INTO "AGENCY" VALUES ($1,$2,$3,$4)"#,
                &[
                    &format!("A{i}"),
                    &format!("Agency {i}"),
                    &format!("http://transport.linkeddata.es/madrid/agency/{i}"),
                    &"Europe/Madrid",
                ],
            )
            .await?;
    }
    for i in 0..n_calendar {
        client
            .execute(
                r#"INSERT INTO "CALENDAR" VALUES ($1,$2,$3)"#,
                &[&format!("S{i}"), &(1i32), &"20260101"],
            )
            .await?;
    }
    for i in 0..n_routes {
        client
            .execute(
                r#"INSERT INTO "ROUTES" VALUES ($1,$2,$3,$4,$5)"#,
                &[
                    &format!("r{i}"),
                    &format!("A{}", i % n_agency),
                    &format!("R{i}"),
                    &format!("Route {i}"),
                    &((i % 4) as i32),
                ],
            )
            .await?;
    }
    for i in 0..n_stops {
        client
            .execute(
                r#"INSERT INTO "STOPS" VALUES ($1,$2,$3,$4,$5)"#,
                &[
                    &format!("s{i}"),
                    &format!("Stop {i}"),
                    &(40.0f64 + (i as f64) * 0.0001),
                    &(-3.7f64 + (i as f64) * 0.0001),
                    &(0i32),
                ],
            )
            .await?;
    }
    for i in 0..n_trips {
        let headsign: Option<String> = if i % 3 == 0 {
            None
        } else {
            Some(format!("Trip headsign {i}"))
        };
        client
            .execute(
                r#"INSERT INTO "TRIPS" VALUES ($1,$2,$3,$4,$5)"#,
                &[
                    &format!("t{i}"),
                    &format!("r{}", i % n_routes),
                    &format!("S{}", i % n_calendar),
                    &headsign,
                    &((i % 2) as i32),
                ],
            )
            .await?;
    }
    let mut stop_times = 0u64;
    for i in 0..n_trips {
        let trip = format!("t{i}");
        for k in 0..STOPS_PER_TRIP {
            let stop = format!("s{}", (i + k) % n_stops);
            let arr = format!("{:02}:{:02}:00", (k * 3) / 60 + 5, (k * 3) % 60);
            let dep = format!("{:02}:{:02}:30", (k * 3) / 60 + 5, (k * 3) % 60);
            let headsign = format!("To stop {}", (i + k + 1) % n_stops);
            client
                .execute(
                    r#"INSERT INTO "STOP_TIMES" VALUES ($1,$2,$3,$4,$5,$6)"#,
                    &[&trip, &stop, &arr, &dep, &(k as i32), &headsign],
                )
                .await?;
            stop_times += 1;
        }
    }
    client.batch_execute("COMMIT").await?;

    Ok(RowCounts {
        agency: n_agency,
        calendar: n_calendar,
        routes: n_routes,
        stops: n_stops,
        trips: n_trips,
        stop_times,
    })
}

/// MySQL DDL for the six GTFS bench tables (ADR-0024 M5 constant-memory variant).
/// Table names are backtick-quoted uppercase to match `rr:tableName "AGENCY"` etc.
/// exactly (case-sensitive on Linux `mysqld`). Key/text columns are `VARCHAR`
/// (MySQL cannot use unbounded `TEXT` in a `PRIMARY KEY`), `DOUBLE` for lat/lon and
/// `INT` for integers — the same logical shape as the SQLite/PG schemas so the same
/// R2RML mapping applies. Statements are `;`-separated (no `;` inside any one), run
/// one at a time by [`generate_mysql`] (`mysql_async` executes one per call).
pub const MYSQL_SCHEMA_SQL: &str = r#"
DROP TABLE IF EXISTS `STOP_TIMES`;
DROP TABLE IF EXISTS `TRIPS`;
DROP TABLE IF EXISTS `STOPS`;
DROP TABLE IF EXISTS `ROUTES`;
DROP TABLE IF EXISTS `CALENDAR`;
DROP TABLE IF EXISTS `AGENCY`;
CREATE TABLE `AGENCY` (
  agency_id VARCHAR(64) PRIMARY KEY, agency_name VARCHAR(255), agency_url VARCHAR(255), agency_timezone VARCHAR(64));
CREATE TABLE `CALENDAR` (
  service_id VARCHAR(64) PRIMARY KEY, monday INT, start_date VARCHAR(32));
CREATE TABLE `ROUTES` (
  route_id VARCHAR(64) PRIMARY KEY, agency_id VARCHAR(64), route_short_name VARCHAR(255),
  route_long_name VARCHAR(255), route_type INT);
CREATE TABLE `STOPS` (
  stop_id VARCHAR(64) PRIMARY KEY, stop_name VARCHAR(255), stop_lat DOUBLE, stop_lon DOUBLE, location_type INT);
CREATE TABLE `TRIPS` (
  trip_id VARCHAR(64) PRIMARY KEY, route_id VARCHAR(64), service_id VARCHAR(64), trip_headsign VARCHAR(255), direction_id INT);
CREATE TABLE `STOP_TIMES` (
  trip_id VARCHAR(64), stop_id VARCHAR(64), arrival_time VARCHAR(32), departure_time VARCHAR(32),
  stop_sequence INT, stop_headsign VARCHAR(255),
  PRIMARY KEY (trip_id, stop_id, arrival_time));
"#;

/// Populate the six GTFS tables in the live **MySQL** database currently selected on
/// `conn` (the caller has already `CREATE DATABASE` + `USE`-d a throwaway db) at
/// `scale`. Runs [`MYSQL_SCHEMA_SQL`] (drop+create) then batches the same rows as
/// [`generate`] / [`generate_pg`], so the three fixtures are directly comparable
/// (identical row counts ⇒ identical dump-triple counts). Inserts use `exec_batch`
/// (one prepared statement, batched params) for speed.
pub async fn generate_mysql(
    conn: &mut mysql_async::Conn,
    scale: u32,
) -> Result<RowCounts, mysql_async::Error> {
    use mysql_async::prelude::Queryable;

    let sf = scale.max(1) as u64;
    let n_agency = 2u64;
    let n_calendar = 3u64;
    let n_routes = 8 * sf;
    let n_stops = 40 * sf;
    let n_trips = 40 * sf;

    for stmt in MYSQL_SCHEMA_SQL.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            conn.query_drop(s).await?;
        }
    }

    let agency: Vec<(String, String, String, String)> = (0..n_agency)
        .map(|i| {
            (
                format!("A{i}"),
                format!("Agency {i}"),
                format!("http://transport.linkeddata.es/madrid/agency/{i}"),
                "Europe/Madrid".to_owned(),
            )
        })
        .collect();
    conn.exec_batch(
        "INSERT INTO `AGENCY` VALUES (?,?,?,?)",
        agency.iter().map(|(a, b, c, d)| (a, b, c, d)),
    )
    .await?;

    let calendar: Vec<(String, i32, String)> = (0..n_calendar)
        .map(|i| (format!("S{i}"), 1i32, "20260101".to_owned()))
        .collect();
    conn.exec_batch(
        "INSERT INTO `CALENDAR` VALUES (?,?,?)",
        calendar.iter().map(|(a, b, c)| (a, b, c)),
    )
    .await?;

    let routes: Vec<(String, String, String, String, i32)> = (0..n_routes)
        .map(|i| {
            (
                format!("r{i}"),
                format!("A{}", i % n_agency),
                format!("R{i}"),
                format!("Route {i}"),
                (i % 4) as i32,
            )
        })
        .collect();
    conn.exec_batch(
        "INSERT INTO `ROUTES` VALUES (?,?,?,?,?)",
        routes.iter().map(|(a, b, c, d, e)| (a, b, c, d, e)),
    )
    .await?;

    let stops: Vec<(String, String, f64, f64, i32)> = (0..n_stops)
        .map(|i| {
            (
                format!("s{i}"),
                format!("Stop {i}"),
                40.0 + (i as f64) * 0.0001,
                -3.7 + (i as f64) * 0.0001,
                0i32,
            )
        })
        .collect();
    conn.exec_batch(
        "INSERT INTO `STOPS` VALUES (?,?,?,?,?)",
        stops.iter().map(|(a, b, c, d, e)| (a, b, c, d, e)),
    )
    .await?;

    let trips: Vec<(String, String, String, Option<String>, i32)> = (0..n_trips)
        .map(|i| {
            let headsign: Option<String> = if i % 3 == 0 {
                None
            } else {
                Some(format!("Trip headsign {i}"))
            };
            (
                format!("t{i}"),
                format!("r{}", i % n_routes),
                format!("S{}", i % n_calendar),
                headsign,
                (i % 2) as i32,
            )
        })
        .collect();
    conn.exec_batch(
        "INSERT INTO `TRIPS` VALUES (?,?,?,?,?)",
        trips.iter().map(|(a, b, c, d, e)| (a, b, c, d, e)),
    )
    .await?;

    let mut stop_times_rows: Vec<(String, String, String, String, i32, String)> = Vec::new();
    for i in 0..n_trips {
        let trip = format!("t{i}");
        for k in 0..STOPS_PER_TRIP {
            let stop = format!("s{}", (i + k) % n_stops);
            let arr = format!("{:02}:{:02}:00", (k * 3) / 60 + 5, (k * 3) % 60);
            let dep = format!("{:02}:{:02}:30", (k * 3) / 60 + 5, (k * 3) % 60);
            let headsign = format!("To stop {}", (i + k + 1) % n_stops);
            stop_times_rows.push((trip.clone(), stop, arr, dep, k as i32, headsign));
        }
    }
    let stop_times = stop_times_rows.len() as u64;
    conn.exec_batch(
        "INSERT INTO `STOP_TIMES` VALUES (?,?,?,?,?,?)",
        stop_times_rows
            .iter()
            .map(|(a, b, c, d, e, f)| (a, b, c, d, e, f)),
    )
    .await?;

    Ok(RowCounts {
        agency: n_agency,
        calendar: n_calendar,
        routes: n_routes,
        stops: n_stops,
        trips: n_trips,
        stop_times,
    })
}
