-- gen_gtfs.sql — load the GTFS-Madrid OBDA subset into PostgreSQL, byte-for-byte
-- matching the deterministic generator in crates/sf-bench/src/workload.rs
-- (fn generate). Run with:  psql -v scale=1 -f gen_gtfs.sql
--
-- The six core GTFS tables and the exact row values sf-bench emits, so Ontop sees
-- the IDENTICAL logical dataset semantic-fabric measures (only the backend
-- differs: sf executes over embedded SQLite, Ontop over this PostgreSQL).
-- Per-scale row counts (s = :scale):
--   agency 2 · calendar 3 · routes 8·s · stops 40·s · trips 40·s · stop_times 800·s
-- Identifiers are lowercased (PostgreSQL folds unquoted names) — the R2RML mapping
-- scripts/ontop/gtfs.r2rml.ttl uses the same lowercase names.

\set s :scale

DROP TABLE IF EXISTS stop_times, trips, stops, routes, calendar, agency CASCADE;

CREATE TABLE agency (
  agency_id text PRIMARY KEY, agency_name text, agency_url text, agency_timezone text);
CREATE TABLE calendar (
  service_id text PRIMARY KEY, monday integer, start_date text);
CREATE TABLE routes (
  route_id text PRIMARY KEY, agency_id text, route_short_name text,
  route_long_name text, route_type integer);
CREATE TABLE stops (
  stop_id text PRIMARY KEY, stop_name text, stop_lat double precision,
  stop_lon double precision, location_type integer);
CREATE TABLE trips (
  trip_id text PRIMARY KEY, route_id text, service_id text,
  trip_headsign text, direction_id integer);
CREATE TABLE stop_times (
  trip_id text, stop_id text, arrival_time text, departure_time text,
  stop_sequence integer, stop_headsign text,
  PRIMARY KEY (trip_id, stop_id, arrival_time));

-- AGENCY: 2 rows (i = 0..1)
INSERT INTO agency
SELECT 'A'||i, 'Agency '||i,
       'http://transport.linkeddata.es/madrid/agency/'||i, 'Europe/Madrid'
FROM generate_series(0, 1) AS i;

-- CALENDAR: 3 rows (i = 0..2); start_date kept as text "20260101" (mirrors sf)
INSERT INTO calendar
SELECT 'S'||i, 1, '20260101'
FROM generate_series(0, 2) AS i;

-- ROUTES: 8*s rows (i = 0..8s-1); agency A{i%2}, route_type i%4
INSERT INTO routes
SELECT 'r'||i, 'A'||(i % 2), 'R'||i, 'Route '||i, (i % 4)
FROM generate_series(0, 8 * :s - 1) AS i;

-- STOPS: 40*s rows (i = 0..40s-1)
INSERT INTO stops
SELECT 's'||i, 'Stop '||i, 40.0 + i * 0.0001, -3.7 + i * 0.0001, 0
FROM generate_series(0, 40 * :s - 1) AS i;

-- TRIPS: 40*s rows; route r{i%(8s)}, service S{i%3}; every 3rd headsign NULL (Q5)
INSERT INTO trips
SELECT 't'||i, 'r'||(i % (8 * :s)), 'S'||(i % 3),
       CASE WHEN i % 3 = 0 THEN NULL ELSE 'Trip headsign '||i END,
       (i % 2)
FROM generate_series(0, 40 * :s - 1) AS i;

-- STOP_TIMES: 800*s rows; for each trip i, 20 stops k=0..19, stop s{(i+k)%(40s)}
INSERT INTO stop_times
SELECT 't'||i,
       's'||((i + k) % (40 * :s)),
       lpad((((k*3)/60)+5)::text, 2, '0')||':'||lpad(((k*3)%60)::text, 2, '0')||':00',
       lpad((((k*3)/60)+5)::text, 2, '0')||':'||lpad(((k*3)%60)::text, 2, '0')||':30',
       k,
       'To stop '||((i + k + 1) % (40 * :s))
FROM generate_series(0, 40 * :s - 1) AS i,
     generate_series(0, 19) AS k;

ANALYZE;

SELECT 'agency'     AS tbl, count(*) FROM agency
UNION ALL SELECT 'calendar',   count(*) FROM calendar
UNION ALL SELECT 'routes',     count(*) FROM routes
UNION ALL SELECT 'stops',      count(*) FROM stops
UNION ALL SELECT 'trips',      count(*) FROM trips
UNION ALL SELECT 'stop_times', count(*) FROM stop_times;
