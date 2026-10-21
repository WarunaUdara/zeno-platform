# 0008. Zeno owns organization membership

- Status: Accepted for the first organizer slice
- Date: 2026-10-08
- Owners: Zeno project maintainers

## Context

Neon Managed Auth can verify user identity, but provider-managed organizations and roles are not a good domain boundary for Zeno's event-scoped permissions, account portability, or future identity-provider adapters. Organizer authorization must use current server-side membership state; a signed subject by itself grants no tenant access.

## Decision

Use the verified pair `(identity issuer, subject)` as the external principal key. Store organizations and memberships in Zeno's PostgreSQL schema. Create the first organization and its `owner` membership in one transaction. List only memberships matching both principal fields and use a bounded keyset cursor. A globally unique slug makes identical retries by the same owner safe; a slug owned by another principal returns a generic conflict. Require every event row to name an organization. Assign the existing synthetic public demo events to a system-owned organization with no human membership.

## Alternatives considered

- Neon-managed organizations: provider coupling and a currently limited role model make domain authorization difficult to extend consistently to event-level duties.
- Match on email: email may be changed or asserted differently across providers; use the issuer's verified subject as the stable external identity.
- Reuse an event ID or client-supplied organization ID as authority: public identifiers locate resources but do not authorize them.

## Consequences

Zeno can add invitations, organization-specific role policy, event assignments, and alternate OIDC issuers without migrating event ownership to another service. Membership reads add a database dependency and every privileged query must scope by the authenticated principal and organization. Current source supports organization creation/listing, issuer/subject-scoped event and ticket authoring, publication, and a Neon Auth web session/editor integration. Provider-backed browser behavior remains unverified; invitations, role management, audit trails, and event-scoped assignments remain future work. The system demo organization is deliberately not user-editable.

## Evidence and revisit trigger

The Neon Managed Auth verifier validates issuer and subject, and the organization integration test proves same-identity create replay, cross-identity list isolation, and slug-conflict behavior against PostgreSQL. Revisit if another identity provider or customer-owned identity system is adopted, or if measured membership-list query plans no longer use the principal index.
