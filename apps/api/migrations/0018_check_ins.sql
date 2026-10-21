-- Admission ledger.
--
-- Every admission decision is appended here and never updated or deleted. A ticket
-- row holds only the current state; this table holds the history, which is what
-- makes "who let this person in, when, and was it corrected" answerable after the
-- fact. Correcting a mistaken admission appends an 'undo' row and returns the
-- ticket to 'valid' rather than erasing the mistake.

CREATE TABLE check_ins (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ticket_id text NOT NULL REFERENCES tickets (id) ON DELETE CASCADE,
    organization_id text NOT NULL,
    event_id text NOT NULL,
    -- admit: guest arrived. undo: admission was retracted. void: ticket was
    -- invalidated after issue (refund, duplicate booking, organiser mistake).
    action text NOT NULL CHECK (action IN ('admit', 'undo', 'void')),
    actor_subject text NOT NULL CHECK (char_length(actor_subject) BETWEEN 1 AND 256),
    actor_issuer text NOT NULL CHECK (char_length(actor_issuer) BETWEEN 1 AND 512),
    -- Client-generated idempotency token. A scanner on flaky venue Wi-Fi will retry
    -- the same scan; this makes the retry return the original outcome instead of
    -- recording a second admission.
    scanner_action_id text NOT NULL CHECK (scanner_action_id ~ '^[A-Za-z0-9_-]{8,64}$'),
    -- Required for undo and void so a correction always carries a reason.
    reason text,
    -- Correlation so a support question can be traced to one API request.
    request_id text,
    trace_id text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (
        (action IN ('admit', 'void') AND reason IS NULL AND scanner_action_id <> '')
        OR (action = 'undo' AND reason IS NOT NULL AND char_length(btrim(reason)) BETWEEN 3 AND 500)
    )
);

-- One scan action, one ledger row. This is what makes a retried scan safe.
CREATE UNIQUE INDEX check_ins_scanner_action
    ON check_ins (scanner_action_id);

-- Live arrival count for an event without scanning tickets.
CREATE INDEX check_ins_event_admitted
    ON check_ins (event_id, created_at DESC)
    WHERE action = 'admit';

-- Attendee lookup at the door and attendance audit for one event.
CREATE INDEX check_ins_event_ticket
    ON check_ins (event_id, ticket_id, created_at DESC);

-- Who is acting at the door, for incident review.
CREATE INDEX check_ins_actor
    ON check_ins (organization_id, actor_issuer, actor_subject, created_at DESC);