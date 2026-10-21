#!/usr/bin/env bash
# Verify the OpenTofu configuration against a local Floci GCP emulator.
#
# Runs the checks that do not need real Google credentials and proves the plan and
# release gate behave. Creates no real cloud resource and spends nothing.
#
#   infra/gcp/verify-floci.sh
#
# Known limitation, re-checked on every run: `tofu apply` cannot create
# google_service_account resources against Floci. The Google provider creates them
# through the IAM admin v1 API, and Floci 0.6.0 implements only IAM v1. This is an
# emulator gap, not a defect in this configuration, so this script stops before
# apply and reports the plan instead.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STATE_BUCKET="zeno-floci-state"
EMULATOR="http://localhost:4588"

failures=0
step() { printf '\n\033[1m== %s\033[0m\n' "$1"; }
check() {
  if [ "$1" -eq 0 ]; then printf '  PASS  %s\n' "$2"; else printf '  FAIL  %s\n' "$2"; failures=$((failures + 1)); fi
}

step "Starting the Floci emulator"
docker compose -f "$REPO_ROOT/compose.floci.yaml" up -d >/dev/null
for _ in $(seq 1 30); do
  curl -fsS "$EMULATOR/_floci-gcp/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS "$EMULATOR/_floci-gcp/health" >/dev/null
printf '  Floci is healthy.\n'

step "Checking the emulated surface"
services=$(curl -fsS "$EMULATOR/_floci-gcp/health")
for service in cloudrun secretmanager iam iamcredentials scheduler logging monitoring gcs; do
  echo "$services" | grep -q "\"$service\":\"running\""
  check $? "$service is running"
done

step "Ensuring the remote state bucket exists"
curl -fsS -X POST "$EMULATOR/storage/v1/b?project=floci-local" \
  -H 'Content-Type: application/json' \
  -d "{\"name\":\"$STATE_BUCKET\"}" >/dev/null 2>&1 || true
curl -fsS "$EMULATOR/storage/v1/b?project=floci-local" | grep -q "$STATE_BUCKET"
check $? "state bucket $STATE_BUCKET exists"

step "Exporting emulator endpoints"
# shellcheck disable=SC1091
set -a
. "$REPO_ROOT/.env.floci.example"
set +a
export CLOUDSDK_API_ENDPOINT_OVERRIDES_STORAGE="$EMULATOR/storage/v1/"
export GOOGLE_OAUTH_ACCESS_TOKEN=floci

step "tofu fmt -check"
(cd "$REPO_ROOT/infra/gcp" && tofu fmt -check)
check $? "configuration is formatted"

step "tofu init against the emulated GCS backend"
(cd "$REPO_ROOT/infra/gcp" && tofu init -input=false -backend-config="bucket=$STATE_BUCKET" >/dev/null)
check $? "backend initialises against the emulator"

step "tofu validate"
(cd "$REPO_ROOT/infra/gcp" && tofu validate >/dev/null)
check $? "configuration is valid"

step "tofu plan"
plan_output="$(cd "$REPO_ROOT/infra/gcp" && tofu plan -input=false -var-file=terraform.floci.tfvars -no-color 2>&1)" || {
  echo "$plan_output" | tail -20
  check 1 "plan succeeded"
}
planned="$(echo "$plan_output" | grep -oE 'Plan: [0-9]+ to add, [0-9]+ to change, [0-9]+ to destroy' || true)"
[ -n "$planned" ]
check $? "plan produced a resource summary (${planned:-none})"

# A plan that reports destruction of an existing resource is the classic sign of a
# backend or credential mistake, so refuse to continue on one.
echo "$planned" | grep -qE '0 to destroy'
check $? "plan destroys nothing"

for expected in \
  "zeno-floci-api" \
  "zeno-floci-web" \
  "zeno-floci-runtime-database-url" \
  "zeno-floci-migration-database-url" \
  "zeno-floci-web-auth-cookie-secret"; do
  echo "$plan_output" | grep -q "$expected"
  check $? "plan includes $expected"
done

step "tofu test (production release gate)"
(cd "$REPO_ROOT/infra/gcp" && tofu test -no-color)
check $? "release gate assertions pass"

printf '\n'
if [ "$failures" -eq 0 ]; then
  printf '\033[32mAll Floci checks passed.\033[0m\n'
  printf 'Not verified here: a real apply. Floci implements IAM v1 only, while the\n'
  printf 'Google provider creates service accounts through IAM admin v1.\n'
else
  printf '\033[31m%s check(s) failed.\033[0m\n' "$failures"
  exit 1
fi