#!/usr/bin/env bash
# Drives the sqlx streaming spike matrix. Fresh process per measurement (clean RSS
# baseline each time). Writes results.csv. THROWAWAY — ADR-0024 §8 spike.
set -euo pipefail
cd "$(dirname "$0")"

BIN=./target/release/sqlx-streaming-spike
cargo build --release

echo "== setup (one-time populate to MAX_ROWS) =="
"$BIN" setup pg
"$BIN" setup mysql

OUT=results.csv
echo "engine,mode,rows_req,rows_read,ttfr_ms,total_ms,baseline_rss_mb,peak_rss_mb,rss_at_first_row_mb" >"$OUT"

echo "== matrix =="
for eng in pg mysql; do
  for n in 10 100000 1000000 2000000; do
    for mode in stream buffer; do
      "$BIN" measure "$eng" "$n" "$mode" >>"$OUT"
    done
  done
done

echo "== results.csv =="
column -t -s, "$OUT"
