ALTER TABLE events
    ADD COLUMN summary text CHECK (summary IS NULL OR length(summary) <= 1000),
    ADD COLUMN starts_at timestamptz,
    ADD COLUMN ends_at timestamptz,
    ADD COLUMN venue jsonb,
    ADD CONSTRAINT event_end_after_start
        CHECK (ends_at IS NULL OR starts_at IS NULL OR ends_at > starts_at);

ALTER TABLE ticket_types
    ADD COLUMN description text CHECK (description IS NULL OR length(description) <= 500);
