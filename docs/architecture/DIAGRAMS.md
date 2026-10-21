# Architecture diagrams

Diagram sources live here as text so they are reviewable in a pull request and
cannot drift from the code the way an exported image does.

The [README](../../README.md) carries a condensed set of three -- the reservation
lifecycle, the trust boundary, and the deployed topology -- so a first-time reader
meets the shape of the system without following a link. Those three are summaries.
This file is the full catalogue and is authoritative where the two differ: it keeps
the credentials, the sequence detail, and the data model that the README leaves out
on purpose.

## How to render or edit these

| Tool | What to do |
|---|---|
| GitHub / GitLab | Paste a block into any Markdown file. It renders natively. |
| <https://mermaid.live> | Paste a block. This is the fastest way to edit. |
| Excalidraw | **More tools → Mermaid to Excalidraw**, paste a block, then style it. This is the path to a hand-polished diagram you can export as PNG or SVG. |
| Eraser, Lucid, Notion | Most accept Mermaid pasted directly, or import it as a starting point. |
| CLI | `bunx @mermaid-js/mermaid-cli -i file.mmd -o out.svg` |

`zeno-architecture.excalidraw` in this directory is a ready-to-open Excalidraw
scene for the hero diagram, if you would rather not do the Mermaid import.

## 1. System context

Who uses Zeno and what it depends on. Everything inside the boundary is this
repository; everything outside is a service somebody else operates.

```mermaid
flowchart LR
    attendee["Attendee<br/>browses, reserves, holds a QR"]
    organizer["Organizer<br/>creates events, publishes"]
    door["Door staff<br/>scans or types a code"]

    subgraph zeno["Zeno"]
        web["Web app<br/>event pages + organizer console"]
        api["Rust API<br/>authority for inventory"]
        worker["Expiry worker<br/>releases abandoned holds"]
    end

    auth["Neon Auth<br/>OIDC, session cookies"]
    db[("PostgreSQL<br/>system of record")]
    store["Object storage<br/>event cover images"]
    mon["Prometheus + Grafana<br/>metrics and alerts"]

    attendee --> web
    organizer --> web
    door --> web
    web --> auth
    web --> store
    web -->|"traceparent"| api
    api --> db
    worker --> db
    api -.->|"/metrics"| mon
    worker -.->|"/metrics"| mon
```

The API is not reachable from the internet in the deployed environment. Only the
web tier can call it, using a Cloud Run identity token. That is why the diagram
shows no direct edge from a person to the API.

## 2. Reservation lifecycle

This is the state machine that keeps a flash sale from overselling. The
interesting part is that "confirmed" and "expired" are mutually exclusive by
construction: confirmation retires the scheduled expiry job inside the same
transaction, so the worker can never release inventory that a guest has claimed.

```mermaid
stateDiagram-v2
    [*] --> held: reserve (seats move available → held)
    held --> confirmed: confirm booking<br/>(held → sold, expiry job retired)
    held --> expired: expiry worker<br/>(held → available)
    held --> released: guest gives up spots
    confirmed --> checked_in: door admits
    confirmed --> voided: organizer voids
    checked_in --> checked_in: rescan reports "already in"
    expired --> [*]
    released --> [*]
    voided --> [*]
    checked_in --> [*]
```

Two invariants hold at every point:

- A seat is counted in exactly one bucket: `available`, `held`, or `sold`. The
  conditional `UPDATE ... WHERE held_count >= n` is what makes concurrent claims
  safe without a lock held across the whole transaction.
- Admission is a conditional update on `tickets.status = 'valid'`, so two scanners
  racing on one credential produce one admission and one honest "already in".

## 3. Data model

The shape that matters: one hold produces one order, and one order produces N
independently check-in-able tickets. That is what lets a company buy ten seats and
have ten colleagues arrive separately.

```mermaid
erDiagram
    ORGANIZATIONS ||--o{ ORGANIZATION_MEMBERSHIPS : has
    ORGANIZATIONS ||--o{ EVENTS : hosts
    EVENTS ||--o{ TICKET_TYPES : offers
    EVENTS ||--o{ REGISTRATION_FORMS : asks
    REGISTRATION_FORMS ||--o{ REGISTRATION_FORM_FIELDS : contains
    EVENTS ||--o{ RESERVATIONS : receives
    RESERVATIONS ||--|| ORDERS : becomes
    ORDERS ||--o{ TICKETS : issues
    ORDERS ||--o{ REGISTRATION_ANSWERS : records
    TICKETS ||--o{ CHECK_INS : records

    ORGANIZATIONS {
        text id PK
        text name
        text slug
    }
    EVENTS {
        text id PK
        text organization_id FK
        text status "draft or published"
        timestamptz starts_at
        text timezone
    }
    TICKET_TYPES {
        text id PK
        int capacity
        int held_count "reserved, not yet sold"
        int sold_count "confirmed"
    }
    RESERVATIONS {
        text id PK
        text status "held, confirmed, expired, released"
        timestamptz expires_at
        int quantity
    }
    ORDERS {
        text id PK
        text reservation_id FK "UNIQUE - this is the idempotency key"
        text payment_status "seam; only ever not_required"
    }
    TICKETS {
        text id PK
        text order_id FK
        smallint sequence "ticket 3 of 10"
        text reference "typed fallback"
        bytea credential_hash "door lookup"
        bytea credential_ciphertext "re-display only"
        text status "valid, checked_in, voided"
    }
    CHECK_INS {
        bigint id PK
        text ticket_id FK
        text action "admit, undo, void"
        text scanner_action_id "makes a retry idempotent"
    }
```

`held_count` and `sold_count` are denormalised counters, not derived on read. That
is a deliberate trade: the read path stays constant-time during a flash sale, and
correctness is carried by conditional updates that cannot decrement below zero.

`credential_hash` and `credential_ciphertext` exist for different reasons and
different failure modes. The door looks tickets up by hash, so admission survives
a lost encryption key. Re-displaying a QR needs the ciphertext, so that does not.

## 4. Confirming a booking

Shows why a retried confirmation cannot create a second order, and why the expiry
worker cannot later steal the seats back.

```mermaid
sequenceDiagram
    autonumber
    participant A as Attendee
    participant W as Web
    participant P as API
    participant D as PostgreSQL

    A->>W: Reserve 2 tickets
    W->>P: POST /v1/events/{event_id}/reservations
    P->>D: UPDATE ticket_types SET held_count += 2<br/>WHERE held_count + sold_count + 2 <= capacity
    D-->>P: 1 row updated
    P->>D: INSERT reservations (status held, expires_at +15m)<br/>INSERT outbox_jobs (reservation.expire)
    P-->>W: Hold + idempotency key
    W-->>A: "Spots held. Time left 15:00"

    A->>W: Confirm booking
    W->>P: POST .../confirm (Idempotency-Key)
    P->>D: BEGIN
    P->>D: INSERT orders (reservation_id UNIQUE)
    Note over P,D: A replay hits the unique constraint<br/>and returns the original order
    P->>D: UPDATE ticket_types held_count -= 2, sold_count += 2
    P->>D: UPDATE reservations SET status = 'confirmed'
    P->>D: UPDATE outbox_jobs SET status = 'completed'
    Note over P,D: Retiring the job here is what stops<br/>the worker expiring a claimed order
    P->>D: INSERT tickets x2 (credential, hash, ciphertext)
    P->>D: COMMIT
    P-->>W: Order + tickets + credentials
    W-->>A: QR code per ticket
```

## 5. Admission at the door

```mermaid
sequenceDiagram
    autonumber
    participant S as Door staff
    participant W as Web console
    participant P as API
    participant D as PostgreSQL

    S->>W: Scan or type a code
    W->>P: POST .../check-ins (codes[], scanner_action_id)
    P->>D: lookup by credential_hash,<br/>else by reference
    alt ticket is valid
        P->>D: UPDATE tickets SET status = 'checked_in'<br/>WHERE id = $1 AND status = 'valid'
        D-->>P: 1 row
        P-->>W: admitted, seat number, name
    else already admitted
        P-->>W: already_admitted + when
    else voided
        P-->>W: voided
    end
    Note over P,D: The conditional UPDATE is the race guard.<br/>Two scanners, one admission.
    W-->>S: Colour + text + icon for the outcome
```

The console shows a specific outcome rather than a generic failure, because "it
did not work" at a door is not actionable and "already used, admitted at 21:07" is.

## 6. Reference deployment topology

```mermaid
flowchart TB
    user["Attendee / Organizer"]

    subgraph gcp["GCP project your-gcp-project · asia-southeast1"]
        lb["Cloud Run<br/>web service<br/>public"]
        apisvc["Cloud Run<br/>API service<br/>identity-gated"]
        job["Cloud Run job<br/>migrations"]
        exp["Cloud Run job<br/>reservation expiry"]
        sched["Cloud Scheduler<br/>drives expiry"]
        ar["Artifact Registry<br/>immutable tags"]
        sm["Secret Manager<br/>6 secrets"]
        cm["Cloud Monitoring<br/>alert policies"]
    end

    subgraph neon["Neon"]
        pg[("PostgreSQL")]
        auth["Neon Auth"]
        obj["Object storage"]
    end

    user -->|HTTPS| lb
    lb -->|"identity token"| apisvc
    lb --> auth
    lb --> obj
    apisvc --> pg
    job --> pg
    sched --> exp
    exp --> pg
    ar -.->|"pinned digest"| lb
    ar -.->|"pinned digest"| apisvc
    sm -.->|"mounted at boot"| lb
    sm -.->|"mounted at boot"| apisvc
    apisvc -.-> cm
```

Images are referenced by digest, not by tag, so a redeploy cannot silently pick
up a different build. The API service has no public ingress; the web tier proves
its identity with a token, which is why the API-to-internet edge does not exist.

## 7. Telemetry path

```mermaid
flowchart LR
    subgraph app["Application"]
        api["API"]
        worker["Worker"]
    end
    apim["/metrics<br/>zeno_* series"]
    prom["Prometheus<br/>scrape + recording rules"]
    rules["rules.yml<br/>14 alert rules"]
    graf["Grafana<br/>zeno-api, zeno-database"]
    logs["Structured JSON logs<br/>route, status, latency, trace_id"]

    api --> apim
    worker --> apim
    apim --> prom
    prom --> rules
    prom --> graf
    rules -->|"fires"| alertmanager["Alertmanager /<br/>Cloud Monitoring"]
    api --> logs
    worker --> logs

    web["Web tier"] -->|"traceparent"| api
```

One `traceparent` from the web tier is continued by the API, so a single trace ID
appears in both services' logs. See
[the observability runbook](../operations/OBSERVABILITY_RUNBOOK.md) for queries
and drills.

## Reading order

If you are reviewing this project rather than running it, this order explains the
most with the least context:

1. **Reservation lifecycle** — the correctness core, and the answer to "how do you
   not oversell a flash sale".
2. **Confirming a booking** — where idempotency and the expiry-retirement
   interaction actually live.
3. **Data model** — why one hold becomes N independently scannable tickets.
4. **Admission at the door** — the conditional update that makes concurrent scans
   safe.
5. **Deployed topology** — the blast radius of each component and its credentials.
