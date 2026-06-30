#!/usr/bin/env bash
# race.sh — the FAIR head-to-head: semantic-fabric (sf-serve) vs Ontop, BOTH as
# warm HTTP SPARQL endpoints over the SAME PostgreSQL backend, measured with the
# IDENTICAL methodology (curl %{time_total} median-of-N, same queries, same Accept).
#
# This is now possible because sf-serve is a real SPARQL 1.2 Protocol endpoint and
# semantic-fabric has a wired PostgreSQL OBDA executor (exec_pg). Same backend +
# same process model (warm HTTP) + same client timer => a genuine apples-to-apples
# query-latency race. (Earlier benches could not do this: sf executed only over
# embedded SQLite, so BENCHMARKS.md compared sf-in-process-SQLite vs Ontop-HTTP-PG,
# an explicit asymmetry. race.sh removes both asymmetries.)
#
# Queries q1..q7 are the simple BGP/join/filter/optional/groupby/orderby set.
# Queries q8..q15 are the ADR-0023 feature classes (UNION, agg-over-UNION, property
# path, MINUS, FILTER EXISTS, SUBQUERY, nested OPTIONAL, DISTINCT). For EACH query the
# race records BOTH engines' HTTP status + row count and classifies the row-parity:
#   OK         — both 200, sf_rows == ontop_rows (sound, latency is comparable)
#   MISMATCH   — both 200 but sf_rows != ontop_rows (a correctness gap vs the oracle)
#   SF-EMPTY   — both 200 but sf returns 0 rows while Ontop returns >0 (sf silent miss)
#   SF-501     — sf returns a non-200 where Ontop answers (an sf engine error/bug)
#   ONTOP-501  — Ontop returns a non-200 where sf answers (an sf capability advantage)
#   BOTH-ERR   — neither engine answers
# Ontop is treated as the correctness oracle (the reference VKG/OBDA engine). When
# the parity is not OK, the latency numbers are NOT a like-for-like comparison (one
# engine is not computing the same result) and must be read with that caveat.
#
# Prereqs:
#   * PostgreSQL at :5432 with the dataset loaded:  scripts/load_gtfs_postgres.sh SCALE
#   * sf-cli release binary built:                  cargo build --release -p sf-cli
#   * Ontop CLI 5.5.0 unpacked, PG JDBC driver in its jdbc/ dir (set ONTOP_HOME).
#
# Usage: ONTOP_HOME=/path/to/ontop-cli scripts/compare/race.sh [SCALE] [RUNS] [PGCONN]
set -euo pipefail

SCALE="${1:-1}"
RUNS="${2:-25}"
PGCONN="${3:-host=localhost port=5432 user=henrik dbname=gtfs_bench}"
SF_PORT="${SF_PORT:-7901}"
ONTOP_PORT="${ONTOP_PORT:-18080}"
ONTOP_HOME="${ONTOP_HOME:?set ONTOP_HOME to the unpacked ontop-cli directory}"
MAXT="${MAXT:-180}"                          # per-curl wall-clock cap (s) — big-scale guard
QUERIES="${QUERIES:-q1 q2 q3 q4 q5 q6 q7 q8 q9 q10 q11 q12 q13 q14 q15}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
M="$REPO/scripts/ontop"                       # shared mapping + queries
ONTOP_PROPS="${ONTOP_PROPS:-$M/gtfs.properties}"   # allow Docker/CI override
SF_BIN="$REPO/target/release/semantic-fabric"
SF_EP="http://127.0.0.1:${SF_PORT}/sparql"
ONTOP_EP="http://127.0.0.1:${ONTOP_PORT}/sparql"

median() { sort -n | awk '{a[NR]=$1} END{n=NR; if(n%2){print a[(n+1)/2]}else{print (a[n/2]+a[n/2+1])/2}}'; }
ms() { awk "BEGIN{printf \"%.2f\", $1*1000}"; }   # seconds -> ms string

wait_ready() {  # $1=endpoint ; probe with a real query (sf-serve 501s on empty ASK{})
  local probe; probe="$(cat "$M/q4.rq")"
  for _ in $(seq 1 90); do
    [ "$(curl -s -o /dev/null -w '%{http_code}' --data-urlencode "query=$probe" -H 'Accept: text/csv' "$1" 2>/dev/null)" = "200" ] && return 0
    sleep 1
  done
  echo "!! endpoint $1 never became ready" >&2; return 1
}

http_code() {  # $1=endpoint $2=query-file -> HTTP status (single call)
  curl -s -o /dev/null -w '%{http_code}' --max-time "$MAXT" \
    --data-urlencode "query=$(cat "$2")" -H "Accept: text/csv" "$1" 2>/dev/null || echo "000"
}

rows_of() {  # $1=endpoint $2=query-file ; count CSV result rows (minus header)
  curl -s --max-time "$MAXT" --data-urlencode "query=$(cat "$2")" -H "Accept: text/csv" "$1" \
    | tail -n +2 | sed '/^[[:space:]]*$/d' | wc -l | tr -d ' '
}

time_query() {  # $1=endpoint $2=query-file -> prints median seconds
  local q; q="$(cat "$2")"
  for _ in 1 2 3; do curl -s -o /dev/null --max-time "$MAXT" --data-urlencode "query=$q" -H "Accept: text/csv" "$1"; done
  local tmp; tmp="$(mktemp)"
  for _ in $(seq 1 "$RUNS"); do
    curl -s -o /dev/null --max-time "$MAXT" -w '%{time_total}\n' --data-urlencode "query=$q" -H "Accept: text/csv" "$1"
  done >"$tmp"
  median <"$tmp"; rm -f "$tmp"
}

echo ">> FAIR RACE  scale=${SCALE}  runs/query=${RUNS}  backend=PostgreSQL ($PGCONN)"

echo ">> starting sf-serve (PostgreSQL OBDA) on :$SF_PORT"
"$SF_BIN" serve --source "pg:$PGCONN" --mapping "$M/gtfs.r2rml.ttl" --bind "127.0.0.1:$SF_PORT" \
  >"/tmp/sf-serve-${SCALE}x.log" 2>&1 &
SF_PID=$!
echo ">> starting Ontop endpoint (PostgreSQL OBDA) on :$ONTOP_PORT"
"$ONTOP_HOME/ontop" endpoint -m "$M/gtfs.r2rml.ttl" -p "$ONTOP_PROPS" --port "$ONTOP_PORT" \
  >"/tmp/ontop-endpoint-${SCALE}x.log" 2>&1 &
ONTOP_PID=$!
trap 'kill $SF_PID $ONTOP_PID 2>/dev/null || true' EXIT

wait_ready "$SF_EP"
wait_ready "$ONTOP_EP"
echo ">> both endpoints warm; timing $RUNS warm runs/query (median %{time_total})"
printf "\n%-5s %12s %12s %8s %9s %10s\n" "query" "sf_ms" "ontop_ms" "sf_rows" "ont_rows" "status"

for q in $QUERIES; do
  qf="$M/$q.rq"
  [ -f "$qf" ] || continue
  sf_code="$(http_code "$SF_EP" "$qf")"
  on_code="$(http_code "$ONTOP_EP" "$qf")"

  if [ "$sf_code" = "200" ]; then sf_ms="$(ms "$(time_query "$SF_EP" "$qf")")"; sf_r="$(rows_of "$SF_EP" "$qf")"; else sf_ms="ERR"; sf_r="ERR"; fi
  if [ "$on_code" = "200" ]; then on_ms="$(ms "$(time_query "$ONTOP_EP" "$qf")")"; on_r="$(rows_of "$ONTOP_EP" "$qf")"; else on_ms="ERR"; on_r="ERR"; fi

  if   [ "$sf_code" != "200" ] && [ "$on_code" = "200" ]; then status="SF-501"
  elif [ "$on_code" != "200" ] && [ "$sf_code" = "200" ]; then status="ONTOP-501"
  elif [ "$sf_code" != "200" ] && [ "$on_code" != "200" ]; then status="BOTH-ERR"
  elif [ "$sf_r" = "$on_r" ]; then status="OK"
  elif [ "$sf_r" = "0" ] && [ "$on_r" != "0" ]; then status="SF-EMPTY"
  else status="MISMATCH"; fi

  printf "%-5s %12s %12s %8s %9s %10s\n" "$q" "$sf_ms" "$on_ms" "$sf_r" "$on_r" "$status"
done

echo ""
echo ">> sf-serve log tail:";    tail -n 2 "/tmp/sf-serve-${SCALE}x.log"    | sed 's/^/   /'
echo ">> ontop endpoint log tail:"; tail -n 2 "/tmp/ontop-endpoint-${SCALE}x.log" | sed 's/^/   /'
