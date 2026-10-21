# 0001. Modular monolith and context boundaries

- Status: Proposed
- Date: 2026-10-06
- Owners: Product and platform

## Context

One developer needs to deliver a coherent demo and pilot without taking on distributed transactions, duplicated deployment systems, or premature service operations. Ticketing still has meaningful domain boundaries and a background job workload.

## Decision

Start with a Rust modular-monolith API and separately deployable worker. Keep identity/access, catalog, orders/inventory, ticketing, entry, notifications, integrations, billing, and platform operations as explicit internal contexts with narrow contracts. Extract a service only when a measured scaling, isolation, ownership, or availability need justifies the operational cost.

## Alternatives considered

- Many microservices: higher deployment, tracing, consistency, and on-call cost without an independent team or scale evidence.
- One undifferentiated application: easy initially, but obscures inventory and authorization invariants.

## Consequences

The monolith requires deliberate module boundaries and dependency rules. The worker can scale separately for asynchronous jobs. Database transactions remain local to one deployment until a real extraction requires a new consistency design.

## Evidence and revisit trigger

Revisit after profiling shows resource contention, a context requires independent availability or release cadence, or a second team owns a boundary.
