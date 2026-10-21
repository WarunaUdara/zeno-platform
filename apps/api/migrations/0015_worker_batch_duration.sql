-- Record how long a worker batch actually took.
--
-- The expiry worker's candidate selection was found to sequentially scan the whole
-- queue on every poll, costing ~41ms per poll at flash-sale volume. Nothing
-- observed that: batch outcome was recorded, batch cost was not. Without a
-- duration a slow worker is indistinguishable from an idle one, because both look
-- healthy while holds pile up.
--
-- Nullable so an existing row written before this migration is not backfilled with
-- a fabricated zero, which would read as "this batch was instant".
ALTER TABLE worker_heartbeats
    ADD COLUMN last_batch_duration_ms integer
        CHECK (last_batch_duration_ms IS NULL OR last_batch_duration_ms >= 0);