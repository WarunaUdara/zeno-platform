-- Orders and tickets.
--
-- A reservation holds inventory for 15 minutes. Confirming it converts the hold
-- into an order and issues one ticket row per unit, so a company that bought ten
-- tickets has ten independently check-in-able tickets. That independence is the
-- whole point: partial arrival is the normal case, not an edge case, so no
-- attendance state lives on the order.
--
-- Payment is deliberately not implemented. `payment_status` exists as a seam so a
-- future payment adapter can drive an order through a payment lifecycle without
-- rewriting orders or backfilling confirmed bookings. No code reads or writes a
-- value other than 'not_required'.

ALTER TABLE reservations
    DROP CONSTRAINT reservations_status_check,
    ADD CONSTRAINT reservations_status_check
        CHECK (status IN ('held', 'confirmed', 'expired', 'released'));

CREATE TABLE orders (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    organization_id text NOT NULL,
    event_id text NOT NULL,
    ticket_type_id text NOT NULL,
    -- One order per hold. This unique constraint *is* the idempotency key for
    -- confirmation: a retried confirm cannot create a second order, so there is no
    -- separate idempotency table to keep consistent.
    reservation_id text NOT NULL UNIQUE REFERENCES reservations (id) ON DELETE RESTRICT,
    quantity smallint NOT NULL CHECK (quantity BETWEEN 1 AND 100),
    total_amount numeric(20, 4) NOT NULL CHECK (total_amount >= 0),
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    purchaser_name text NOT NULL CHECK (char_length(btrim(purchaser_name)) BETWEEN 1 AND 120),
    purchaser_email text NOT NULL CHECK (char_length(purchaser_email) BETWEEN 3 AND 254),
    status text NOT NULL DEFAULT 'confirmed'
        CHECK (status IN ('confirmed', 'canceled')),
    -- Seam for a future payment adapter. Nothing in this repository sets any
    -- value other than 'not_required'.
    payment_status text NOT NULL DEFAULT 'not_required'
        CHECK (payment_status IN ('not_required', 'pending', 'paid', 'failed', 'refunded')),
    -- High-entropy secret the purchaser uses to reopen their order later. Only the
    -- digest is stored, so a database read cannot recover the value.
    retrieval_token_hash bytea,
    -- The form version the stored answers were validated against.
    form_id text,
    form_version integer,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    confirmed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    canceled_at timestamptz,
    FOREIGN KEY (organization_id, event_id)
        REFERENCES events (organization_id, id) ON DELETE RESTRICT,
    -- Keyed by (event_id, id) like reservations, because that is the unique key
    -- ticket_types already carries. Organization scope is implied because
    -- ticket_types.organization_id is constrained to match its event's.
    FOREIGN KEY (event_id, ticket_type_id)
        REFERENCES ticket_types (event_id, id) ON DELETE RESTRICT
);

ALTER TABLE orders
    ADD CONSTRAINT orders_retrieval_token_length
        CHECK (retrieval_token_hash IS NULL OR octet_length(retrieval_token_hash) = 32),
    ADD CONSTRAINT orders_form_version_present
        CHECK ((form_id IS NULL) = (form_version IS NULL)),
    ADD CONSTRAINT orders_canceled_has_timestamp
        CHECK ((status = 'canceled') = (canceled_at IS NOT NULL));

-- Organizer order lists page newest-first per event.
CREATE INDEX orders_event_created
    ON orders (organization_id, event_id, created_at DESC, id DESC);

CREATE TABLE tickets (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    order_id text NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    organization_id text NOT NULL,
    event_id text NOT NULL,
    ticket_type_id text NOT NULL,
    -- 1..quantity. Gives staff a stable "ticket 3 of 10" and keeps ticket issuance
    -- deterministic when a batch is retried.
    sequence smallint NOT NULL CHECK (sequence >= 1),
    attendee_name text,
    attendee_email text,
    -- Short human-readable fallback for a damaged or unreadable QR code.
    reference text NOT NULL CHECK (reference ~ '^[A-Z0-9]{6,10}$'),
    -- sha256 of the 256-bit credential actually encoded in the QR code. Storing a
    -- digest means a database read yields no usable credential, and because the
    -- preimage is high entropy a lookup by digest cannot be brute forced.
    credential_hash bytea NOT NULL CHECK (octet_length(credential_hash) = 32),
    status text NOT NULL DEFAULT 'valid'
        CHECK (status IN ('valid', 'checked_in', 'voided')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    checked_in_at timestamptz,
    UNIQUE (order_id, sequence),
    FOREIGN KEY (organization_id, event_id)
        REFERENCES events (organization_id, id) ON DELETE RESTRICT,
    FOREIGN KEY (event_id, ticket_type_id)
        REFERENCES ticket_types (event_id, id) ON DELETE RESTRICT
);

-- THE hot path of the product: a scanner presents a credential and we must find
-- its ticket. Scoped to an event so a credential from another event cannot be
-- admitted at this one, and unique so one credential can never map to two tickets.
CREATE UNIQUE INDEX tickets_event_credential
    ON tickets (event_id, credential_hash);

-- Staff can admit by typing the short reference when a camera cannot read a code.
CREATE UNIQUE INDEX tickets_event_reference
    ON tickets (event_id, reference);

-- Partial attendance count for an order. Without this the dashboard would sum
-- per-ticket rows on every poll; the index only carries live tickets, so it stays
-- small even when an event has many completed admissions.
CREATE INDEX tickets_event_status
    ON tickets (event_id, status, order_id);

ALTER TABLE tickets
    ADD CONSTRAINT tickets_checkin_has_timestamp
        CHECK ((status = 'checked_in') = (checked_in_at IS NOT NULL));

-- Answers are snapshotted rather than referenced. The literal label is stored
-- alongside the field key so an organizer's later relabel cannot change how a
-- past answer reads, and so a retired form is still fully explainable.
CREATE TABLE registration_answers (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id text NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    ticket_id text REFERENCES tickets (id) ON DELETE CASCADE,
    field_id text NOT NULL,
    field_key text NOT NULL,
    field_label text NOT NULL CHECK (char_length(field_label) BETWEEN 1 AND 160),
    field_type text NOT NULL,
    value text NOT NULL CHECK (char_length(value) BETWEEN 1 AND 2000),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- One answer per question per order. ticket_id stays NULL for order-level answers,
-- which is what the current checkout collects; the column exists so per-attendee
-- answers can be added later without migrating confirmed bookings.
CREATE UNIQUE INDEX registration_answers_order_field
    ON registration_answers (order_id, ticket_id, field_id)
    WHERE ticket_id IS NULL;

CREATE UNIQUE INDEX registration_answers_ticket_field
    ON registration_answers (order_id, ticket_id, field_id)
    WHERE ticket_id IS NOT NULL;

-- Reading one order's answers back in the order they were asked.
CREATE INDEX registration_answers_order_position
    ON registration_answers (order_id, id);