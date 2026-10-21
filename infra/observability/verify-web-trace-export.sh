#!/usr/bin/env bash
# Prove that the web tier exports spans, without a GCP project.
#
# The Node exporter needs the same two things the Rust one does: a token from the
# metadata server, and somewhere to post spans. otlp-standin.py provides both, so the
# token fetch, the auth header, and the span structure are all observable here.
#
# Unlike the Rust exporter this one speaks OTLP/JSON, so the payload can be parsed
# rather than grepped, and the assertions can be about the shape of the trace: a root
# span, children of that root, and the service name.
#
#   (cd apps/web && bun run build)                 # produces the standalone output
#   infra/observability/verify-web-trace-export.sh
#
# Requires python3, curl, and a production build. Creates no cloud resource.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

STANDIN_PORT="${STANDIN_PORT:-4600}"
APP_PORT="${APP_PORT:-13000}"
BODY_LOG="$(mktemp "${TMPDIR:-/tmp}/zeno-web-otlp-bodies.XXXXXX")"
READY_TIMEOUT="${READY_TIMEOUT:-60}"
EXPORT_TIMEOUT="${EXPORT_TIMEOUT:-30}"
SERVER_JS="$REPO_ROOT/apps/web/.next/standalone/apps/web/server.js"

failures=0
standin_pid=""
app_pid=""

log() { printf '  %s\n' "$*"; }
ok() { printf '  \033[32mpass\033[0m  %s\n' "$*"; }
bad() { printf '  \033[31mfail\033[0m  %s\n' "$*"; failures=$((failures + 1)); }

cleanup() {
  # `wait` reaps each child, which is what stops the shell printing "Terminated"
  # after a successful run.
  [[ -n "$app_pid" ]] && { kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true; }
  [[ -n "$standin_pid" ]] && { kill "$standin_pid" 2>/dev/null || true; wait "$standin_pid" 2>/dev/null || true; }
  rm -f "$BODY_LOG"
}
trap cleanup EXIT

for tool in python3 curl node; do
  command -v "$tool" >/dev/null 2>&1 || { printf 'error: %s is required but not on PATH\n' "$tool" >&2; exit 1; }
done
if [[ ! -f "$SERVER_JS" ]]; then
  printf 'error: %s is missing. Build first: (cd apps/web && bun run build)\n' "$SERVER_JS" >&2
  exit 1
fi

exports() {
  curl -fsS "http://127.0.0.1:$STANDIN_PORT/__state" |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["export_requests"])'
}

printf '\nZeno web trace export verification\n\n'

python3 infra/observability/otlp-standin.py "$STANDIN_PORT" "$BODY_LOG" &
standin_pid=$!
sleep 1

# The env block is spelled out rather than exported, because the app port and the
# stand-in port are different and reusing one variable name for both silently points
# the exporter at the app itself -- a failure that looks like the exporter doing
# nothing at all.
log "starting the web tier against the stand-in"
env PORT="$APP_PORT" HOSTNAME=127.0.0.1 \
  OTEL_EXPORTER_OTLP_ENDPOINT="http://127.0.0.1:$STANDIN_PORT" \
  GCE_METADATA_HOST="127.0.0.1:$STANDIN_PORT" \
  GOOGLE_CLOUD_PROJECT=zeno-verification \
  node "$SERVER_JS" >/dev/null 2>&1 &
app_pid=$!

ready=0
for _ in $(seq 1 "$READY_TIMEOUT"); do
  if curl -fsS -o /dev/null -m 5 "http://127.0.0.1:$APP_PORT/" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 1
done
if [[ "$ready" -ne 1 ]]; then
  printf 'error: the web tier did not become ready\n' >&2
  exit 1
fi

# The request that is being traced. Whatever the page renders, the request itself
# must produce a root span, and the work it does must hang off that root.
log "requesting / (this is the trace under test)"
curl -fsS -o /dev/null -m 30 "http://127.0.0.1:$APP_PORT/" || true

log "waiting for the batch export"
for _ in $(seq 1 "$EXPORT_TIMEOUT"); do
  [[ "$(exports)" -gt 0 ]] && break
  sleep 1
done

# ---------------------------------------------------------------------------
# Assertions.
# ---------------------------------------------------------------------------
state="$(curl -fsS "http://127.0.0.1:$STANDIN_PORT/__state")"
read_state() { printf '%s' "$state" | python3 -c "import json,sys; print(json.load(sys.stdin).get('$1'))"; }

printf '\n  assertions\n'

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

# The payload is OTLP/JSON, so it is parsed rather than grepped, and the assertions
# are about the shape of the trace rather than the presence of a string.
python3 - "$BODY_LOG" <<'PY'
import json
import sys

path = sys.argv[1]
spans = []
service = None

with open(path, "rb") as handle:
    while True:
        header = handle.read(4)
        if len(header) < 4:
            break
        size = int.from_bytes(header, "big")
        payload = json.loads(handle.read(size))
        for resource_spans in payload.get("resourceSpans", []):
            for attribute in resource_spans.get("resource", {}).get("attributes", []):
                if attribute.get("key") == "service.name":
                    service = attribute["value"].get("stringValue")
            for scope_spans in resource_spans.get("scopeSpans", []):
                spans.extend(scope_spans.get("spans", []))

failures = 0


def check(condition, message):
    global failures
    if condition:
        print(f"  \033[32mpass\033[0m  {message}")
    else:
        print(f"  \033[31mfail\033[0m  {message}")
        failures += 1


check(service == "zeno-web", f"the resource names the service as zeno-web (saw {service})")

roots = [span for span in spans if not span.get("parentSpanId")]
check(len(roots) >= 1, f"the trace has a root span ({len(roots)} root(s))")

# A root span with children is the difference between "spans are exported" and "a
# trace is exported": a batch of unparented spans would satisfy the first and not
# the second.
root_ids = {span.get("spanId") for span in roots}
children = [span for span in spans if span.get("parentSpanId") in root_ids]
check(len(children) >= 1, f"work is nested under the root ({len(children)} child span(s))")

check(
    any(span.get("name", "").startswith("GET /") for span in roots),
    "the root span names the request",
)

names = sorted({span.get("name", "") for span in spans})
print("\n  span names")
for name in names[:12]:
    print(f"    {name}")

sys.exit(1 if failures else 0)
PY
payload_failures=$?
[[ "$payload_failures" -eq 0 ]] || failures=$((failures + 1))

printf '\n'
if [[ "$failures" -eq 0 ]]; then
  printf '  all assertions passed\n\n'
else
  printf '  %d assertion(s) failed\n\n' "$failures"
  exit 1
fi
