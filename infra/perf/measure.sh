#!/usr/bin/env bash
# Build a disposable performance database on the real schema, load flash-sale
# volume, and print the hot-path query plans.
#
#   infra/perf/measure.sh            # measure current schema
#   APPLY_INDEXES=1 infra/perf/measure.sh   # measure after applying 0014
#
# The database is dropped on exit. Nothing here touches a shared or production
# database: the script refuses to run against anything but zeno_perf.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PERF_DB="zeno_perf"
COMPOSE=(docker compose -f "$REPO_ROOT/compose.yaml")

psql_perf() {
  "${COMPOSE[@]}" exec -T postgres psql -X -v ON_ERROR_STOP=1 -U zeno -d "$PERF_DB" "$@"
}

if [[ "${1:-}" == "--apply" ]]; then
  psql_perf -f - < "$REPO_ROOT/apps/api/migrations/0014_hot_path_indexes.sql"
  echo
  echo "Applied 0014_hot_path_indexes.sql"
  echo
fi

echo "Recreating $PERF_DB on the real schema..."
"${COMPOSE[@]}" exec -T postgres psql -X -v ON_ERROR_STOP=1 -U zeno -d postgres \
  -c "DROP DATABASE IF EXISTS $PERF_DB;" \
  -c "CREATE DATABASE $PERF_DB;" >/dev/null

DATABASE_URL="postgres://zeno:zeno_dev_only@127.0.0.1:5432/$PERF_DB" \
  cargo run --quiet -p zeno-api -- migrate >/dev/null

psql_perf -f - < "$REPO_ROOT/infra/perf/seed-load.sql" >/dev/null

# Index-only scans report heap-free reads only once the visibility map is set,
# which is autovacuum's job in production. Match that so the buffer counts here
# are comparable with a running service rather than with a freshly bulk-loaded
# table.
psql_perf -c "VACUUM (ANALYZE) outbox_jobs;" >/dev/null
psql_perf -c "VACUUM (ANALYZE) reservations;" >/dev/null
psql_perf -c "VACUUM (ANALYZE) ticket_types;" >/dev/null

echo "Row counts:"
psql_perf -t -c "
  SELECT 'reservations=' || (SELECT count(*) FROM reservations)
      || ' outbox_jobs=' || (SELECT count(*) FROM outbox_jobs)
      || ' ticket_types=' || (SELECT count(*) FROM ticket_types)
      || ' events=' || (SELECT count(*) FROM events);"

echo
psql_perf -f - < "$REPO_ROOT/infra/perf/explain-hot-paths.sql" \
  | grep -E "^==|Seq Scan|Index Scan|Index Only Scan|Bitmap|Execution Time|Buffers: shared"

echo
echo "Dropping $PERF_DB"
"${COMPOSE[@]}" exec -T postgres psql -X -U zeno -d postgres \
  -c "DROP DATABASE IF EXISTS $PERF_DB;" >/dev/null