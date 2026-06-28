#!/usr/bin/env bash
# footprint.sh — leanness / footprint of each engine as a warm PostgreSQL OBDA
# endpoint: on-disk artifact size, cold-start (launch -> first HTTP 200), and
# resident set size (RSS) while serving. All real measurements on this machine.
#
# RSS is the fair cross-runtime memory axis here (both are live HTTP servers):
# native allocator vs JVM heap+metaspace are not directly comparable internally,
# but the OS resident set of each *serving process* is. We measure it after the
# same warm-up + the same handful of queries, so both are in steady state.
#
# Usage: ONTOP_HOME=/path/to/ontop-cli scripts/compare/footprint.sh [PGCONN]
set -euo pipefail

PGCONN="${1:-host=localhost port=5432 user=henrik dbname=gtfs_bench}"
SF_PORT="${SF_PORT:-7911}"
ONTOP_PORT="${ONTOP_PORT:-18090}"
ONTOP_HOME="${ONTOP_HOME:?set ONTOP_HOME to the unpacked ontop-cli directory}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
M="$REPO/scripts/ontop"
SF_BIN="$REPO/target/release/semantic-fabric"
SF_EP="http://127.0.0.1:${SF_PORT}/sparql"
ONTOP_EP="http://127.0.0.1:${ONTOP_PORT}/sparql"
PROBE="$(cat "$M/q4.rq")"

# rss_kb PID -> resident set size in KB (sum of the process; macOS `ps`)
rss_kb() { ps -o rss= -p "$1" | tr -d ' '; }
mib() { awk "BEGIN{printf \"%.1f\", $1/1024}"; }   # KB -> MiB

# time_to_ready EP -> seconds (float) from now until first HTTP 200 on EP
wait_ready_timed() {
  local ep="$1" start end
  start="$(python3 -c 'import time;print(time.time())')"
  for _ in $(seq 1 120); do
    if [ "$(curl -s -o /dev/null -w '%{http_code}' --data-urlencode "query=$PROBE" -H 'Accept: text/csv' "$ep" 2>/dev/null)" = "200" ]; then
      end="$(python3 -c 'import time;print(time.time())')"
      awk "BEGIN{printf \"%.2f\", $end-$start}"; return 0
    fi
    sleep 0.1
  done
  echo "ERR"; return 1
}

echo "=== artifact size (on disk) ==="
sf_bytes="$(stat -f%z "$SF_BIN")"
sf_mib="$(awk "BEGIN{printf \"%.1f\", $sf_bytes/1048576}")"
printf "semantic-fabric single binary : %s bytes (%s MiB)\n" "$sf_bytes" "$sf_mib"
ontop_dist_kb="$(du -sk "$ONTOP_HOME" | awk '{print $1}')"
ontop_jars="$(ls "$ONTOP_HOME"/lib/*.jar 2>/dev/null | wc -l | tr -d ' ')"
printf "Ontop CLI dist (unpacked)     : %s MiB across %s lib jars (+ JVM required, not counted)\n" \
  "$(mib "$ontop_dist_kb")" "$ontop_jars"

echo ""
echo "=== cold start (process launch -> first HTTP 200) + serving RSS ==="

# --- semantic-fabric ---
"$SF_BIN" serve --source "pg:$PGCONN" --mapping "$M/gtfs.r2rml.ttl" --bind "127.0.0.1:$SF_PORT" \
  >/tmp/sf-footprint.log 2>&1 &
SF_PID=$!
sf_cold="$(wait_ready_timed "$SF_EP")"
for _ in $(seq 1 20); do curl -s -o /dev/null --data-urlencode "query=$PROBE" -H 'Accept: text/csv' "$SF_EP"; done
sf_rss="$(rss_kb "$SF_PID")"
kill "$SF_PID" 2>/dev/null || true

# --- Ontop ---
"$ONTOP_HOME/ontop" endpoint -m "$M/gtfs.r2rml.ttl" -p "$M/gtfs.properties" --port "$ONTOP_PORT" \
  >/tmp/ontop-footprint.log 2>&1 &
ONTOP_PID=$!
ontop_cold="$(wait_ready_timed "$ONTOP_EP")"
for _ in $(seq 1 20); do curl -s -o /dev/null --data-urlencode "query=$PROBE" -H 'Accept: text/csv' "$ONTOP_EP"; done
# Ontop's `ontop` launcher is a shell script that execs `java`; find the java child.
ontop_java_pid="$(pgrep -P "$ONTOP_PID" java || true)"
[ -z "$ontop_java_pid" ] && ontop_java_pid="$ONTOP_PID"
ontop_rss="$(rss_kb "$ontop_java_pid")"
kill "$ONTOP_PID" "$ontop_java_pid" 2>/dev/null || true

printf "\n%-18s %14s %16s\n" "engine" "cold_start_s" "serving_RSS_MiB"
printf "%-18s %14s %16s\n" "semantic-fabric" "$sf_cold" "$(mib "$sf_rss")"
printf "%-18s %14s %16s\n" "Ontop 5.5.0"     "$ontop_cold" "$(mib "$ontop_rss")"
