# Local observability

The local stack uses Prometheus for metrics and alert evaluation, Grafana for dashboards, and `postgres_exporter` for database telemetry. The API exports request and worker/outbox metrics at `/metrics`. Prometheus also scrapes its own metrics and Grafana's built-in metrics endpoint. The exporter connects only over the Compose network; its port is not published to the host.

## Dashboards and endpoints

| View | Local URL | What it shows |
| --- | --- | --- |
| Zeno API | `http://localhost:3002/d/zeno-api` | Request rate, 5xx ratio, p95 latency, in-flight work, API pool, API target, expiry worker heartbeats, outbox depth and age |
| Zeno Database | `http://localhost:3002/d/zeno-database` | Exporter health, connection-limit use, active database sessions, database size, transactions, buffer-cache hit ratio, deadlocks, and locks by mode |
| Prometheus | `http://localhost:9090` | Targets, raw PromQL queries, active alert rules, and firing alerts |

Grafana's local default login is `admin` / `zeno_local_only`; set `GRAFANA_ADMIN_USER` and `GRAFANA_ADMIN_PASSWORD` in the local environment to override it. Prometheus and Grafana bind only to loopback in Compose. Change these controls before any shared deployment.

## Signal inventory

- **API:** request totals and duration histograms by normalized method/route/status, in-flight requests, and API SQLx pool size/idle count. IDs, emails, raw URLs, and request IDs are deliberately excluded from metric labels.
- **PostgreSQL:** exporter scrape/connection health, sessions by state, maximum connections, commit/rollback throughput, deadlocks, lock counts, buffer activity, table I/O, and database size.
- **Expiry worker:** persisted last-batch timestamp/result and consecutive failures per worker instance, plus pending due-job count and oldest due-job age.
- **Monitoring stack:** Prometheus and Grafana `/metrics` endpoints are scraped so their availability and internal metrics are visible alongside the application.

Prometheus uses a five-second local scrape/evaluation interval and keeps seven days of samples. These settings suit a short demo, not a production capacity estimate. Recording and alerting rules are tested with `promtool` in `infra/observability/tests/zeno.rules.test.yml`.

## First checks during an incident

1. In Prometheus **Targets**, confirm `zeno-api`, `postgres-exporter`, `prometheus`, and `grafana` are `UP`. If the exporter is up but `pg_up` is zero, inspect Postgres readiness and the exporter connection settings.
2. In **Zeno API**, compare request rate, 5xx ratio, and p95 latency. Check the route with errors and request volume before treating a low-volume percentage as an incident.
3. In **Zeno Database**, inspect connection-limit use, active/idle sessions, transactions, locks, deadlocks, and cache behavior. Compare with the API SQLx pool panel before increasing either pool.
4. For expiry alerts, compare heartbeat age and consecutive failures with due-job depth and oldest-job age. A healthy heartbeat with a growing queue points to throughput or queue-query pressure; a stale heartbeat points to the worker or database connection path.
5. Inspect structured service logs with `docker compose logs --since=10m api worker`. API responses include `X-Request-ID`; use it to correlate a client report with the matching JSON log event.

## Alert behavior and limits

Rules cover API target availability, sustained API 5xx/latency, missing/failing/stale expiry workers, expiry backlog/age, PostgreSQL exporter/database availability, high database connection use, deadlocks, and Prometheus/Grafana target health. The thresholds and short pending windows are demonstration triggers; tune them against load tests and service SLOs before production.

No Alertmanager receiver is configured. Firing alerts are visible in Prometheus but do not page or send notifications. API logs remain in the container logging backend and are available through `docker compose logs`; this setup does not yet provide centralized log retention, distributed traces, or alert delivery. Add those only with an explicit retention and access policy.
