ALTER TABLE events
    ADD COLUMN cover_image_key text,
    ADD COLUMN cover_image_alt text,
    ADD CONSTRAINT events_cover_image_pair
        CHECK ((cover_image_key IS NULL) = (cover_image_alt IS NULL)),
    ADD CONSTRAINT events_cover_image_alt_length
        CHECK (cover_image_alt IS NULL OR char_length(cover_image_alt) BETWEEN 1 AND 240),
    ADD CONSTRAINT events_cover_image_key_scope
        CHECK (
            cover_image_key IS NULL
            OR (
                left(cover_image_key, char_length('events/' || organization_id || '/' || id || '/'))
                    = 'events/' || organization_id || '/' || id || '/'
                AND substring(cover_image_key FROM char_length('events/' || organization_id || '/' || id || '/') + 1)
                    ~ '^[0-9A-HJKMNP-TV-Z]{26}[.](jpg|png|webp)$'
            )
        );
