CREATE TABLE events (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    title text NOT NULL CHECK (length(title) BETWEEN 1 AND 160),
    visibility text NOT NULL CHECK (visibility IN ('draft', 'public')),
    timezone text NOT NULL
);

CREATE TABLE ticket_types (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    event_id text NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    unit_amount numeric(18, 4) NOT NULL CHECK (unit_amount >= 0),
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    capacity bigint NOT NULL CHECK (capacity >= 0),
    held_count bigint NOT NULL DEFAULT 0 CHECK (held_count >= 0),
    sold_count bigint NOT NULL DEFAULT 0 CHECK (sold_count >= 0),
    sale_starts_at timestamptz NOT NULL,
    sale_ends_at timestamptz NOT NULL,
    CHECK (sale_ends_at > sale_starts_at),
    CHECK (held_count + sold_count <= capacity),
    UNIQUE (event_id, id)
);

CREATE TABLE reservations (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    event_id text NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    ticket_type_id text NOT NULL,
    buyer_email text NOT NULL CHECK (length(buyer_email) <= 254),
    quantity smallint NOT NULL CHECK (quantity BETWEEN 1 AND 100),
    amount_due numeric(20, 4) NOT NULL CHECK (amount_due >= 0),
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    status text NOT NULL CHECK (status IN ('held', 'expired')),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (event_id, ticket_type_id)
        REFERENCES ticket_types(event_id, id) ON DELETE CASCADE
);

CREATE INDEX reservations_active_holds
    ON reservations (ticket_type_id, expires_at)
    WHERE status = 'held';

CREATE TABLE idempotency_keys (
    idempotency_key text PRIMARY KEY CHECK (length(idempotency_key) BETWEEN 22 AND 128),
    fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32),
    replay_until timestamptz NOT NULL,
    reservation_id text REFERENCES reservations(id) ON DELETE CASCADE,
    response jsonb,
    CHECK ((reservation_id IS NULL) = (response IS NULL))
);

CREATE INDEX idempotency_keys_replay_until ON idempotency_keys (replay_until);
