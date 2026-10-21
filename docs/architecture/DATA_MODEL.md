# Data model and integrity

## Tenant isolation

Every organizer-owned row has a non-null `organization_id`. Queries begin with a server-derived authorized tenant context, then constrain by resource ID and tenant in SQL. Composite foreign keys include `organization_id` wherever two rows must belong to the same tenant. PostgreSQL row-level security is an additional defense where it fits transaction-scoped connection pools; it does not replace application authorization or safe connection reset behavior.

Use `UUIDv7` or another opaque sortable ID for internal/public records after verifying ecosystem support. Identifiers never grant access. Avoid sequential public IDs for attendee and order links. Use separate random, revocable credentials for ticket QR and invitation links.

## Core records

| Aggregate | Core records | Important unique/check constraints |
| --- | --- | --- |
| Identity | account, identity subject, verified contact method, device credential | Unique `(issuer, subject)`; no password storage in application |
| Organization | organization, membership, invitation, policy assignment, audit event | One active membership per account/organization; invitation hash unique and expiring |
| Event | event, published revision, venue, ticket type, sales window, theme revision, optional access/cancellation disclosures and public support email | Tenant/event FK; valid start/end; disclosure length and support-email checks; currency and capacity format fixed per ticket type |
| Inventory | inventory counter, hold, hold line | Nonnegative capacity/sold/held; unique idempotency scope; one valid transition per hold |
| Commerce | checkout, order, order line, payment attempt, refund request | External payment IDs unique per provider; order amount and currency snapshot; legal status transitions |
| Ticket | admission entitlement, ticket credential, assignment, transfer, revocation | Credential digest unique; at most one active owner; revoked credentials cannot be reactivated silently |
| Entry | check-in event, device, batch job, reconciliation record | Unique scanner action ID; admitted quantity never exceeds remaining entitlement; correction links original event |
| Subscription | plan version, subscription, entitlement snapshot, usage counter, provider event | Provider event ID unique; quota and ticket validity have separate lifecycles |
| Integration | API client, webhook endpoint, delivery attempt, message job | Secret digest/rotation ID; stable outbound event ID; attempt count and lease deadline bounded |

The current outbox is a narrow reservation-expiry queue: a reservation and its scheduled expiry job commit together, and the worker completes the job and inventory transition in one PostgreSQL transaction. It does not send email or webhooks. External delivery jobs still need leases, retry backoff, dead-letter handling, stable provider idempotency keys, and retention before pilot use.

The current schema now includes `organizations` and `organization_memberships`, keyed by verified identity issuer and subject. The first organization operation creates the organization and owner membership atomically; listing is filtered by both identity fields and uses a bounded keyset cursor. Existing seeded public demo events are attached to a system-owned demo organization during migration and remain non-editable. `events.organization_id` is mandatory so organizer writes cannot accidentally create tenantless events. Invitations, member revocation, role changes, and durable audit records are not implemented yet.

Organizer drafts may store plain-text accessibility details, cancellation/transfer terms, and a support email. These are optional public disclosures, exposed on the published event detail endpoint only when configured; public catalog and organizer event-list summaries omit them so free-form content and contact details are fetched only when someone opens a specific event. They do not represent an implemented refund, support, or accessibility-accommodation workflow.

Money is stored as integer minor units plus an ISO 4217 currency code, or an explicitly chosen decimal representation with fixed scale. Never use IEEE floating point. Snapshot name, price, currency, taxes, and terms relevant to an order at confirmation time so later event edits do not mutate a receipt.

## Order and hold state machines

Illustrative states:

```text
Checkout: created -> inventory_held -> payment_pending -> confirmed
                                |              |-> payment_failed -> released
                                |              |-> manual_review
                                |-> expired -> released
Order:    pending -> confirmed -> partially_refunded -> refunded/cancelled
Ticket:   unassigned -> assigned -> transferred -> revoked/used
Hold:     active -> confirmed | expired | released
```

Exact states and transitions depend on the payment provider and the selected country. Each transition has an actor (attendee, organizer, provider, worker, support), request/event ID, timestamp, and reason where it is an administrative correction. A provider callback cannot confirm an order that has no matching hold and expected amount/currency.

## Ticket and admission quantities

Separate an order line's purchased quantity from the individual credentials that can be presented. A group order creates claimable assignments or separately assigned attendee credentials. The purchaser cannot silently keep access to every assigned attendee's personal profile. Define transfer, refund, and unclaimed-ticket policies before generating durable credentials.

For multi-admission credentials, persist a bounded `admission_limit` and atomic `admitted_count`; each accepted scan requests quantity `q > 0` only when `admitted_count + q <= admission_limit`. The check-in event records quantity and scanner action ID so duplicate replays return the original result. If scan count is expected to be a set of individual guests, issue child ticket credentials instead and aggregate them under the order.

## Concurrency-sensitive operations

- Capacity holds use a guarded SQL update inside a transaction; do not `SELECT` availability then later decrement without a concurrency guard.
- Payment state transitions use conditional updates with an expected previous state and provider event uniqueness.
- Ticket transfer locks/updates the active assignment atomically and invalidates the old credential.
- Partial check-in consumes admission quantity with one compare-and-update; parallel scans for the same credential cannot exceed the limit.
- Bulk changes stage a job with a stable row key; each row has a result and idempotency identity. A bulk file import need not share one giant transaction.
- Worker leases include ownership and expiry. Workers can crash and retry; handlers must be idempotent.

Transactions should be short, use explicit isolation only where justified, and avoid network I/O. Record retryable serialization/deadlock outcomes. Test behavior at public application/database seams after the user approves those seams.

## Privacy, retention, and deletion

Store only attendee fields necessary for event delivery, legal receipts, accessibility requests explicitly opted into, and entry. Separate sensitive free-text accommodation data from broad exports. Restrict exports by purpose and permission; log export metadata without copying the content into audit logs. Define event retention, billing retention, and deletion obligations for the pilot region. A data export/deletion workflow must account for financial/audit retention before erasing linked records.

Never log QR secrets, bearer tokens, full email lists, payment credentials, invite codes, or accommodation notes. Use keyed digests for revocable credentials, apply retention to audit and delivery logs, and document backup expiry/deletion behavior.
