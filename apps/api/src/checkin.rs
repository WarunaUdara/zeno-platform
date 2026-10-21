//! Admission at the door.
//!
//! This is the highest-concurrency, highest-stakes path in the product: several
//! gates may scan the same guest within the same second, scanners retry over bad
//! venue Wi-Fi, and staff make mistakes that must be correctable.
//!
//! ## Why the update is written the way it is
//!
//! Admission is a conditional state transition, expressed as one statement:
//!
//! ```sql
//! UPDATE tickets SET status = 'checked_in', checked_in_at = ...
//! WHERE id = $1 AND status = 'valid'
//! ```
//!
//! The row lock Postgres takes for that update is what serialises two gates. The
//! loser updates zero rows and is told the guest is already admitted. Doing this
//! as a read-then-write would admit the same guest twice under exactly the race the
//! feature exists to prevent.
//!
//! ## Partial arrival
//!
//! Nothing here knows how many tickets an order has. Each ticket is admitted on its
//! own, so six of a company's ten guests arriving at once and the remaining four an
//! hour later needs no coordination. `attendance_for_event` reports the split.
//!
//! ## Retry safety
//!
//! `scanner_action_id` is client-generated and unique in the ledger, so a scanner
//! that never saw a response retries and receives the original outcome instead of
//! recording a second admission.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

use crate::{auth::AuthenticatedIdentity, credential, db_error};

/// Largest batch a single scan request may carry. Bounds the query and stops one
/// request from locking an unbounded number of rows at a gate.
const MAX_SCAN_BATCH: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckInError {
    NotFound,
    /// Ticket exists but cannot be admitted: already used, or voided.
    NotAdmissible,
    /// Held by a different member of the organization, so the caller must not learn
    /// that it exists.
    ScopeMismatch,
    InvalidInput,
    Unavailable,
}

/// What happened to one scanned code.
///
/// Deliberately flat and explicit rather than a boolean. A scanner needs to say
/// "already used at 18:42" or "this was voided", because those need different
/// words at the door.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ScanOutcome {
    Admitted {
        ticket_id: String,
        sequence: i16,
        reference: String,
        attendee_name: Option<String>,
    },
    /// Already admitted. Carries when and by whom, so staff can resolve a
    /// "my colleague is already inside" argument without leaving the gate.
    AlreadyAdmitted {
        ticket_id: String,
        sequence: i16,
        reference: String,
        checked_in_at: DateTime<Utc>,
    },
    Voided {
        ticket_id: String,
        sequence: i16,
        reference: String,
    },
    /// No ticket in this event matches the code. Indistinguishable from a typo, and
    /// deliberately so.
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub outcome: ScanOutcome,
    /// True when this request was a retry of a scan already recorded.
    pub replayed: bool,
}

#[derive(FromRow)]
struct TicketLookup {
    id: String,
    sequence: i16,
    reference: String,
    attendee_name: Option<String>,
    status: String,
    checked_in_at: Option<DateTime<Utc>>,
    organization_id: String,
}

#[derive(FromRow)]
struct TicketIdentity {
    id: String,
    sequence: i16,
    reference: String,
    attendee_name: Option<String>,
}

#[derive(FromRow)]
struct LedgerRow {
    ticket_id: String,
}

/// Admit up to `MAX_SCAN_BATCH` codes in one round trip.
///
/// A group walking in together produces one request rather than ten, which is the
/// difference between a snappy door and a visible queue.
///
/// Spanned as one operation rather than one per code: when a group queues the
/// useful question is how long the batch took, not how long seven statements took.
#[tracing::instrument(
    skip_all,
    fields(
        check_in.operation = "scan",
        check_in.event_id = %event_id,
        check_in.codes = codes.len()
    )
)]
pub async fn scan(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
    scanner_action_id: &str,
    codes: &[String],
) -> Result<Vec<ScanResult>, CheckInError> {
    if codes.is_empty() || codes.len() > MAX_SCAN_BATCH {
        return Err(CheckInError::InvalidInput);
    }
    let has_membership = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events e \
         JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.id = $1 AND e.organization_id = $2 \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager', 'check_in_manager'))",
    )
    .bind(event_id)
    .bind(organization_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(pool)
    .await
    .map_err(|error| db_error::record_as(error, "check_in_scan", CheckInError::Unavailable))?;
    if !has_membership {
        return Err(CheckInError::ScopeMismatch);
    }

    let mut results = Vec::with_capacity(codes.len());
    for (index, code) in codes.iter().enumerate() {
        // A batch is one scanner action but many tickets. Reusing the caller's
        // action id for every code would make the second code's retry check match
        // the first code's ledger row and report the wrong guest, so each item
        // gets its own derived, still-deterministic action id.
        let item_action_id = per_item_action_id(scanner_action_id, index);
        results.push(
            scan_one(
                pool,
                identity,
                organization_id,
                event_id,
                &item_action_id,
                code,
            )
            .await?,
        );
    }
    Ok(results)
}

/// Derive a per-ticket action id from the caller's batch id.
///
/// Deterministic, so retrying the identical batch replays instead of
/// re-recording, and short enough for the ledger's length check.
fn per_item_action_id(batch_id: &str, index: usize) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let material = format!("{batch_id}:{index}");
    URL_SAFE_NO_PAD.encode(credential::sha256(material.as_bytes()))
}

async fn scan_one(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
    scanner_action_id: &str,
    code: &str,
) -> Result<ScanResult, CheckInError> {
    // The retry case is settled before any lookup: if this exact scan was already
    // recorded, return what was recorded and touch nothing.
    if let Some(previous) =
        replayed_action(pool, organization_id, event_id, scanner_action_id).await?
    {
        return Ok(ScanResult {
            outcome: previous,
            replayed: true,
        });
    }

    // Accept either a QR credential or a typed short reference. Rejecting a
    // malformed value here means rubbish never reaches a query.
    let lookup = if credential::is_credential_shape(code) {
        sqlx::query_as::<_, TicketLookup>(
            "SELECT id, sequence, reference, attendee_name, status, checked_in_at, \
                    organization_id \
             FROM tickets WHERE event_id = $1 AND credential_hash = $2",
        )
        .bind(event_id)
        .bind(credential::sha256(code.as_bytes()).to_vec())
        .fetch_optional(pool)
        .await
    } else if credential::is_reference_shape(code) {
        sqlx::query_as::<_, TicketLookup>(
            "SELECT id, sequence, reference, attendee_name, status, checked_in_at, \
                    organization_id \
             FROM tickets WHERE event_id = $1 AND reference = $2",
        )
        .bind(event_id)
        .bind(code.to_ascii_uppercase())
        .fetch_optional(pool)
        .await
    } else {
        return Ok(ScanResult {
            outcome: ScanOutcome::Unknown,
            replayed: false,
        });
    }
    .map_err(|error| db_error::record_as(error, "check_in_scan", CheckInError::Unavailable))?;

    let Some(ticket) = lookup else {
        return Ok(ScanResult {
            outcome: ScanOutcome::Unknown,
            replayed: false,
        });
    };
    if ticket.organization_id != organization_id {
        // Cross-tenant scan. Report it as unknown rather than forbidden, so a
        // scanner cannot probe for which ticket ids exist elsewhere.
        return Ok(ScanResult {
            outcome: ScanOutcome::Unknown,
            replayed: false,
        });
    }

    match ticket.status.as_str() {
        "checked_in" => {
            // A repeat scan is not an admission and not a correction, so it writes
            // nothing to the ledger. Recording it would put a second "admit" row
            // for one guest in the audit trail.
            let checked_in_at = ticket.checked_in_at.unwrap_or_else(Utc::now);
            return Ok(ScanResult {
                outcome: ScanOutcome::AlreadyAdmitted {
                    ticket_id: ticket.id,
                    sequence: ticket.sequence,
                    reference: ticket.reference,
                    checked_in_at,
                },
                replayed: false,
            });
        }
        "voided" => {
            return Ok(ScanResult {
                outcome: ScanOutcome::Voided {
                    ticket_id: ticket.id,
                    sequence: ticket.sequence,
                    reference: ticket.reference,
                },
                replayed: false,
            });
        }
        _ => {}
    }

    // The conditional update is the serialisation point for concurrent gates. Two
    // scanners reaching this line for the same ticket are serialised by the row
    // lock, and exactly one sees a changed row.
    let admitted = sqlx::query_as::<_, TicketIdentity>(
        "UPDATE tickets SET status = 'checked_in', checked_in_at = clock_timestamp() \
         WHERE id = $1 AND status = 'valid' RETURNING id, sequence, reference, attendee_name",
    )
    .bind(&ticket.id)
    .fetch_optional(pool)
    .await
    .map_err(|error| db_error::record_as(error, "check_in_scan", CheckInError::Unavailable))?;

    let Some(admitted) = admitted else {
        // Lost the race: another gate admitted this guest a moment ago.
        let current = sqlx::query_as::<_, TicketLookup>(
            "SELECT id, sequence, reference, attendee_name, status, checked_in_at, \
                    organization_id \
             FROM tickets WHERE id = $1",
        )
        .bind(&ticket.id)
        .fetch_one(pool)
        .await
        .map_err(|error| db_error::record_as(error, "check_in_scan", CheckInError::Unavailable))?;
        return Ok(ScanResult {
            outcome: ScanOutcome::AlreadyAdmitted {
                ticket_id: current.id,
                sequence: current.sequence,
                reference: current.reference,
                checked_in_at: current.checked_in_at.unwrap_or_else(Utc::now),
            },
            replayed: false,
        });
    };

    append_ledger(
        pool,
        &admitted.id,
        identity,
        organization_id,
        event_id,
        scanner_action_id,
        "admit",
    )
    .await?;

    Ok(ScanResult {
        outcome: ScanOutcome::Admitted {
            ticket_id: admitted.id,
            sequence: admitted.sequence,
            reference: admitted.reference,
            attendee_name: admitted.attendee_name,
        },
        replayed: false,
    })
}

async fn append_ledger(
    pool: &PgPool,
    ticket_id: &str,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
    scanner_action_id: &str,
    action: &str,
) -> Result<(), CheckInError> {
    sqlx::query(
        "INSERT INTO check_ins (ticket_id, organization_id, event_id, action, \
             actor_subject, actor_issuer, scanner_action_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (scanner_action_id) DO NOTHING",
    )
    .bind(ticket_id)
    .bind(organization_id)
    .bind(event_id)
    .bind(action)
    .bind(&identity.subject)
    .bind(&identity.issuer)
    .bind(scanner_action_id)
    .execute(pool)
    .await
    .map_err(|error| db_error::record_as(error, "check_in_scan", CheckInError::Unavailable))?;
    Ok(())
}

async fn replayed_action(
    pool: &PgPool,
    organization_id: &str,
    event_id: &str,
    scanner_action_id: &str,
) -> Result<Option<ScanOutcome>, CheckInError> {
    // The action id is caller-controlled and the ledger's uniqueness is global,
    // so scope the replay lookup to the caller's tenant. Otherwise a member of
    // one organization could replay another organization's action id and read
    // the ticket it admitted.
    let row = sqlx::query_as::<_, LedgerRow>(
        "SELECT c.ticket_id FROM check_ins c \
         WHERE c.scanner_action_id = $1 AND c.organization_id = $2 AND c.event_id = $3",
    )
    .bind(scanner_action_id)
    .bind(organization_id)
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| db_error::record_as(error, "check_in_scan", CheckInError::Unavailable))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let ticket = sqlx::query_as::<_, TicketLookup>(
        "SELECT id, sequence, reference, attendee_name, status, checked_in_at, \
                organization_id FROM tickets \
         WHERE id = $1 AND organization_id = $2 AND event_id = $3",
    )
    .bind(&row.ticket_id)
    .bind(organization_id)
    .bind(event_id)
    .fetch_one(pool)
    .await
    .map_err(|error| db_error::record_as(error, "check_in_scan", CheckInError::Unavailable))?;

    Ok(Some(if ticket.status == "checked_in" {
        ScanOutcome::AlreadyAdmitted {
            ticket_id: ticket.id,
            sequence: ticket.sequence,
            reference: ticket.reference,
            checked_in_at: ticket.checked_in_at.unwrap_or_else(Utc::now),
        }
    } else {
        ScanOutcome::Admitted {
            ticket_id: ticket.id,
            sequence: ticket.sequence,
            reference: ticket.reference,
            attendee_name: ticket.attendee_name,
        }
    }))
}

#[derive(Debug, Serialize)]
pub struct Attendance {
    pub event_id: String,
    pub issued: i64,
    pub admitted: i64,
    pub awaiting: i64,
    pub voided: i64,
}

/// Arrival counts for one event, derived from ticket rows.
///
/// Derived rather than incremented, so it cannot drift from the ticket state the
/// same way a stored counter would.
pub async fn attendance_for_event(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
) -> Result<Attendance, CheckInError> {
    let may_view = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events e \
         JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.id = $1 AND e.organization_id = $2 \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager', 'check_in_manager', 'analyst'))",
    )
    .bind(event_id)
    .bind(organization_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        db_error::record_as(error, "check_in_attendance", CheckInError::Unavailable)
    })?;
    if !may_view {
        return Err(CheckInError::ScopeMismatch);
    }

    let row = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT count(*)::bigint, \
                count(*) FILTER (WHERE status = 'checked_in'), \
                count(*) FILTER (WHERE status = 'voided'), \
                count(*) FILTER (WHERE status = 'valid') \
         FROM tickets WHERE event_id = $1 AND organization_id = $2",
    )
    .bind(event_id)
    .bind(organization_id)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        db_error::record_as(error, "check_in_attendance", CheckInError::Unavailable)
    })?;

    Ok(Attendance {
        event_id: event_id.to_owned(),
        issued: row.0,
        admitted: row.1,
        voided: row.2,
        awaiting: row.3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scan_batch_is_bounded() {
        // One request must not be able to lock an unbounded number of rows at a
        // gate, which is how a single malformed client request becomes an outage.
        assert_eq!(MAX_SCAN_BATCH, 50);
    }

    #[test]
    fn each_code_in_a_batch_gets_its_own_replay_key() {
        // Sharing one key across a batch made the second code's retry check match
        // the first code's ledger row, so a group walking in together was reported
        // as one admission and nine already-admitted guests.
        let first = per_item_action_id("scan-batch-1", 0);
        let second = per_item_action_id("scan-batch-1", 1);
        assert_ne!(first, second);

        // The same batch retried must replay rather than record again.
        assert_eq!(first, per_item_action_id("scan-batch-1", 0));
        assert_ne!(first, per_item_action_id("scan-batch-2", 0));

        // Within the ledger's shape limits.
        for id in [first, second] {
            assert!((8..=64).contains(&id.len()), "unexpected length {id}");
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            );
        }
    }
}
