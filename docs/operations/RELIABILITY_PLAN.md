# Reliability, load, and incident plan

This document defines candidate reliability targets and experiments. The product has not run these workloads yet; the targets must be checked against representative organizer traffic and cloud costs.

## User-facing SLI candidates

| Journey | SLI | Initial target for design review |
| --- | --- | --- |
| Browse a published event | Eligible requests return a correct published page | 99.9% over rolling 30 days; separate freshness bound for title/price/availability |
| Create/confirm an order | Successful valid purchase attempts complete without platform-caused error | 99.9% over rolling 30 days, payment-provider failures reported separately but included in user journey view |
| Check in online | Valid online scans produce an unambiguous outcome | 99.9% over rolling 30 days; no oversell/over-admission is a correctness invariant, not a percentage SLO |
| Find organizer event | Authorized searches return within a latency budget | p95 under 300 ms at a measured synthetic baseline, excluding remote user network and clearly specifying data size |
| Checkout API | Request/hold endpoint returns promptly, with external payment handled asynchronously | p95 under 500 ms in the stated load profile; end-to-end payment completion measured separately |
| Restore from backup | Known committed records and associated metadata are recovered | Candidate RPO <= 5 min and RTO < 30 min for pilot; prove in an isolated restore drill |

Every SLO needs an exact eligible-request denominator, exclusions, aggregation window, source, and owner. A testing target is not a customer SLA. Low traffic makes percentile and success ratios noisy; include request counts and confidence. Don't make error budgets disappear by excluding hard dependencies or attendees' failed journeys.

### Mathematical SLI & PromQL Definitions

```text
SLI 1: Event Browse Availability (Target: 99.9% over rolling 30 days)
Formula: sum(rate(http_requests_total{route="/v1/events/{event_id}",status=~"2.."}[5m]))
         / sum(rate(http_requests_total{route="/v1/events/{event_id}"}[5m]))

SLI 2: Reservation Hold Integrity & Availability (Target: 99.9% over rolling 30 days)
Formula: sum(rate(http_requests_total{route=~".*/reservations.*",status=~"2..|409"}[5m]))
         / sum(rate(http_requests_total{route=~".*/reservations.*"}[5m]))
Note: 409 INVENTORY_UNAVAILABLE is an expected business rejection when sold out, counted as good.

SLI 3: Reservation Hold Latency (Target: p95 < 500ms under nominal load)
Formula: histogram_quantile(0.95, sum(rate(http_request_duration_seconds_bucket{route=~".*/reservations.*"}[5m])) by (le))

SLI 4: Expiry Worker Heartbeat Freshness (Target: Heartbeat age < 30 seconds)
Formula: zeno_worker_heartbeat_age_seconds{job="zeno-api"} < 30
```

### Multi-Window Multi-Burn-Rate Alert Policy (Google SRE Standard)

| Alert Severity | Burn Rate | Short Window | Long Window | Budget Consumed | Notification Action |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Page (Critical)** | **14.4x** | 5 minutes | 1 hour | 2.0% | PagerDuty / On-Call immediately |
| **Page (High)** | **6.0x** | 30 minutes | 6 hours | 5.0% | PagerDuty / On-Call within 15 mins |
| **Ticket (Medium)** | **1.0x** | 2 hours | 24 hours | 10.0% | File an operational issue for next day |

### Capacity Optimization & Architectural Thresholds

- **API Database Connection Pool:** Minimum 1, Maximum 4 connections per Cloud Run container. `DATABASE_ACQUIRE_TIMEOUT_SECONDS=15` (prevents pool thrashing during Neon compute wake-up).
- **Worker Batch Sizing:** 100 outbox jobs per transaction (`ZENO_WORKER_BATCH_SIZE=100`), polling interval 1000ms (`ZENO_WORKER_POLL_INTERVAL_MS=1000`).
- **Deadlock-Free Locking Hierarchy:** Strict lock acquisition sequence in all write transactions:
  1. `ticket_types` row (`SELECT ... FOR UPDATE`)
  2. `outbox_jobs` row (`SELECT ... FOR UPDATE`)
  3. `reservations` row (`SELECT ... FOR UPDATE`)
- **Index Optimization Invariant:** The bounded outbox candidate scan in `worker.rs` must stay on an index. It reads the oldest due jobs through `outbox_jobs_pending_due` (`(available_at, id) WHERE status = 'pending'`) and aggregates only that clamped look-back window. Grouping the whole pending queue by `payload->>'ticket_type_id'` was measured at 41 ms with a 151k-row sequential scan on every poll, and no index can serve that grouping, so the bound in code is the optimisation rather than an index. Re-run `infra/perf/measure.sh` after any change to the worker query or the outbox schema.
- **Database Failure Classification:** Domain modules collapse every `sqlx::Error` into one opaque response so no SQL detail reaches a client. Each of those sites records the failure first, tagged with its enclosing operation and a SQLSTATE classification (`lock_not_available`, `query_canceled`, `too_many_connections`, `connection_failure`, `deadlock_detected`, `pool_timed_out`, `io_failure`). A spike in any single classification is the difference between "the database is saturated" and "the database is unreachable"; without it every 503 looks the same. Alert on these counts before adding latency alerts.

The API's `/metrics` endpoint exports request count, duration, in-flight requests, PostgreSQL pool gauges, worker heartbeat health, and due outbox queue depth/age in Prometheus text format. Request labels use normalized route templates and status codes; never add event IDs, tenant IDs, user data, or raw URLs as metric labels. Keep `/metrics` on a private scrape network or behind monitoring-specific access controls in deployed environments. The local Compose stack exposes Prometheus and Grafana only on loopback; its development Grafana credential must be replaced outside local use. A pinned `postgres_exporter` image adds database-level connection, transaction, lock, cache, table I/O, and size metrics. Its scrape credentials stay on the internal Compose network and reuse the local database role; use a dedicated least-privilege monitoring role for any shared or production database.

The local Prometheus rules precompute request rate, 5xx ratio, and p95 latency for dashboards, and flag a missing scrape target, sustained 5xx ratio above 5%, or p95 above one second. Error and latency alerts require more than one request per second to reduce noise. The API scrape also exports each reservation expiry worker's last batch timestamp, result, and consecutive failure count from persisted heartbeats. It reports due outbox queue depth and the age of its oldest job, excluding future scheduled expirations from backlog. The API dashboard shows heartbeat age, batch health, due queue depth, and oldest due job age. The database dashboard shows exporter health, sessions, connection-limit use, transaction throughput, buffer-cache hit ratio, deadlocks, and locks by mode. Alerts cover missing or failing workers, queue backlog, PostgreSQL availability, connection pressure, deadlocks, and Prometheus/Grafana scrape health. The short `for` windows are demo thresholds, not production paging policy or evidence that the candidate SLOs are met. Rules have `promtool` syntax and behavior tests. No Alertmanager receiver is configured, so these alerts are visible in Prometheus but do not notify an on-call destination. The 15-second worker heartbeat alert is a local demo threshold and should be tuned to the deployed worker poll interval.

## Error budget and alerting

Alert on user impact and sustained SLO burn. Use multi-window, multi-burn-rate rules for a high-volume service and tune notification to actual operational coverage. A fast demo scenario may shorten windows to show behavior, but report that it is an accelerated simulation, not 30-day SLO evidence. Page for ongoing rapid budget consumption; create a ticket for slow burn; dashboard capacity/saturation and no-data separately.

Define named service owner, event-day escalation and manual fallback before a real pilot. With no staffed on-call, do not promise 24/7 support. Provide organizers an accurate support window and event-day contact method.

## Capacity and backpressure

Capacity test the whole critical path and each limiting dependency. Profile hot PostgreSQL rows, connection pools, locks, index bloat, worker queue age, CPU, heap/RSS, allocator, file descriptors, and end-to-end p95/p99 latency. Test one popular event with a sold inventory cap, not only evenly distributed traffic across many events.

Proposed stages: establish a steady baseline; ramp arrival rate; sustain a flash sale; exceed capacity safely; recover after load. Record environment size, database tier, container limits, query plan, seed data, request mix, ramp, duration, tool revision, and raw results. State the first bottleneck and one measured remedy per iteration.

Apply per-IP and per-account controls plus event/tenant fairness. Limit database pool capacity to the database's actual connection budget across all replicas. Use timeouts and bounded retries with jitter, not retry storms. Push asynchronous email/export work into a bounded queue. Return `429` for caller limits and a transparent retry response for overload. Do not promise inventory based on stale cache.

## Game-day plan

| Scenario | Expected behavior | Evidence |
| --- | --- | --- |
| 100 seats / thousands of concurrent requests | No oversell; bounded response; stable sold-out result | Inventory/order/hold reconciliation; latency and rejection chart |
| Payment provider timeout and duplicate webhook | Order remains pending/recoverable; one confirmation side effect | State transition and provider-event audit |
| Worker killed during outbox expiry processing | The transaction rolls back; the pending job can be claimed again without releasing inventory twice | Pending/completed job state, reservation state, and inventory reconciliation |
| Database connections exhausted | Admission control and useful dependency alert; no replica crash loop | Pool saturation, readiness and recovery timeline |
| Bad release increases 5xx/latency | Rollout pauses/aborts and returns traffic to healthy version | Version-specific analysis data and exact timestamps |
| Database or region interruption | Documented degraded response and restore/failover steps | Incident timeline and actual RPO/RTO |
| QR scanned simultaneously at two gates | One permitted admission count; other action gets used/remainder result | Atomic check-in records and reconciliation |
| Device reconnects after offline duplicate | Conflict is visible and resolvable; no silently dropped scan | Signed event sequence and manual exception record |
| Subscription expires mid-event | Existing valid tickets remain verifiable; new plan-limited organizer writes follow grace policy | Entitlement transition and ticket checks |
| Restore plus changed code/schema | Backup is restored using compatible migration sequence and verified by invariants | Version, seed count, checksum, and operator notes |

Run one fault at a time against a throwaway environment. Record hypothesis, preconditions, injected failure, stop condition, observed effect, cleanup, owner, and follow-up. If a test damages non-test data, stop and preserve evidence.

## Incident and recovery documents

Keep one playbook per actionable alert and high-risk dependency: event sale overload, payment status mismatch, outbox backlog, scanner sync conflict, database capacity, identity provider outage, and failed restore. Each runbook starts with symptom and customer impact, safe read-only checks, mitigation and rollback, customer communication owner, data repair path, and post-incident evidence. Postmortems focus on contributing conditions and follow-up ownership rather than individual blame.

Backups must be encrypted, isolated from normal app deletion permissions, retained by policy, and restore-tested. Snapshot creation alone does not prove recoverability. Include encryption key availability, DNS/secrets, object blobs, database extensions, and audit records in restoration. Test a clean environment, not just restoring over a running database.
