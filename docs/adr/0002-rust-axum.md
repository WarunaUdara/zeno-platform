# 0002. Rust API with Axum

- Status: Proposed
- Date: 2026-10-06
- Owners: Product and API

## Context

The project needs a resource-aware service implementation where concurrency and
failure behavior are visible in the code, and where the runtime cost of the
choice is understood rather than assumed.

## Decision

Use Rust with Axum/Tokio for the API and worker. Keep domain logic independent of transport and asynchronous runtime. Avoid unsafe Rust. Do not add extra languages solely to advertise breadth.

## Alternatives considered

- Go: excellent cloud service tooling and faster onboarding, with broad SRE relevance; reconsider if Rust learning cost blocks the first reliable vertical slice.
- TypeScript end-to-end: faster shared-language web work, but less systems-language practice for the stated objective.

## Consequences

Rust demonstrates explicit error/resource handling but adds compile and ownership learning overhead. Measure implementation progress and operational behavior; do not imply the language alone makes the service faster.

## Evidence and revisit trigger

Revisit if delivery repeatedly stalls on Rust-specific complexity, provider SDK coverage is inadequate, or a benchmark shows no useful benefit for the measured resource constraints.
