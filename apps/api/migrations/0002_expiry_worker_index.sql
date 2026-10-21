-- Supports the worker's global oldest-expired-hold scan without indexing
-- reservations that have already reached a terminal state.
CREATE INDEX reservations_expired_holds
    ON reservations (expires_at, id)
    INCLUDE (ticket_type_id, quantity)
    WHERE status = 'held';
