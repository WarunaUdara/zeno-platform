# User journeys and customization

## Current implementation status

These journeys describe the target product and identify which pieces have shipped. The current attendee flow discovers a published event, reviews its published time, location, optional organizer-provided access/cancellation/contact details, and ticket options, and creates a 15-minute inventory hold that can be released early from its confirmation. Anonymous release requires the same high-entropy idempotency key used to create the hold. It does not complete a purchase, issue a ticket, collect attendee contact details, or deliver a receipt. With Neon Auth configured, the API can verify identity, create/list app-owned organizations, and let authorized members create/edit draft events, add retry-safe ticket types, and publish ready events. The web source includes Neon Auth sign-in/session handling and an organizer workspace/editor; provider-backed browser behavior and the complete signed-in journey have not been verified in this environment. Invitations, role administration, group orders, and event check-in are unimplemented. See the [implemented runtime](../architecture/IMPLEMENTED_RUNTIME.md) for verified scope.

## Journey 1 — attendee purchase (target; not yet implemented)

1. Discover a public event from a shareable event page or catalog search.
2. Confirm local date/time, venue/online link, accessibility details, total amount/currency, organizer contact, and refund terms before checkout.
3. Choose ticket type and quantity; see a brief, visible hold-expiry timer only if capacity is actually held, with a way to release an unwanted hold early.
4. Enter only the required purchaser/attendee fields. For a group purchase, choose named assignment now or secure claim links for attendees later.
5. Complete payment with the selected provider's hosted or tokenized flow. A return URL says “processing” until the server verifies provider state.
6. Receive a durable receipt and independently accessible ticket; assign or transfer only using an authenticated/verified claim.
7. View cancellation/transfer policy and support route without needing to log into the organizer console.

Measure checkout errors by stage and device/network class. Recover typed information locally only as necessary, with expiration and consent. Never show “ticket confirmed” before the authoritative order state is confirmed.

## Journey 2 — organizer publishes an event (editor implemented in source; provider flow unverified)

1. Create an organization and verify ownership/contact.
2. Choose a plan or trial and understand event, teammate, branding, and support limits.
3. Team invitations and event-specific role assignments are not available yet; do not share passwords or promise teammate access.
4. Create an event, set its supported schedule, location, and public details, optionally add accessibility, cancellation, and guest-support details in the collapsed attendee section, configure ticket prices, capacity, and sales windows, and publish when readiness checks pass. These optional details are public and organizer-supplied; they do not create payment, refund, or support workflows. Registration forms and brand themes are not yet available in the organizer UI.
5. A dedicated pre-publish attendee/mobile preview and editable share metadata are not implemented yet; verify the entered details before publishing, then use the public page to review the result.
6. Publish the event. The current demo treats publication as one-way: published event and ticket details are read-only, and there is no withdrawal or revision history. Before a real organizer pilot, provide a safe correction/withdrawal flow and a way to notify affected attendees; do not mutate published details silently.
7. Sales, delivery monitoring, attendee export, and immutable purchase history are not yet available because orders and tickets are not implemented.

## Journey 3 — company/group order (target; not yet implemented)

Buyer selects event/ticket type and quantity; UI explains who can assign or claim each admission and who receives order/support notices. The buyer can enter a bounded CSV or invite each attendee. An async job validates rows, reports line-level errors, and creates claimable or assigned records. Each attendee sees the valid event, ticket, consent choices, and transfer rules. The buyer receives order-level reconciliation but cannot inspect unrelated account details. A partial refund/cancellation identifies which admission units change.

Set maximum file size, rows, quantity, and per-organization/plan quotas. Prevent spreadsheet formula injection on exported CSV; protect claim URLs as bearer credentials; avoid loading full bulk files into API process memory.

## Journey 4 — scanner setup and entry (target; not yet implemented)

1. Authorized manager signs in, selects organization/event, gate, shift, and check-in mode.
2. Enroll or revoke a device; sync only event-scoped ticket status and necessary attendee lookup fields if the chosen mode requires it.
3. Scanner tests camera, haptic/text output, network, clock, power-saving behavior, and data retention before doors open.
4. Scan and receive clear success, used/revoked/wrong-event, partial-quantity remainder, or offline-provisional state.
5. Ask for manager correction only when policy requires it; reason and actor are recorded.
6. Close shift, sync queued records, resolve conflict report, and reconcile admitted quantity against valid entitlements.

Use large high-contrast targets, gloved-hand friendly controls, audio/haptic options, readable daylight screens, and a rapid manual lookup fallback. Never display more PII than an entry operator needs.

## Customization model

**Organizer editor:** logo, color/font tokens, supported layout blocks and ordering, event-specific content, field labels, ticket descriptions, published URLs, and approved image assets. Changes are previewable, versioned, publishable, and reversible. Accessibility constraints and contrast warnings remain part of every theme.

**Developer customization:** a stable headless API, generated TypeScript SDK, Web Components/React components with public props, signed outbound webhooks, documented OAuth/OIDC integration, and CSS design tokens. A developer may render the event experience in their own frontend and consume the same API. Keep code examples from accidentally exposing admin scopes to an attendee app.

Arbitrary HTML and JS are not part of the pilot. If custom CSS is added, scope it to an isolated subtree, validate size/content, prevent access to parent secrets, and retain platform baseline contrast/keyboard/navigation. For integrations, use Webhooks as the extension surface before loading third-party scripts into checkout.

## Service design principles

Show attendees price, status, terms, and next action clearly. Show organizers whether an operation is saved, queued, processed, or failed. Make destructive or financially significant controls explicit, reversible where practical, and permission-gated. Keep table filters in validated URL state for reproducible admin work. Optimize for mobile view, keyboard operation, screen readers, slow network, reduced motion, and event-day stress.
