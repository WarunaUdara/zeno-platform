# 0003. PostgreSQL is authoritative for inventory

- Status: Proposed
- Date: 2026-10-06
- Owners: API and data

## Context

Concurrent ticket sales must not oversell. Caches and queues are useful for performance and asynchronous work but can be stale or duplicated.

## Decision

Use PostgreSQL transactions, constraints, and explicit idempotency records as the authority for reservation and ticket state. Any cache or broker is an optimization or delivery mechanism; it cannot independently approve a sale.

## Alternatives considered

- Redis-only decrement: low-latency but creates a second recovery and reconciliation authority.
- Queue-based reservations: useful at high contention but adds queueing latency and more complex user-visible outcomes before scale requires it.

## Consequences

The initial correctness path is inspectable and recoverable through database invariants. Hot inventory rows can become a bottleneck; admission control, bounded transactions, and load evidence come before introducing another authority.

## Evidence and revisit trigger

Revisit transaction shape and indexing after representative load testing identifies lock contention or unacceptable tail latency.
