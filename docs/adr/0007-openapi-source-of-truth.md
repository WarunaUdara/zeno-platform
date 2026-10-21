# 0007. OpenAPI contract is the cross-client source of truth

- Status: Proposed
- Date: 2026-10-06
- Owners: API and client applications

## Context

Zeno serves a web app, a mobile scanner, and organizer/developer integrations. Generating an API contract only from Rust types risks coupling the public API to implementation details and leaves non-Rust clients dependent on informal behavior.

## Decision

Author the versioned OpenAPI 3.1 document first. Validate it in CI. Implement Rust request/response types and Axum handlers against the reviewed contract, and generate TypeScript SDK types/client code from that same document. Generated artifacts are not edited by hand and do not replace the contract.

## Alternatives considered

- Generate the public contract from Rust `utoipa` annotations: convenient implementation proximity, but less independent review and a weaker starting point for mobile/developer clients.
- Hand-maintain separate schemas for Rust, web, and mobile: creates drift and incompatible retry/auth semantics.

## Consequences

Contract changes receive API review before implementation. CI needs a pinned OpenAPI linter and later a generated-client drift check. The Rust implementation still validates runtime invariants; a valid schema does not prove behavior.

## Evidence and revisit trigger

Revisit if OpenAPI-first slows a single-team change without improving client consistency, or the chosen code generation cannot accurately model Rust/API constraints.
