# API design

## Contract surface

Use REST over HTTPS with `/v1` resource routes, JSON request/response bodies, OpenAPI 3.1, RFC 9457 problem details, UTC timestamps, opaque public IDs, and cursor pagination. Keep route handlers thin; define authentication, authorization, rate limits, body limits, timeout budgets, idempotency requirements, and audit behavior alongside each operation.

OpenAPI is the API source of truth because web, mobile, and external developers consume the same contract. Redocly validates the committed document in CI. Rust handlers and request types implement that contract; typed TypeScript clients are generated from it with `openapi-typescript` plus a small fetch client. Generated SDK files are never edited by hand. If Rust schema derives are used for local documentation, they do not overwrite the reviewed contract.

### Resource outline

The contract lives in [`api/openapi/v1.yaml`](../../api/openapi/v1.yaml). It covers public published-event reads, ticket-type availability, idempotent reservations and anonymous release of a reservation using its original key, a Neon identity probe, and authenticated organization list/create operations. Organization creation binds the verified `(issuer, subject)` identity as owner in the same transaction and retries by the same identity/name/slug safely. Organization list queries are membership-scoped and cursor-paginated. Event authoring, membership invitations and role changes, payment, assignment, check-in, webhooks, and bulk jobs remain future work; every operation must consume the server-derived tenant context rather than accept a client-supplied role.

Avoid RPC-like action routes unless the action has its own state transition, idempotency, and result semantics. Do not expose tenant database keys or internal object paths.

## HTTP and retry rules

- Use standard methods and status codes where their semantics fit. `POST` commands that may be retried after a timeout require a caller-supplied `Idempotency-Key` with a documented scope, retention window, and payload mismatch behavior.
- For authenticated commands, scope key uniqueness to principal plus operation. For anonymous reservation, scope it to the operation and key across all event routes; include event ID, ticket-type ID, and quantity in the request fingerprint. Clients generate at least 128 bits of cryptographic randomness and encode it as unpadded base64url. Persist the fingerprint, first response, and resource reference in the same transaction as the side effect. Replaying the same key and fingerprint returns the original response; a different fingerprint returns `409 IDEMPOTENCY_KEY_REUSED`. The hold flow collects no attendee contact data; transactional delivery must be introduced as a distinct, consented flow with a retention policy.
- Retain reservation idempotency records for 24 hours from the first accepted request, including after the hold expires. Return that absolute deadline in `Idempotency-Key-Expires-At`. Clients must use a fresh key after the deadline. The reservation hold has a separate, shorter expiry. A replay during the retention window returns the originally stored response, even if its `expires_at` has passed; clients should inspect that timestamp and start a new reservation with a fresh key if needed.
- Releasing an anonymous hold requires the same high-entropy key used to create it; the public reservation ID alone is not authorization. Make release retries safe and update the reservation, held inventory count, and expiry outbox job in one transaction. Keep release and expiry worker lock ordering aligned so only one path frees inventory.
- Use `ETag` / `If-Match` for editor changes where concurrent organizer edits should conflict instead of silently overwriting. Include revision numbers in theme and event drafts.
- Return `202 Accepted` and a job resource for genuinely large bulk imports, ticket issuance, or exports. A finished job returns per-row outcomes and stable counts; never claim that an asynchronous job has finished when only queued.
- Return `429` with `Retry-After` for caller-scoped rate limits. Return a retryable overload response when system capacity is exhausted. Distinguish permanent validation errors from transient dependencies.
- Sort every paginated result by a deterministic tuple, for example `(created_at, id)`, and sign/validate opaque cursor contents. Cap page size and reject malformed or stale cursors explicitly.
- Include `request_id` and `error.code`; do not return SQL, secrets, provider payloads, scanner tokens, or stack traces.

## Idempotency and webhook guarantees

Idempotency applies to the side effect, not just the response. Reservations, ticket transfer, bulk assignment, check-in, and refund commands require a defined uniqueness key and state machine. The anonymous reservation key scope, fingerprint, 24-hour retention, and replay behavior are defined above and on the OpenAPI operation. The first reservation is a database transaction over one ticket-type inventory row and the idempotency record: either the complete hold and stable response are stored, or nothing is. Multi-ticket-type holds will lock rows in deterministic ID order before their contract is introduced. Provider webhooks validate signature against raw bytes before parsing; enforce timestamp tolerance, provider event ID uniqueness, bounded body size, and tenant/payment-reference ownership. A webhook handler records receipt and enqueues state change durably before returning success.

Outbound webhooks contain a versioned event envelope, a timestamp, event ID, attempt number, and HMAC signature. Consumers may receive duplicates and must be able to replay missed events from a bounded retention window. Rotate signing secrets with overlap. Restrict callback URL schemes and resolved addresses to reduce server-side request forgery; revalidate DNS/redirect targets and block internal metadata networks.

## Public, organizer, and mobile API boundaries

Public event reads expose only organizer-approved fields from published versions and can be cached with an explicit version key. Organizer routes require membership and resource-level policy checks on every request, including exports and background jobs. Mobile routes use short-lived access tokens and rotating refresh credentials stored in platform secure storage. Do not place a privileged organizer API secret in a mobile binary.

The web/mobile SDK is generated from the same contract. Avoid using framework server functions as the source of truth for APIs shared by Rust, mobile, integrations, and event developers. The Rust API remains the contract owner.

## Compatibility and evolution

Add optional fields before required fields. Preserve old enum values and document unknown values. Once external clients are supported, make breaking changes with a new API version or explicit migration period and publish a deprecation date. The current v1 client is repository-local and pre-pilot; migration 0008 intentionally removes an unused purchaser field in a coordinated web/API release. Database migrations generally use expand/migrate/contract; privacy-driven removal is an explicit exception with a migration that protects inventory idempotency while removing the field.

## API version one review seams

Before implementing checkout, agree how to observe: (1) idempotent retries after a lost response, (2) competing reservations against exact capacity, (3) payment confirmation arriving before/after timeout, and (4) authorization failures across two organizations. Before check-in, agree on one QR credential, a partial quantity, duplicate replay, a revoked ticket, and offline sync conflicts.
