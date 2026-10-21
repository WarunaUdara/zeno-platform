CREATE TABLE outbox_jobs (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    job_type text NOT NULL CHECK (job_type = 'reservation.expire'),
    aggregate_id text NOT NULL REFERENCES reservations(id) ON DELETE CASCADE,
    payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed')),
    available_at timestamptz NOT NULL,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    completed_at timestamptz,
    UNIQUE (job_type, aggregate_id),
    CHECK (payload ? 'ticket_type_id'),
    CHECK ((status = 'completed') = (completed_at IS NOT NULL))
);

CREATE INDEX outbox_jobs_pending_due
    ON outbox_jobs (available_at, id)
    WHERE status = 'pending';

INSERT INTO outbox_jobs (job_type, aggregate_id, payload, available_at)
SELECT 'reservation.expire', id, jsonb_build_object('ticket_type_id', ticket_type_id), expires_at
FROM reservations
WHERE status = 'held'
ON CONFLICT (job_type, aggregate_id) DO NOTHING;
