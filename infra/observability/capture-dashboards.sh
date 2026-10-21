#!/usr/bin/env bash
# Regenerate the dashboard screenshots described in the observability runbook against the local stack.
#
# This script is committed and the images are not, on purpose: a screenshot that
# nobody can reproduce is a claim nobody can check. The steps are deterministic,
# so the same commands produce the same pictures on another machine.
#
#   bun run stack:up                                     # once, in another shell
#   infra/observability/capture-dashboards.sh              # everything
#   infra/observability/capture-dashboards.sh --no-drill   # skip the chaos drill
#
# Requires k6, playwright-cli and python3 on PATH, plus a .env holding
# GRAFANA_ADMIN_USER and GRAFANA_ADMIN_PASSWORD.
#
# The Grafana password is templated into a Playwright script that is created with
# mode 0600 inside a temporary directory and removed on exit. It is never echoed,
# passed as an argument, or written to the output directory.
#
# Output lands in docs/dashboard-captures/, which is gitignored.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${OUT_DIR:-$REPO_ROOT/docs/dashboard-captures}"
GRAFANA_URL="${GRAFANA_URL:-http://127.0.0.1:3002}"
PROMETHEUS_URL="${PROMETHEUS_URL:-http://127.0.0.1:9090}"
DASHBOARD_PATH="/d/zeno-api/zeno-api"
SPIKE_RATE="${SPIKE_RATE:-40}"
SPIKE_DURATION="${SPIKE_DURATION:-120s}"
RECOVERY_WAIT="${RECOVERY_WAIT:-55}"
STALL_ALERT="ZenoExpiryWorkerHeartbeatStale"

run_drill=1
[[ "${1:-}" == "--no-drill" ]] && run_drill=0

log() { printf '  %s\n' "$*"; }
die() { printf '\nerror: %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Preconditions. Fail early and say exactly what is missing, because a capture
# script that silently produces an empty chart is worse than one that refuses.
# ---------------------------------------------------------------------------
for tool in docker k6 playwright-cli python3 curl; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required but not on PATH"
done

for svc in api web prometheus grafana worker; do
  state="$(docker compose ps --format '{{.Service}} {{.State}}' | awk -v s="$svc" '$1 == s {print $2; exit}')"
  [[ "$state" == "running" ]] || die "$svc is not running. Start the stack first: bun run stack:up"
done

[[ -f .env ]] || die ".env is required for the Grafana credentials"
# .env cannot be sourced here: its connection strings contain unquoted '&'.
grafana_user="$(grep -E '^GRAFANA_ADMIN_USER=' .env | head -1 | cut -d= -f2-)"
grafana_pass="$(grep -E '^GRAFANA_ADMIN_PASSWORD=' .env | head -1 | cut -d= -f2-)"
[[ -n "${grafana_pass:-}" ]] || die "GRAFANA_ADMIN_PASSWORD is not set in .env"
grafana_user="${grafana_user:-admin}"

work="$(mktemp -d)"
cleanup() {
  rm -rf "$work"
  # Never leave the worker paused, even if the drill was interrupted.
  docker compose unpause worker >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p "$OUT_DIR"

firing_alerts() {
  curl -fsS "$PROMETHEUS_URL/api/v1/alerts" | python3 -c '
import json, sys
data = json.load(sys.stdin)["data"]["alerts"]
for alert in data:
    if alert["state"] == "firing":
        print(alert["labels"].get("alertname", ""))
'
}

# The drill is only evidence if the alert actually fired. Wait for it rather
# than sleeping a fixed amount and hoping.
wait_for_alert() {
  local want="$1" deadline=$((SECONDS + 180))
  while (( SECONDS < deadline )); do
    firing_alerts | grep -qx "$want" && return 0
    sleep 5
  done
  return 1
}

wait_for_no_alert() {
  local want="$1" deadline=$((SECONDS + 180))
  while (( SECONDS < deadline )); do
    firing_alerts | grep -qx "$want" || return 0
    sleep 5
  done
  return 1
}

# Substitute the credentials into a Playwright script just before running it.
render_and_run() {
  python3 - "$1" "$grafana_user" "$grafana_pass" <<'PY'
import json, os, sys
path, user, password = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path) as handle:
    source = handle.read()
source = source.replace("__GRAFANA_USER__", json.dumps(user))
source = source.replace("__GRAFANA_PASS__", json.dumps(password))
with open(path, "w") as handle:
    handle.write(source)
os.chmod(path, 0o600)
PY
  ( cd "$work" && playwright-cli run-code --filename="$1" >/dev/null )
}

capture_dashboard() {
  cat > "$work/dashboard.js" <<JS
async page => {
  const USER = __GRAFANA_USER__;
  const PASS = __GRAFANA_PASS__;
  const DASH = '${GRAFANA_URL}${DASHBOARD_PATH}?kiosk&from=now-15m&to=now&refresh=5s';
  // A tall viewport keeps every panel on screen. A fullPage screenshot resizes
  // the page instead, which clears the canvas plots and yields empty panels.
  await page.setViewportSize({ width: 1920, height: 1560 });
  await page.goto(DASH);
  await page.waitForTimeout(4000);
  if (page.url().includes('/login')) {
    await page.fill('input[name="user"]', USER);
    await page.fill('input[name="password"]', PASS);
    await page.click('button[type="submit"]');
    await page.waitForTimeout(6000);
    await page.goto(DASH);
  }
  await page.waitForSelector('canvas', { timeout: 30000 });
  await page.waitForTimeout(14000);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForTimeout(3000);
  await page.screenshot({ path: '$1' });
}
JS
  render_and_run "$work/dashboard.js"
}

capture_alerts() {
  cat > "$work/alerts.js" <<JS
async page => {
  await page.setViewportSize({ width: 1500, height: 1250 });
  await page.goto('${PROMETHEUS_URL}/alerts');
  await page.waitForTimeout(7000);
  await page.screenshot({ path: '$1', fullPage: true });
}
JS
  ( cd "$work" && playwright-cli run-code --filename="$work/alerts.js" >/dev/null )
}

# ---------------------------------------------------------------------------
# 1. Grafana under load.
# ---------------------------------------------------------------------------
echo
echo "Capturing into $OUT_DIR"
echo

log "1/5  driving ${SPIKE_RATE} req/s for ${SPIKE_DURATION} and capturing the spike"
RATE="$SPIKE_RATE" DURATION="$SPIKE_DURATION" \
  k6 run --quiet --summary-export="$work/k6.json" infra/observability/spike-traffic.js \
  >"$work/k6.log" 2>&1 &
k6_pid=$!
sleep 25
capture_dashboard "$OUT_DIR/01-grafana-api-spike.png"
if ! wait "$k6_pid"; then
  die "the load run failed its threshold, so the spike screenshot would overstate things. See $work/k6.log"
fi
log "     $(python3 -c "
import json
s = json.load(open('$work/k6.json'))['metrics']
print(f\"requests={s['http_reqs']['count']:.0f} failed={s['http_req_failed']['value']*100:.2f}% p95={s['http_req_duration']['p(95)']:.1f}ms\")
")"

if (( ! run_drill )); then
  echo
  log "--no-drill given, stopping after the spike capture"
  echo
  exit 0
fi

# ---------------------------------------------------------------------------
# 2-4. Chaos drill: break it, prove the alert fired, then recover.
# ---------------------------------------------------------------------------
log "2/5  pausing the expiry worker on purpose"
docker compose pause worker >/dev/null
if wait_for_alert "$STALL_ALERT"; then
  log "     $STALL_ALERT is firing"
else
  die "$STALL_ALERT never fired; the drill proves nothing. Check the worker metrics in Prometheus."
fi
capture_alerts "$OUT_DIR/02-prometheus-alert-firing.png"
capture_dashboard "$OUT_DIR/03-grafana-worker-stalled.png"

log "3/5  unpausing the worker"
docker compose unpause worker >/dev/null
if wait_for_no_alert "$STALL_ALERT"; then
  log "     $STALL_ALERT has resolved"
else
  die "$STALL_ALERT did not resolve after the worker resumed."
fi
sleep "$RECOVERY_WAIT"
capture_alerts "$OUT_DIR/04-prometheus-alert-resolved.png"
capture_dashboard "$OUT_DIR/05-grafana-recovered.png"

echo
log "done. Files in $OUT_DIR"
echo
cat <<'CAPTIONS'
  Suggested captions, one per screenshot:

  01-grafana-api-spike.png
    "Load stepped up by an order of magnitude. p95 rose and stayed bounded, and
     the error-ratio panel stayed empty: the system degraded by getting slower,
     not by getting wrong."

  02-prometheus-alert-firing.png
    "One rule firing, thirteen inactive. The alert is specific, not a dashboard
     that turns red whenever anything moves."

  03-grafana-worker-stalled.png
    "The stalled worker's heartbeat age, climbing. This is the same fault as the
     rule above, seen from the metric instead of the alert."

  04-prometheus-alert-resolved.png
    "The same page after recovery: nothing firing."

  05-grafana-recovered.png
    "Detected, then recovered. The recovery frame is what separates this from a
     screenshot of a dashboard."

  Do not quote the laptop's absolute latency numbers in any of these. A screenshot
  is a claim: the chart has to show the traffic the caption describes.
CAPTIONS
