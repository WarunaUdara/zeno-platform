# Threat model and security plan

## Assets

Ticket inventory and payments; account sessions and tenant membership; bearer QR credentials and admission records; attendee identity/contact fields; organizer API and webhook secrets; subscription entitlements; custom domains and theme content; backups and signing keys; cloud deployment identities; and event publication integrity.

## Trust boundaries

Attendee devices and browsers are untrusted. Organizer users may be compromised or malicious within assigned permissions. Scanner devices can be lost or network-partitioned. Payment/email/identity providers and arbitrary webhook receivers are separate systems. CI can deploy code and therefore is a privileged identity. An organization's theme/content is data and must not become executable code.

## Abuse cases and primary controls

| Abuse | Primary control | Evidence to retain |
| --- | --- | --- |
| Guess or alter tenant/event/order IDs | Default-deny object authorization; composite tenant constraints; opaque IDs | Cross-tenant access tests at approved seams; denial audit metadata |
| Oversell through flash-sale races or repeated checkout | Transactional capacity guard; expiring holds; idempotency; admission controls | Load scenario; order/hold/inventory reconciliation |
| Replay/copy QR at two gates | Online atomic consumption; device scope; offline risk model; duplicate alert | Concurrent scan experiment and accepted event record |
| Forge/replay provider webhook | Verify raw-body signature/timestamp; dedupe provider event; amount/tenant check | Signature/replay test and delivery record |
| Steal organizer or scanner credential | PKCE, secure storage, short lifetimes, device revocation, phishing-resistant MFA where supported | Session/device inventory and revocation drills |
| Exfiltrate PII with an export | Permission per export; async scoped job; expiry; audit; minimized columns | Export request/result audit, access review, purge proof |
| Abuse a webhook URL to reach cloud metadata/internal service | Scheme/host policy, DNS+redirect revalidation, block private ranges, egress controls | SSRF rejection cases and egress policy evidence |
| Execute code through custom event page | Versioned data-only theme schema; sanitized content; CSP | Stored/reflected XSS validation and CSP report |
| Compromise build/deploy identity or artifact | Repository-bound federated CI; immutable digest; pinned actions; later signature/attestation admission | Auth context and artifact provenance; rejected deployment evidence |
| Abuse operator access | Separate support roles; JIT/time-bound approval; data minimization; audit review | Break-glass drill and completed review |
| Corrupt/delete operational data | Separate access for backups; encryption; restore tests; migration rollback/forward plans | Restore report and backup policy check |

## Security lifecycle

Keep an owner and severity for each risk. Review dependency and image updates, SAST/secret scans, SBOM, TLS settings, cloud IAM, infrastructure plan changes, and vulnerability-response deadlines on every release. Threat-model checkout, ticket transfer, device/offline reconciliation, billing, public page customization, and export before their pilot use.

Security checks protect every tenant. Synthetic security review data stays separate from organizer data. Logs and monitoring do not become a second unauthorized PII database.
