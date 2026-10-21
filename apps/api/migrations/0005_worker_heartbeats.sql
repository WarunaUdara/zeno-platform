CREATE TABLE worker_heartbeats (
    worker_name text NOT NULL,
    instance_id text NOT NULL,
    last_batch_at timestamptz NOT NULL,
    last_batch_succeeded boolean NOT NULL,
    consecutive_failures integer NOT NULL CHECK (consecutive_failures >= 0),
    PRIMARY KEY (worker_name, instance_id)
);
