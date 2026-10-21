use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use ulid::Ulid;

use crate::{
    checkout::{CheckoutError, InventoryRepository, ReservationCommand},
    model::{Money, ReleaseReservationResult, Reservation, ReservationResult},
};

#[derive(Clone)]
pub(crate) struct PostgresInventoryRepository {
    pool: PgPool,
}

impl PostgresInventoryRepository {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Releases an anonymous reservation only when the caller proves possession of
/// the same high-entropy key used to create it. Locks follow the expiry worker's
/// ticket-type -> outbox-job -> reservation order.
pub(crate) async fn release_reservation(
    pool: &PgPool,
    event_id: &str,
    reservation_id: &str,
    idempotency_key: &str,
) -> Result<ReleaseReservationResult, CheckoutError> {
    let mut transaction = pool.begin().await.map_err(|error| {
        crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
    })?;
    let ticket_type_id = sqlx::query_scalar::<_, String>(
        "SELECT r.ticket_type_id FROM reservations r \
         JOIN idempotency_keys k ON k.reservation_id = r.id \
         WHERE r.id = $1 AND r.event_id = $2 AND k.idempotency_key = $3",
    )
    .bind(reservation_id)
    .bind(event_id)
    .bind(idempotency_key)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
    })?
    .ok_or(CheckoutError::ReservationNotFound)?;

    let ticket_type_locked =
        sqlx::query_scalar::<_, String>("SELECT id FROM ticket_types WHERE id = $1 FOR UPDATE")
            .bind(&ticket_type_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| {
                crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
            })?;
    if ticket_type_locked.is_none() {
        return Err(CheckoutError::ReservationNotFound);
    }

    let job_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM outbox_jobs WHERE job_type = 'reservation.expire' \
         AND aggregate_id = $1 FOR UPDATE",
    )
    .bind(reservation_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
    })?
    .ok_or(CheckoutError::Unavailable)?;

    let current = sqlx::query_as::<_, (String, i16, DateTime<Utc>)>(
        "SELECT status, quantity, expires_at FROM reservations \
         WHERE id = $1 AND event_id = $2 FOR UPDATE",
    )
    .bind(reservation_id)
    .bind(event_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
    })?
    .ok_or(CheckoutError::ReservationNotFound)?;

    let final_status = if current.0 == "held" {
        let now = sqlx::query_scalar::<_, DateTime<Utc>>("SELECT clock_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| {
                crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
            })?;
        let status = if current.2 <= now {
            "expired"
        } else {
            "released"
        };
        let updated =
            sqlx::query("UPDATE reservations SET status = $2 WHERE id = $1 AND status = 'held'")
                .bind(reservation_id)
                .bind(status)
                .execute(&mut *transaction)
                .await
                .map_err(|error| {
                    crate::db_error::record_as(
                        error,
                        "release_reservation",
                        CheckoutError::Unavailable,
                    )
                })?;
        if updated.rows_affected() != 1 {
            return Err(CheckoutError::Unavailable);
        }
        let inventory = sqlx::query(
            "UPDATE ticket_types SET held_count = held_count - $2 \
             WHERE id = $1 AND held_count >= $2",
        )
        .bind(&ticket_type_id)
        .bind(i64::from(current.1))
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
        })?;
        if inventory.rows_affected() != 1 {
            return Err(CheckoutError::Unavailable);
        }
        sqlx::query(
            "UPDATE outbox_jobs SET status = 'completed', attempts = attempts + 1, \
             completed_at = clock_timestamp() WHERE id = $1 AND status = 'pending'",
        )
        .bind(job_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
        })?;
        status
    } else {
        current.0.as_str()
    };

    transaction.commit().await.map_err(|error| {
        crate::db_error::record_as(error, "release_reservation", CheckoutError::Unavailable)
    })?;
    Ok(ReleaseReservationResult {
        status: final_status.to_owned(),
    })
}

#[derive(FromRow)]
struct IdempotencyRecord {
    fingerprint: Vec<u8>,
    replay_until: DateTime<Utc>,
    response: Option<Value>,
}

#[derive(FromRow)]
struct TicketTypeRow {
    unit_amount: Decimal,
    currency: String,
    capacity: i64,
    held_count: i64,
    sold_count: i64,
    sale_starts_at: DateTime<Utc>,
    sale_ends_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct ExpiredHold {
    id: String,
    quantity: i16,
}

const MAX_INLINE_EXPIRY_HOLDS: i64 = 100;

impl InventoryRepository for PostgresInventoryRepository {
    async fn reserve(
        &self,
        command: ReservationCommand,
    ) -> Result<ReservationResult, CheckoutError> {
        let mut transaction = self.pool.begin().await.map_err(|error| {
            crate::db_error::record_as(error, "reserve", CheckoutError::Unavailable)
        })?;
        let result = reserve_in_transaction(&mut transaction, command).await;
        match result {
            Ok(result) => {
                transaction.commit().await.map_err(|error| {
                    crate::db_error::record_as(error, "reserve", CheckoutError::Unavailable)
                })?;
                Ok(result)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }
}

async fn reserve_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    command: ReservationCommand,
) -> Result<ReservationResult, CheckoutError> {
    let owned_replay_window = sqlx::query_scalar::<_, DateTime<Utc>>(
        "INSERT INTO idempotency_keys (idempotency_key, fingerprint, replay_until) \
         VALUES ($1, $2, clock_timestamp() + interval '24 hours') \
         ON CONFLICT (idempotency_key) DO UPDATE \
         SET fingerprint = EXCLUDED.fingerprint, replay_until = EXCLUDED.replay_until, \
             reservation_id = NULL, response = NULL \
         WHERE idempotency_keys.replay_until <= clock_timestamp() \
         RETURNING replay_until",
    )
    .bind(&command.idempotency_key)
    .bind(&command.fingerprint)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "reserve_in_transaction", CheckoutError::Unavailable)
    })?;

    if let Some(replay_until) = owned_replay_window {
        create_reservation(transaction, command, replay_until).await
    } else {
        let record = sqlx::query_as::<_, IdempotencyRecord>(
            "SELECT fingerprint, replay_until, response \
             FROM idempotency_keys WHERE idempotency_key = $1",
        )
        .bind(&command.idempotency_key)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "reserve_in_transaction", CheckoutError::Unavailable)
        })?;

        if record.fingerprint != command.fingerprint {
            return Err(CheckoutError::IdempotencyConflict);
        }
        let response = record.response.ok_or(CheckoutError::Unavailable)?;
        let reservation =
            serde_json::from_value(response).map_err(|_| CheckoutError::Unavailable)?;
        Ok(ReservationResult {
            reservation,
            replay_until: record.replay_until,
        })
    }
}

async fn create_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    command: ReservationCommand,
    replay_until: DateTime<Utc>,
) -> Result<ReservationResult, CheckoutError> {
    let ticket_type = sqlx::query_as::<_, TicketTypeRow>(
        "SELECT t.unit_amount, t.currency, t.capacity, t.held_count, t.sold_count, \
                t.sale_starts_at, t.sale_ends_at \
         FROM ticket_types t \
         JOIN events e ON e.id = t.event_id \
         WHERE e.id = $1 AND t.id = $2 AND e.visibility = 'public' \
         FOR UPDATE OF t",
    )
    .bind(&command.event_id)
    .bind(&command.ticket_type_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
    })?
    .ok_or(CheckoutError::NotFound)?;

    let now = sqlx::query_scalar::<_, DateTime<Utc>>("SELECT clock_timestamp()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
        })?;

    if now < ticket_type.sale_starts_at || now >= ticket_type.sale_ends_at {
        return Err(CheckoutError::SaleClosed);
    }

    let current_free = ticket_type.capacity - ticket_type.sold_count - ticket_type.held_count;
    let needed_release = (i64::from(command.quantity) - current_free).max(0);
    let expired_holds = if needed_release > 0 {
        sqlx::query_as::<_, ExpiredHold>(
            "WITH due_candidates AS MATERIALIZED ( \
                 SELECT id, quantity, expires_at FROM reservations \
                 WHERE ticket_type_id = $1 AND status = 'held' AND expires_at <= $2 \
                 ORDER BY expires_at, id LIMIT $3 FOR UPDATE \
             ), due_with_cumulative AS ( \
                 SELECT id, quantity, \
                        COALESCE(sum(quantity) OVER (ORDER BY expires_at, id \
                            ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING), 0) AS released_before \
                 FROM due_candidates \
             ) \
             UPDATE reservations AS r SET status = 'expired' \
             FROM due_with_cumulative AS due \
             WHERE r.id = due.id AND due.released_before < $4 \
             RETURNING r.id, r.quantity",
        )
        .bind(&command.ticket_type_id)
        .bind(now)
        .bind(MAX_INLINE_EXPIRY_HOLDS)
        .bind(needed_release)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable))?
    } else {
        Vec::new()
    };
    let released = expired_holds
        .iter()
        .map(|hold| i64::from(hold.quantity))
        .sum::<i64>();
    let held_count = ticket_type.held_count - released;
    if released > 0 {
        sqlx::query("UPDATE ticket_types SET held_count = $2 WHERE id = $1")
            .bind(&command.ticket_type_id)
            .bind(held_count)
            .execute(&mut **transaction)
            .await
            .map_err(|error| {
                crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
            })?;
    }
    if !expired_holds.is_empty() {
        let expired_ids = expired_holds
            .iter()
            .map(|hold| hold.id.clone())
            .collect::<Vec<_>>();
        sqlx::query(
            "UPDATE outbox_jobs SET status = 'completed', attempts = attempts + 1, \
             completed_at = clock_timestamp() \
             WHERE job_type = 'reservation.expire' AND status = 'pending' \
               AND aggregate_id = ANY($1)",
        )
        .bind(expired_ids)
        .execute(&mut **transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
        })?;
    }

    if ticket_type.capacity - ticket_type.sold_count - held_count < i64::from(command.quantity) {
        return Err(CheckoutError::InventoryUnavailable);
    }
    let amount_due = ticket_type
        .unit_amount
        .checked_mul(Decimal::from(command.quantity))
        .ok_or(CheckoutError::Unavailable)?;
    let reservation_id = format!("zeno_{}", Ulid::new());
    let expires_at = now + Duration::minutes(15);
    let reservation = Reservation {
        id: reservation_id.clone(),
        event_id: command.event_id,
        ticket_type_id: command.ticket_type_id.clone(),
        quantity: command.quantity as u32,
        amount_due: Money {
            amount: amount_due.normalize().to_string(),
            currency: ticket_type.currency,
        },
        status: "held".to_owned(),
        expires_at,
    };

    sqlx::query("UPDATE ticket_types SET held_count = held_count + $2 WHERE id = $1")
        .bind(&command.ticket_type_id)
        .bind(i64::from(command.quantity))
        .execute(&mut **transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
        })?;

    sqlx::query(
        "INSERT INTO reservations \
         (id, event_id, ticket_type_id, quantity, amount_due, currency, status, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, 'held', $7)",
    )
    .bind(&reservation.id)
    .bind(&reservation.event_id)
    .bind(&reservation.ticket_type_id)
    .bind(command.quantity)
    .bind(amount_due)
    .bind(&reservation.amount_due.currency)
    .bind(reservation.expires_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
    })?;

    sqlx::query(
        "INSERT INTO outbox_jobs (job_type, aggregate_id, payload, available_at) \
         VALUES ('reservation.expire', $1, jsonb_build_object('ticket_type_id', $2), $3)",
    )
    .bind(&reservation.id)
    .bind(&reservation.ticket_type_id)
    .bind(reservation.expires_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
    })?;

    let response = serde_json::to_value(&reservation).map_err(|_| CheckoutError::Unavailable)?;
    sqlx::query(
        "UPDATE idempotency_keys SET reservation_id = $2, response = $3 \
         WHERE idempotency_key = $1",
    )
    .bind(&command.idempotency_key)
    .bind(&reservation.id)
    .bind(response)
    .execute(&mut **transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "create_reservation", CheckoutError::Unavailable)
    })?;

    Ok(ReservationResult {
        reservation,
        replay_until,
    })
}
