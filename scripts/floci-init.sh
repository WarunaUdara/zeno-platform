#!/usr/bin/env bash
set -euo pipefail

# Initialize local GCP resources in Floci (Port 4588)
# Zero cloud spend, 100% reproducible local environment

ENDPOINT="${FLOCI_ENDPOINT:-http://127.0.0.1:4588}"
PROJECT="${FLOCI_PROJECT:-floci-local}"

echo "==> Checking Floci GCP emulator health at ${ENDPOINT}..."
for attempt in $(seq 1 30); do
  if curl -fsS "${ENDPOINT}/_floci-gcp/health" >/dev/null 2>&1; then
    echo "==> Floci GCP is healthy and listening."
    break
  fi
  if [ "$attempt" -eq 30 ]; then
    echo "ERROR: Floci GCP did not become ready within 30 seconds."
    exit 1
  fi
  sleep 1
done

echo "==> Ensuring Secret Manager secrets exist..."
create_secret_if_missing() {
  local secret_id="$1"
  local initial_value="$2"

  local check_code
  check_code=$(curl -s -o /dev/null -w "%{http_code}" "${ENDPOINT}/v1/projects/${PROJECT}/secrets/${secret_id}" || true)
  if [ "$check_code" = "404" ]; then
    echo "    Creating secret: ${secret_id}"
    curl -fsS -X POST -H "Content-Type: application/json" \
      -d '{"replication":{"automatic":{}}}' \
      "${ENDPOINT}/v1/projects/${PROJECT}/secrets?secretId=${secret_id}" >/dev/null
    
    local encoded
    encoded=$(echo -n "${initial_value}" | base64)
    curl -fsS -X POST -H "Content-Type: application/json" \
      -d "{\"payload\":{\"data\":\"${encoded}\"}}" \
      "${ENDPOINT}/v1/projects/${PROJECT}/secrets/${secret_id}:addVersion" >/dev/null
    echo "    Created ${secret_id} with version 1."
  else
    echo "    Secret ${secret_id} already exists."
  fi
}

create_secret_if_missing "zeno-local-runtime-database-url" "postgres://zeno:zeno_dev_only@localhost:5432/zeno"
create_secret_if_missing "zeno-local-web-auth-secret" "zeno_local_cookie_secret_at_least_32_characters_long"

echo "==> Ensuring Cloud Storage buckets exist..."
create_bucket_if_missing() {
  local bucket_name="$1"
  local check_code
  check_code=$(curl -s -o /dev/null -w "%{http_code}" "${ENDPOINT}/storage/v1/b/${bucket_name}" || true)
  if [ "$check_code" = "404" ]; then
    echo "    Creating bucket: ${bucket_name}"
    curl -fsS -X POST -H "Content-Type: application/json" \
      -d "{\"name\":\"${bucket_name}\"}" \
      "${ENDPOINT}/storage/v1/b?project=${PROJECT}" >/dev/null
    echo "    Created bucket ${bucket_name}."
  else
    echo "    Bucket ${bucket_name} already exists."
  fi
}

create_bucket_if_missing "zeno-local-cover-images"

echo "==> Floci GCP local initialization complete!"
