# 0006. Next.js for event web and organizer console

- Status: Proposed
- Date: 2026-10-06
- Owners: Web

## Context

Public event pages benefit from server rendering and metadata. The organizer console needs a well-supported React framework that can run as a standard container while a separate Rust API owns business rules.

## Decision

Use Next.js with server-rendered public pages and organizer UI, calling the versioned Rust API through generated clients. Use standard Node/container deployment paths and define shared caching/invalidation before horizontally scaling dynamic server caches.

## Alternatives considered

- TanStack Start: strong type-safe routing and loaders, but official docs currently identify its framework as Release Candidate and the employer ecosystem is smaller.
- SPA-only Vite: simpler deployment, but requires separately designing public rendering and metadata behavior.

## Consequences

The team must understand server/client boundaries, cache behavior, and serialized data. Framework-specific server functions do not become the domain API.

## Evidence and revisit trigger

Revisit if deployment constraints, measured UX, or ecosystem changes make another framework a better fit. Verify maturity from official documentation at review time.
