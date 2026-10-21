# Authentication, authorization, and IAM

## Separate four questions

1. **Authentication:** which person, workload, or device presented a valid credential?
2. **Tenant membership:** which organizations currently recognize that account?
3. **Authorization:** may this identity take this action on this resource in this context?
4. **Entitlement:** is the organization's plan entitled to the capability or volume of use?

None of these is implied by knowing an organization ID or having a valid access token. Authorization defaults to deny and runs on the server for every route, list, export, webhook action, and background job.

## Sign-in flow

- Use an OIDC-compatible identity provider. Zeno currently integrates Neon Auth for the browser; keep provider-specific identities behind the trusted `(issuer, subject)` principal key rather than using email as a domain identity. A different local or hosted provider should be added only with an explicit migration and token contract.
- Browser sign-in uses Authorization Code with PKCE. Keep browser tokens in secure, HttpOnly, SameSite cookies at the web boundary; use a BFF only if needed to keep tokens out of browser storage. If web calls the Rust API directly, document and implement a strict token/cookie/CSRF model.
- Native mobile sign-in uses system-browser Authorization Code + PKCE, app/universal-link redirect validation, short access-token lifetime, and rotating refresh tokens in Keychain/Keystore. Never embed a client secret in a mobile bundle.
- API access tokens validate issuer, audience, signature, expiry, authorized algorithm, and subject. Resolve current membership/role state server-side or use revision-aware short-lived claims so revoked members lose access promptly.
- Email ownership is established only through provider-verified claims or a verified one-time flow. Avoid homegrown password reset and MFA implementations.

## Roles and resource policies

Role templates are bundles of named permissions, not booleans scattered throughout screens. Initial templates: `organization_owner`, `organization_admin`, `event_manager`, `check_in_manager`, `analyst`, `billing_manager`, and `integration_developer`. One person can have several event-scoped assignments. The UI can hide actions for usability; API policy remains authoritative.

Model permissions as resource + action + scope. Examples: `event:publish` on event 123; `order:read_pii` on event 123; `checkin:admit` at gate A; `billing:manage` on organization X. Check the organization owner of each requested resource, then evaluate membership, event assignment, action, and relevant conditions. Avoid a global `admin` bit for ordinary operators.

Create organization owner succession, invitation expiry, email/domain restrictions, member removal, active scanner revocation, and audited role-change flows. Do not treat an event-level assignment as a way to change plan settings or read unrelated events.

## Platform operations and break-glass access

Keep organizer administration separate from platform operations. Support and site reliability accounts have no ordinary right to open attendee personal data. Emergency cross-tenant access must be time-limited, tied to an incident/ticket, approved by an authorized second person when feasible, limited to the necessary data/action, prominently audited, and reviewed after use. Expose operational health aggregates by default; reveal attendee content only when explicitly justified.

## Workload and device identities

- Each production workload has a distinct identity, namespace, environment, and minimum resource permissions. No shared project-wide administrator service account.
- CI obtains short-lived cloud credentials through repository-bound OIDC federation with restricted audience, branch/environment, and deployment target. Do not put long-lived service account JSON in GitHub secrets.
- A worker's runtime identity is distinct from CI identity. On GCP Cloud Run, use its dedicated service identity; if workloads move to GKE, use Workload Identity Federation for GKE. On AWS, use an ECS task role, App Runner instance role, or EKS Pod Identity/IRSA as appropriate. Do not replace workload identity with shared static keys.
- A scanner is enrolled to one organization and event assignment with a revocable device credential, platform secure storage, expiry, and user attribution. Device credentials cannot change event configuration or access billing.
- External API clients are per integration with narrow scopes, rotation and last-used audit metadata, expiry/revocation, and rate limits. Prefer OAuth client credentials/short-lived tokens over static personal tokens.

## Web security and audit

Use strict tenant-scoped SQL, parameterized queries, output encoding, CSRF protection on cookie-authenticated mutations, origin checks, CSP, CORS allowlists, security headers, request-size and timeout budgets, and server-side URL validation for webhooks. Never return access tokens in URLs or application logs. Secrets are retrievable only by the minimum runtime process that needs them.

The current Next.js app applies `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, and `Referrer-Policy: strict-origin-when-cross-origin` globally, and disables the framework-identifying `X-Powered-By` response header. A content security policy is still required before organizer-controlled rich event content is rendered. Add HSTS at the HTTPS edge after the deployed domain and TLS termination path are verified; do not enable it for the local HTTP preview.

Security audit events record actor/service/device, tenant, action, resource, result, reason/correlation ID, source class, and time. Do not place credentials, full QR values, card data, or excessive PII in the log. Preserve event history against normal tenant-admin edits and define retention/access review.

## Permission acceptance criteria

- A user belonging to organization A cannot read or mutate organization B by substituting IDs or following an exported URL.
- Event-limited staff cannot manage plan/billing, or other events, by changing the route, API body, cursor, job ID, or object key.
- A revoked member/device cannot act after the documented revocation window.
- A scanner credential cannot create events, issue refunds, or export attendee PII beyond its declared scope.
- Every privileged operation and support-access elevation leaves an attributable audit event.

## Current implementation boundary

The Rust API has an opt-in `GET /v1/auth/me` identity check for Neon Managed Auth tokens. It is enabled only when both `NEON_AUTH_BASE_URL` and `NEON_AUTH_JWKS_URL` are present. It pins EdDSA, validates issuer, expiry, optional not-before time and subject, and bounds JWKS response size and cache lifetime. Audience validation is disabled because the current integration assumes Neon tokens do not carry a configurable audience; verify this against an isolated provider configuration before treating the token contract as production-ready.

The web source includes Neon Auth sign-in/session routes, server-side token forwarding, protected organizer pages, and an organizer workspace/editor. This is implementation evidence only: no provider-backed browser sign-in, refresh/expiry, sign-out, or cross-tenant browser flow has been verified in the current environment. Keep organizer writes unavailable when Auth is missing or the provider cannot validate a session. Never enable Auth on a linked production branch as a side effect of local development.

### API permissions currently implemented

The role names below describe the current API predicates. They are organization-scoped; they do not imply event assignments, guest access, billing rights, or a membership-management UI. `analyst` and `check_in_manager` have no mutations in the present API.

| API operation | Anonymous | Authenticated user | Any current organization member | Owner / admin / event manager | Analyst | Check-in manager |
|---|---:|---:|---:|---:|---:|---:|
| Public published event/ticket reads | Yes | Yes | Yes | Yes | Yes | Yes |
| Anonymous reservation hold | Yes | Yes | Yes | Yes | Yes | Yes |
| `/v1/auth/me` | No | Yes | Yes | Yes | Yes | Yes |
| List organizations | No | No | Yes | Yes | Yes | Yes |
| Create organization and become its owner | No | Yes | Yes | Yes | Yes | Yes |
| List/read organization events | No | No | Yes | Yes | Yes | Yes |
| List organizer ticket types | No | No | No | Yes | Yes | No |
| Create/edit draft events or ticket types | No | No | No | Yes | No | No |
| Publish an event | No | No | No | Yes | No | No |

The API enforces these checks using the verified issuer/subject and current membership rows, with tenant and parent-resource IDs included in SQL predicates. Every membership can currently read its organization's event details; only the roles shown above can read organizer ticket configuration. No invitation, role-change/removal, ownership-transfer, event assignment, audit-log, guest-list, order, scanner, or check-in API is implemented. Do not expose controls implying those capabilities. Revisit the matrix with an organizer before expanding roles or data access.
