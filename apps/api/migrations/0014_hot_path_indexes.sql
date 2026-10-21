-- Hot-path index changes for flash-sale traffic.
--
-- Measured with infra/perf/explain-hot-paths.sql against 200k reservations,
-- 200k outbox jobs, 20k ticket types and 5k events. Re-run that script after any
-- change here and keep every hot-path plan index-driven.
--
-- 1. The expiry worker claims jobs for specific ticket types and skips locked
--    rows, but `outbox_jobs_pending_due` is keyed on (available_at, id) and so
--    cannot narrow by the ticket type held in the payload. The claim therefore
--    read about 90 buffers per claimed job. Measured with this index it read 142
--    buffers for 100 jobs and dropped from 3.03ms to 0.50ms.
-- 2. Availability is a pure aggregate over held rows that reads only quantity.
--    Including quantity keeps it on the index: measured 1.77ms -> 0.50ms as an
--    Index Only Scan.
-- 3. `reservations_active_holds` is keyed on exactly what the covering index
--    above now uses, so the two would be identical for both remaining consumers
--    (the availability aggregate and the inline-expiry UPDATE). Dropping it
--    removes one index entry written on every hold insert and every expiry
--    status update, which is pure write cost for no read benefit.
-- 4. `reservations_expired_holds` existed to serve the worker's global
--    oldest-expired-hold scan. Migration 0006 replaced that scan with the
--    transactional outbox, and no statement in the API or the worker now orders
--    held reservations by expires_at without also filtering ticket_type_id. It
--    selected zero scans across every measured hot path, and it is the largest
--    index on the hottest table, maintained on every insert and every expiry.
--
-- Both drops are ordinary DROP INDEX, which sqlx runs inside the migration
-- transaction. They cannot be CONCURRENTLY there, but an unused index is dropped
-- without rewriting the table and the lock is held only briefly.
--
-- The worker's oldest-first candidate query is bounded in code rather than
-- indexed here. Grouping all pending rows by a payload expression cannot be
-- served by any index, and forcing an index path measured slower (62ms) than the
-- sequential scan it replaced. Capping the look-back window in worker.rs reduced
-- the same query from 41ms to 3.3ms and made its cost independent of backlog size.

CREATE INDEX outbox_jobs_expiry_due_by_ticket_type
    ON outbox_jobs ((payload->>'ticket_type_id'), available_at, id)
    WHERE status = 'pending' AND job_type = 'reservation.expire';

CREATE INDEX reservations_active_holds_covering
    ON reservations (ticket_type_id, expires_at)
    INCLUDE (quantity)
    WHERE status = 'held';

DROP INDEX reservations_active_holds;
DROP INDEX reservations_expired_holds;