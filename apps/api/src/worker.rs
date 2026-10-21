use std::collections::BTreeMap;

use sqlx::{FromRow, PgPool};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExpiryBatch {
    pub reservations_expired: u64,
    pub tickets_released: u64,
}

const RESERVATION_EXPIRY_WORKER: &str = "reservation-expiry";

/// Minimum number of oldest due jobs examined when choosing which ticket types a
/// batch will serve. Sized well above the batch so many distinct ticket types stay
/// reachable in one pass, and clamped so a large configured batch cannot turn this
/// into an unbounded scan again.
const MIN_CANDIDATE_WINDOW: u32 = 2_000;
const MAX_CANDIDATE_WINDOW: u32 = 20_000;

fn candidate_window(batch_size: u32) -> u32 {
    batch_size
        .saturating_mul(20)
        .clamp(MIN_CANDIDATE_WINDOW, MAX_CANDIDATE_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::{MAX_CANDIDATE_WINDOW, MIN_CANDIDATE_WINDOW, candidate_window};

    #[test]
    fn candidate_window_is_bounded_and_scales_with_the_batch() {
        // The default 100-job batch still examines enough due jobs to find a
        // spread of ticket types rather than one busy queue.
        assert_eq!(candidate_window(100), MIN_CANDIDATE_WINDOW);
        assert_eq!(candidate_window(0), MIN_CANDIDATE_WINDOW);
        // Growth is proportional until it is clamped.
        assert_eq!(candidate_window(1_000), 20_000);
        assert_eq!(candidate_window(10_000), MAX_CANDIDATE_WINDOW);
        // A pathological configuration cannot exceed the clamp.
        assert_eq!(candidate_window(u32::MAX), MAX_CANDIDATE_WINDOW);
    }
}

/// Record whether this instance's most recent expiry attempt completed, and how
/// long it took. Duration is what distinguishes a worker that is keeping up from
/// one that is succeeding slowly.
pub async fn record_heartbeat(
    pool: &PgPool,
    instance_id: &str,
    succeeded: bool,
    duration_ms: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO worker_heartbeats \
             (worker_name, instance_id, last_batch_at, last_batch_succeeded, \
              consecutive_failures, last_batch_duration_ms) \
         VALUES ($1, $2, clock_timestamp(), $3, CASE WHEN $3 THEN 0 ELSE 1 END, $4) \
         ON CONFLICT (worker_name, instance_id) DO UPDATE SET \
             last_batch_at = clock_timestamp(), \
             last_batch_succeeded = EXCLUDED.last_batch_succeeded, \
             last_batch_duration_ms = EXCLUDED.last_batch_duration_ms, \
             consecutive_failures = CASE WHEN EXCLUDED.last_batch_succeeded \
                 THEN 0 ELSE worker_heartbeats.consecutive_failures + 1 END",
    )
    .bind(RESERVATION_EXPIRY_WORKER)
    .bind(instance_id)
    .bind(succeeded)
    .bind(duration_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// A worker is healthy only when its own latest batch succeeded recently.
pub async fn is_healthy(
    pool: &PgPool,
    instance_id: &str,
    max_age_ms: u64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM worker_heartbeats \
             WHERE worker_name = $1 AND instance_id = $2 \
               AND last_batch_succeeded \
               AND last_batch_at >= clock_timestamp() - ($3 * interval '1 millisecond') \
         )",
    )
    .bind(RESERVATION_EXPIRY_WORKER)
    .bind(instance_id)
    .bind(i64::try_from(max_age_ms).unwrap_or(i64::MAX))
    .fetch_one(pool)
    .await
}

/// Remove a bounded number of idempotency records after their replay window.
/// Rows are locked in expiry order so concurrent API retries and worker runs
/// can safely continue without a table-wide cleanup lock.
pub async fn purge_expired_idempotency_keys_once(
    pool: &PgPool,
    batch_size: u32,
) -> Result<u64, sqlx::Error> {
    if batch_size == 0 {
        return Ok(0);
    }

    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        "WITH expired AS ( \
             SELECT idempotency_key FROM idempotency_keys \
             WHERE replay_until <= clock_timestamp() \
             ORDER BY replay_until, idempotency_key \
             LIMIT $1 FOR UPDATE SKIP LOCKED \
         ) \
         DELETE FROM idempotency_keys AS ik USING expired \
         WHERE ik.idempotency_key = expired.idempotency_key",
    )
    .bind(i64::from(batch_size))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok(result.rows_affected())
}

#[derive(FromRow)]
struct ExpiryJob {
    id: i64,
    reservation_id: String,
    ticket_type_id: String,
}

#[derive(FromRow)]
struct ExpiredReservation {
    ticket_type_id: String,
    quantity: i16,
}

/// Processes due reservation-expiry jobs and releases their inventory.
/// Reservation creation writes the job in its transaction, so a committed hold
/// cannot be lost between the database write and the worker queue.
/// Multiple worker processes can share the queue through SKIP LOCKED.
pub async fn expire_holds_once(pool: &PgPool, batch_size: u32) -> Result<ExpiryBatch, sqlx::Error> {
    if batch_size == 0 {
        return Ok(ExpiryBatch::default());
    }

    let mut transaction = pool.begin().await?;
    // Select due queues by age so a continuously busy low-ID ticket type cannot
    // permanently keep later ticket types out of the bounded batch. Ticket row
    // locks below remain ordered by ID to keep checkout/worker lock order stable.
    //
    // Grouping every pending row by the ticket type in the payload cannot be
    // served by any index, so the candidate list is read from a bounded slice of
    // the oldest due jobs instead. That keeps this poll proportional to
    // `candidate_window` rather than to the size of the queue, which is what
    // matters when the backlog is exactly when this code runs most often. A ticket
    // type whose work is among the oldest due jobs is always in the window; one
    // whose jobs are all newer than the window is not urgent yet.
    let candidate_ticket_types = sqlx::query_scalar::<_, String>(
        "WITH due AS ( \
             SELECT payload->>'ticket_type_id' AS ticket_type_id, available_at \
             FROM outbox_jobs \
             WHERE job_type = 'reservation.expire' AND status = 'pending' \
               AND available_at <= clock_timestamp() \
             ORDER BY available_at, id \
             LIMIT $2 \
         ) \
         SELECT ticket_type_id FROM due \
         GROUP BY ticket_type_id \
         ORDER BY min(available_at), ticket_type_id LIMIT $1",
    )
    .bind(i64::from(batch_size))
    .bind(i64::from(candidate_window(batch_size)))
    .fetch_all(&mut *transaction)
    .await?;

    if candidate_ticket_types.is_empty() {
        transaction.commit().await?;
        return Ok(ExpiryBatch::default());
    }

    // Reservation creation locks the ticket type before touching holds. Keep
    // the same lock order here so expiry cannot deadlock with checkout.
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM ticket_types WHERE id = ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(&candidate_ticket_types)
    .fetch_all(&mut *transaction)
    .await?;

    let jobs = sqlx::query_as::<_, ExpiryJob>(
        "SELECT id, aggregate_id AS reservation_id, payload->>'ticket_type_id' AS ticket_type_id \
         FROM outbox_jobs \
         WHERE job_type = 'reservation.expire' AND status = 'pending' \
           AND available_at <= clock_timestamp() \
           AND payload->>'ticket_type_id' = ANY($1) \
         ORDER BY available_at, id FOR UPDATE SKIP LOCKED LIMIT $2",
    )
    .bind(&candidate_ticket_types)
    .bind(i64::from(batch_size))
    .fetch_all(&mut *transaction)
    .await?;

    let reservation_ids = jobs
        .iter()
        .map(|job| job.reservation_id.clone())
        .collect::<Vec<_>>();
    let ticket_type_ids = jobs
        .iter()
        .map(|job| job.ticket_type_id.clone())
        .collect::<Vec<_>>();
    let job_ids = jobs.iter().map(|job| job.id).collect::<Vec<_>>();

    // Update reservations and outbox jobs in bounded batch statements instead
    // of one database round trip per job while the ticket-type locks are held.
    let expired_reservations = sqlx::query_as::<_, ExpiredReservation>(
        "UPDATE reservations AS r SET status = 'expired' \
         FROM UNNEST($1::text[], $2::text[]) AS selected(reservation_id, ticket_type_id) \
         WHERE r.id = selected.reservation_id \
           AND r.ticket_type_id = selected.ticket_type_id \
           AND r.status = 'held' AND r.expires_at <= clock_timestamp() \
         RETURNING r.ticket_type_id, r.quantity",
    )
    .bind(&reservation_ids)
    .bind(&ticket_type_ids)
    .fetch_all(&mut *transaction)
    .await?;

    let mut released_by_ticket_type = BTreeMap::<String, i64>::new();
    let mut result = ExpiryBatch::default();
    for reservation in expired_reservations {
        result.reservations_expired += 1;
        result.tickets_released += u64::try_from(reservation.quantity)
            .expect("reservation quantities are positive by the database constraint");
        *released_by_ticket_type
            .entry(reservation.ticket_type_id)
            .or_default() += i64::from(reservation.quantity);
    }

    sqlx::query(
        "UPDATE outbox_jobs SET status = 'completed', attempts = attempts + 1, \
         completed_at = clock_timestamp() WHERE id = ANY($1)",
    )
    .bind(&job_ids)
    .execute(&mut *transaction)
    .await?;

    if !released_by_ticket_type.is_empty() {
        let release_ticket_type_ids = released_by_ticket_type.keys().cloned().collect::<Vec<_>>();
        let release_quantities = released_by_ticket_type
            .values()
            .copied()
            .collect::<Vec<_>>();
        let updated_ticket_types = sqlx::query(
            "UPDATE ticket_types AS t SET held_count = t.held_count - released.quantity \
             FROM UNNEST($1::text[], $2::bigint[]) AS released(ticket_type_id, quantity) \
             WHERE t.id = released.ticket_type_id AND t.held_count >= released.quantity",
        )
        .bind(&release_ticket_type_ids)
        .bind(&release_quantities)
        .execute(&mut *transaction)
        .await?;
        if updated_ticket_types.rows_affected()
            != u64::try_from(released_by_ticket_type.len()).unwrap_or(u64::MAX)
        {
            return Err(sqlx::Error::Protocol(
                "expired holds exceed the recorded inventory counter".to_owned(),
            ));
        }
    }

    transaction.commit().await?;
    Ok(result)
}
