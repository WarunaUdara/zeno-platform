use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(crate) fn is_public_id(value: &str) -> bool {
    value.strip_prefix("zeno_").is_some_and(|suffix| {
        (16..=32).contains(&suffix.len()) && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateReservationRequest {
    pub quantity: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Money {
    pub amount: String,
    pub currency: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Reservation {
    pub id: String,
    pub event_id: String,
    pub ticket_type_id: String,
    pub quantity: u32,
    pub amount_due: Money,
    pub status: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub(crate) struct ReservationResult {
    pub reservation: Reservation,
    pub replay_until: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReleaseReservationResult {
    pub status: String,
}
