-- Registration forms.
--
-- An organizer authors questions an attendee answers when confirming a booking
-- ("Do you have any access needs?", "Which team are you with?"). Each event
-- points at one active form.
--
-- Forms are immutable once used. An organizer editing questions must publish a new
-- version rather than mutate the old one, because every confirmed booking stores
-- the form version it was answered against. Mutating in place would leave old
-- answers unreadable the next time a question is renamed or removed.

CREATE TABLE registration_forms (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    organization_id text NOT NULL,
    event_id text NOT NULL,
    name text NOT NULL CHECK (char_length(btrim(name)) BETWEEN 1 AND 120),
    -- draft: editable by its author only. published: attached to an event and
    -- frozen. retired: superseded, kept for reading historical answers.
    status text NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'retired')),
    version integer NOT NULL CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    published_at timestamptz,
    UNIQUE (event_id, version),
    FOREIGN KEY (organization_id, event_id)
        REFERENCES events (organization_id, id) ON DELETE CASCADE
);

-- A draft is editable; a published or retired form must never change, because
-- confirmed bookings already reference its rows.
CREATE UNIQUE INDEX registration_forms_single_draft
    ON registration_forms (event_id)
    WHERE status = 'draft';

-- Only one form may be active for an event at a time.
CREATE UNIQUE INDEX registration_forms_single_published
    ON registration_forms (event_id)
    WHERE status = 'published';

CREATE TABLE registration_form_fields (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    form_id text NOT NULL REFERENCES registration_forms (id) ON DELETE CASCADE,
    position integer NOT NULL CHECK (position >= 0),
    -- Stable machine key stored on each answer so a later relabel of the question
    -- does not orphan historical data.
    field_key text NOT NULL CHECK (field_key ~ '^[a-z0-9]([a-z0-9_]{0,38}[a-z0-9])$'),
    label text NOT NULL CHECK (char_length(btrim(label)) BETWEEN 1 AND 160),
    help_text text CHECK (help_text IS NULL OR char_length(help_text) <= 500),
    field_type text NOT NULL CHECK (
        field_type IN (
            'short_text', 'long_text', 'email', 'phone', 'number',
            'single_choice', 'multi_choice', 'checkbox'
        )
    ),
    required boolean NOT NULL DEFAULT false,
    -- Choice options, as a JSON array of strings. Empty for every other type.
    options jsonb NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(options) = 'array'),
    UNIQUE (form_id, position),
    UNIQUE (form_id, field_key)
);

-- Answering a question that is not on the form, or answering a free-text question
-- with a choice list, must be rejected in the database rather than only in the
-- application, so a direct database write cannot produce an unreadable answer.
ALTER TABLE registration_form_fields
    ADD CONSTRAINT registration_form_fields_options_match_type
    CHECK (
        (field_type IN ('single_choice', 'multi_choice') AND jsonb_array_length(options) BETWEEN 1 AND 50)
        OR (field_type NOT IN ('single_choice', 'multi_choice') AND jsonb_array_length(options) = 0)
    );

-- The door and the attendee lookup both read fields by event, so index the join
-- path rather than leaving every lookup to hunt for a form first.
CREATE INDEX registration_form_fields_form_order
    ON registration_form_fields (form_id, position, id);