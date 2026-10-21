# Roadmap

Zeno is pre-1.0. The API contract may change between commits, and there are no
tagged releases yet. This roadmap lists the work that is either in progress or
required before a small organizer could run a real event on it.

A phase is complete only when its stated evidence exists. Nothing here is a
release promise.

## Done

- **Runnable foundation:** Bun and Rust workspaces, local PostgreSQL and
  migrations, a Rust/Axum API and expiry worker, Next.js web app, OpenAPI
  contract with a generated client, Docker Compose stack, and CI.
- **Organizations and authoring:** tenant-scoped organizations and memberships,
  role checks, draft events, ticket types, versioned registration forms,
  publication readiness, and event cover images in private object storage.
- **Reservations:** 15-minute inventory holds with an outbox-driven expiry
  worker, conditional updates that cannot oversell, and idempotent retries.
- **Orders and tickets:** confirmation issues one independently scannable ticket
  per seat, with a SHA-256 digest for lookup and an AES-256-GCM sealed copy for
  re-display.
- **Entry:** a door console that accepts a scanned credential or typed reference
  and reports a specific outcome per scan, including repeat admission.
- **Observability:** Prometheus metrics, provisioned Grafana dashboards, tested
  alert rules, OTLP trace export, and an operations runbook with failure drills.
- **Reference deployment:** OpenTofu for Cloud Run, Secret Manager, Cloud
  Scheduler, Cloud Monitoring, and an Artifact Registry with immutable images.

## Next

### Payments and refunds

Add a payment-provider adapter, signed webhook processing, receipts, refunds,
cancellations, and the reconciliation needed to prove an order and a payment
agree. A free-event flow does not need this; a paid pilot does.

**Exit evidence:** a sandbox payment completes, a duplicate webhook is a no-op,
a refund is reflected in ticket state, and reconciliation catches a deliberately
mismatched order.

### Offline and adversarial entry

Harden the door path for poor networks and repeated scans: device enrollment,
explicit gate selection, a manual-lookup correction flow, and an offline mode
only after the tradeoffs around stale revocation and duplicate admission have
been accepted and tested.

**Exit evidence:** a documented drill for each exception, including a device that
was offline across a revocation.

### Operations proof

Close the gaps listed in the README: a verified alert destination, a completed
restore and incident exercise with measured RTO/RPO, and a rollback rehearsal.

**Exit evidence:** a written report for each exercise, with the commands and the
observed result.

### Accessibility and internationalisation

Complete keyboard and screen-reader journeys for booking, wallet, organizer, and
door flows. Move user-facing copy behind a message catalogue before adding a
second locale.

**Exit evidence:** an accessibility review with reproducible findings, and a
locale switch that changes every user-facing string.

## Later

Advanced analytics, wallet passes, seat maps, wait lists, discount codes,
calendar integrations, additional identity providers, and additional cloud
adapters. These are candidates, not commitments; priority should come from real
organizer feedback and measured load rather than from a feature checklist.
