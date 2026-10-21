use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::model::{CreateReservationRequest, ReservationResult, is_public_id};

#[derive(Debug, Error)]
pub(crate) enum CheckoutError {
    #[error("invalid public id")]
    InvalidPublicId,
    #[error("invalid idempotency key")]
    InvalidIdempotencyKey,
    #[error("invalid quantity")]
    InvalidQuantity,
    #[error("event or ticket type was not found")]
    NotFound,
    #[error("reservation was not found")]
    ReservationNotFound,
    #[error("ticket sale is not open")]
    SaleClosed,
    #[error("requested inventory is unavailable")]
    InventoryUnavailable,
    #[error("idempotency key was used for a different request")]
    IdempotencyConflict,
    #[error("reservation service is temporarily unavailable")]
    Unavailable,
}

pub(crate) fn validate_idempotency_key(key: &str) -> Result<(), CheckoutError> {
    let decoded_key = URL_SAFE_NO_PAD
        .decode(key)
        .map_err(|_| CheckoutError::InvalidIdempotencyKey)?;
    if decoded_key.len() < 16 || key.len() > 128 {
        return Err(CheckoutError::InvalidIdempotencyKey);
    }
    Ok(())
}

pub(crate) struct ReservationCommand {
    pub event_id: String,
    pub ticket_type_id: String,
    pub idempotency_key: String,
    pub fingerprint: Vec<u8>,
    pub quantity: i16,
}

pub(crate) trait InventoryRepository: Send + Sync {
    async fn reserve(
        &self,
        command: ReservationCommand,
    ) -> Result<ReservationResult, CheckoutError>;
}

#[derive(Clone)]
pub(crate) struct CheckoutService<R> {
    repository: R,
}

impl<R> CheckoutService<R>
where
    R: InventoryRepository,
{
    pub(crate) fn new(repository: R) -> Self {
        Self { repository }
    }

    pub(crate) async fn reserve(
        &self,
        event_id: &str,
        ticket_type_id: &str,
        key: &str,
        request: CreateReservationRequest,
    ) -> Result<ReservationResult, CheckoutError> {
        if !is_public_id(event_id) || !is_public_id(ticket_type_id) {
            return Err(CheckoutError::InvalidPublicId);
        }
        validate_idempotency_key(key)?;
        if !(1..=100).contains(&request.quantity) {
            return Err(CheckoutError::InvalidQuantity);
        }
        let payload = FingerprintPayload {
            event_id,
            ticket_type_id,
            quantity: request.quantity,
        };
        let canonical = serde_json::to_vec(&payload).map_err(|_| CheckoutError::Unavailable)?;
        let fingerprint = Sha256::digest(canonical).to_vec();

        self.repository
            .reserve(ReservationCommand {
                event_id: event_id.to_owned(),
                ticket_type_id: ticket_type_id.to_owned(),
                idempotency_key: key.to_owned(),
                fingerprint,
                quantity: request.quantity as i16,
            })
            .await
    }
}

#[derive(Serialize)]
struct FingerprintPayload<'a> {
    event_id: &'a str,
    ticket_type_id: &'a str,
    quantity: u32,
}

#[cfg(test)]
mod tests {
    use crate::model::is_public_id;

    #[test]
    fn public_ids_follow_the_openapi_shape() {
        assert!(is_public_id("zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K"));
        assert!(!is_public_id("event_01J9WZ6Q2CX7F2YV8AX0P4TD7K"));
        assert!(!is_public_id("zeno_short"));
        assert!(!is_public_id("zeno_01J9-WZ6Q2CX7F2YV8AX0P4TD7K"));
    }
}
