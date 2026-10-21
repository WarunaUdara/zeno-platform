# Isolated reservation load test

This profile builds the current API image and starts a separate, disposable PostgreSQL database with seeded demo inventory. It does not use the normal Compose stack, Neon, GCP, or any `.env` database URL. The API is exposed only on `127.0.0.1:18080`.

Start the isolated database and API:

```sh
docker compose --project-name zeno-loadtest -f infra/load-tests/compose.yaml up --build -d api
```

Run the bounded hot-ticket workload. The defaults start 25 reservation attempts per second for 30 seconds; both rate and duration are capped by the script. It refuses non-loopback targets. Inventory exhaustion (HTTP 409 `INVENTORY_UNAVAILABLE`) is expected; other application outcomes fail the run.

```sh
k6 run infra/load-tests/reservation-hot-ticket.js
```

After the run, verify the inventory invariant in the isolated database:

```sh
docker compose --project-name zeno-loadtest -f infra/load-tests/compose.yaml exec -T postgres \
  psql -U zeno_loadtest -d zeno_loadtest \
  -c "SELECT capacity, sold_count, held_count, capacity - sold_count - held_count AS remaining FROM ticket_types WHERE id = 'zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7L';"
```

The test profile does not start the expiry worker; created holds remain in this disposable database until it is removed. Review the k6 p50/p95/p99, iteration drops, accepted holds, expected inventory rejections, and the SQL remaining-inventory value together. Do not present this synthetic, resource-limited local run as hosted performance or an SLO.

Remove only the isolated test containers and database volume after preserving any result you need:

```sh
docker compose --project-name zeno-loadtest -f infra/load-tests/compose.yaml down -v --remove-orphans
```
