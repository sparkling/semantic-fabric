#!/usr/bin/env bash
# Root-specific dependency boundary, not binary provenance or backend admission.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

# Never select the workspace or sf-conformance here: Cargo feature unification
# would bring developer-only backends back into this graph.
serving_graph="$(cargo tree --color never --locked -p sf-cli --no-default-features \
  --edges normal,build --prefix none --format '{p}|{f}')"
printf '%s\n' "$serving_graph" | awk -F '|' '
  {
    split($1, package, " "); name = package[1]; seen[name] = 1;
    if (name ~ /^(sf-conformance|sf-bench|sf-capture-supervisor|criterion|tiberius|duckdb|odbc-api)$/) {
      print "Forbidden serving dependency: " name > "/dev/stderr"; failed = 1;
    }
    # Product crates currently need no feature opt-ins. Reject newly enabled
    # evidence/prototype/backend features pending explicit profile review.
    if (name ~ /^sf-/ && $2 !~ /^(| \(\*\))$/) {
      print "Unreviewed serving features: " name "|" $2 > "/dev/stderr"; failed = 1;
    }
  }
  END {
    split("sf-cli sf-serve sf-sparql sf-sql sf-mapping sf-validation rusqlite tokio-postgres mysql_async", required, " ");
    for (i in required) if (!seen[required[i]]) {
      print "Missing required serving dependency: " required[i] > "/dev/stderr"; failed = 1;
    }
    exit failed;
  }'
printf '%s\n' 'Serving graph: required product crates and all three database drivers present; development crates/features absent.'
