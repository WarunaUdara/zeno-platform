# Observability and incident drills

This is the operator's runbook: how to see what the system is doing, how to prove
a claim with a query instead of an opinion, and how to break the system on
purpose so you can watch it recover.

Everything below is executable. Where a command needs a credential, it says which
one and where it comes from; nothing here carries a literal secret.

## What is instrumented

### Metrics

The API and the worker expose Prometheus metrics on `/metrics`. Every series has
the `zeno_` prefix so a scrape target's own metrics can never be confused with
application ones.

| Metric | Type | Labels | Answers |
|---|---|---|---|
| `zeno_http_requests_total` | counter | `route`, `method`, `status_code` | How much traffic, and how much of it failed |
| `zeno_http_request_duration_seconds` | histogram | `route`, `le` | Where the latency is, per route |
| `zeno_http_requests_in_flight` | gauge | | Whether requests are piling up inside the process |
| `zeno_database_pool_connections` | gauge | | How close the pool is to its ceiling |
| `zeno_database_pool_idle_connections` | gauge | | Whether capacity is idle or contended |
| `zeno_worker_last_batch_timestamp_seconds` | gauge | `worker_name` | Liveness of the expiry worker |
| `zeno_worker_last_batch_succeeded` | gauge | `worker_name` | Whether the last batch worked |
| `zeno_worker_last_batch_duration_seconds` | gauge | `worker_name` | Whether holds are released on time |
| `zeno_worker_consecutive_failures` | gauge | `worker_name` | Whether a failure is a blip or a trend |
| `zeno_outbox_due_jobs` | gauge | | Depth of scheduled work |
| `zeno_outbox_oldest_due_job_age_seconds` | gauge | | How late that work is |

`route` is the matched route template, not the raw path, so `/v1/events/{event_id}`
is one series instead of one per event. Cardinality stays bounded.

### Recording rules

Three rules precompute the expensive reductions so dashboards and alerts agree on
one definition:

- `zeno:http_requests:rate5m`
- `zeno:http_server_errors:ratio5m`
- `zeno:http_request_duration_seconds:p95_5m`

### Logs

The API emits one structured JSON line when a request starts and one when it
finishes. The finish line carries:

```json
{"fields": {"message": "finished processing request", "status": 200, "latency": "10 ms"},
 "spans": [{"name": "http_request", "route": "/v1/events", "trace_id": "…", "span_id": "…",
            "parent_span_id": "…", "request_id": "…"}]}
```

Database failures are logged with a SQLSTATE classification rather than a driver
string, so log search matches on cause instead of on wording. Error responses
never carry the database message or a stack trace.

### Trace correlation

The web tier sets a `traceparent` header on every call into the API, and it now
takes that header from its own span, so one browser request is one trace: the web
tier's request span, the render and fetch spans under it, and the API's
`http_request` span below those. The same trace ID is on every log line either tier
writes, so a trace and a set of logs are the same request:

```sh
gcloud logging read 'jsonPayload.spans.trace_id="TRACE_ID"' --project=your-gcp-project --limit=20
```

That identifier also opens a waterfall, so you can see where the time went rather
than only that the request happened:

```sh
curl -s -H "Authorization: Bearer $(gcloud auth print-access-token)" \
  "https://cloudtrace.googleapis.com/v1/projects/your-gcp-project/traces?pageSize=20&view=COMPLETE"
```

Read an empty list carefully before concluding that tracing is broken. Three things
produce one legitimately:

- **The service is idle.** Cloud Run scales to zero, and a revision with no traffic
  exports nothing. Send a request through the web tier and look again.
- **Health probes are not traced.** At a ten-second liveness interval they were 40 of
  45 spans in a sampled window, which left the traffic worth looking at as the
  exception.
- **Spans are batched.** The exporter flushes on a timer, so a request made seconds
  ago may not have landed yet.

Cloud Run also emits a platform span for every request -- `/component: HTTP load
balancer` for the API, `AppServer` for the web tier -- and those exist whether or not
this code does. What OTLP adds is the route template, the method, the request ID, and
the trace ID that ties a span to its log lines. The platform spans carry none of
that, so a trace list containing both is expected.

Export is opt-in: without `OTEL_EXPORTER_OTLP_ENDPOINT` the process runs untraced,
which is why local runs and CI produce no spans. `ZENO_OTLP_SAMPLE_RATIO` lowers the
ratio from its default of exporting everything, and sampling is parent-based, so the
root span decides once and every downstream span inherits that decision.

## Seeing it locally

```sh
bun run stack:up
```

| Surface | URL | Credential |
|---|---|---|
| Grafana | http://127.0.0.1:3002 | `GRAFANA_ADMIN_USER` / `GRAFANA_ADMIN_PASSWORD`, default `admin` / a local-only value |
| Prometheus | http://127.0.0.1:9090 | none |
| Web | http://127.0.0.1:3000 | none |
| Postgres | 127.0.0.1:5432 | `POSTGRES_USER` / `POSTGRES_PASSWORD` |
| API metrics | on the compose network | none |

Two dashboards are provisioned as code, so they cannot drift from the repository:
`zeno-api` and `zeno-database`. Both are asserted by CI, which fails if a panel is
deleted or renamed.

### Verifying span export without a cloud project

Both exporters need the same two things: a token from the metadata server, and
somewhere to post spans. Both are ordinary HTTP, so `infra/observability/` stands in
for them and the whole path becomes checkable on a laptop:

```sh
bun run db:up                                  # once, in another shell
infra/observability/verify-trace-export.sh     # the Rust API

(cd apps/web && bun run build)
infra/observability/verify-web-trace-export.sh # the Next.js tier
```

`GCE_METADATA_HOST` is the seam, and it is Google's own convention for relocating the
metadata client rather than a hook invented for testing -- a harness that needs a
production code path to bend for it is testing a different program.

The two assert different things because the two exporters speak different encodings.
The Rust one sends protobuf, so its payload is checked for the span name and the
caller's trace id, and the script says so rather than implying a decode. The Node one
sends OTLP/JSON, so its payload is parsed and the assertions are about the shape of
the trace: the resource names the service, there is a root span, and work is nested
under that root. That last one is the assertion worth having, because a batch of
unparented spans would satisfy "spans are exported" and not "a trace is exported".

Both run in CI.

### When spans do not appear

An export the endpoint rejects is silent unless something logs it, and that is worth
knowing before losing a day to it. The OTLP exporter's Node transport resolves a non-2xx
response to `{ status: 'failure' }` and logs nothing, so a rejected batch and a delivered
one are indistinguishable from inside the process: both are quiet, and the only
difference is that one set of spans exists.

That cost real time here. The tier built, started, served 200s, and put nothing in Cloud
Trace, with nothing in its logs either way -- which reads like an application fault and
is not one necessarily.

The web tier now says what it is doing, at a level nobody has to remember to set:

```text
otlp span export enabled: service=zeno-prod-web endpoint=https://telemetry.googleapis.com project=your-gcp-project
otlp export failed, 12 span(s) not recorded
```

Read them in that order:

- **No `span export enabled` line.** The instrumentation never ran. Export is opt-in, so
  check `OTEL_EXPORTER_OTLP_ENDPOINT` is set on the revision before looking anywhere else.
- **`enabled` present, `export failed` present.** The batch reached the exporter and was
  refused. The logged error carries the reason.
- **`enabled` present, no `export failed`, still no spans in Cloud Trace.** The batch never
  reached the exporter. Look at the flush rather than the POST: the default batch delay is
  five seconds and a Cloud Run instance that has scaled to zero does not get to spend it.

## Answering the questions that matter

Run these at http://127.0.0.1:9090. Each one is a question an on-call engineer
actually asks.

**Is the API up, and is it the target or the process that is wrong?**

```promql
up{job="zeno-api"}
```

`0` means the scrape failed. Check `zeno_http_requests_in_flight` next: a
non-zero value that never drains means the process is alive but wedged, whereas
a scrape failure with no traffic means it is not there at all.

**What is the error rate, among requests that are actually flowing?**

```promql
zeno:http_server_errors:ratio5m
```

The recording rule is deliberately a ratio rather than a count. A rate alert on a
quiet system fires on a single failure and trains people to ignore it.

**Which route is slow, not just "the API is slow"?**

```promql
topk(5, zeno:http_request_duration_seconds:p95_5m)
```

Because `route` is the template, this names one endpoint. Replacing the `sum by`
with `sum by (route, le)` is what makes that possible.

**Is the pool saturated or just busy?**

```promql
zeno_database_pool_connections - zeno_database_pool_idle_connections
```

Sustained at the maximum while `zeno_database_pool_idle_connections` sits at zero
is saturation. Both high means the pool is oversized for the traffic, not
exhausted.

**Is the expiry worker keeping up?**

```promql
time() - zeno_worker_last_batch_timestamp_seconds{worker_name="reservation-expiry"}
```

Holds last 15 minutes. If this exceeds roughly the batch interval, expired
inventory is not being returned to sale.

**Is scheduled work backing up?**

```promql
zeno_outbox_due_jobs
zeno_outbox_oldest_due_job_age_seconds
```

Depth without age is a burst. Age is the number that hurts, because it is how
late a guest's hold is released.

## Alert catalogue

Fourteen rules live in `infra/observability/rules.yml`. Each entry says what
fires it and the first thing to check, so the alert and the response stay
together.

| Alert | Fires when | Severity | First check |
|---|---|---|---|
| `ZenoApiTargetDown` | `up{job="zeno-api"} == 0` for 15s | critical | Is the API container running? Then `/metrics` reachability |
| `ZenoApiHigh5xxRatio` | 5xx ratio > 5% above 1 rps for 30s | warning | `topk(5, zeno:http_request_duration_seconds:p95_5m)`, then logs by `route` |
| `ZenoApiHighP95Latency` | p95 > 1s under sustained traffic for 30s | warning | Route breakdown, then `zeno_database_pool_connections` |
| `ZenoPostgresExporterDown` | `up{job="postgres-exporter"} == 0` for 15s | critical | Exporter container; this is a blind spot, not an outage |
| `ZenoPostgresUnavailable` | `pg_up == 0` for 15s | critical | Exporter reachable but cannot read Postgres: check the database itself |
| `ZenoPostgresConnectionsHigh` | > 80% of connection limit for 2m | warning | Pool gauges; look for a leak before raising the limit |
| `ZenoPostgresDeadlocks` | any deadlock rate over 5m, for 1m | warning | Query logs; deadlocks are a code path problem, not load |
| `ZenoObservabilityTargetDown` | Prometheus or Grafana unscrapable for 30s | warning | The monitoring itself, before trusting its silence |
| `ZenoExpiryWorkerMissing` | no heartbeat series at all | critical | The worker has never reported: wrong config or never started |
| `ZenoExpiryWorkerHeartbeatStale` | no completed batch for > 15s, for 15s | critical | Worker stalled; check `consecutive_failures` |
| `ZenoExpiryWorkerBatchFailing` | last batch failed, for 10s | warning | Classified SQLSTATE in logs |
| `ZenoExpiryWorkerBatchSlow` | batch > 1s for 2m | warning | Batch size against database latency |
| `ZenoOutboxDueJobsBacklog` | > 100 due jobs for 1m | warning | Throughput, not correctness |
| `ZenoOutboxOldestDueJob` | oldest due job > 30s old for 15s | critical | This one loses inventory; treat as a real incident |

`ZenoExpiryWorkerMissing` and `ZenoExpiryWorkerHeartbeatStale` are separate on
purpose. A missing series and a stale series look identical on a dashboard and
are completely different problems: one is a worker that never started, the other
is a worker that stopped.

## Breaking it on purpose

Alerts that have never fired are decoration. Each drill below produces one alert,
and then proves recovery. Run them locally.

Before you start, tail what you are about to cause:

```sh
docker compose logs -f worker
```

### Drill 1: stall the expiry worker

```sh
docker compose pause worker
```

Within about 30 seconds `ZenoExpiryWorkerHeartbeatStale` fires (critical), and
`zeno_outbox_oldest_due_job_age_seconds` climbs. Recover with:

```sh
docker compose unpause worker
```

The alert resolves without intervention. Watch `zeno_outbox_oldest_due_job_age_seconds`
fall back below 30s. If it does not, work genuinely backed up and the worker is
draining a queue rather than idle.

### Drill 2: make the worker's batches fail

```sh
docker compose exec -T postgres psql -U zeno -d zeno \
  -c "ALTER TABLE orders RENAME TO orders_offline;"
```

`ZenoExpiryWorkerBatchFailing` fires and `zeno_worker_consecutive_failures` rises.
The logs name the failure by SQLSTATE class rather than by message. Undo it:

```sh
docker compose exec -T postgres psql -U zeno -d zeno \
  -c "ALTER TABLE orders_offline RENAME TO orders;"
```

This is the drill that proves a failure is visible rather than silent. Note that
`ZenoExpiryWorkerMissing` does **not** fire here, because the heartbeat is still
being written; that distinction is the point of having both alerts.

### Drill 3: starve the database of connections

```sh
docker compose exec -T postgres psql -U zeno -d zeno \
  -c "ALTER SYSTEM SET max_connections = 12;" \
  -c "SELECT pg_reload_conf();"
```

Drive traffic, then watch `pg_stat_activity` against the limit and
`ZenoPostgresConnectionsHigh`. Revert with:

```sh
docker compose exec -T postgres psql -U zeno -d zeno \
  -c "ALTER SYSTEM RESET max_connections;" -c "SELECT pg_reload_conf();"
```

### Drill 4: generate server errors

Point the API at a database role that lacks permission on one table, then exercise
that route through the web tier. `ZenoApiHigh5xxRatio` fires only once the error
ratio clears the threshold under real traffic, which is exactly the behaviour you
want someone to see rather than read about.

### Drill 5: take a scrape target down

```sh
docker compose stop prometheus
```

`ZenoObservabilityTargetDown` fires. This drill is worth doing precisely because
it is the least satisfying: it proves that a *silent* monitoring system alerts on
its own silence rather than looking like a healthy system.

### After every drill

Confirm the system returned to its baseline rather than merely stopped alerting:

```promql
up{job="zeno-api"}
zeno_worker_last_batch_succeeded{worker_name="reservation-expiry"}
time() - zeno_worker_last_batch_timestamp_seconds{worker_name="reservation-expiry"}
```

## Performance evidence

`infra/perf/measure.sh` builds a disposable database on the real schema, loads
flash-sale volume, and prints the hot-path query plans. It refuses to run against
anything but `zeno_perf`, and drops the database on exit.

```sh
infra/perf/measure.sh              # current schema
APPLY_INDEXES=1 infra/perf/measure.sh   # compare with and without 0014
```

Read the result as buffer counts and plan shape, not as wall-clock on a laptop.
Node counts are the transferable number; the local timing floor is noise on any
single machine. See `docs/operations/LOAD_TEST_RESULTS.md` for recorded figures
and the exact conditions they were measured under.

The correctness property that matters under load is not throughput. It is that a
flash sale never oversells. That is a test assertion, not a dashboard: see the
concurrency tests in `apps/api/tests/`.

## In GCP

Locally the stack is self-hosted. In the deployed environment the same signals
come from managed services:

| Signal | Where | How to read it |
|---|---|---|
| Service logs | Cloud Logging | `gcloud logging read 'resource.type="cloud_run_revision"'` |
| Request latency and error counts | Cloud Run built-in metrics | Cloud Console, Cloud Run revision metrics |
| Alert policies | Cloud Monitoring | Latency, error rate, and expiry-job failure policies |
| Trace correlation | Log fields | `jsonPayload.spans.trace_id` joins web to API |
| Traces | Cloud Trace | Spans from all three workloads; the web tier's request span is the root of a browser request |

Two things to know before you trust the deployed alerting:

**A notification channel is configured, and delivery is unproven.** All five policies
reference an email channel. An unverified channel accepts a policy and delivers
nothing, so confirm the subscription from the recipient's inbox before treating any of
this as alerting. A policy that fires in the console has reached nobody, and the two
states are indistinguishable from inside the console.

**Health probes are not the same as application health.** `/health/live` returning
200 proves the process is up. It says nothing about the database, the worker, or
whether holds are expiring.

## Known gaps

Recorded so they are not mistaken for coverage.

- The web tier exports spans but no metrics. Its request rate and latency come from
  Cloud Run's built-in metrics rather than from the application, so there is no
  route-level breakdown and no dashboard panel built from it.
- Reserved instances are off. A cold API can exceed the web tier's request
  timeout, which surfaces as a briefly `unavailable` events page rather than a
  slow one.
- No synthetic probe against the public URL. Availability is observed by asking,
  not by monitoring.
- Nothing here measures the venue network, which is the failure mode the product
  is actually designed around.
