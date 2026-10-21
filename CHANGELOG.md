# Changelog

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[intelligent versioning](https://handbook.gitlab.com/handbook/engineering/architecture/design-documents/software_development_lifecycle/).

## Versioning status

**No releases are tagged yet.** Everything below is `Unreleased` and lands on
`main`. The API is not version-stable: the OpenAPI document in
`api/openapi/v1.yaml` is the contract, and it may change between commits while
the product is pre-1.0.

Recording this explicitly is the point. A changelog that lists versions nobody
tagged would be a worse signal than an empty one.

## [Unreleased]

### Added

- **Registry**: organizations, memberships, and role-scoped access, so events
  belong to a tenant rather than to whoever created them.
- **Organizer authoring**: draft events, ticket types with capacity and pricing,
  versioned registration forms, publication readiness checks, and event cover
  images stored in object storage.
- **Public catalog**: a browseable list of published events and a public event
  page with capacity, pricing, and reservation terms.
- **Reservations**: 15-minute inventory holds with an outbox-driven expiry worker
  that returns abandoned holds to sale.
- **Orders and tickets**: confirmation issues one independently scannable ticket
  per seat, so a group booking can arrive separately. Each ticket carries an
  opaque credential for QR presentation, a SHA-256 digest used by the door, and
  an AES-256-GCM sealed copy used to re-display the code.
- **QR entry**: the ticket wallet renders each credential as an SVG QR code, with
  the short reference kept as a typed fallback.
- **Door console**: a check-in page that accepts a scanned credential or a typed
  reference and reports a specific outcome per scan, including "already admitted"
  with the original time.
- **Observability**: Prometheus metrics for request rate, latency, and status by
  route; database pool saturation; worker heartbeat, batch duration, and
  consecutive failures; and outbox depth and age. Two provisioned Grafana
  dashboards and fourteen alert rules, asserted by CI.
- **Trace correlation**: a `traceparent` header on every web-to-API call, so one
  identifier appears in both services' logs.
- **Infrastructure**: OpenTofu for Cloud Run API, web, and jobs, Secret Manager
  wiring with per-service identities, Cloud Scheduler, Cloud Monitoring policies,
  and an Artifact Registry with immutable tags.
- **Emulated GCP**: a local testbed that exercises the infrastructure plan and
  release gates without a cloud project or billing.
- **OpenAPI contract**: a typed client generated from the specification and
  checked in CI for drift.

### Changed

- Reservation candidate scanning, claim, and availability reads were reduced to
  index-only plans. Redundant indexes on the reservation path were removed rather
  than added to.
- The reservation expiry worker now reports its batch duration, which is what
  makes a slow batch visible instead of merely late.
- Documentation across `docs/` was corrected where it described planned behaviour
  as implemented, including claims that the product does not issue tickets.

### Fixed

- A 403 from the authentication provider surfaced only in the browser console, so
  a failed sign-up looked like a dead button. The form now reports the failure.
- Confirming a booking left the hold countdown and the "give up my spots" control
  on screen, offering to release inventory that had already been claimed.
- Admission is a conditional update, so two scanners racing on one credential
  produce one admission rather than two.

### Security

- Added a reproducible secret scan (`gitleaks` with a documented allowlist) and
  verified the full history contains no credentials.
- CI generates a per-run credential for its observability smoke test rather than
  relying on a committed development default.

## Notes on what this changelog is not

- It is not a list of commits. Commits explain intent; this explains what changed
  for anyone using the software.
- It is not a roadmap. See `docs/ROADMAP.md` for what is planned.
