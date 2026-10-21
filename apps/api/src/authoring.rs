use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, types::Json};

use crate::{auth::AuthenticatedIdentity, catalog::Venue};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateEventRequest {
    pub slug: String,
    pub title: String,
    pub timezone: String,
    pub summary: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub venue: Option<Venue>,
    pub accessibility_info: Option<String>,
    pub cancellation_policy: Option<String>,
    pub support_email: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateTicketRequest {
    pub name: String,
    pub description: Option<String>,
    pub amount: String,
    pub currency: String,
    pub capacity: i64,
    pub sale_starts_at: DateTime<Utc>,
    pub sale_ends_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateEventRequest {
    pub title: String,
    pub timezone: String,
    pub summary: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub venue: Option<Venue>,
    pub accessibility_info: Option<String>,
    pub cancellation_policy: Option<String>,
    pub support_email: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCoverUploadRequest {
    pub content_type: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct OrganizerEvent {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub summary: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub visibility: String,
    pub venue: Option<Venue>,
    pub accessibility_info: Option<String>,
    pub cancellation_policy: Option<String>,
    pub support_email: Option<String>,
    pub cover_image_key: Option<String>,
    pub cover_image_alt: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoverImageRequest {
    #[serde(deserialize_with = "deserialize_required_option")]
    pub key: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub alt: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OrganizerEventSummary {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub summary: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub visibility: String,
    pub venue: Option<Venue>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OrganizerTicket {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub amount: String,
    pub currency: String,
    pub capacity: i64,
    pub sale_starts_at: DateTime<Utc>,
    pub sale_ends_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OrganizerEventPage {
    pub items: Vec<OrganizerEventSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct OrganizerEventCursor {
    starts_at: Option<DateTime<Utc>>,
    id: String,
}

impl OrganizerEventCursor {
    pub(crate) fn decode(value: &str) -> Option<Self> {
        if value.len() > 512 {
            return None;
        }
        let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
        let decoded = std::str::from_utf8(&decoded).ok()?;
        let mut fields = decoded.splitn(3, '\n');
        if fields.next()? != "v1" {
            return None;
        }
        let starts_at = match fields.next()? {
            "~" => None,
            value => Some(
                DateTime::parse_from_rfc3339(value)
                    .ok()?
                    .with_timezone(&Utc),
            ),
        };
        let id = fields.next()?;
        if !crate::model::is_public_id(id) {
            return None;
        }
        Some(Self {
            starts_at,
            id: id.to_owned(),
        })
    }
}

fn encode_event_cursor(starts_at: Option<DateTime<Utc>>, id: &str) -> String {
    let starts_at = starts_at
        .map(|date| date.to_rfc3339())
        .unwrap_or_else(|| "~".to_owned());
    URL_SAFE_NO_PAD.encode(format!("v1\n{starts_at}\n{id}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthoringError {
    InvalidInput,
    Conflict,
    NotFound,
    NotReady,
    Unavailable,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(FromRow)]
struct EventRow {
    id: String,
    slug: String,
    title: String,
    summary: Option<String>,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
    timezone: String,
    visibility: String,
    venue: Option<Json<Venue>>,
    accessibility_info: Option<String>,
    cancellation_policy: Option<String>,
    support_email: Option<String>,
    cover_image_key: Option<String>,
    cover_image_alt: Option<String>,
}

#[derive(FromRow)]
struct EventSummaryRow {
    id: String,
    slug: String,
    title: String,
    summary: Option<String>,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
    timezone: String,
    visibility: String,
    venue: Option<Json<Venue>>,
}

#[derive(FromRow)]
struct TicketRow {
    id: String,
    name: String,
    description: Option<String>,
    unit_amount: Decimal,
    currency: String,
    capacity: i64,
    sale_starts_at: DateTime<Utc>,
    sale_ends_at: DateTime<Utc>,
}

pub(crate) fn is_valid_event(request: &CreateEventRequest) -> bool {
    is_valid_slug(&request.slug)
        && (1..=160).contains(&request.title.trim().chars().count())
        && !request.timezone.trim().is_empty()
        && request.timezone.len() <= 128
        && request
            .summary
            .as_ref()
            .is_none_or(|value| value.chars().count() <= 1000)
        && request
            .ends_at
            .zip(request.starts_at)
            .is_none_or(|(end, start)| end > start)
        && request.venue.as_ref().is_none_or(is_valid_venue)
        && is_valid_attendee_disclosures(
            &request.accessibility_info,
            &request.cancellation_policy,
            &request.support_email,
        )
}

pub(crate) fn is_valid_update(request: &UpdateEventRequest) -> bool {
    (1..=160).contains(&request.title.trim().chars().count())
        && !request.timezone.trim().is_empty()
        && request.timezone.len() <= 128
        && request
            .summary
            .as_ref()
            .is_none_or(|value| value.chars().count() <= 1000)
        && request
            .ends_at
            .zip(request.starts_at)
            .is_none_or(|(end, start)| end > start)
        && request.venue.as_ref().is_none_or(is_valid_venue)
        && is_valid_attendee_disclosures(
            &request.accessibility_info,
            &request.cancellation_policy,
            &request.support_email,
        )
}

pub(crate) fn cover_image_extension(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

pub(crate) fn is_valid_cover_image(
    organization_id: &str,
    event_id: &str,
    key: &str,
    alt: &str,
) -> bool {
    let Some(filename) = key.strip_prefix(&format!("events/{organization_id}/{event_id}/")) else {
        return false;
    };
    let Some((ulid, extension)) = filename.rsplit_once('.') else {
        return false;
    };
    (1..=240).contains(&alt.trim().chars().count())
        && !alt.chars().any(char::is_control)
        && ulid.len() == 26
        && ulid
            .bytes()
            .all(|byte| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&byte))
        && matches!(extension, "jpg" | "png" | "webp")
}

fn clean_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn is_valid_attendee_disclosures(
    accessibility_info: &Option<String>,
    cancellation_policy: &Option<String>,
    support_email: &Option<String>,
) -> bool {
    accessibility_info
        .as_ref()
        .is_none_or(|value| value.chars().count() <= 2000)
        && cancellation_policy
            .as_ref()
            .is_none_or(|value| value.chars().count() <= 2000)
        && support_email
            .as_ref()
            .is_none_or(|email| is_valid_support_email(email))
}

fn is_valid_support_email(email: &str) -> bool {
    let email = email.trim();
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    let labels = domain.split('.').collect::<Vec<_>>();
    email.len() <= 254
        && local.len() <= 64
        && !local.is_empty()
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&byte))
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && label
                    .bytes()
                    .last()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_valid_slug(slug: &str) -> bool {
    (3..=80).contains(&slug.len())
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && slug.as_bytes()[0].is_ascii_alphanumeric()
        && slug.as_bytes()[slug.len() - 1].is_ascii_alphanumeric()
}

fn is_valid_venue(venue: &Venue) -> bool {
    match venue {
        Venue::Online => true,
        Venue::Physical {
            name,
            city,
            country_code,
        } => {
            (1..=160).contains(&name.trim().chars().count())
                && (1..=120).contains(&city.trim().chars().count())
                && country_code.len() == 2
                && country_code.bytes().all(|byte| byte.is_ascii_uppercase())
        }
    }
}

pub(crate) fn is_valid_ticket(request: &CreateTicketRequest) -> bool {
    (1..=120).contains(&request.name.trim().chars().count())
        && request
            .description
            .as_ref()
            .is_none_or(|value| value.chars().count() <= 500)
        && is_valid_amount(&request.amount)
        && request.currency.len() == 3
        && request
            .currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
        && request.capacity > 0
        && request.sale_ends_at > request.sale_starts_at
}

fn is_valid_amount(amount: &str) -> bool {
    let mut parts = amount.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    let decimal_shape = !whole.is_empty()
        && whole.len() <= 14
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && (whole == "0" || !whole.starts_with('0'))
        && fraction.is_none_or(|fraction| {
            (1..=4).contains(&fraction.len()) && fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
        && parts.next().is_none();
    decimal_shape && Decimal::from_str_exact(amount).is_ok_and(|value| value >= Decimal::ZERO)
}

pub(crate) async fn list_events(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    cursor: Option<OrganizerEventCursor>,
    limit: i64,
) -> Result<OrganizerEventPage, AuthoringError> {
    let rows = match cursor {
        Some(OrganizerEventCursor {
            starts_at: Some(starts_at),
            id,
        }) => sqlx::query_as::<_, EventSummaryRow>(
            "SELECT e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue \
             FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
             WHERE e.organization_id = $1 AND m.identity_issuer = $2 AND m.identity_subject = $3 \
               AND (e.starts_at > $4 OR (e.starts_at = $4 AND e.id > $5) OR e.starts_at IS NULL) \
             ORDER BY e.starts_at NULLS LAST, e.id LIMIT $6",
        )
        .bind(organization_id)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .bind(starts_at)
        .bind(id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await,
        Some(OrganizerEventCursor { starts_at: None, id }) => sqlx::query_as::<_, EventSummaryRow>(
            "SELECT e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue \
             FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
             WHERE e.organization_id = $1 AND m.identity_issuer = $2 AND m.identity_subject = $3 \
               AND e.starts_at IS NULL AND e.id > $4 \
             ORDER BY e.starts_at NULLS LAST, e.id LIMIT $5",
        )
        .bind(organization_id)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .bind(id)
        .bind(limit + 1)
        .fetch_all(pool)
        .await,
        None => sqlx::query_as::<_, EventSummaryRow>(
            "SELECT e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue \
             FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
             WHERE e.organization_id = $1 AND m.identity_issuer = $2 AND m.identity_subject = $3 \
             ORDER BY e.starts_at NULLS LAST, e.id LIMIT $4",
        )
        .bind(organization_id)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .bind(limit + 1)
        .fetch_all(pool)
        .await,
    }
    .map_err(|error| crate::db_error::record_as(error, "list_events", AuthoringError::Unavailable))?;

    let has_more = rows.len() as i64 > limit;
    let items = rows
        .into_iter()
        .take(limit as usize)
        .map(OrganizerEventSummary::from)
        .collect::<Vec<_>>();
    let next_cursor = if has_more {
        items
            .last()
            .map(|event| encode_event_cursor(event.starts_at, &event.id))
    } else {
        None
    };
    Ok(OrganizerEventPage { items, next_cursor })
}

pub(crate) async fn get_event(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
) -> Result<Option<OrganizerEvent>, AuthoringError> {
    let row = sqlx::query_as::<_, EventRow>(
        "SELECT e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue, e.accessibility_info, e.cancellation_policy, e.support_email, e.cover_image_key, e.cover_image_alt \
         FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 AND m.identity_issuer = $3 AND m.identity_subject = $4",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "get_event", AuthoringError::Unavailable))?;
    Ok(row.map(OrganizerEvent::from))
}

pub(crate) async fn authorize_cover_upload(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
) -> Result<(), AuthoringError> {
    let visibility = sqlx::query_scalar::<_, String>(
        "SELECT e.visibility FROM events e \
         JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager')",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(pool)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "authorize_cover_upload", AuthoringError::Unavailable)
    })?
    .ok_or(AuthoringError::NotFound)?;

    if visibility != "draft" {
        return Err(AuthoringError::Conflict);
    }
    Ok(())
}

pub(crate) async fn update_cover_image(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
    request: CoverImageRequest,
) -> Result<OrganizerEvent, AuthoringError> {
    let (Some(key), Some(alt)) = (request.key.as_deref(), request.alt.as_deref()) else {
        if request.key.is_some() || request.alt.is_some() {
            return Err(AuthoringError::InvalidInput);
        }
        return clear_cover_image(pool, identity, organization_id, event_id).await;
    };
    if !is_valid_cover_image(organization_id, event_id, key, alt) {
        return Err(AuthoringError::InvalidInput);
    }

    let row = sqlx::query_as::<_, EventRow>(
        "UPDATE events e SET cover_image_key = $3, cover_image_alt = $4 \
         WHERE e.organization_id = $1 AND e.id = $2 AND e.visibility = 'draft' \
           AND EXISTS (SELECT 1 FROM organization_memberships m WHERE m.organization_id = e.organization_id \
             AND m.identity_issuer = $5 AND m.identity_subject = $6 \
             AND m.role IN ('owner', 'admin', 'event_manager')) \
         RETURNING e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue, e.accessibility_info, e.cancellation_policy, e.support_email, e.cover_image_key, e.cover_image_alt",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(key)
    .bind(alt.trim())
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "update_cover_image", AuthoringError::Unavailable))?;
    if let Some(row) = row {
        return Ok(row.into());
    }
    let manageable = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 AND m.identity_issuer = $3 \
           AND m.identity_subject = $4 AND m.role IN ('owner', 'admin', 'event_manager'))",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "update_cover_image", AuthoringError::Unavailable))?;
    if manageable {
        Err(AuthoringError::Conflict)
    } else {
        Err(AuthoringError::NotFound)
    }
}

async fn clear_cover_image(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
) -> Result<OrganizerEvent, AuthoringError> {
    let row = sqlx::query_as::<_, EventRow>(
        "UPDATE events e SET cover_image_key = NULL, cover_image_alt = NULL \
         WHERE e.organization_id = $1 AND e.id = $2 AND e.visibility = 'draft' \
           AND EXISTS (SELECT 1 FROM organization_memberships m WHERE m.organization_id = e.organization_id \
             AND m.identity_issuer = $3 AND m.identity_subject = $4 \
             AND m.role IN ('owner', 'admin', 'event_manager')) \
         RETURNING e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue, e.accessibility_info, e.cancellation_policy, e.support_email, e.cover_image_key, e.cover_image_alt",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "clear_cover_image", AuthoringError::Unavailable))?;
    if let Some(row) = row {
        return Ok(row.into());
    }
    let manageable = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 AND m.identity_issuer = $3 \
           AND m.identity_subject = $4 AND m.role IN ('owner', 'admin', 'event_manager'))",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "clear_cover_image", AuthoringError::Unavailable))?;
    if manageable {
        Err(AuthoringError::Conflict)
    } else {
        Err(AuthoringError::NotFound)
    }
}

pub(crate) async fn create_event(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    request: CreateEventRequest,
    id: String,
) -> Result<(OrganizerEvent, bool), AuthoringError> {
    let timezone_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $1)",
    )
    .bind(&request.timezone)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "create_event", AuthoringError::Unavailable)
    })?;
    if !timezone_exists {
        return Err(AuthoringError::InvalidInput);
    }
    let title = request.title.trim();
    let accessibility_info = clean_optional_text(request.accessibility_info);
    let cancellation_policy = clean_optional_text(request.cancellation_policy);
    let support_email = clean_optional_text(request.support_email);
    let mut transaction = pool.begin().await.map_err(|error| {
        crate::db_error::record_as(error, "create_event", AuthoringError::Unavailable)
    })?;
    let created = sqlx::query_scalar::<_, String>(
        "INSERT INTO events (id, organization_id, slug, title, visibility, timezone, summary, starts_at, ends_at, venue, accessibility_info, cancellation_policy, support_email) \
         SELECT $1, o.id, $3, $4, 'draft', $5, $6, $7, $8, $9, $10, $11, $12 \
         FROM organizations o JOIN organization_memberships m ON m.organization_id = o.id \
         WHERE o.id = $2 AND m.identity_issuer = $13 AND m.identity_subject = $14 \
           AND m.role IN ('owner', 'admin', 'event_manager') \
         ON CONFLICT (organization_id, slug) WHERE slug IS NOT NULL DO NOTHING RETURNING id",
    )
    .bind(&id)
    .bind(organization_id)
    .bind(&request.slug)
    .bind(title)
    .bind(&request.timezone)
    .bind(&request.summary)
    .bind(request.starts_at)
    .bind(request.ends_at)
    .bind(request.venue.clone().map(Json))
    .bind(&accessibility_info)
    .bind(&cancellation_policy)
    .bind(&support_email)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "create_event", AuthoringError::Unavailable))?;

    let was_created = created.is_some();
    let row = sqlx::query_as::<_, EventRow>(
        "SELECT e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue, e.accessibility_info, e.cancellation_policy, e.support_email, e.cover_image_key, e.cover_image_alt \
         FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.slug = $2 AND e.title = $3 AND e.timezone = $4 \
           AND e.summary IS NOT DISTINCT FROM $5 AND e.starts_at IS NOT DISTINCT FROM $6 \
           AND e.ends_at IS NOT DISTINCT FROM $7 AND e.venue IS NOT DISTINCT FROM $8 \
           AND e.accessibility_info IS NOT DISTINCT FROM $9 \
           AND e.cancellation_policy IS NOT DISTINCT FROM $10 \
           AND e.support_email IS NOT DISTINCT FROM $11 \
           AND m.identity_issuer = $12 AND m.identity_subject = $13 \
           AND m.role IN ('owner', 'admin', 'event_manager')",
    )
    .bind(organization_id)
    .bind(&request.slug)
    .bind(title)
    .bind(&request.timezone)
    .bind(&request.summary)
    .bind(request.starts_at)
    .bind(request.ends_at)
    .bind(request.venue.map(Json))
    .bind(&accessibility_info)
    .bind(&cancellation_policy)
    .bind(&support_email)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "create_event", AuthoringError::Unavailable))?;
    let Some(row) = row else {
        let can_create = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM organization_memberships m WHERE m.organization_id = $1 \
             AND m.identity_issuer = $2 AND m.identity_subject = $3 \
             AND m.role IN ('owner', 'admin', 'event_manager'))",
        )
        .bind(organization_id)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "create_event", AuthoringError::Unavailable)
        })?;
        transaction.commit().await.map_err(|error| {
            crate::db_error::record_as(error, "create_event", AuthoringError::Unavailable)
        })?;
        return if can_create {
            Err(AuthoringError::Conflict)
        } else {
            Err(AuthoringError::NotFound)
        };
    };
    transaction.commit().await.map_err(|error| {
        crate::db_error::record_as(error, "create_event", AuthoringError::Unavailable)
    })?;
    Ok((row.into(), was_created))
}

pub(crate) async fn update_event(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
    request: UpdateEventRequest,
) -> Result<OrganizerEvent, AuthoringError> {
    let timezone_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $1)",
    )
    .bind(&request.timezone)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "update_event", AuthoringError::Unavailable)
    })?;
    if !timezone_exists {
        return Err(AuthoringError::InvalidInput);
    }
    let accessibility_info = clean_optional_text(request.accessibility_info);
    let cancellation_policy = clean_optional_text(request.cancellation_policy);
    let support_email = clean_optional_text(request.support_email);
    let row = sqlx::query_as::<_, EventRow>(
        "UPDATE events e SET title = $3, timezone = $4, summary = $5, starts_at = $6, ends_at = $7, venue = $8, accessibility_info = $9, cancellation_policy = $10, support_email = $11 \
         WHERE e.organization_id = $1 AND e.id = $2 AND e.visibility = 'draft' \
           AND EXISTS (SELECT 1 FROM organization_memberships m WHERE m.organization_id = e.organization_id \
             AND m.identity_issuer = $12 AND m.identity_subject = $13 \
             AND m.role IN ('owner', 'admin', 'event_manager')) \
         RETURNING e.id, COALESCE(e.slug, e.id) AS slug, e.title, e.summary, e.starts_at, e.ends_at, e.timezone, e.visibility, e.venue, e.accessibility_info, e.cancellation_policy, e.support_email, e.cover_image_key, e.cover_image_alt",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(request.title.trim())
    .bind(&request.timezone)
    .bind(&request.summary)
    .bind(request.starts_at)
    .bind(request.ends_at)
    .bind(request.venue.map(Json))
    .bind(&accessibility_info)
    .bind(&cancellation_policy)
    .bind(&support_email)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "update_event", AuthoringError::Unavailable))?;
    if let Some(row) = row {
        return Ok(row.into());
    }
    let manageable = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 AND m.identity_issuer = $3 \
           AND m.identity_subject = $4 AND m.role IN ('owner', 'admin', 'event_manager'))",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "update_event", AuthoringError::Unavailable))?;
    if manageable {
        Err(AuthoringError::Conflict)
    } else {
        Err(AuthoringError::NotFound)
    }
}

pub(crate) async fn create_ticket(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
    idempotency_key: &str,
    request: CreateTicketRequest,
    id: String,
) -> Result<(OrganizerTicket, bool), AuthoringError> {
    let amount =
        Decimal::from_str_exact(&request.amount).map_err(|_| AuthoringError::InvalidInput)?;
    let mut transaction = pool.begin().await.map_err(|error| {
        crate::db_error::record_as(error, "create_ticket", AuthoringError::Unavailable)
    })?;
    let visibility = sqlx::query_scalar::<_, String>(
        "SELECT e.visibility FROM events e \
         JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager') \
         FOR UPDATE OF e, m",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "create_ticket", AuthoringError::Unavailable)
    })?
    .ok_or(AuthoringError::NotFound)?;
    let created = if visibility == "draft" {
        sqlx::query_scalar::<_, String>(
            "INSERT INTO ticket_types (id, organization_id, event_id, name, description, unit_amount, currency, capacity, sale_starts_at, sale_ends_at, idempotency_key) \
             SELECT $1, e.organization_id, e.id, $5, $6, $7, $8, $9, $10, $11, $4 \
             FROM events e \
             WHERE e.organization_id = $2 AND e.id = $3 AND e.visibility = 'draft' \
             ON CONFLICT (event_id, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING RETURNING id",
        )
        .bind(&id)
        .bind(organization_id)
        .bind(event_id)
        .bind(idempotency_key)
        .bind(request.name.trim())
        .bind(&request.description)
        .bind(amount)
        .bind(&request.currency)
        .bind(request.capacity)
        .bind(request.sale_starts_at)
        .bind(request.sale_ends_at)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| crate::db_error::record_as(error, "create_ticket", AuthoringError::Unavailable))?
    } else {
        None
    };

    let was_created = created.is_some();
    let row = sqlx::query_as::<_, TicketRow>(
        "SELECT t.id, t.name, t.description, t.unit_amount, t.currency, t.capacity, t.sale_starts_at, t.sale_ends_at \
         FROM ticket_types t JOIN events e ON e.id = t.event_id AND e.organization_id = t.organization_id \
         JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 AND t.idempotency_key = $3 \
           AND t.name = $4 AND t.description IS NOT DISTINCT FROM $5 AND t.unit_amount = $6 \
           AND t.currency = $7 AND t.capacity = $8 AND t.sale_starts_at = $9 AND t.sale_ends_at = $10 \
           AND m.identity_issuer = $11 AND m.identity_subject = $12 \
           AND m.role IN ('owner', 'admin', 'event_manager')",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(idempotency_key)
    .bind(request.name.trim())
    .bind(&request.description)
    .bind(amount)
    .bind(&request.currency)
    .bind(request.capacity)
    .bind(request.sale_starts_at)
    .bind(request.sale_ends_at)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "create_ticket", AuthoringError::Unavailable))?;
    let Some(row) = row else {
        let manageable = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM events e JOIN organization_memberships m ON m.organization_id = e.organization_id \
             WHERE e.organization_id = $1 AND e.id = $2 AND m.identity_issuer = $3 \
               AND m.identity_subject = $4 AND m.role IN ('owner', 'admin', 'event_manager'))",
        )
        .bind(organization_id)
        .bind(event_id)
        .bind(&identity.issuer)
        .bind(&identity.subject)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| crate::db_error::record_as(error, "create_ticket", AuthoringError::Unavailable))?;
        transaction.commit().await.map_err(|error| {
            crate::db_error::record_as(error, "create_ticket", AuthoringError::Unavailable)
        })?;
        return if manageable {
            Err(AuthoringError::Conflict)
        } else {
            Err(AuthoringError::NotFound)
        };
    };
    transaction.commit().await.map_err(|error| {
        crate::db_error::record_as(error, "create_ticket", AuthoringError::Unavailable)
    })?;
    Ok((row.into(), was_created))
}

pub(crate) async fn list_tickets(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
) -> Result<Vec<OrganizerTicket>, AuthoringError> {
    let can_view = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events e JOIN organization_memberships m \
         ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager', 'analyst'))",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(pool)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "list_tickets", AuthoringError::Unavailable)
    })?;
    if !can_view {
        return Err(AuthoringError::NotFound);
    }
    let rows = sqlx::query_as::<_, TicketRow>(
        "SELECT t.id, t.name, t.description, t.unit_amount, t.currency, t.capacity, t.sale_starts_at, t.sale_ends_at \
         FROM ticket_types t WHERE t.organization_id = $1 AND t.event_id = $2 \
         ORDER BY t.display_order, t.id LIMIT 100",
    )
    .bind(organization_id)
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|error| crate::db_error::record_as(error, "list_tickets", AuthoringError::Unavailable))?;
    Ok(rows.into_iter().map(OrganizerTicket::from).collect())
}

pub(crate) async fn update_ticket(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
    ticket_type_id: &str,
    request: CreateTicketRequest,
) -> Result<OrganizerTicket, AuthoringError> {
    let amount =
        Decimal::from_str_exact(&request.amount).map_err(|_| AuthoringError::InvalidInput)?;
    let mut transaction = pool.begin().await.map_err(|error| {
        crate::db_error::record_as(error, "update_ticket", AuthoringError::Unavailable)
    })?;
    let visibility = sqlx::query_scalar::<_, String>(
        "SELECT e.visibility FROM events e \
         JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager') \
         FOR UPDATE OF e, m",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "update_ticket", AuthoringError::Unavailable)
    })?
    .ok_or(AuthoringError::NotFound)?;
    if visibility != "draft" {
        transaction.commit().await.map_err(|error| {
            crate::db_error::record_as(error, "update_ticket", AuthoringError::Unavailable)
        })?;
        return Err(AuthoringError::Conflict);
    }
    let row = sqlx::query_as::<_, TicketRow>(
        "UPDATE ticket_types t SET name = $4, description = $5, unit_amount = $6, currency = $7, \
             capacity = $8, sale_starts_at = $9, sale_ends_at = $10 \
         WHERE t.organization_id = $1 AND t.event_id = $2 AND t.id = $3 \
           AND $8 >= t.sold_count + t.held_count \
         RETURNING t.id, t.name, t.description, t.unit_amount, t.currency, t.capacity, t.sale_starts_at, t.sale_ends_at",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(ticket_type_id)
    .bind(request.name.trim())
    .bind(&request.description)
    .bind(amount)
    .bind(&request.currency)
    .bind(request.capacity)
    .bind(request.sale_starts_at)
    .bind(request.sale_ends_at)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| crate::db_error::record_as(error, "update_ticket", AuthoringError::Unavailable))?;
    let Some(row) = row else {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM ticket_types WHERE organization_id = $1 \
             AND event_id = $2 AND id = $3)",
        )
        .bind(organization_id)
        .bind(event_id)
        .bind(ticket_type_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| {
            crate::db_error::record_as(error, "update_ticket", AuthoringError::Unavailable)
        })?;
        transaction.commit().await.map_err(|error| {
            crate::db_error::record_as(error, "update_ticket", AuthoringError::Unavailable)
        })?;
        return if exists {
            Err(AuthoringError::Conflict)
        } else {
            Err(AuthoringError::NotFound)
        };
    };
    transaction.commit().await.map_err(|error| {
        crate::db_error::record_as(error, "update_ticket", AuthoringError::Unavailable)
    })?;
    Ok(row.into())
}

pub(crate) async fn publish_event(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    organization_id: &str,
    event_id: &str,
) -> Result<(), AuthoringError> {
    let mut transaction = pool.begin().await.map_err(|error| {
        crate::db_error::record_as(error, "publish_event", AuthoringError::Unavailable)
    })?;
    let visibility = sqlx::query_scalar::<_, String>(
        "SELECT e.visibility FROM events e \
         JOIN organization_memberships m ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager') \
         FOR UPDATE OF e, m",
    )
    .bind(organization_id)
    .bind(event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "publish_event", AuthoringError::Unavailable)
    })?
    .ok_or(AuthoringError::NotFound)?;
    if visibility == "public" {
        transaction.commit().await.map_err(|error| {
            crate::db_error::record_as(error, "publish_event", AuthoringError::Unavailable)
        })?;
        return Ok(());
    }

    let ready = sqlx::query_scalar::<_, bool>(
        "SELECT e.starts_at IS NOT NULL AND e.starts_at > clock_timestamp() \
           AND (e.ends_at IS NULL OR e.ends_at > e.starts_at) \
           AND e.venue IS NOT NULL \
           AND EXISTS (SELECT 1 FROM pg_timezone_names z WHERE z.name = e.timezone) \
           AND EXISTS (SELECT 1 FROM ticket_types t WHERE t.organization_id = e.organization_id \
             AND t.event_id = e.id AND t.sale_ends_at > clock_timestamp() \
             AND t.capacity > t.sold_count + t.held_count) \
         FROM events e WHERE e.organization_id = $1 AND e.id = $2 AND e.visibility = 'draft'",
    )
    .bind(organization_id)
    .bind(event_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "publish_event", AuthoringError::Unavailable)
    })?
    .unwrap_or(false);
    if !ready {
        transaction.commit().await.map_err(|error| {
            crate::db_error::record_as(error, "publish_event", AuthoringError::Unavailable)
        })?;
        return Err(AuthoringError::NotReady);
    }
    sqlx::query(
        "UPDATE events SET visibility = 'public' \
         WHERE organization_id = $1 AND id = $2 AND visibility = 'draft'",
    )
    .bind(organization_id)
    .bind(event_id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| {
        crate::db_error::record_as(error, "publish_event", AuthoringError::Unavailable)
    })?;
    transaction.commit().await.map_err(|error| {
        crate::db_error::record_as(error, "publish_event", AuthoringError::Unavailable)
    })
}

impl From<EventRow> for OrganizerEvent {
    fn from(row: EventRow) -> Self {
        Self {
            id: row.id,
            slug: row.slug,
            title: row.title,
            summary: row.summary,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            timezone: row.timezone,
            visibility: row.visibility,
            venue: row.venue.map(|venue| venue.0),
            accessibility_info: row.accessibility_info,
            cancellation_policy: row.cancellation_policy,
            support_email: row.support_email,
            cover_image_key: row.cover_image_key,
            cover_image_alt: row.cover_image_alt,
        }
    }
}

impl From<EventSummaryRow> for OrganizerEventSummary {
    fn from(row: EventSummaryRow) -> Self {
        Self {
            id: row.id,
            slug: row.slug,
            title: row.title,
            summary: row.summary,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            timezone: row.timezone,
            visibility: row.visibility,
            venue: row.venue.map(|venue| venue.0),
        }
    }
}

impl From<TicketRow> for OrganizerTicket {
    fn from(row: TicketRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            amount: row.unit_amount.normalize().to_string(),
            currency: row.currency,
            capacity: row.capacity,
            sale_starts_at: row.sale_starts_at,
            sale_ends_at: row.sale_ends_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CreateEventRequest, CreateTicketRequest, OrganizerEvent, OrganizerEventCursor,
        cover_image_extension, encode_event_cursor, is_valid_amount, is_valid_cover_image,
        is_valid_event, is_valid_ticket,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use chrono::{TimeZone, Timelike, Utc};

    #[test]
    fn organizer_event_cursor_roundtrips_timestamps_and_null_sort_values() {
        let event = OrganizerEvent {
            id: "zeno_01J9QY7B5C6D7E8F9G0H1J2K3M".to_owned(),
            slug: "cursor-test".to_owned(),
            title: "Cursor test".to_owned(),
            summary: None,
            starts_at: Some(
                Utc.with_ymd_and_hms(2027, 4, 10, 4, 0, 0)
                    .unwrap()
                    .with_nanosecond(123_000_000)
                    .unwrap(),
            ),
            ends_at: None,
            timezone: "Asia/Colombo".to_owned(),
            visibility: "draft".to_owned(),
            venue: Some(crate::catalog::Venue::Online),
            accessibility_info: None,
            cancellation_policy: None,
            support_email: None,
            cover_image_key: None,
            cover_image_alt: None,
        };
        let cursor = OrganizerEventCursor::decode(&encode_event_cursor(event.starts_at, &event.id))
            .expect("decode emitted timestamp cursor");
        assert_eq!(cursor.id, event.id);
        assert_eq!(cursor.starts_at, event.starts_at);

        let null_cursor = URL_SAFE_NO_PAD.encode(format!("v1\n~\n{}", event.id));
        let decoded = OrganizerEventCursor::decode(&null_cursor).expect("decode null cursor");
        assert_eq!(decoded.id, event.id);
        assert_eq!(decoded.starts_at, None);
    }

    #[test]
    fn organizer_event_cursor_rejects_malformed_values() {
        assert!(OrganizerEventCursor::decode("not-a-cursor").is_none());
        assert!(OrganizerEventCursor::decode(&"a".repeat(513)).is_none());
        assert!(
            OrganizerEventCursor::decode(
                &URL_SAFE_NO_PAD.encode("v2\n~\nzeno_01J9QY7B5C6D7E8F9G0H1J2K3M")
            )
            .is_none()
        );
        assert!(
            OrganizerEventCursor::decode(&URL_SAFE_NO_PAD.encode("v1\n~\nnot-an-id")).is_none()
        );
    }

    #[test]
    fn event_input_checks_slug_lengths_and_time_order() {
        let event = CreateEventRequest {
            slug: "campus-summit-2027".to_owned(),
            title: "Campus Summit".to_owned(),
            timezone: "Asia/Colombo".to_owned(),
            summary: None,
            starts_at: Some(Utc.with_ymd_and_hms(2027, 1, 1, 8, 0, 0).unwrap()),
            ends_at: Some(Utc.with_ymd_and_hms(2027, 1, 1, 9, 0, 0).unwrap()),
            venue: Some(crate::catalog::Venue::Online),
            accessibility_info: Some("Step-free access".to_owned()),
            cancellation_policy: Some("Contact us 48 hours before the event.".to_owned()),
            support_email: Some("help@example.com".to_owned()),
        };
        assert!(is_valid_event(&event));
        let mut invalid = event.clone();
        invalid.slug = "-bad".to_owned();
        assert!(!is_valid_event(&invalid));
        invalid = event;
        invalid.ends_at = invalid.starts_at;
        assert!(!is_valid_event(&invalid));
    }

    #[test]
    fn attendee_disclosures_are_optional_bounded_and_email_is_validated() {
        let mut event = CreateEventRequest {
            slug: "disclosure-check".to_owned(),
            title: "Disclosure check".to_owned(),
            timezone: "Asia/Colombo".to_owned(),
            summary: None,
            starts_at: None,
            ends_at: None,
            venue: Some(crate::catalog::Venue::Online),
            accessibility_info: None,
            cancellation_policy: None,
            support_email: None,
        };
        assert!(is_valid_event(&event));
        event.accessibility_info = Some("a".repeat(2001));
        assert!(!is_valid_event(&event));
        event.accessibility_info = None;
        event.cancellation_policy = Some("c".repeat(2001));
        assert!(!is_valid_event(&event));
        event.cancellation_policy = None;
        event.support_email = Some("not-an-email".to_owned());
        assert!(!is_valid_event(&event));
        event.support_email = Some("guest@example.com?subject=spam".to_owned());
        assert!(!is_valid_event(&event));
        event.support_email = Some("help@example.com".to_owned());
        assert!(is_valid_event(&event));
    }

    #[test]
    fn cover_image_upload_content_types_are_raster_only() {
        assert_eq!(cover_image_extension("image/jpeg"), Some("jpg"));
        assert_eq!(cover_image_extension("image/png"), Some("png"));
        assert_eq!(cover_image_extension("image/webp"), Some("webp"));
        assert_eq!(cover_image_extension("image/svg+xml"), None);
        assert_eq!(cover_image_extension("application/octet-stream"), None);
    }

    #[test]
    fn cover_image_key_and_alt_text_are_event_scoped_and_bounded() {
        let organization_id = "zeno_01J9QY7B5C6D7E8F9G0H1J2K3M";
        let event_id = "zeno_01J9QY7B5C6D7E8F9G0H1J2K3N";
        let key = format!("events/{organization_id}/{event_id}/01J9QY7B5C6D7E8F9G0H1J2K3M.jpg");
        assert!(is_valid_cover_image(
            organization_id,
            event_id,
            &key,
            "Event poster"
        ));
        assert!(!is_valid_cover_image(
            organization_id,
            "zeno_01J9QY7B5C6D7E8F9G0H1J2K3P",
            &key,
            "Event poster"
        ));
        assert!(!is_valid_cover_image(organization_id, event_id, &key, " "));
    }

    #[test]
    fn ticket_input_preserves_decimal_precision_and_bounds_inventory() {
        let ticket = CreateTicketRequest {
            name: "Student".to_owned(),
            description: None,
            amount: "1250.50".to_owned(),
            currency: "LKR".to_owned(),
            capacity: 100,
            sale_starts_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            sale_ends_at: Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap(),
        };
        assert!(is_valid_ticket(&ticket));
        let mut invalid = ticket.clone();
        invalid.amount = "1.12345".to_owned();
        assert!(!is_valid_ticket(&invalid));
        invalid = ticket;
        invalid.capacity = 0;
        assert!(!is_valid_ticket(&invalid));
        assert!(is_valid_amount("99999999999999.9999"));
        assert!(!is_valid_amount("100000000000000.0000"));
        assert!(!is_valid_amount("01.00"));
        assert!(!is_valid_amount("1e3"));
    }
}
