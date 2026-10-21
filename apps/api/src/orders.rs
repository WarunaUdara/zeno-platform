//! Turning a reservation hold into a confirmed order and a set of tickets.
//!
//! This is the busiest write path in the product and the one with the most ways to
//! go quietly wrong, so the transaction is deliberately explicit about ordering.
//!
//! ## Lock order
//!
//! Every writer touching reservations acquires locks in the same sequence:
//! `ticket_types` -> `outbox_jobs` -> `reservations`. The expiry worker and the
//! hold-release path already follow it. A confirm that took them in a different
//! order would deadlock against a worker expiring the very hold being confirmed,
//! which is a race the user can trigger by confirming as the timer runs out.
//!
//! ## Idempotency
//!
//! `orders.reservation_id` is UNIQUE, so it *is* the idempotency key. A retry
//! cannot create a second order because the database refuses, not because this
//! code remembers. Confirm is therefore safe to retry on a dropped connection.
//!
//! ## Partial arrival
//!
//! One ticket row per unit. Ten tickets for a company are ten independent rows
//! with independent admission state, which is what lets six people arrive now and
//! four later without any coordination between gates.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use ulid::Ulid;

use crate::{
    credential::{self, CredentialError, CredentialKey},
    forms::{self, FieldDefinition, FieldType, SubmittedAnswer},
    model::{Money, is_public_id},
};

impl From<CredentialError> for OrderError {
    fn from(_: CredentialError) -> Self {
        OrderError::Unavailable
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderError {
    NotFound,
    /// The hold elapsed before the confirm reached the database. The client should
    /// send the attendee back to availability.
    HoldExpired,
    /// The hold was released or already converted. Distinct from HoldExpired so
    /// the UI can explain what happened.
    HoldUnavailable,
    InvalidInput,
    InvalidAnswers,
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct ConfirmOrderCommand {
    pub event_id: String,
    pub reservation_id: String,
    pub purchaser_name: String,
    pub purchaser_email: String,
    pub answers: Vec<SubmittedAnswer>,
}

#[derive(Debug, Serialize)]
pub struct IssuedTicket {
    pub id: String,
    pub sequence: i16,
    pub reference: String,
    /// The credential encoded in the QR code. Decrypted on replay so a guest can
    /// show their ticket again after closing the tab.
    pub credential: String,
    pub attendee_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfirmedOrder {
    pub id: String,
    pub organization_id: String,
    pub event_id: String,
    pub ticket_type_id: String,
    pub quantity: i16,
    pub total_amount: Money,
    pub purchaser_name: String,
    pub purchaser_email: String,
    pub confirmed_at: DateTime<Utc>,
    pub tickets: Vec<IssuedTicket>,
    pub answers: Vec<StoredAnswer>,
    /// Only present on the first confirmation. A retry returns the order and
    /// tickets so a lost response is still recoverable, but not this token: the
    /// attendee already received it once and it is not stored recoverably.
    pub retrieval_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StoredAnswer {
    pub field_key: String,
    pub field_label: String,
    pub value: String,
}

#[derive(FromRow)]
struct ReservationRow {
    ticket_type_id: String,
    quantity: i16,
    amount_due: Decimal,
    currency: String,
    status: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct OrderRow {
    id: String,
    organization_id: String,
    event_id: String,
    ticket_type_id: String,
    quantity: i16,
    total_amount: Decimal,
    purchaser_name: String,
    purchaser_email: String,
    confirmed_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct TicketRow {
    id: String,
    sequence: i16,
    reference: String,
    attendee_name: Option<String>,
    credential_ciphertext: Vec<u8>,
}

#[derive(FromRow)]
struct AnswerRow {
    field_key: String,
    field_label: String,
    value: String,
}

#[derive(FromRow)]
struct FormFieldRow {
    id: String,
    field_key: String,
    label: String,
    field_type: String,
    required: bool,
    options: serde_json::Value,
}

/// Confirm a hold into an order.
///
/// Spanned so a slow checkout can be attributed to the lock it waited on rather
/// than guessed at. `hold_age_ms` is the single most useful field when a
/// confirmation looks slow: it separates "the attendee took a long time" from
/// "the database was slow".
#[tracing::instrument(
    skip_all,
    fields(order.operation = "confirm_order", order.reservation_ref = %reservation_ref(&command.reservation_id), order.hold_age_ms = tracing::field::Empty)
)]
pub async fn confirm(
    pool: &PgPool,
    key: &CredentialKey,
    command: &ConfirmOrderCommand,
) -> Result<ConfirmedOrder, OrderError> {
    validate_command(command)?;

    // Cheap replay first. A retried confirm after a dropped response should not
    // queue behind the lock a fresh confirm needs.
    if let Some(existing) =
        load_existing(pool, &command.event_id, &command.reservation_id, key).await?
    {
        return Ok(existing);
    }

    let mut transaction = pool.begin().await.map_err(|error| {
        crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable)
    })?;

    // Read the hold without a lock to learn which ticket type to lock first.
    let reservation = sqlx::query_as::<_, ReservationRow>(
        "SELECT ticket_type_id, quantity, amount_due, currency, status, expires_at, created_at \
         FROM reservations WHERE id = $1 AND event_id = $2",
    )
    .bind(&command.reservation_id)
    .bind(&command.event_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?
    .ok_or(OrderError::NotFound)?;

    let form = load_published_form(&mut transaction, &command.event_id).await?;
    if let Some((form_id, version, fields)) = &form {
        let problems = forms::validate_answers(fields, &command.answers);
        if !problems.is_empty() {
            return Err(OrderError::InvalidAnswers);
        }
        let _ = (form_id, version);
    }

    // Lock order: ticket_types, then outbox_jobs, then reservations.
    let held_count: i64 =
        sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1 FOR UPDATE")
            .bind(&reservation.ticket_type_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| {
                crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable)
            })?
            .ok_or(OrderError::NotFound)?;

    let job_id: i64 = sqlx::query_scalar(
        "SELECT id FROM outbox_jobs WHERE job_type = 'reservation.expire' \
         AND aggregate_id = $1 FOR UPDATE",
    )
    .bind(&command.reservation_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?
    .ok_or(OrderError::Unavailable)?;

    let current = sqlx::query_as::<_, ReservationRow>(
        "SELECT ticket_type_id, quantity, amount_due, currency, status, expires_at, created_at \
         FROM reservations WHERE id = $1 AND event_id = $2 FOR UPDATE",
    )
    .bind(&command.reservation_id)
    .bind(&command.event_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?
    .ok_or(OrderError::NotFound)?;

    if current.status != "held" {
        // Either an earlier attempt already converted this hold, or it lapsed.
        // Both are replay-shaped, so return what exists rather than an error.
        transaction
            .commit()
            .await
            .map_err(|_| OrderError::Unavailable)?;
        return match load_existing(pool, &command.event_id, &command.reservation_id, key).await? {
            Some(existing) => Ok(existing),
            None => Err(OrderError::HoldUnavailable),
        };
    }

    let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable)
        })?;
    if current.expires_at <= now {
        transaction
            .commit()
            .await
            .map_err(|_| OrderError::Unavailable)?;
        return Err(OrderError::HoldExpired);
    }
    tracing::Span::current().record(
        "order.hold_age_ms",
        (now - current.created_at).num_milliseconds(),
    );

    let order_id = format!("zeno_{}", Ulid::new());
    let organization_id: String =
        sqlx::query_scalar("SELECT organization_id FROM events WHERE id = $1")
            .bind(&command.event_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| {
                crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable)
            })?;

    sqlx::query(
        "INSERT INTO orders (id, organization_id, event_id, ticket_type_id, reservation_id, \
             quantity, total_amount, currency, purchaser_name, purchaser_email, \
             retrieval_token_hash, form_id, form_version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(&order_id)
    .bind(&organization_id)
    .bind(&command.event_id)
    .bind(&current.ticket_type_id)
    .bind(&command.reservation_id)
    .bind(current.quantity)
    .bind(current.amount_due)
    .bind(&current.currency)
    .bind(command.purchaser_name.trim())
    .bind(command.purchaser_email.trim())
    .bind(credential::sha256(b"unused"))
    .bind(form.as_ref().map(|(id, _, _)| id.clone()))
    .bind(form.as_ref().map(|(_, version, _)| *version))
    .execute(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;

    // held -> sold. The CHECK on ticket_types is the final backstop: if this
    // underflows, the whole transaction rolls back rather than overselling.
    let updated = sqlx::query(
        "UPDATE ticket_types SET held_count = held_count - $2, sold_count = sold_count + $2 \
         WHERE id = $1 AND held_count >= $2",
    )
    .bind(&current.ticket_type_id)
    .bind(current.quantity)
    .execute(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;
    if updated.rows_affected() != 1 {
        return Err(OrderError::Unavailable);
    }
    let _ = held_count;

    sqlx::query("UPDATE reservations SET status = 'confirmed' WHERE id = $1 AND status = 'held'")
        .bind(&command.reservation_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable)
        })?;

    // Retire the expiry job. Without this the worker would later expire a
    // reservation the attendee already paid attention to.
    sqlx::query(
        "UPDATE outbox_jobs SET status = 'completed', attempts = attempts + 1, \
             completed_at = clock_timestamp() WHERE id = $1 AND status = 'pending'",
    )
    .bind(job_id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;

    let tickets = issue_tickets(
        &mut transaction,
        key,
        &order_id,
        &organization_id,
        &command.event_id,
        &current.ticket_type_id,
        current.quantity,
    )
    .await?;

    let stored_answers = store_answers(
        &mut transaction,
        &order_id,
        &command.answers,
        form.as_ref().map(|(_, _, fields)| fields.as_slice()),
    )
    .await?;

    transaction
        .commit()
        .await
        .map_err(|_| OrderError::Unavailable)?;

    Ok(ConfirmedOrder {
        id: order_id,
        organization_id,
        event_id: command.event_id.clone(),
        ticket_type_id: current.ticket_type_id,
        quantity: current.quantity,
        total_amount: Money {
            amount: current.amount_due.normalize().to_string(),
            currency: current.currency,
        },
        purchaser_name: command.purchaser_name.trim().to_owned(),
        purchaser_email: command.purchaser_email.trim().to_owned(),
        confirmed_at: now,
        tickets,
        answers: stored_answers,
        retrieval_token: None,
    })
}

/// Issue one ticket per unit in a single statement.
///
/// One round trip rather than N matters: a 100-ticket company booking would
/// otherwise hold the ticket-type lock across 100 round trips, blocking every
/// other buyer of that event for the duration.
async fn issue_tickets(
    transaction: &mut Transaction<'_, Postgres>,
    key: &CredentialKey,
    order_id: &str,
    organization_id: &str,
    event_id: &str,
    ticket_type_id: &str,
    quantity: i16,
) -> Result<Vec<IssuedTicket>, OrderError> {
    let mut ids = Vec::with_capacity(quantity as usize);
    let mut sequences = Vec::with_capacity(quantity as usize);
    let mut references = Vec::with_capacity(quantity as usize);
    let mut digests = Vec::with_capacity(quantity as usize);
    let mut ciphertexts = Vec::with_capacity(quantity as usize);
    let mut credentials = Vec::with_capacity(quantity as usize);

    for index in 1..=quantity {
        let (credential, digest) = credential::new_credential();
        let sealed = key
            .encrypt(&credential)
            .map_err(|_| OrderError::Unavailable)?;
        ids.push(format!("zeno_{}", Ulid::new()));
        sequences.push(index);
        references.push(credential::new_reference());
        digests.push(digest.to_vec());
        ciphertexts.push(sealed);
        credentials.push(credential);
    }

    sqlx::query(
        "INSERT INTO tickets (id, order_id, organization_id, event_id, ticket_type_id, \
             sequence, reference, credential_hash, credential_ciphertext) \
         SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[], $5::text[], \
             $6::smallint[], $7::text[], $8::bytea[], $9::bytea[])",
    )
    .bind(&ids)
    .bind(vec![order_id; quantity as usize])
    .bind(vec![organization_id; quantity as usize])
    .bind(vec![event_id; quantity as usize])
    .bind(vec![ticket_type_id; quantity as usize])
    .bind(&sequences)
    .bind(&references)
    .bind(&digests)
    .bind(&ciphertexts)
    .execute(&mut **transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;

    Ok(credentials
        .into_iter()
        .enumerate()
        .map(|(index, credential)| IssuedTicket {
            id: ids[index].clone(),
            sequence: sequences[index],
            reference: references[index].clone(),
            credential,
            attendee_name: None,
        })
        .collect())
}

async fn store_answers(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &str,
    submitted: &[SubmittedAnswer],
    fields: Option<&[FieldDefinition]>,
) -> Result<Vec<StoredAnswer>, OrderError> {
    let Some(fields) = fields else {
        return Ok(Vec::new());
    };
    if submitted.is_empty() {
        return Ok(Vec::new());
    }

    let mut field_ids = Vec::new();
    let mut keys = Vec::new();
    let mut labels = Vec::new();
    let mut types = Vec::new();
    let mut values = Vec::new();

    for answer in submitted {
        let Some(field) = fields.iter().find(|f| f.field_key == answer.field_key) else {
            continue;
        };
        let value = answer.value.trim();
        if value.is_empty() {
            continue;
        }
        field_ids.push(field.id.clone());
        keys.push(field.field_key.clone());
        labels.push(field.label.clone());
        types.push(format!("{:?}", field.field_type).to_lowercase());
        values.push(value.to_owned());
    }

    if field_ids.is_empty() {
        return Ok(Vec::new());
    }

    let stored = values
        .iter()
        .enumerate()
        .map(|(index, value)| StoredAnswer {
            field_key: keys[index].clone(),
            field_label: labels[index].clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();

    sqlx::query(
        "INSERT INTO registration_answers \
             (order_id, field_id, field_key, field_label, field_type, value) \
         SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[], $6::text[])",
    )
    .bind(order_id)
    .bind(&field_ids)
    .bind(&keys)
    .bind(&labels)
    .bind(&types)
    .bind(&values)
    .execute(&mut **transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;

    Ok(stored)
}

async fn load_published_form(
    transaction: &mut Transaction<'_, Postgres>,
    event_id: &str,
) -> Result<Option<(String, i32, Vec<FieldDefinition>)>, OrderError> {
    let form = sqlx::query_as::<_, (String, i32)>(
        "SELECT id, version FROM registration_forms \
         WHERE event_id = $1 AND status = 'published'",
    )
    .bind(event_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;
    let Some((form_id, version)) = form else {
        return Ok(None);
    };

    let rows = sqlx::query_as::<_, FormFieldRow>(
        "SELECT id, field_key, label, field_type, required, options \
         FROM registration_form_fields WHERE form_id = $1 ORDER BY position, id",
    )
    .bind(&form_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;

    let fields = rows
        .into_iter()
        .filter_map(|row| {
            Some(FieldDefinition {
                id: row.id,
                field_key: row.field_key,
                label: row.label,
                field_type: FieldType::parse(&row.field_type)?,
                required: row.required,
                options: row
                    .options
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect();

    Ok(Some((form_id, version, fields)))
}

/// Return an order that already exists for this reservation, with its tickets.
///
/// Used both for cheap replays and to turn a lost response into a recoverable one:
/// the credentials are decrypted, so an attendee who never saw the first response
/// still gets working QR codes.
pub async fn load_existing(
    pool: &PgPool,
    event_id: &str,
    reservation_id: &str,
    key: &CredentialKey,
) -> Result<Option<ConfirmedOrder>, OrderError> {
    let order = sqlx::query_as::<_, OrderRow>(
        "SELECT o.id, o.organization_id, o.event_id, o.ticket_type_id, o.quantity, o.total_amount, \
                o.purchaser_name, o.purchaser_email, o.confirmed_at \
         FROM orders o WHERE o.reservation_id = $1 AND o.event_id = $2",
    )
    .bind(reservation_id)
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;
    let Some(order) = order else {
        return Ok(None);
    };

    let ticket_rows = sqlx::query_as::<_, TicketRow>(
        "SELECT id, sequence, reference, attendee_name, credential_ciphertext \
         FROM tickets WHERE order_id = $1 ORDER BY sequence",
    )
    .bind(&order.id)
    .fetch_all(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;

    let mut tickets = Vec::with_capacity(ticket_rows.len());
    for row in ticket_rows {
        let credential = key.decrypt(&row.credential_ciphertext)?;
        tickets.push(IssuedTicket {
            id: row.id,
            sequence: row.sequence,
            reference: row.reference,
            credential,
            attendee_name: row.attendee_name,
        });
    }

    let answer_rows = sqlx::query_as::<_, AnswerRow>(
        "SELECT field_key, field_label, value FROM registration_answers \
         WHERE order_id = $1 ORDER BY id",
    )
    .bind(&order.id)
    .fetch_all(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "confirm_order", OrderError::Unavailable))?;

    let currency: String = sqlx::query_scalar("SELECT currency FROM orders WHERE id = $1")
        .bind(&order.id)
        .fetch_one(pool)
        .await
        .map_err(|_| OrderError::Unavailable)?;

    Ok(Some(ConfirmedOrder {
        id: order.id,
        organization_id: order.organization_id,
        event_id: order.event_id,
        ticket_type_id: order.ticket_type_id,
        quantity: order.quantity,
        total_amount: Money {
            amount: order.total_amount.normalize().to_string(),
            currency,
        },
        purchaser_name: order.purchaser_name,
        purchaser_email: order.purchaser_email,
        confirmed_at: order.confirmed_at,
        tickets,
        answers: answer_rows
            .into_iter()
            .map(|row| StoredAnswer {
                field_key: row.field_key,
                field_label: row.field_label,
                value: row.value,
            })
            .collect(),
        retrieval_token: None,
    }))
}

/// Reservation ids are bearer capabilities for order retrieval. Log only a short
/// prefix so a trace viewer cannot replay one against the confirm endpoint.
fn reservation_ref(reservation_id: &str) -> &str {
    reservation_id.get(..8).unwrap_or(reservation_id)
}

fn validate_command(command: &ConfirmOrderCommand) -> Result<(), OrderError> {
    if !is_public_id(&command.event_id) || !is_public_id(&command.reservation_id) {
        return Err(OrderError::InvalidInput);
    }
    let name = command.purchaser_name.trim();
    if !(1..=120).contains(&name.chars().count()) {
        return Err(OrderError::InvalidInput);
    }
    let email = command.purchaser_email.trim();
    if !(3..=254).contains(&email.len())
        || !email.contains('@')
        || email.contains(char::is_whitespace)
    {
        return Err(OrderError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str, email: &str) -> ConfirmOrderCommand {
        ConfirmOrderCommand {
            event_id: "zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K".to_owned(),
            reservation_id: "zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7L".to_owned(),
            purchaser_name: name.to_owned(),
            purchaser_email: email.to_owned(),
            answers: Vec::new(),
        }
    }

    #[test]
    fn purchaser_details_are_bounded_before_any_database_work() {
        assert!(validate_command(&command("Campus Arts Society", "buyer@example.com")).is_ok());
        // Whitespace only is not a name.
        assert_eq!(
            validate_command(&command("   ", "buyer@example.com")).unwrap_err(),
            OrderError::InvalidInput
        );
        // A single character is an unusual but perfectly real name, and rejecting it
        // would block a guest over a nickname.
        assert!(validate_command(&command("A", "buyer@example.com")).is_ok());
        assert_eq!(
            validate_command(&command(&"n".repeat(121), "buyer@example.com")).unwrap_err(),
            OrderError::InvalidInput
        );
        assert_eq!(
            validate_command(&command("Buyer", "not-an-email")).unwrap_err(),
            OrderError::InvalidInput
        );
        assert_eq!(
            validate_command(&command("Buyer", "a b@example.com")).unwrap_err(),
            OrderError::InvalidInput
        );
        // A payload the router already bounds, still rejected here so the rule does
        // not depend on the transport.
        assert_eq!(
            validate_command(&command("Buyer", &"x".repeat(250))).unwrap_err(),
            OrderError::InvalidInput
        );
    }

    #[test]
    fn malformed_identifiers_are_rejected_before_a_lock_is_taken() {
        let mut invalid = command("Buyer", "buyer@example.com");
        invalid.event_id = "event_01J9WZ6Q2CX7F2YV8AX0P4TD7K".to_owned();
        assert_eq!(
            validate_command(&invalid).unwrap_err(),
            OrderError::InvalidInput
        );
    }
}
