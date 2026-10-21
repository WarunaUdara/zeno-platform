#!/usr/bin/env bash
# Prove that span export works, without a GCP project.
#
# The API only exports when OTEL_EXPORTER_OTLP_ENDPOINT is set, and in production it
# also has to fetch an access token from the metadata server. Both of those are
# ordinary HTTP, so both can be stood up locally: point the process at
# otlp-standin.py and the token fetch, the auth header, and the span payload all
# become observable.
#
# This exists because "spans reach Cloud Trace" is otherwise a claim that needs a
# cloud project and a console to check. Here it is a script with an exit code.
#
#   bun run db:up                                  # once, in another shell
#   infra/observability/verify-trace-export.sh
#
# Requires python3, curl, and a Rust toolchain. Uses the local development database
# and creates no cloud resource.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

STANDIN_PORT="${STANDIN_PORT:-4599}"
API_PORT="${API_PORT:-18080}"
DATABASE_URL="${DATABASE_URL:-postgres://zeno:zeno_dev_only@127.0.0.1:5432/zeno}"
BODY_LOG="$(mktemp "${TMPDIR:-/tmp}/zeno-otlp-bodies.XXXXXX")"
READY_TIMEOUT="${READY_TIMEOUT:-40}"
EXPORT_TIMEOUT="${EXPORT_TIMEOUT:-25}"
# Longer than the batch processor's scheduled delay, so a probe-only batch would
# have had every chance to flush before it is asserted absent.
PROBE_WAIT="${PROBE_WAIT:-12}"

failures=0
standin_pid=""
api_pid=""

log() { printf '  %s\n' "$*"; }
ok() { printf '  \033[32mpass\033[0m  %s\n' "$*"; }
bad() { printf '  \033[31mfail\033[0m  %s\n' "$*"; failures=$((failures + 1)); }

cleanup() {
  # `wait` reaps each child, which is what stops the shell printing "Terminated"
  # after a successful run.
  [[ -n "$api_pid" ]] && { kill "$api_pid" 2>/dev/null || true; wait "$api_pid" 2>/dev/null || true; }
  [[ -n "$standin_pid" ]] && { kill "$standin_pid" 2>/dev/null || true; wait "$standin_pid" 2>/dev/null || true; }
  rm -f "$BODY_LOG"
}
trap cleanup EXIT

for tool in python3 curl cargo; do
  command -v "$tool" >/dev/null 2>&1 || { printf 'error: %s is required but not on PATH\n' "$tool" >&2; exit 1; }
done

exports() {
  curl -fsS "http://127.0.0.1:$STANDIN_PORT/__state" |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["export_requests"])'
}

printf '\nZeno trace export verification\n\n'

python3 infra/observability/otlp-standin.py "$STANDIN_PORT" "$BODY_LOG" &
standin_pid=$!
sleep 1

log "building the API"
cargo build -q -p zeno-api 2>/dev/null || { printf 'error: cargo build failed\n' >&2; exit 1; }

log "starting the API against the stand-in"
env DATABASE_URL="$DATABASE_URL" \
  ZENO_API_BIND="127.0.0.1:$API_PORT" \
  OTEL_EXPORTER_OTLP_ENDPOINT="http://127.0.0.1:$STANDIN_PORT" \
  GCE_METADATA_HOST="127.0.0.1:$STANDIN_PORT" \
  GOOGLE_CLOUD_PROJECT=zeno-verification \
  ./target/debug/zeno-api serve >/dev/null 2>&1 &
api_pid=$!

ready=0
for _ in $(seq 1 "$READY_TIMEOUT"); do
  if curl -fsS -o /dev/null -m 2 "http://127.0.0.1:$API_PORT/health/live" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 1
done
if [[ "$ready" -ne 1 ]]; then
  printf 'error: the API did not become ready. Is the database up (bun run db:up)?\n' >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Probes first, and wait long enough that a probe-only batch would have flushed.
# A liveness check every ten seconds would otherwise dominate the trace list, which
# is the whole reason probes were excluded.
# ---------------------------------------------------------------------------
log "checking that health probes export nothing"
for _ in $(seq 1 6); do
  curl -fsS -o /dev/null -m 5 "http://127.0.0.1:$API_PORT/health/live" || true
  curl -fsS -o /dev/null -m 5 "http://127.0.0.1:$API_PORT/health/ready" || true
  sleep 2
done
sleep "$PROBE_WAIT"
probe_exports="$(exports)"

# ---------------------------------------------------------------------------
# Now a real request, carrying a caller trace context so the span is parented and
# reports the identifiers a log line would carry.
# ---------------------------------------------------------------------------
log "sending a traced request"
curl -fsS -o /dev/null -m 10 \
  -H 'traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01' \
  "http://127.0.0.1:$API_PORT/v1/events?limit=1" || true

log "waiting for the batch export"
for _ in $(seq 1 "$EXPORT_TIMEOUT"); do
  [[ "$(exports)" -gt "$probe_exports" ]] && break
  sleep 1
done

# ---------------------------------------------------------------------------
# Assertions.
# ---------------------------------------------------------------------------
state="$(curl -fsS "http://127.0.0.1:$STANDIN_PORT/__state")"
read_state() { printf '%s' "$state" | python3 -c "import json,sys; print(json.load(sys.stdin).get('$1'))"; }

printf '\n  assertions\n'

if [[ "$probe_exports" -eq 0 ]]; then
  ok "health probes exported nothing ($PROBE_WAIT s of probes, zero batches)"
else
  bad "health probes produced $probe_exports export(s)"
fi

[[ "$(read_state token_requests)" -ge 1 ]] \
  && ok "the metadata server was asked for a token" \
  || bad "no token request reached the metadata server"

[[ "$(read_state token_metadata_flavor)" == "Google" ]] \
  && ok "the token request carried Metadata-Flavor: Google" \
  || bad "the token request did not carry Metadata-Flavor: Google"

[[ "$(read_state export_path)" == "/v1/traces" ]] \
  && ok "spans were posted to /v1/traces" \
  || bad "spans went to $(read_state export_path), not /v1/traces"

case "$(read_state export_authorization)" in
  "Bearer "*) ok "the export carried a bearer token" ;;
  *) bad "the export carried no bearer token: $(read_state export_authorization)" ;;
esac

[[ "$(read_state export_bytes)" -gt 0 ]] \
  && ok "the export body was not empty ($(read_state export_bytes) bytes)" \
  || bad "the export body was empty"

# The Rust exporter sends protobuf, so the payload is checked for the strings it
# must contain rather than decoded. A decoder would be more code than the assertion
# is worth; the live check in Cloud Trace is the real proof.
if grep -aq 'http_request' "$BODY_LOG"; then
  ok "the payload names the http_request span"
else
  bad "the payload does not contain the http_request span name"
fi

if grep -aq '4bf92f3577b34da6a3ce929d0e0e4736' "$BODY_LOG"; then
  ok "the span carries the caller's trace id"
else
  bad "the span does not carry the caller's trace id"
fi

printf '\n'
if [[ "$failures" -eq 0 ]]; then
  printf '  all assertions passed\n\n'
else
  printf '  %d assertion(s) failed\n\n' "$failures"
  exit 1
fi
