# Test seams for the first implementation slice

The first code slice should be small enough to validate one important guarantee: a ticket category with finite inventory cannot be oversold by concurrent checkout attempts, and retrying the same request does not create a second hold.

## Proposed public seams

1. `InventoryRepository::reserve(event_id, ticket_type_id, quantity, idempotency_key, now)` — transaction boundary for one ticket type and idempotency. Return a stable reservation result or a typed conflict/validation error.
2. `CheckoutService::reserve(command)` — application-level operation that validates quantity and sale window, calls the repository, and emits the reservation outcome without depending on HTTP or a cloud provider.
3. `POST /v1/events/{event_id}/ticket-types/{ticket_type_id}/reservations` — HTTP boundary in [`api/openapi/v1.yaml`](../../api/openapi/v1.yaml) that requires an idempotency key and maps domain outcomes to documented status/problem responses.
4. `MockPaymentProvider::authorize(order_id, amount, idempotency_key)` — deterministic provider seam for a later order/checkout slice; no network calls or real charges in local development.

## Observable invariants

- A reservation either commits the entire quantity or commits nothing; available inventory never becomes negative or exceeds capacity.
- Repeating the same idempotency key with the same request returns the same logical reservation.
- Reusing a key with a different request is rejected.
- A reservation against a closed sale window or invalid quantity creates no inventory side effect.
- Transaction rollback leaves neither partial inventory changes nor a misleading idempotency record.

## Slice boundary

Begin with domain and repository behavior for reserve/retry, then the HTTP adapter and local Postgres wiring. Keep payment, user interface, QR creation, and cloud provisioning out of this first test-driven slice. Implement only what the observable invariants require.

The TDD skill installed in this repository requires the developer to confirm the seams before writing tests. This file is the proposal for that review; implementation and tests should wait for explicit acceptance or edits to these public seams.
