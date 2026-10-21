# Ticket credentials and venue check-in

## Threats to design against

A QR image can be screenshotted, forwarded, copied into a group chat, photographed at the gate, or scanned in parallel at two gates. A signed QR proves the issuer created the payload; it does not prove the person presenting it is the rightful owner or that another gate has not already admitted it. A short-lived token reduces the time it can be replayed but can fail after a legitimate attendee closes their wallet.

## Online mode is the pilot default

Generate a cryptographically random, high-entropy opaque QR credential. Persist only a keyed digest or otherwise non-reversible lookup value and credential version. Treat the QR as a bearer secret. QR URLs contain no email, name, seat, order price, or other personal data. Tickets can be revoked and reissued through an audited assignment flow.

An enrolled scanner authenticates with a short-lived user session plus device credential. The app verifies event and gate selection before showing the camera. The API checks event/sales/ticket state and atomically consumes admission units, storing a stable scanner action ID and result. Online checks prevent two separate scanners from accepting the same final remaining admission if they share the authoritative API/database.

The scan screen immediately shows a specific result (admit quantity, already used, wrong event, refunded/revoked, not yet valid, expired, manual review, or no connection). Color is reinforced with text/icon/haptic feedback for accessibility. Operators can search a narrow attendee index or enter a correction; exceptions require an action reason and audit entry.

## Offline mode and the unavoidable tradeoff

Two disconnected scanners cannot coordinate whether they are consuming the last admission for one copied credential. Strong offline acceptance and guaranteed global single-use are mutually incompatible without pre-partitioning a finite set of rights or later rejecting one device's scan. Do not promise fraud-proof or globally unique offline use.

If a later pilot requires offline mode, make it a per-event operational choice with an explicit risk acknowledgement. Issue signed admission credentials and allocate disjoint admission ranges/ticket subsets to enrolled devices before disconnection; device keys are registered and revocable. Queue signed, timestamped scan events under stable device action IDs. On reconnect, reconcile ordered event history, revoked credentials, clock skew, expired device leases, and conflicts; alert the organizer to duplicates and provide a resolution workflow. Define maximum offline duration and stale revocation exposure. If allocations are impractical, offline scans remain provisional until online reconciliation, with the duplicate-admission possibility documented.

Potential future implementation: an offline verifier could validate a compact Ed25519-signed ticket record using a public key distributed to the app. Private signing keys remain server-side or HSM-protected. Key rotation has key IDs and overlapping verification windows. A signed record still cannot reveal a later revocation or prevent duplicate admission.

## Partial and bulk admission

If one credential admits a group of `N`, each scan consumes requested quantity `q` only when `0 < q <= N - admitted_count`. The server commits the count and scan event atomically. Replaying the same scanner action ID returns the same response, not another admission. A response lost after commit is safe to retry. Independent action IDs compete through the remaining count guard.

Prefer one assigned ticket credential per named or claimable attendee when people may arrive separately or transfers/refunds must be granular. A group QR is appropriate only when the group enters together and the organizer understands the partial-admission behavior. The checkout and ticket page must display what the QR admits.

Bulk scanning/import is an asynchronous job with authorization frozen to a named event scope, uploader identity, input checksum, per-row idempotency keys, bounded file size, complete result counts, and an audit trail. Scan jobs cannot grant admission through a CSV export accidentally sent to another tenant.

## Check-in corrections

Keep accepted/rejected scan events immutable. A manager can add a correction/refund/re-entry or mark an entry as erroneous with a reason; this produces a linked correction fact and updates the derived attendance balance. Never delete a scan to hide a duplicate or tell an organizer a failed device write definitely did not reach the server. Reconciliation compares order entitlements, ticket credentials, accepted admissions, transfers/revocations, and corrections.

## Privacy and retention

QR contents and scanner logs are credentials/attendance data. Mask QR values in screenshots and app logs. Limit attendee search fields to the least amount needed at the entrance. Device downloads are encrypted by platform storage, automatically purged after the event's sync/retention interval, and revocable. Set a retention schedule for rejected scans and operational audit records after the first legal review.
