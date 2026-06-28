#!/usr/bin/env bash
# run_ontop_bench.sh — REAL Ontop (VKG/OBDA) numbers over the SAME GTFS subset
# semantic-fabric measures, for the head-to-head in BENCHMARKS.md.
#
# Ontop is a JVM batch/endpoint tool, so its `query` CLI pays a multi-second JVM
# cold start that would swamp the query itself. The fair, standard measurement
# (used by GTFS-Madrid-Bench) is the WARM SPARQL HTTP endpoint: boot it once, warm
# each query, then take the median wall-clock of N timed HTTP round-trips. This
# still includes HTTP + result serialization (overhead semantic-fabric's
# in-process library does NOT pay) — see BENCHMARKS.md "Measurement asymmetry".
#
# Prereqs:
#   * PostgreSQL at :5432 with the dataset loaded:  scripts/load_gtfs_postgres.sh SCALE
#   * Ontop CLI 5.5.0 unpacked, PostgreSQL JDBC driver dropped in its jdbc/ dir.
#     Download:  https://github.com/ontop/ontop/releases/download/ontop-5.5.0/ontop-cli-5.5.0.zip
#
# Usage: ONTOP_HOME=/path/to/ontop-cli scripts/run_ontop_bench.sh [SCALE] [PORT] [RUNS]
set -euo pipefail

SCALE="${1:-1}"
PORT="${2:-18080}"
RUNS="${3:-15}"
ONTOP_HOME="${ONTOP_HOME:?set ONTOP_HOME to the unpacked ontop-cli directory}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
M="$HERE/ontop"
EP="http://localhost:${PORT}/sparql"

median() { sort -n | awk '{a[NR]=$1} END{n=NR; if(n%2){print a[(n+1)/2]}else{print (a[n/2]+a[n/2+1])/2}}'; }

echo ">> starting Ontop endpoint on :$PORT (scale=$SCALE assumed already loaded)"
"$ONTOP_HOME/ontop" endpoint -m "$M/gtfs.r2rml.ttl" -p "$M/gtfs.properties" --port "$PORT" \
  >/tmp/ontop-endpoint-${SCALE}x.log 2>&1 &
EP_PID=$!
trap 'kill $EP_PID 2>/dev/null || true' EXIT

for i in $(seq 1 60); do
  curl -s -o /dev/null -w "%{http_code}" "$EP?query=ASK%7B%7D" 2>/dev/null | grep -q 200 && break
  sleep 1
done
echo ">> endpoint ready; timing $RUNS warm runs/query (median wall-clock, seconds)"
printf "%-32s %12s %8s\n" "query" "median_s" "rows"

for q in q1 q2 q3 q4 q5; do
  query="$(cat "$M/$q.rq")"
  for w in 1 2 3; do curl -s -o /dev/null --data-urlencode "query=$query" -H "Accept: text/csv" "$EP"; done
  tmp="$(mktemp)"
  for r in $(seq 1 "$RUNS"); do
    curl -s -o /dev/null -w "%{time_total}\n" --data-urlencode "query=$query" -H "Accept: text/csv" "$EP"
  done >"$tmp"
  rows="$(curl -s --data-urlencode "query=$query" -H "Accept: text/csv" "$EP" | tail -n +2 | wc -l | tr -d ' ')"
  printf "%-32s %12s %8s\n" "$q" "$(median <"$tmp")" "$rows"
  rm -f "$tmp"
done

# CONSTRUCT dump (whole virtual graph, Turtle)
query="$(cat "$M/dump.rq")"
for w in 1 2; do curl -s -o /dev/null --data-urlencode "query=$query" -H "Accept: text/turtle" "$EP"; done
tmp="$(mktemp)"
for r in $(seq 1 8); do
  curl -s -o /dev/null -w "%{time_total}\n" --data-urlencode "query=$query" -H "Accept: text/turtle" "$EP"
done >"$tmp"
printf "%-32s %12s\n" "dump (CONSTRUCT ?s ?p ?o)" "$(median <"$tmp")"
rm -f "$tmp"
