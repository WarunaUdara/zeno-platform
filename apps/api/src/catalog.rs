use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, QueryBuilder, types::Json};

use crate::model::Money;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Venue {
    Physical {
        name: String,
        city: String,
        country_code: String,
    },
    Online,
}

#[derive(Debug, Serialize)]
pub(crate) struct PublishedEvent {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub visibility: &'static str,
    pub venue: Venue,
    pub accessibility_info: Option<String>,
    pub cancellation_policy: Option<String>,
    pub support_email: Option<String>,
    pub cover_image_key: Option<String>,
    pub cover_image_alt: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PublishedEventSummary {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub visibility: &'static str,
    pub venue: Venue,
}

#[derive(Debug, Serialize)]
pub(crate) struct PublishedEventPage {
    pub items: Vec<PublishedEventSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct CatalogCursor {
    starts_at: DateTime<Utc>,
    id: String,
}

pub(crate) fn decode_cursor(cursor: &str) -> Option<CatalogCursor> {
    if cursor.len() > 512 {
        return None;
    }
    let payload = URL_SAFE_NO_PAD.decode(cursor).ok()?;
    let payload = std::str::from_utf8(&payload).ok()?;
    let mut parts = payload.splitn(3, '\n');
    if parts.next()? != "v1" {
        return None;
    }
    let starts_at = parts.next()?;
    let id = parts.next()?;
    if !crate::model::is_public_id(id) {
        return None;
    }
    let starts_at = DateTime::parse_from_rfc3339(starts_at)
        .ok()?
        .with_timezone(&Utc);
    Some(CatalogCursor {
        starts_at,
        id: id.to_owned(),
    })
}

fn encode_cursor(starts_at: DateTime<Utc>, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("v1\n{}\n{id}", starts_at.to_rfc3339()))
}

#[cfg(test)]
mod tests {
    use super::{PublishedEventSummary, URL_SAFE_NO_PAD, Venue, decode_cursor, encode_cursor};
    use base64::Engine as _;
    use chrono::{DateTime, Utc};

    #[test]
    fn catalog_cursor_roundtrips_the_ordering_key() {
        let event = PublishedEventSummary {
            id: "zeno_01J9QY7B5C6D7E8F9G0H1J2K3M".to_owned(),
            title: "Cursor test".to_owned(),
            summary: None,
            starts_at: DateTime::parse_from_rfc3339("2099-01-01T04:30:00.123456Z")
                .expect("valid test timestamp")
                .with_timezone(&Utc),
            ends_at: None,
            timezone: "Asia/Colombo".to_owned(),
            visibility: "public",
            venue: Venue::Online,
        };

        let decoded = decode_cursor(&encode_cursor(event.starts_at, &event.id))
            .expect("decode emitted cursor");
        assert_eq!(decoded.id, event.id);
        assert_eq!(decoded.starts_at, event.starts_at);
    }

    #[test]
    fn catalog_cursor_rejects_malformed_or_unbounded_values() {
        assert!(decode_cursor("not-a-cursor").is_none());
        assert!(decode_cursor(&"a".repeat(513)).is_none());
        assert!(
            decode_cursor(
                &URL_SAFE_NO_PAD
                    .encode("v2\n2099-01-01T00:00:00Z\nzeno_01J9QY7B5C6D7E8F9G0H1J2K3M")
            )
            .is_none()
        );
        let invalid_id = URL_SAFE_NO_PAD.encode("v1\n2099-01-01T00:00:00Z\nnot-a-public-id");
        assert!(decode_cursor(&invalid_id).is_none());
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct PublishedTicketType {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub price: Money,
    pub availability: Availability,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Availability {
    Available,
    SoldOut,
    NotOnSale,
}

#[derive(FromRow)]
struct PublishedEventRow {
    id: String,
    title: String,
    summary: Option<String>,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
    timezone: String,
    venue: Option<Json<Venue>>,
    accessibility_info: Option<String>,
    cancellation_policy: Option<String>,
    support_email: Option<String>,
    cover_image_key: Option<String>,
    cover_image_alt: Option<String>,
}

#[derive(FromRow)]
struct PublishedEventSummaryRow {
    id: String,
    title: String,
    summary: Option<String>,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
    timezone: String,
    venue: Option<Json<Venue>>,
}

pub(crate) async fn list_published_events(
    pool: &PgPool,
    cursor: Option<CatalogCursor>,
    limit: i64,
) -> Result<PublishedEventPage, sqlx::Error> {
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT id, title, summary, starts_at, ends_at, timezone, venue \
         FROM events \
         WHERE visibility = 'public' AND starts_at >= clock_timestamp() AND venue IS NOT NULL",
    );
    if let Some(cursor) = cursor {
        query
            .push(" AND (starts_at, id) > (")
            .push_bind(cursor.starts_at)
            .push(", ")
            .push_bind(cursor.id)
            .push(")");
    }
    let rows = query
        .push(" ORDER BY starts_at, id LIMIT ")
        .push_bind(limit + 1)
        .build_query_as::<PublishedEventSummaryRow>()
        .fetch_all(pool)
        .await?;

    let has_more = rows.len() as i64 > limit;
    let items = rows
        .into_iter()
        .take(limit as usize)
        .map(published_event_summary_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if has_more {
        items
            .last()
            .map(|event| encode_cursor(event.starts_at, &event.id))
    } else {
        None
    };
    Ok(PublishedEventPage { items, next_cursor })
}

fn published_event_summary_from_row(
    event: PublishedEventSummaryRow,
) -> Result<PublishedEventSummary, sqlx::Error> {
    let starts_at = event
        .starts_at
        .ok_or_else(|| sqlx::Error::Protocol("published event has no start time".to_owned()))?;
    let venue = event
        .venue
        .ok_or_else(|| sqlx::Error::Protocol("published event has no venue".to_owned()))?
        .0;
    Ok(PublishedEventSummary {
        id: event.id,
        title: event.title,
        summary: event.summary,
        starts_at,
        ends_at: event.ends_at,
        timezone: event.timezone,
        visibility: "public",
        venue,
    })
}

fn published_event_from_row(event: PublishedEventRow) -> Result<PublishedEvent, sqlx::Error> {
    let starts_at = event
        .starts_at
        .ok_or_else(|| sqlx::Error::Protocol("published event has no start time".to_owned()))?;
    let venue = event
        .venue
        .ok_or_else(|| sqlx::Error::Protocol("published event has no venue".to_owned()))?
        .0;
    Ok(PublishedEvent {
        id: event.id,
        title: event.title,
        summary: event.summary,
        starts_at,
        ends_at: event.ends_at,
        timezone: event.timezone,
        visibility: "public",
        venue,
        accessibility_info: event.accessibility_info,
        cancellation_policy: event.cancellation_policy,
        support_email: event.support_email,
        cover_image_key: event.cover_image_key,
        cover_image_alt: event.cover_image_alt,
    })
}

#[derive(FromRow)]
struct PublishedTicketTypeRow {
    id: String,
    name: String,
    description: Option<String>,
    unit_amount: Decimal,
    currency: String,
    availability: String,
}

pub(crate) async fn get_published_event(
    pool: &PgPool,
    event_id: &str,
) -> Result<Option<PublishedEvent>, sqlx::Error> {
    let row = sqlx::query_as::<_, PublishedEventRow>(
        "SELECT id, title, summary, starts_at, ends_at, timezone, venue, accessibility_info, cancellation_policy, support_email, cover_image_key, cover_image_alt \
         FROM events \
         WHERE id = $1 AND visibility = 'public' \
           AND starts_at IS NOT NULL AND venue IS NOT NULL",
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await?;

    let Some(event) = row else {
        return Ok(None);
    };
    Ok(Some(published_event_from_row(event)?))
}

pub(crate) async fn list_published_ticket_types(
    pool: &PgPool,
    event_id: &str,
) -> Result<Option<Vec<PublishedTicketType>>, sqlx::Error> {
    // held_count includes expired rows until the worker releases them. Add back
    // only that due expiry backlog so this read does not sum every active hold.
    let rows = sqlx::query_as::<_, PublishedTicketTypeRow>(
        "SELECT t.id, t.name, t.description, t.unit_amount, t.currency, \
         CASE \
           WHEN clock_timestamp() < t.sale_starts_at OR clock_timestamp() >= t.sale_ends_at \
             THEN 'not_on_sale' \
           WHEN t.capacity - t.sold_count - t.held_count + COALESCE(( \
             SELECT sum(r.quantity)::bigint FROM reservations AS r \
             WHERE r.ticket_type_id = t.id AND r.status = 'held' \
               AND r.expires_at <= clock_timestamp() \
           ), 0) <= 0 THEN 'sold_out' \
           ELSE 'available' \
         END AS availability \
         FROM ticket_types AS t \
         JOIN events AS e ON e.id = t.event_id \
         WHERE e.id = $1 AND e.visibility = 'public' \
           AND e.starts_at IS NOT NULL AND e.venue IS NOT NULL \
         ORDER BY t.display_order, t.id",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        let published = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM events WHERE id = $1 AND visibility = 'public' \
             AND starts_at IS NOT NULL AND venue IS NOT NULL)",
        )
        .bind(event_id)
        .fetch_one(pool)
        .await?;
        return Ok(published.then(Vec::new));
    }

    rows.into_iter()
        .map(|ticket| {
            let availability = match ticket.availability.as_str() {
                "available" => Availability::Available,
                "sold_out" => Availability::SoldOut,
                "not_on_sale" => Availability::NotOnSale,
                _ => {
                    return Err(sqlx::Error::Protocol(
                        "unknown ticket availability".to_owned(),
                    ));
                }
            };
            Ok(PublishedTicketType {
                id: ticket.id,
                name: ticket.name,
                description: ticket.description,
                price: Money {
                    amount: ticket.unit_amount.normalize().to_string(),
                    currency: ticket.currency,
                },
                availability,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}
