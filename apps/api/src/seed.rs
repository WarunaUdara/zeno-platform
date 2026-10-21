use rust_decimal::Decimal;
use sqlx::PgPool;

pub const DEMO_EVENT_ID: &str = "zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K";
pub const DEMO_ORGANIZATION_ID: &str = "zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N";
pub const GENERAL_TICKET_ID: &str = "zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7L";
pub const STUDIO_TICKET_ID: &str = "zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7M";

/// Insert synthetic data for the local event-page demo without changing existing sales.
pub async fn seed_demo(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO events \
        (id, organization_id, title, visibility, timezone, summary, starts_at, ends_at, venue) \
         VALUES ($1, $2, 'Designing with AI Summit', 'public', 'Asia/Colombo', \
                 'A gathering for curious minds shaping how we create, work, and live alongside artificial intelligence.', \
                 '2026-10-18 09:00:00+05:30', '2026-10-18 18:00:00+05:30', \
                 '{\"kind\":\"physical\",\"name\":\"Colombo Design District\",\"city\":\"Colombo 07\",\"country_code\":\"LK\"}'::jsonb) \
         ON CONFLICT (id) DO UPDATE SET \
             summary = COALESCE(events.summary, EXCLUDED.summary), \
             starts_at = COALESCE(events.starts_at, EXCLUDED.starts_at), \
             ends_at = COALESCE(events.ends_at, EXCLUDED.ends_at), \
             venue = COALESCE(events.venue, EXCLUDED.venue)",
    )
    .bind(DEMO_EVENT_ID)
    .bind(DEMO_ORGANIZATION_ID)
    .execute(&mut *transaction)
    .await?;

    for (id, name, description, amount, capacity, display_order) in [
        (
            GENERAL_TICKET_ID,
            "General admission",
            "A full day of talks, workshops and good coffee.",
            Decimal::from(2500),
            600_i64,
            0_i16,
        ),
        (
            STUDIO_TICKET_ID,
            "Studio pass",
            "Front-row access, a workshop seat and speaker lounge.",
            Decimal::from(7500),
            30_i64,
            1_i16,
        ),
    ] {
        sqlx::query(
            "INSERT INTO ticket_types \
             (id, organization_id, event_id, name, description, unit_amount, currency, capacity, display_order, sale_starts_at, sale_ends_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'LKR', $7, $8, clock_timestamp() - interval '30 days', \
                     clock_timestamp() + interval '14 days') \
             ON CONFLICT (id) DO UPDATE SET \
                 description = COALESCE(ticket_types.description, EXCLUDED.description)",
        )
        .bind(id)
        .bind(DEMO_ORGANIZATION_ID)
        .bind(DEMO_EVENT_ID)
        .bind(name)
        .bind(description)
        .bind(amount)
        .bind(capacity)
        .bind(display_order)
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await
}
