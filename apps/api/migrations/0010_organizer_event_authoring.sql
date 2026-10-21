ALTER TABLE events
    ADD COLUMN slug text,
    ADD CONSTRAINT events_slug_format
        CHECK (slug IS NULL OR slug ~ '^[a-z0-9]([a-z0-9-]{1,78}[a-z0-9])$');

CREATE UNIQUE INDEX events_organization_slug
    ON events (organization_id, slug)
    WHERE slug IS NOT NULL;

CREATE INDEX events_organization_event_order
    ON events (organization_id, starts_at NULLS LAST, id);

ALTER TABLE ticket_types
    ADD COLUMN organization_id text,
    ADD COLUMN idempotency_key text
        CHECK (idempotency_key IS NULL OR length(idempotency_key) BETWEEN 22 AND 128);

UPDATE ticket_types t
SET organization_id = e.organization_id
FROM events e
WHERE e.id = t.event_id;

ALTER TABLE ticket_types
    ALTER COLUMN organization_id SET NOT NULL,
    ADD CONSTRAINT ticket_types_organization_event_fkey
        FOREIGN KEY (organization_id, event_id)
        REFERENCES events (organization_id, id) ON DELETE CASCADE;

CREATE UNIQUE INDEX ticket_types_idempotency
    ON ticket_types (event_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX ticket_types_organizer_list
    ON ticket_types (organization_id, event_id, display_order, id);
