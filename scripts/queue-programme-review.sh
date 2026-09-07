#!/usr/bin/env bash
# Queue into the pinned native conversation; never create a second resume writer.
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo 'usage: queue-programme-review.sh /absolute/codex THREAD_UUID /absolute/prompt' >&2
  exit 64
fi
codex_executable=$1
review_thread=$2
review_prompt_path=$3
if [[ $codex_executable != /* || ! -f $codex_executable || ! -x $codex_executable ]]; then
  echo 'review queue: Codex executable must be an existing absolute executable file' >&2
  exit 64
fi
if [[ ! $review_thread =~ ^[[:xdigit:]]{8}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{12}$ ]]; then
  echo 'review queue: an explicit conversation UUID is required' >&2
  exit 64
fi
if [[ $review_prompt_path != /* || ! -f $review_prompt_path || ! -r $review_prompt_path || ! -s $review_prompt_path ]]; then
  echo 'review queue: prompt must be an existing nonempty readable absolute file' >&2
  exit 64
fi
review_prompt=$(< "$review_prompt_path")
if [[ ! $review_prompt =~ [^[:space:]] ]]; then
  echo 'review queue: prompt must contain instructions' >&2
  exit 64
fi
repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)

# Forward the native queue's exit status. A queue failure must NOT fall back to
# exec/resume, a different model, a provider API, or another writer.
exec /usr/bin/env -u OPENAI_API_KEY -u CODEX_API_KEY -u OPENAI_BASE_URL \
  -u ANTHROPIC_API_KEY -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_BASE_URL \
  -u OPENROUTER_API_KEY \
  "$codex_executable" queue --thread "$review_thread" \
  --message "$review_prompt" --cd "$repository_root"
