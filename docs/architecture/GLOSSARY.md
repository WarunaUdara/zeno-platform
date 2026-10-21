# Domain glossary

| Term | Definition | Avoid using it to mean |
| --- | --- | --- |
| Organization | A tenant that owns organizer users, events, plans, and integrations | Venue or legal company unless explicitly modeled |
| Organizer | An account acting for an organization with permission on a resource | The organization record itself |
| Event | A scheduled experience published by an organization | Ticket or order |
| Ticket type | A sellable admission category with a price, capacity, and sales window | Individual ticket credential |
| Admission entitlement | The right to admit a bounded number of people or visits under event policy | Payment receipt |
| Ticket credential | The revocable token presented to claim an admission entitlement | QR image; the image is only one representation |
| Order | A snapshot of a purchaser's requested/purchased line items, price, currency, and lifecycle | Inventory counter |
| Hold | A time-limited claim on capacity awaiting checkout completion | Confirmed sale |
| Check-in | An immutable accepted or rejected attempt to consume event admission | A permanent assertion that a guest entered if a correction is later recorded |
| Partial check-in | Consumption of fewer than all remaining admissions on an entitlement | Reuse of an identical scanner request |
| Scanner device | A registered device acting under a scoped event-entry policy | A user identity or global admin credential |
| Plan | A commercial product definition with versioned capabilities/limits | One hard-coded UI feature flag |
| Entitlement | The organization's currently effective right to use a named platform capability | Authorization to read another tenant's data |
| Idempotency key | A caller identity for making one side effect safe to retry | A transaction ID supplied by an untrusted browser without server scoping |
| Outbox event | A durable fact recorded with the business transaction and delivered asynchronously | A guarantee that a consumer receives it exactly once |
