-- Load synthetic flash-sale volume into the real schema.
-- Apply only after `zeno-api migrate` has run on this database, because every
-- statement below is checked by the same constraints the API relies on.
--
--   psql -X -d zeno_perf -f infra/perf/seed-load.sql
--
-- Row counts are sized so plan choice is representative rather than accidental:
-- a few hot ticket types carry most held rows, which is what a single popular
-- event looks like, and a uniform spread would hide the real cost.
--
-- Foreign keys are always derived from the parent table rather than recomputed,
-- so the fixture cannot drift from the schema it is meant to model.
BEGIN;

INSERT INTO organizations (id, name, slug)
SELECT 'zeno_' || lpad(to_hex(i), 32, '0'), 'Org ' || i, 'org-' || i
FROM generate_series(1, 50) AS i;

-- Starts in the past would be invisible to the public catalog, which only lists
-- upcoming events, so every event starts now or later.
INSERT INTO events (id, organization_id, slug, title, summary, visibility, timezone,
                    starts_at, ends_at, venue)
SELECT 'zeno_' || lpad(to_hex(i), 32, '0'),
       'zeno_' || lpad(to_hex(1 + (i % 50)), 32, '0'),
       'event-' || i,
       'Event ' || i,
       'Summary for event ' || i,
       'public', 'Asia/Colombo',
       clock_timestamp() + ((i % 400) || ' days')::interval,
       clock_timestamp() + ((i % 400) + 1 || ' days')::interval,
       '{"kind":"physical","name":"Hall","city":"Colombo","country_code":"LK"}'::jsonb
FROM generate_series(1, 5000) AS i;

-- Four ticket types per event. Ticket types 1..4 are the hot ones and carry the
-- concentrated hold count a flash sale produces.
INSERT INTO ticket_types (id, organization_id, event_id, name, description, unit_amount,
                          currency, capacity, held_count, sold_count,
                          sale_starts_at, sale_ends_at, display_order)
SELECT 'zeno_' || lpad(to_hex(n.base + k.k), 32, '0'),
       n.organization_id,
       n.id,
       'Tier ' || k.k,
       NULL,
       (1000 + ((n.base + k.k) % 9000))::numeric(18,4),
       'LKR',
       100000::bigint,
       CASE WHEN (n.base + k.k) <= 4 THEN 60::bigint ELSE ((n.base + k.k) % 30)::bigint END,
       ((n.base + k.k) % 20)::bigint,
       clock_timestamp() - interval '1 day',
       clock_timestamp() + interval '30 days',
       k.k::smallint
FROM (
  SELECT e.id, e.organization_id,
         ((row_number() OVER (ORDER BY e.id) - 1) * 4)::bigint AS base
  FROM events e
) AS n
CROSS JOIN generate_series(1, 4) AS k(k);

INSERT INTO reservations (id, event_id, ticket_type_id, quantity, amount_due, currency,
                          status, expires_at, created_at)
SELECT 'zeno_' || lpad(to_hex(n.n * 1000 + g.g), 32, '0'),
       n.event_id,
       n.id,
       (1 + ((n.n * 1000 + g.g) % 4))::smallint,
       n.unit_amount,
       'LKR',
       CASE WHEN ((n.n * 1000 + g.g) % 7) = 0 THEN 'expired'
            WHEN ((n.n * 1000 + g.g) % 11) = 0 THEN 'released'
            ELSE 'held' END,
       clock_timestamp() - (((n.n * 1000 + g.g) % 3600) || ' seconds')::interval,
       clock_timestamp()
FROM (
  SELECT t.id, t.event_id, t.unit_amount,
         row_number() OVER (ORDER BY t.id) AS n
  FROM ticket_types t
) AS n
CROSS JOIN LATERAL generate_series(1, CASE WHEN n.n <= 4 THEN 500 ELSE 10 END) AS g(g);

INSERT INTO outbox_jobs (job_type, aggregate_id, payload, status, available_at, attempts,
                         created_at, completed_at)
SELECT 'reservation.expire',
       n.id,
       jsonb_build_object('ticket_type_id', n.ticket_type_id),
       CASE WHEN n.n % 4 = 0 THEN 'completed' ELSE 'pending' END,
       n.expires_at,
       0,
       clock_timestamp(),
       CASE WHEN n.n % 4 = 0 THEN clock_timestamp() ELSE NULL END
FROM (
  SELECT r.id, r.ticket_type_id, r.expires_at,
         row_number() OVER (ORDER BY r.id) AS n
  FROM reservations r
) AS n;

ANALYZE organizations;
ANALYZE events;
ANALYZE ticket_types;
ANALYZE reservations;
ANALYZE outbox_jobs;

COMMIT;