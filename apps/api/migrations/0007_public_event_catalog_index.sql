CREATE INDEX events_public_catalog_order
    ON events (starts_at, id)
    WHERE visibility = 'public' AND starts_at IS NOT NULL AND venue IS NOT NULL;
