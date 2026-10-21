# Zeno

Zeno is an open-source ticketing platform for events that have to stay correct
under pressure.

Organizers create an event, configure ticket types, publish it, and operate entry.
Attendees browse a catalog, hold inventory for 15 minutes, confirm a booking, and
each receive their own entry ticket with a QR code. Door staff scan a code, or type
the short reference when a camera will not cooperate, and the system reports a
precise outcome per scan.

The part worth your attention is not the feature list. It is that the feature list
stays correct under a flash sale, and that this is enforced by tests rather than
asserted in a README.

## The hard part

Three properties hold at every moment, and each one is checked by something you
can run:

**Inventory cannot be oversold.** A seat is counted in exactly one bucket:
available, held, or sold. Reserving and confirming are conditional updates that
cannot decrement below zero, so concurrent claims serialise instead of racing.
A parallel-reservation test asserts the invariant holds.

**A retried confirmation cannot create a second order.** There is no separate
idempotency table to keep consistent. `orders.reservation_id` is `UNIQUE`, so a
replay hits the constraint and returns the original order.

**Two scanners racing on one credential produce one admission.** Admission is a
conditional update on the ticket's current status, so the second write is a no-op
and the console reports "already in" with the original time rather than failing.

```mermaid
stateDiagram-v2
    [*] --> held: reserve<br/>available → held
    held --> confirmed: confirm<br/>held → sold, expiry job retired
    held --> expired: expiry worker<br/>held → available
    held --> released: guest gives up the spots
    confirmed --> checked_in: door admits
    confirmed --> voided: organizer voids
    checked_in --> checked_in: rescan reports "already in"
    expired --> [*]
    released --> [*]
    voided --> [*]
    checked_in --> [*]
```

Every arrow leaving `held` is a conditional update, so no two of them can both win:
a hold that expires while a confirmation is in flight resolves to exactly one of
the two outcomes, never both.

This is why one hold becomes N independently scannable tickets: a company buying
ten seats should not have to hand one code around at the door.

## Architecture

```mermaid
flowchart LR
    web["Next.js web tier<br/>the only public surface"]

    attendee["Attendee"] --> web
    organizer["Organizer"] --> web
    door["Door staff"] --> web

    subgraph authority["Inventory authority"]
        api["Rust API<br/>no public ingress"]
        worker["Expiry worker<br/>scheduled job"]
    end

    web -->|"identity token + traceparent"| api
    api --> db[("PostgreSQL<br/>system of record")]
    worker --> db
```

Only the web tier can reach the API, and only the API writes inventory, so a person
is never one hop from the database. The expiry worker shares the API's code and
schema but runs on its own schedule, which is what lets it be paused, starved, and
restarted in a drill without touching the request path.

- **Rust** with Axum and Tokio for the API and the separately runnable expiry
  worker.
- **PostgreSQL** as the system of record. Conditional updates are the concurrency
  primitive; nothing else is the authority for inventory.
- **Next.js** for public event pages, attendee booking, the ticket wallet, and the
  organizer console.
- **OpenAPI** as the contract, with a generated TypeScript client checked in CI
  for drift.
- **Cloud Run** on GCP as the deployment target, with Secret Manager, Cloud
  Scheduler, Cloud Monitoring, and an Artifact Registry using immutable tags.

A React Native scanner app is planned, not built. Redis is deliberately absent;
it would be a disposable cache and never the authority for inventory.

These are decisions with recorded tradeoffs. See the [ADRs](docs/adr/README.md),
[system architecture](docs/architecture/SYSTEM.md), and the
[architecture diagrams](docs/architecture/DIAGRAMS.md).

### Deployed topology

![The deployed topology: a public web tier, an API gated by Cloud Run IAM, Neon as the data plane, and OTLP spans to Cloud Trace](docs/architecture/images/deployed-architecture.png)

The shape that ran in production, as verified on 20 October 2026. GitHub Actions
builds and pins a digest into Artifact Registry, Cloud Run serves it, Secret
Manager mounts six version-pinned secrets at boot, and both tiers export OTLP
spans to Cloud Trace. The boxes marked *not deployed* are the local observability
stack: Prometheus and Grafana run on a laptop against the compose stack, and are
deliberately not a second metrics store in production.

## Repository map

```text
apps/
  api/                  Rust HTTP API, migrations, and the expiry worker
  web/                  Next.js event pages, wallet, and organizer console
api/openapi/            Versioned API contract
infra/observability/    Prometheus rules, rule tests, provisioned Grafana dashboards
infra/perf/             Flash-sale query-plan measurement
infra/gcp/              OpenTofu for the Cloud Run deployment
packages/api-client/    OpenAPI-generated TypeScript client
docs/                   Product, architecture, security, operations, decisions, ADRs
```

## Running it locally

Docker, Bun, and Rust (stable) are all you need. No cloud credentials.

```sh
cp .env.example .env
bun install
bun run stack:up
```

Compose starts Postgres, applies migrations, seeds synthetic inventory, and runs
the API, the expiry worker, the web app, Prometheus, Grafana, and a PostgreSQL
exporter. Each reservation writes its expiry job into PostgreSQL before commit;
the worker then claims due jobs in bounded batches and returns abandoned holds to
sale.

| Surface | URL |
|---|---|
| Web app | <http://localhost:3000> |
| Grafana | <http://localhost:3002> |
| Prometheus | <http://localhost:9090> |

Ports bind to loopback only. Override the local Grafana login with
`GRAFANA_ADMIN_USER` and `GRAFANA_ADMIN_PASSWORD`. If port 3000 is taken, run
`ZENO_WEB_PORT=3001 bun run stack:up`.

Running services from source instead:

```sh
bun install
bun run db:up
export DATABASE_URL=postgres://zeno:zeno_dev_only@127.0.0.1:5432/zeno
bun run api:migrate && bun run api:seed-demo && bun run api:dev
bun run --filter @zeno/web dev
```

## Verifying it

```sh
# Correctness: unit and database integration tests
DATABASE_URL=postgres://zeno:zeno_dev_only@127.0.0.1:5432/zeno cargo test --workspace

# Web: tests, types, lint, production build
cd apps/web && bun run test && bun run typecheck && bun run lint && bun run build

# Contract
bun run api:lint && bun run api:client:check

# Secrets, across every commit and the working tree
gitleaks git --config .gitleaks.toml --redact
```

`infra/perf/measure.sh` builds a disposable database on the real migrations, loads
flash-sale volume (200k reservations, 200k outbox jobs, 20k ticket types, 5k
events), prints the query plan for every reservation and expiry statement, and
drops the database on exit. That work moved the expiry worker's candidate
selection from a 41 ms sequential scan of the whole queue to a bounded 3.3 ms index
scan, the expiry claim from 3.03 ms to 0.50 ms, and availability from 1.77 ms to a
0.50 ms index-only read. Read those as plan-shape and buffer-count results; the
laptop's own latency floor is not a result.

`infra/gcp/verify-floci.sh` runs the OpenTofu configuration against a local GCP
emulator: formatting, validation, a 25-resource plan that destroys nothing, and all
five release-gate assertions, with no cloud credentials and no spend.

## Observability

The API reports request rate, latency, and status by route; database pool
saturation; worker heartbeat, batch duration, and consecutive failures; and outbox
depth and age. `route` is the matched template, so cardinality stays bounded and a
slow panel names one endpoint rather than "the API".

Two Grafana dashboards are provisioned as code and asserted by CI, so a removed
panel fails the build. Fourteen Prometheus alert rules cover API errors and
latency, worker liveness and batch health, queue backlog and age, PostgreSQL
availability, connection pressure, and deadlocks, and monitoring self-health.
Every web-to-API call carries a W3C `traceparent` taken from the web tier's own
span, so one browser request is one trace: the web tier's request span at the root,
its render and fetch spans below, and the API's span below those. The same ID is on
every log line either tier writes, and it opens a waterfall in Cloud Trace instead of
only proving the request happened. Export is off unless `OTEL_EXPORTER_OTLP_ENDPOINT`
is set, so local runs and CI stay untraced, and health probes are excluded on purpose
because at a ten-second interval they were the traffic.

Alert rules are tested, not just written:

```sh
docker compose exec -T prometheus promtool test rules /etc/prometheus/tests/zeno.rules.test.yml
```

And they are exercised against real failure, because an alert that has never fired
is decoration. The runbook contains five drills that break the system on purpose
and then prove recovery:

```sh
docker compose pause worker     # worker stalls -> heartbeat-stale fires, then resolves
docker compose unpause worker
```

See the [observability runbook](docs/operations/OBSERVABILITY_RUNBOOK.md) for the
queries, the alert catalogue, and every drill.

## Reference deployment

`infra/gcp` contains an OpenTofu reference deployment for Cloud Run: a public web
service, an identity-gated Rust API, a scheduled expiry job, a migration job,
Secret Manager, and Cloud Monitoring, backed by Neon for PostgreSQL,
authentication, and object storage. The configuration has been applied end to end
in a disposable project and destroyed afterwards; a fresh apply recreates the same
topology, down to the pinned secret versions.

Only the web tier accepts requests without an identity token. The API is deployed
with `INGRESS_TRAFFIC_ALL` but grants `roles/run.invoker` to the web service
account alone, so a request without a Cloud Run ID token is rejected by the
platform before it reaches the container.

```mermaid
flowchart TB
    user["Attendee / Organizer / Door staff"]

    subgraph gcp["GCP · your-project · asia-southeast1"]
        web["Cloud Run<br/>web service<br/>public"]
        api["Cloud Run<br/>API service<br/>identity-gated"]
        migrate["Cloud Run job<br/>migrations"]
        expire["Cloud Run job<br/>expire holds"]
        sched["Cloud Scheduler"]
        ar["Artifact Registry<br/>immutable tags"]
        sm["Secret Manager"]
    end

    subgraph neon["Neon"]
        pg[("PostgreSQL")]
        auth["Neon Auth"]
        obj["Object storage"]
    end

    ci["GitHub Actions<br/>push to main"] -.->|"build + push"| ar
    user --> web
    web -->|"identity token"| api
    web --> auth
    web --> obj
    api --> pg
    migrate --> pg
    sched --> expire
    expire --> pg
    ar -.->|"pinned digest"| web
    ar -.->|"pinned digest"| api
    sm -.->|"mounted at boot"| web
    sm -.->|"mounted at boot"| api
```

Images are referenced by digest rather than by tag, so a redeploy cannot quietly
pick up a different build.

Delivery is continuous when the deployment is configured. A push to `main` builds
both images natively on `ubuntu-latest`, runs the migration job, rolls all four
workloads forward to the digests it just pushed, and smoke-tests the web tier.
GitHub Actions authenticates with Workload Identity Federation pinned to the
deploying repository, so no long-lived cloud key exists in the repository or in
GitHub, and `main` can require an approving review so the pipeline cannot be
started by a direct push.

The runtime connects as a dedicated least-privilege role with DML and sequence
access and nothing else. Schema changes run under a separate owner credential that
only the migration job mounts, and `CREATE TABLE` is refused for the runtime.

The reference deployment was exercised end to end: sign-up, workspace creation,
event and ticket-type authoring, publication, catalog browsing, a 15-minute
reservation, confirmation issuing a per-guest ticket, and door check-in including
the double-scan case.

**This is a reference implementation, not a production service.** Known limits of
the reference topology:

- Cloud Monitoring alert policies require a verified notification channel; an
  unverified channel notifies nobody.
- Without a custom domain there is no load balancer and no Cloud Armor in front of
  the web tier.
- Reserved instances are off, so a cold API can exceed the web tier's request
  timeout. The events page briefly reports `unavailable` rather than rendering
  slowly.
- The web tier exports traces but no metrics, so its request rate and latency come
  from Cloud Run rather than from the application.
- The runtime reaches PostgreSQL over the public endpoint. There is no Serverless
  VPC Access connector and no VPC Service Controls perimeter, so the database
  connection is authenticated and TLS-terminated but not contained.
- Infrastructure changes are applied with `tofu apply`; only the image is rolled
  out by CI. A merged infrastructure change is not live until someone applies it.

See [`docs/operations/SECRET_ROTATION.md`](docs/operations/SECRET_ROTATION.md) for
the credential posture and the rotation procedure.

## Documentation

| Start here | |
|---|---|
| [Architecture diagrams](docs/architecture/DIAGRAMS.md) | System context, lifecycle, data model, deploy topology |
| [Observability runbook](docs/operations/OBSERVABILITY_RUNBOOK.md) | Queries, alerts, incident drills |
| [Order and entry](docs/architecture/ORDER_AND_ENTRY.md) | How confirmation and admission work |
| [Threat model](docs/security/THREAT_MODEL.md) | Trust boundaries and assumptions |
| [Secret rotation](docs/operations/SECRET_ROTATION.md) | Including what each rotation breaks |
| [Security policy](SECURITY.md) | Reporting, and the threat assumptions |
| [Security audit](docs/security/SECURITY_AUDIT.md) | Pre-publication review and residual risks |
| [ADRs](docs/adr/README.md) | What was evaluated and declined |
| [Roadmap](docs/ROADMAP.md) | What is planned |

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Participation is covered by
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Report vulnerabilities privately per
[`SECURITY.md`](SECURITY.md).

## License

GNU Affero General Public License v3.0. See [`LICENSE`](LICENSE).

AGPL was chosen deliberately over a permissive licence: the source is fully open
and anyone may run it, but operating a modified version as a network service
requires publishing those modifications. That keeps the option of commercialising
this work open, which a permissive licence would give away. Dependencies keep their
upstream licences.
