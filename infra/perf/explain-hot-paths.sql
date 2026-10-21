-- Measure the statements that decide whether a flash sale stays correct and fast.
-- Run against a database that already has the real migrations applied:
--
--   psql -X -d zeno_perf -f infra/perf/explain-hot-paths.sql
--
-- Each query is copied from the statement the API or the expiry worker issues, so
-- a plan change here is a plan change in production, not in a paraphrase.
\timing off
\pset pager off

\echo '== 1. ticket availability: add back due-but-unreleased holds (catalog.rs list_published_ticket_types) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
SELECT t.id,
       CASE
         WHEN clock_timestamp() < t.sale_starts_at OR clock_timestamp() >= t.sale_ends_at
           THEN 'not_on_sale'
         WHEN t.capacity - t.sold_count - t.held_count + COALESCE((
           SELECT sum(r.quantity)::bigint FROM reservations AS r
           WHERE r.ticket_type_id = t.id AND r.status = 'held'
             AND r.expires_at <= clock_timestamp()), 0) <= 0
           THEN 'sold_out'
         ELSE 'available'
       END AS availability
FROM ticket_types AS t
JOIN events AS e ON e.id = t.event_id
WHERE e.id = 'zeno_' || lpad(to_hex(1), 32, '0')
  AND e.visibility = 'public' AND e.starts_at IS NOT NULL AND e.venue IS NOT NULL
ORDER BY t.display_order, t.id;

\echo '== 2. expiry candidate ticket types, oldest due first (worker.rs expire_holds_once) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
WITH due AS (
  SELECT payload->>'ticket_type_id' AS ticket_type_id, available_at
  FROM outbox_jobs
  WHERE job_type = 'reservation.expire' AND status = 'pending'
    AND available_at <= clock_timestamp()
  ORDER BY available_at, id
  LIMIT 2000
)
SELECT ticket_type_id FROM due
GROUP BY ticket_type_id
ORDER BY min(available_at), ticket_type_id LIMIT 100;

\echo '== 3. due expiry job claim with SKIP LOCKED (worker.rs expire_holds_once) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
SELECT id, aggregate_id AS reservation_id, payload->>'ticket_type_id' AS ticket_type_id
FROM outbox_jobs
WHERE job_type = 'reservation.expire' AND status = 'pending'
  AND available_at <= clock_timestamp()
  AND payload->>'ticket_type_id' = ANY(ARRAY[
        'zeno_' || lpad(to_hex(1), 32, '0'),
        'zeno_' || lpad(to_hex(2), 32, '0'),
        'zeno_' || lpad(to_hex(3), 32, '0')])
ORDER BY available_at, id FOR UPDATE SKIP LOCKED LIMIT 100;

\echo '== 4. inline expiry of stale holds on the hot ticket type (postgres.rs create_reservation) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
WITH due_candidates AS MATERIALIZED (
  SELECT id, quantity, expires_at FROM reservations
  WHERE ticket_type_id = 'zeno_' || lpad(to_hex(1), 32, '0')
    AND status = 'held' AND expires_at <= clock_timestamp()
  ORDER BY expires_at, id LIMIT 100 FOR UPDATE),
due_with_cumulative AS (
  SELECT id, quantity,
         COALESCE(sum(quantity) OVER (ORDER BY expires_at, id
             ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING), 0) AS released_before
  FROM due_candidates)
UPDATE reservations AS r SET status = 'expired'
FROM due_with_cumulative AS due
WHERE r.id = due.id AND due.released_before < 50
RETURNING r.id, r.quantity;

\echo '== 5. public catalog first page (catalog.rs list_published_events) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
SELECT id, title, summary, starts_at, ends_at, timezone, venue
FROM events
WHERE visibility = 'public' AND starts_at >= clock_timestamp() AND venue IS NOT NULL
ORDER BY starts_at, id LIMIT 21;

\echo '== 6. organizer event list page (authoring.rs list_events) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
SELECT e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at,
       e.timezone, e.visibility, e.venue
FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id
WHERE e.organization_id = 'zeno_' || lpad(to_hex(7), 32, '0')
  AND m.identity_issuer = 'https://auth.example.test'
  AND m.identity_subject = 'user-1'
ORDER BY e.starts_at NULLS LAST, e.id LIMIT 21;

\echo '== 7. reserve: lock the ticket type and read its window (postgres.rs create_reservation) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
SELECT t.unit_amount, t.currency, t.capacity, t.held_count, t.sold_count,
       t.sale_starts_at, t.sale_ends_at
FROM ticket_types t
JOIN events e ON e.id = t.event_id
WHERE e.id = 'zeno_' || lpad(to_hex(1), 32, '0') AND t.id = 'zeno_' || lpad(to_hex(1), 32, '0')
  AND e.visibility = 'public'
FOR UPDATE OF t;

\echo '== 8. idempotency replay read (postgres.rs reserve_in_transaction) =='
EXPLAIN (ANALYZE, BUFFERS, COSTS OFF)
SELECT fingerprint, replay_until, response FROM idempotency_keys
WHERE idempotency_key = repeat('k', 22);