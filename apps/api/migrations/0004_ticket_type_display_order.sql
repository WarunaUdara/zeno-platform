ALTER TABLE ticket_types
    ADD COLUMN display_order smallint NOT NULL DEFAULT 0 CHECK (display_order >= 0);
