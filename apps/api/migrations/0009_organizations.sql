CREATE TABLE organizations (
    id text PRIMARY KEY CHECK (id ~ '^zeno_[A-Za-z0-9]{16,32}$'),
    name text NOT NULL CHECK (length(name) BETWEEN 2 AND 120),
    slug text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9]([a-z0-9-]{1,60}[a-z0-9])$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

INSERT INTO organizations (id, name, slug)
VALUES ('zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N', 'Zeno demo events', 'zeno-demo-system');

ALTER TABLE events ADD COLUMN organization_id text;

UPDATE events
SET organization_id = 'zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N';

ALTER TABLE events
    ALTER COLUMN organization_id SET NOT NULL,
    ADD CONSTRAINT events_organization_id_fkey
        FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE RESTRICT,
    ADD CONSTRAINT events_organization_id_id_unique UNIQUE (organization_id, id);

CREATE TABLE organization_memberships (
    organization_id text NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    identity_issuer text NOT NULL CHECK (length(identity_issuer) BETWEEN 1 AND 512),
    identity_subject text NOT NULL CHECK (length(identity_subject) BETWEEN 1 AND 256),
    role text NOT NULL CHECK (role IN ('owner', 'admin', 'event_manager', 'check_in_manager', 'analyst')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (organization_id, identity_issuer, identity_subject)
);

CREATE INDEX organization_memberships_principal
    ON organization_memberships (identity_issuer, identity_subject, created_at DESC, organization_id DESC)
    INCLUDE (role);
