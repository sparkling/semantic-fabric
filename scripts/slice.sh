#!/usr/bin/env bash
# One-command wrappers around the delivery harness lifecycle for one slice.
#
#   scripts/slice.sh start  <task.json> <native.json>   begin + bind + advance
#   scripts/slice.sh impl   <task-id> <summary-file>     submit completed implementation
#   scripts/slice.sh checks <task-id>                    run every declared check, stop on failure
#   scripts/slice.sh review <task-id>                    advance; print review request id/digest
#   scripts/slice.sh verdict <task-id> <executor-id> <pass|changes> <summary-file>
#   scripts/slice.sh close  <task-id> <commit-message-file>  verify, commit scope, finish
#
# Owner defaults to claude-sonnet-coordinator (SLICE_OWNER overrides). The
# script never pushes; it commits only the task's declared scope.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OWNER="${SLICE_OWNER:-claude-sonnet-coordinator}"
STATE="${SLICE_STATE:-/tmp/sf-tasks}"
mkdir -p "$STATE"

harness() {
  # Retry transient index.lock / out-of-scope races from concurrent sessions.
  local attempt out
  for attempt in 1 2 3 4 5; do
    if out="$(npm --prefix "$ROOT/coding-harness" run -s delivery -- "$ROOT" "$@" 2>&1)"; then
      printf '%s\n' "$out"
      return 0
    fi
    if ! grep -qE 'DELIVERY_(GIT_OPERATION|OUT_OF_SCOPE_CHANGE)' <<<"$out"; then
      printf '%s\n' "$out" >&2
      return 1
    fi
    sleep $((attempt * 2))
  done
  printf '%s\n' "$out" >&2
  return 1
}

field() { node -e "const r=JSON.parse(require('fs').readFileSync(0,'utf8'));const q=r.request??r;console.log(q[process.argv[1]]??'')" "$1"; }

task_json() { node -e "const r=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(JSON.stringify(r.task??r))"; }

advance() { harness advance "$1" "$OWNER" | tee "$STATE/$1.request.json" >/dev/null; }

respond() {
  # respond <task-id> <executor-id> <outcome> <summary-file>
  local id="$1" executor="$2" outcome="$3" summary="$4" req="$STATE/$1.request.json"
  node - "$req" "$executor" "$outcome" "$summary" "$STATE/$id.response.json" <<'JS'
const fs = require('fs');
const [req, executor, outcome, summary, out] = process.argv.slice(2);
const q = JSON.parse(fs.readFileSync(req, 'utf8')).request;
fs.writeFileSync(out, JSON.stringify({
  schemaVersion: 1, requestId: q.id, sourceDigest: q.sourceDigest,
  native: { ...q.route, executorId: executor, authentication: 'native-subscription',
    observation: 'Claude Code native session via the user-authorized local 9router subscription transport.' },
  outcome, summary: fs.readFileSync(summary, 'utf8').trim(), issues: [],
}));
JS
  harness submit "$id" "$OWNER" "$STATE/$id.response.json" >/dev/null
}

cmd="${1:?command required}"; shift
case "$cmd" in
  start)
    id="$(node -e "console.log(JSON.parse(require('fs').readFileSync(process.argv[1],'utf8')).id)" "$1")"
    harness begin "$1" >/dev/null
    harness bind "$id" "$OWNER" "$2" >/dev/null
    advance "$id"
    echo "started $id: $(field stage < "$STATE/$id.request.json") $(field id < "$STATE/$id.request.json")"
    ;;
  impl)
    executor="$(field executorId < "$STATE/$1.request.json")"
    respond "$1" "$executor" completed "$2"
    echo "implementation submitted for $1"
    ;;
  checks)
    for check in $(harness status "$1" | node -e "const r=JSON.parse(require('fs').readFileSync(0,'utf8'));for(const c of (r.task??r.run?.task??{}).checks??[])console.log(c.id)"); do
      if harness check "$1" "$OWNER" "$check" | grep -q '"passed": true'; then
        echo "pass $check"
      else
        echo "FAIL $check"; exit 1
      fi
    done
    ;;
  review)
    advance "$1"
    echo "review $1: $(field id < "$STATE/$1.request.json") digest $(field sourceDigest < "$STATE/$1.request.json")"
    ;;
  verdict)
    outcome=completed; [ "$3" = changes ] && outcome=changes-requested
    respond "$1" "$2" "$outcome" "$4"
    echo "verdict $3 submitted for $1"
    ;;
  close)
    harness verify "$1" "$OWNER" >/dev/null
    mapfile -t scope < <(harness status "$1" | node -e "const r=JSON.parse(require('fs').readFileSync(0,'utf8'));for(const p of (r.task??r.run?.task??{}).scope??[])console.log(p)")
    git -C "$ROOT" add -- "${scope[@]}"
    git -C "$ROOT" commit -q -F "$2" -- "${scope[@]}"
    harness finish "$1" "$OWNER" "$(git -C "$ROOT" rev-parse HEAD)" >/dev/null
    echo "closed $1 at $(git -C "$ROOT" rev-parse --short HEAD)"
    ;;
  *) echo "unknown command: $cmd" >&2; exit 2 ;;
esac
