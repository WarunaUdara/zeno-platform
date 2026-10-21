use std::{net::IpAddr, str::FromStr};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sqlx::{
    PgPool, Postgres, pool::PoolConnection, postgres::PgConnectOptions, postgres::PgPoolOptions,
};
use tower::ServiceExt;
use ulid::Ulid;
use zeno_api::{app, seed, worker};

static PUBLIC_CATALOG_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn database() -> PgPool {
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must point to local test Postgres");
    let host = PgConnectOptions::from_str(&url)
        .expect("DATABASE_URL must be a valid PostgreSQL connection URL")
        .get_host()
        .to_owned();
    assert!(
        is_loopback_host(&host),
        "refusing to run destructive integration tests against a non-local database"
    );
    let pool = PgPoolOptions::new()
        .max_connections(40)
        .connect(&url)
        .await
        .expect("connect to local test Postgres");
    sqlx::migrate!().run(&pool).await.expect("run migrations");
    pool
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[test]
fn integration_tests_reject_remote_database_hosts() {
    assert!(is_loopback_host("localhost"));
    assert!(is_loopback_host("127.0.0.1"));
    assert!(is_loopback_host("::1"));
    assert!(!is_loopback_host("example.neon.tech"));
}

async fn lock_worker_queue_for_test(pool: &PgPool) -> PoolConnection<Postgres> {
    let mut connection = pool
        .acquire()
        .await
        .expect("acquire worker test lock connection");
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(0x5A454E4F_i64)
        .fetch_one(&mut *connection)
        .await
        .expect("serialize tests that mutate the shared worker queue");
    connection
}

async fn fixture(pool: &PgPool, capacity: i64) -> (String, String) {
    let event_id = format!("zeno_{}", Ulid::new());
    let ticket_type_id = format!("zeno_{}", Ulid::new());
    sqlx::query(
        "INSERT INTO events (id, organization_id, title, visibility, timezone) \
         VALUES ($1, $2, 'Test event', 'public', 'Asia/Colombo')",
    )
    .bind(&event_id)
    .bind(seed::DEMO_ORGANIZATION_ID)
    .execute(pool)
    .await
    .expect("insert event fixture");
    sqlx::query(
        "INSERT INTO ticket_types (id, organization_id, event_id, name, unit_amount, currency, capacity, sale_starts_at, sale_ends_at) \
         VALUES ($1, (SELECT organization_id FROM events WHERE id = $2), $2, 'General', 1250.00, 'LKR', $3, now() - interval '1 hour', now() + interval '1 hour')",
    )
    .bind(&ticket_type_id)
    .bind(&event_id)
    .bind(capacity)
    .execute(pool)
    .await
    .expect("insert ticket type fixture");
    (event_id, ticket_type_id)
}

fn idempotency_key() -> String {
    URL_SAFE_NO_PAD.encode(Ulid::new().to_bytes())
}

#[tokio::test]
async fn worker_purges_expired_idempotency_keys_in_bounded_batches() {
    let pool = database().await;
    let prefix = format!("zeno_purge_{}", Ulid::new());
    sqlx::query(
        "INSERT INTO idempotency_keys (idempotency_key, fingerprint, replay_until) \
         SELECT $1 || '_' || value::text, decode(repeat('00', 32), 'hex'), \
           clock_timestamp() - interval '1 second' \
         FROM generate_series(1, 1001) AS generated(value)",
    )
    .bind(&prefix)
    .execute(&pool)
    .await
    .expect("insert expired idempotency fixtures");
    let live_key = format!("{prefix}_live");
    sqlx::query(
        "INSERT INTO idempotency_keys (idempotency_key, fingerprint, replay_until) \
         VALUES ($1, decode(repeat('00', 32), 'hex'), clock_timestamp() + interval '1 hour')",
    )
    .bind(&live_key)
    .execute(&pool)
    .await
    .expect("insert unexpired idempotency fixture");

    let first_batch = worker::purge_expired_idempotency_keys_once(&pool, 1000)
        .await
        .expect("purge first idempotency batch");
    assert_eq!(first_batch, 1000);
    let second_batch = worker::purge_expired_idempotency_keys_once(&pool, 1000)
        .await
        .expect("purge remaining idempotency key");
    assert_eq!(second_batch, 1);
    let remaining: Vec<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM idempotency_keys WHERE left(idempotency_key, length($1)) = $1 ORDER BY idempotency_key",
    )
    .bind(&prefix)
    .fetch_all(&pool)
    .await
    .expect("read remaining idempotency fixtures");
    assert_eq!(remaining, vec![live_key]);

    sqlx::query("DELETE FROM idempotency_keys WHERE left(idempotency_key, length($1)) = $1")
        .bind(&prefix)
        .execute(&pool)
        .await
        .expect("clean idempotency fixtures");
}

fn request(event_id: &str, ticket_type_id: &str, key: &str, quantity: u32) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!(
            "/v1/events/{event_id}/ticket-types/{ticket_type_id}/reservations"
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Idempotency-Key", key)
        .body(Body::from(json!({"quantity": quantity}).to_string()))
        .expect("build request")
}

fn release_request(event_id: &str, reservation_id: &str, key: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(format!(
            "/v1/events/{event_id}/reservations/{reservation_id}"
        ))
        .header("Idempotency-Key", key)
        .body(Body::empty())
        .expect("build release request")
}

async fn insert_catalog_event(
    pool: &PgPool,
    id: &str,
    title: &str,
    starts_at: &str,
    visibility: &str,
    venue: bool,
) {
    sqlx::query(
        "INSERT INTO events (id, organization_id, title, visibility, timezone, starts_at, venue) \
         VALUES ($1, $2, $3, $4, 'Asia/Colombo', $5::timestamptz, \
           CASE WHEN $6 THEN '{\"kind\":\"online\"}'::jsonb ELSE NULL END)",
    )
    .bind(id)
    .bind(seed::DEMO_ORGANIZATION_ID)
    .bind(title)
    .bind(visibility)
    .bind(starts_at)
    .bind(venue)
    .execute(pool)
    .await
    .expect("insert catalog event fixture");
}

/// Catalog fixtures are asserted against the real public catalog, which is only
/// filtered by "upcoming". A fixed timestamp would therefore let a seeded event
/// or a leftover row from an earlier run sort ahead of them. Deriving the window
/// from a fresh ULID gives each run an instant no other row can share.
fn catalog_window() -> DateTime<Utc> {
    let offset = Ulid::new().timestamp_ms() % 3_000_000;
    DateTime::from_timestamp(4_083_456_000, 0).expect("2099-01-01 is representable")
        + Duration::seconds(i64::try_from(offset).unwrap_or(0))
}

fn encode_catalog_cursor(starts_at: DateTime<Utc>, id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("v1\n{}\n{id}", starts_at.to_rfc3339()))
}

/// Remove far-future demo-organization rows left behind by a previous run that
/// aborted before its own cleanup. Scoped to the synthetic demo organization and
/// the fixture titles so seeded or authored demo events are left untouched.
async fn clear_stale_catalog_fixtures(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM events WHERE organization_id = $1 \
           AND title IN ('First event', 'Second event', 'Draft event', 'Incomplete event') \
           AND starts_at > clock_timestamp() + interval '1 year'",
    )
    .bind(seed::DEMO_ORGANIZATION_ID)
    .execute(pool)
    .await
    .expect("clear stale catalog fixtures from an earlier run");
}

async fn send(router: axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.oneshot(request).await.expect("route request");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body = serde_json::from_slice(&bytes).expect("response body should be JSON");
    (status, body)
}

#[tokio::test]
async fn public_event_catalog_is_upcoming_ordered_and_cursor_paginated() {
    let pool = database().await;
    let _catalog_lock = PUBLIC_CATALOG_TEST_LOCK.lock().await;
    clear_stale_catalog_fixtures(&pool).await;
    let id_prefix = Ulid::new();
    let first_id = format!("zeno_{id_prefix}01");
    let second_id = format!("zeno_{id_prefix}02");
    let draft_id = format!("zeno_{}", Ulid::new());
    let no_venue_id = format!("zeno_{}", Ulid::new());
    let past_id = format!("zeno_{}", Ulid::new());
    let window = catalog_window();
    let window_text = window.to_rfc3339();
    insert_catalog_event(
        &pool,
        &second_id,
        "Second event",
        &window_text,
        "public",
        true,
    )
    .await;
    insert_catalog_event(
        &pool,
        &past_id,
        "Past event",
        "2000-01-03 10:00:00+05:30",
        "public",
        true,
    )
    .await;
    insert_catalog_event(
        &pool,
        &first_id,
        "First event",
        &window_text,
        "public",
        true,
    )
    .await;
    insert_catalog_event(
        &pool,
        &draft_id,
        "Draft event",
        &(window + Duration::days(1)).to_rfc3339(),
        "draft",
        true,
    )
    .await;
    insert_catalog_event(
        &pool,
        &no_venue_id,
        "Incomplete event",
        &(window + Duration::days(2)).to_rfc3339(),
        "public",
        false,
    )
    .await;

    let router = app::router(pool.clone());
    // Start the walk one second before this run's window so only these fixtures
    // can be returned, whatever else the local database already holds.
    let anchor_cursor = encode_catalog_cursor(
        window - Duration::seconds(1),
        &format!("zeno_{}", "0".repeat(26)),
    );
    let first_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/events?limit=1&cursor={anchor_cursor}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("list first catalog page");
    assert_eq!(first_response.status(), StatusCode::OK);
    let first_bytes = to_bytes(first_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let first: Value = serde_json::from_slice(&first_bytes).unwrap();
    assert_eq!(first["items"][0]["id"], first_id);
    assert_eq!(first["items"][0]["title"], "First event");
    let cursor = first["next_cursor"]
        .as_str()
        .expect("catalog returns an opaque next cursor");
    assert_ne!(cursor, first_id);
    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(&first_id)
        .execute(&pool)
        .await
        .expect("remove page anchor before requesting the next page");

    let next_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/events?limit=1&cursor={cursor}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("list next catalog page");
    let next_bytes = to_bytes(next_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let next: Value = serde_json::from_slice(&next_bytes).unwrap();
    assert_eq!(next["items"][0]["id"], second_id);
    assert!(next["next_cursor"].is_null());

    let invalid_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/events?limit=51")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("reject unbounded catalog page");
    assert_eq!(invalid_response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let malformed_cursor_response = router
        .oneshot(
            Request::builder()
                .uri("/v1/events?cursor=not-a-cursor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("reject malformed catalog cursor");
    assert_eq!(
        malformed_cursor_response.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    sqlx::query("DELETE FROM events WHERE id = ANY($1)")
        .bind(vec![first_id, second_id, draft_id, no_venue_id, past_id])
        .execute(&pool)
        .await
        .expect("clean catalog fixtures");
}

#[tokio::test]
async fn reservation_contract_rejects_unused_contact_data_and_schema_drops_it() {
    let pool = database().await;
    let (event_id, ticket_type_id) = fixture(&pool, 2).await;
    let email_column_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'reservations' \
           AND column_name = 'buyer_email')",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect reservation columns");
    assert!(
        !email_column_exists,
        "purchaser email should not be persisted"
    );

    let response = app::router(pool.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/events/{event_id}/ticket-types/{ticket_type_id}/reservations"
                ))
                .header(header::CONTENT_TYPE, "application/json")
                .header("Idempotency-Key", idempotency_key())
                .body(Body::from(
                    json!({"quantity": 1, "buyer_email": "guest@example.com"}).to_string(),
                ))
                .expect("build request with deprecated personal data"),
        )
        .await
        .expect("reject unused contact data");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read unchanged ticket inventory");
    assert_eq!(held_count, 0);
    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean privacy fixture");
}

#[tokio::test]
async fn reservation_replays_identical_requests_and_rejects_key_reuse() {
    let pool = database().await;
    let (event_id, ticket_type_id) = fixture(&pool, 8).await;
    let router = app::router(pool.clone());
    let key = idempotency_key();
    let (created_status, created) =
        send(router.clone(), request(&event_id, &ticket_type_id, &key, 2)).await;
    let (replayed_status, replayed) =
        send(router.clone(), request(&event_id, &ticket_type_id, &key, 2)).await;
    assert_eq!(created_status, StatusCode::CREATED);
    assert_eq!(replayed_status, StatusCode::CREATED);
    assert_eq!(created, replayed);
    assert_eq!(created["quantity"], 2);
    assert_eq!(created["amount_due"]["amount"], "2500");
    let queued_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_jobs WHERE job_type = 'reservation.expire' AND aggregate_id = $1 AND status = 'pending'",
    )
    .bind(created["id"].as_str().expect("reservation id"))
    .fetch_one(&pool)
    .await
    .expect("read enqueued reservation expiry job");
    assert_eq!(queued_jobs, 1);

    let (conflict_status, conflict) =
        send(router.clone(), request(&event_id, &ticket_type_id, &key, 3)).await;
    assert_eq!(conflict_status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"]["code"], "IDEMPOTENCY_KEY_REUSED");

    sqlx::query("UPDATE idempotency_keys SET replay_until = clock_timestamp() - interval '1 second' WHERE idempotency_key = $1")
        .bind(&key)
        .execute(&pool)
        .await
        .expect("expire idempotency key fixture");
    let (reused_status, reused) = send(router, request(&event_id, &ticket_type_id, &key, 3)).await;
    assert_eq!(reused_status, StatusCode::CREATED);
    assert_ne!(created["id"], reused["id"]);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn attendee_can_release_only_their_own_hold_and_retries_are_safe() {
    let pool = database().await;
    let (event_id, ticket_type_id) = fixture(&pool, 3).await;
    let key = idempotency_key();
    let (status, reservation) = send(
        app::router(pool.clone()),
        request(&event_id, &ticket_type_id, &key, 2),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let reservation_id = reservation["id"].as_str().expect("reservation id");

    let (status, _) = send(
        app::router(pool.clone()),
        release_request(&event_id, reservation_id, &idempotency_key()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read inventory after unauthorized release");
    assert_eq!(held_count, 2);

    for _ in 0..2 {
        let (status, body) = send(
            app::router(pool.clone()),
            release_request(&event_id, reservation_id, &key),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "released");
    }

    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read inventory after release");
    assert_eq!(held_count, 0);
    let job_status: String = sqlx::query_scalar(
        "SELECT status FROM outbox_jobs WHERE aggregate_id = $1 AND job_type = 'reservation.expire'",
    )
    .bind(reservation_id)
    .fetch_one(&pool)
    .await
    .expect("read expiry job after release");
    assert_eq!(job_status, "completed");

    let expired_key = idempotency_key();
    let (status, expired_reservation) = send(
        app::router(pool.clone()),
        request(&event_id, &ticket_type_id, &expired_key, 1),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let expired_id = expired_reservation["id"].as_str().expect("reservation id");
    sqlx::query("UPDATE reservations SET expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(expired_id)
        .execute(&pool)
        .await
        .expect("expire hold fixture");
    let (status, body) = send(
        app::router(pool.clone()),
        release_request(&event_id, expired_id, &expired_key),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "expired");
}

#[tokio::test]
async fn concurrent_reservations_never_oversell_ticket_type() {
    let pool = database().await;
    let (event_id, ticket_type_id) = fixture(&pool, 10).await;
    let router = app::router(pool.clone());
    let mut requests = Vec::new();
    for _ in 0..32 {
        let router = router.clone();
        let event_id = event_id.clone();
        let ticket_type_id = ticket_type_id.clone();
        requests.push(tokio::spawn(async move {
            send(
                router,
                request(&event_id, &ticket_type_id, &idempotency_key(), 1),
            )
            .await
            .0
        }));
    }

    let mut created = 0;
    let mut conflicts = 0;
    for request in requests {
        match request.await.expect("reservation task") {
            StatusCode::CREATED => created += 1,
            StatusCode::CONFLICT => conflicts += 1,
            status => panic!("unexpected response status: {status}"),
        }
    }
    assert_eq!(created, 10);
    assert_eq!(conflicts, 22);

    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read inventory counter");
    assert_eq!(held_count, 10);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn expired_holds_release_capacity_before_the_next_reservation() {
    let pool = database().await;
    let _worker_queue_lock = lock_worker_queue_for_test(&pool).await;
    let (event_id, ticket_type_id) = fixture(&pool, 1).await;
    let router = app::router(pool.clone());
    let (status, reservation) = send(
        router.clone(),
        request(&event_id, &ticket_type_id, &idempotency_key(), 1),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    sqlx::query("UPDATE reservations SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(reservation["id"].as_str().expect("reservation id"))
        .execute(&pool)
        .await
        .expect("expire reservation fixture");
    sqlx::query("UPDATE outbox_jobs SET available_at = 'epoch' WHERE aggregate_id = $1")
        .bind(reservation["id"].as_str().expect("reservation id"))
        .execute(&pool)
        .await
        .expect("make expiry job due");

    let (status, next_reservation) = send(
        router,
        request(&event_id, &ticket_type_id, &idempotency_key(), 1),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(reservation["id"], next_reservation["id"]);
    let expiry_job_status: (String, i32) =
        sqlx::query_as("SELECT status, attempts FROM outbox_jobs WHERE aggregate_id = $1")
            .bind(reservation["id"].as_str().expect("reservation id"))
            .fetch_one(&pool)
            .await
            .expect("read inline-completed expiry job");
    assert_eq!(
        (expiry_job_status.0.as_str(), expiry_job_status.1),
        ("completed", 1)
    );

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn reservation_expires_only_the_bounded_holds_needed_to_free_inventory() {
    let pool = database().await;
    let _worker_queue_lock = lock_worker_queue_for_test(&pool).await;
    let (event_id, ticket_type_id) = fixture(&pool, 150).await;
    let expired_ids = (0..150)
        .map(|_| format!("zeno_{}", Ulid::new()))
        .collect::<Vec<_>>();

    sqlx::query(
        "INSERT INTO reservations \
         (id, event_id, ticket_type_id, quantity, amount_due, currency, status, expires_at) \
         SELECT id, $1, $2, 1, 1250, 'LKR', 'held', 'epoch' \
         FROM unnest($3::text[]) AS id",
    )
    .bind(&event_id)
    .bind(&ticket_type_id)
    .bind(&expired_ids)
    .execute(&pool)
    .await
    .expect("insert expired reservation fixtures");
    sqlx::query(
        "INSERT INTO outbox_jobs (job_type, aggregate_id, payload, available_at) \
         SELECT 'reservation.expire', id, jsonb_build_object('ticket_type_id', $2), 'epoch' \
         FROM unnest($1::text[]) AS id",
    )
    .bind(&expired_ids)
    .bind(&ticket_type_id)
    .execute(&pool)
    .await
    .expect("insert expiry job fixtures");
    sqlx::query("UPDATE ticket_types SET held_count = 150 WHERE id = $1")
        .bind(&ticket_type_id)
        .execute(&pool)
        .await
        .expect("set fixture inventory counter");

    let (status, _) = send(
        app::router(pool.clone()),
        request(&event_id, &ticket_type_id, &idempotency_key(), 100),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (expired, still_held): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status = 'expired'), \
                count(*) FILTER (WHERE status = 'held') \
         FROM reservations WHERE id = ANY($1)",
    )
    .bind(&expired_ids)
    .fetch_one(&pool)
    .await
    .expect("read inline-expired fixture states");
    assert_eq!((expired, still_held), (100, 50));

    let (completed, pending): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status = 'completed'), \
                count(*) FILTER (WHERE status = 'pending') \
         FROM outbox_jobs WHERE aggregate_id = ANY($1)",
    )
    .bind(&expired_ids)
    .fetch_one(&pool)
    .await
    .expect("read inline-completed expiry jobs");
    assert_eq!((completed, pending), (100, 50));

    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read inventory after bounded inline expiry");
    assert_eq!(held_count, 150);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn worker_releases_expired_holds_in_bounded_idempotent_batches() {
    let pool = database().await;
    let _worker_queue_lock = lock_worker_queue_for_test(&pool).await;
    let (event_id, ticket_type_id) = fixture(&pool, 5).await;
    let router = app::router(pool.clone());
    let mut reservation_ids = Vec::new();
    for quantity in [1, 2, 1] {
        let (status, reservation) = send(
            router.clone(),
            request(&event_id, &ticket_type_id, &idempotency_key(), quantity),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        reservation_ids.push(reservation["id"].as_str().unwrap().to_owned());
    }
    sqlx::query("UPDATE reservations SET expires_at = 'epoch' WHERE id = ANY($1)")
        .bind(&reservation_ids[..2])
        .execute(&pool)
        .await
        .expect("expire worker fixtures");
    sqlx::query("UPDATE outbox_jobs SET available_at = 'epoch' WHERE aggregate_id = ANY($1)")
        .bind(&reservation_ids[..2])
        .execute(&pool)
        .await
        .expect("make expiry jobs due");

    let first_batch = worker::expire_holds_once(&pool, 1)
        .await
        .expect("process first expiry batch");
    assert_eq!(first_batch.reservations_expired, 1);
    assert_eq!(first_batch.tickets_released, 1);

    let second_batch = worker::expire_holds_once(&pool, 10)
        .await
        .expect("process second expiry batch");
    assert!(second_batch.reservations_expired >= 1);
    assert!(second_batch.tickets_released >= 2);

    worker::expire_holds_once(&pool, 10)
        .await
        .expect("repeat expiry batch without releasing a hold twice");

    let (expired_count, held_count): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status = 'expired'), count(*) FILTER (WHERE status = 'held') \
         FROM reservations WHERE ticket_type_id = $1",
    )
    .bind(&ticket_type_id)
    .fetch_one(&pool)
    .await
    .expect("read worker reservation states");
    let inventory_held: i64 =
        sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
            .bind(&ticket_type_id)
            .fetch_one(&pool)
            .await
            .expect("read worker inventory counter");
    assert_eq!((expired_count, held_count, inventory_held), (2, 1, 1));
    let completed_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_jobs WHERE aggregate_id = ANY($1) AND status = 'completed' AND attempts = 1",
    )
    .bind(&reservation_ids[..2])
    .fetch_one(&pool)
    .await
    .expect("read completed expiry jobs");
    assert_eq!(completed_jobs, 2);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn worker_expires_holds_across_ticket_types_in_one_batch() {
    let pool = database().await;
    let _worker_queue_lock = lock_worker_queue_for_test(&pool).await;
    let (event_id, first_ticket_type_id) = fixture(&pool, 2).await;
    let second_ticket_type_id = format!("zeno_{}", Ulid::new());
    sqlx::query(
        "INSERT INTO ticket_types \
         (id, organization_id, event_id, name, unit_amount, currency, capacity, sale_starts_at, sale_ends_at) \
         VALUES ($1, (SELECT organization_id FROM events WHERE id = $2), $2, 'Studio', 2500.00, 'LKR', 2, now() - interval '1 hour', now() + interval '1 hour')",
    )
    .bind(&second_ticket_type_id)
    .bind(&event_id)
    .execute(&pool)
    .await
    .expect("insert second ticket type fixture");

    let ticket_type_ids = [first_ticket_type_id, second_ticket_type_id];
    let router = app::router(pool.clone());
    let mut reservation_ids = Vec::new();
    for ticket_type_id in &ticket_type_ids {
        for _ in 0..2 {
            let (status, reservation) = send(
                router.clone(),
                request(&event_id, ticket_type_id, &idempotency_key(), 1),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            reservation_ids.push(reservation["id"].as_str().unwrap().to_owned());
        }
    }
    sqlx::query("UPDATE reservations SET expires_at = 'epoch' WHERE id = ANY($1)")
        .bind(&reservation_ids)
        .execute(&pool)
        .await
        .expect("expire reservations in both ticket types");
    sqlx::query("UPDATE outbox_jobs SET available_at = 'epoch' WHERE aggregate_id = ANY($1)")
        .bind(&reservation_ids)
        .execute(&pool)
        .await
        .expect("make both ticket types' expiry jobs due");

    let batch = worker::expire_holds_once(&pool, 100)
        .await
        .expect("process a multi-ticket expiry batch");
    assert_eq!(batch.reservations_expired, 4);
    assert_eq!(batch.tickets_released, 4);

    for ticket_type_id in &ticket_type_ids {
        let (expired, held, inventory_held): (i64, i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE status = 'expired'), \
                    count(*) FILTER (WHERE status = 'held'), \
                    (SELECT held_count FROM ticket_types WHERE id = $1) \
             FROM reservations WHERE ticket_type_id = $1",
        )
        .bind(ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read expiry results for ticket type");
        assert_eq!((expired, held, inventory_held), (2, 0, 0));
    }

    let completed_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_jobs \
         WHERE aggregate_id = ANY($1) AND status = 'completed' AND attempts = 1",
    )
    .bind(&reservation_ids)
    .fetch_one(&pool)
    .await
    .expect("read completed expiry jobs");
    assert_eq!(completed_jobs, 4);

    let empty_batch = worker::expire_holds_once(&pool, 100)
        .await
        .expect("replay the completed batch without double-releasing inventory");
    assert_eq!(empty_batch.reservations_expired, 0);
    assert_eq!(empty_batch.tickets_released, 0);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn worker_selects_the_oldest_due_ticket_type_before_lexically_earlier_queues() {
    let pool = database().await;
    let _worker_queue_lock = lock_worker_queue_for_test(&pool).await;
    let (event_id, later_id_ticket_type_id) = fixture(&pool, 1).await;
    let earlier_id_ticket_type_id = "zeno_0000000000000000";
    sqlx::query(
        "INSERT INTO ticket_types \
         (id, organization_id, event_id, name, unit_amount, currency, capacity, sale_starts_at, sale_ends_at) \
         VALUES ($1, (SELECT organization_id FROM events WHERE id = $2), $2, 'Earlier ID', 2500.00, 'LKR', 1, now() - interval '1 hour', now() + interval '1 hour')",
    )
    .bind(earlier_id_ticket_type_id)
    .bind(&event_id)
    .execute(&pool)
    .await
    .expect("insert lexically earlier ticket type");

    let router = app::router(pool.clone());
    let (_, older_reservation) = send(
        router.clone(),
        request(&event_id, &later_id_ticket_type_id, &idempotency_key(), 1),
    )
    .await;
    let (_, newer_reservation) = send(
        router,
        request(&event_id, earlier_id_ticket_type_id, &idempotency_key(), 1),
    )
    .await;
    let older_reservation_id = older_reservation["id"].as_str().unwrap();
    let newer_reservation_id = newer_reservation["id"].as_str().unwrap();

    sqlx::query(
        "UPDATE reservations SET expires_at = CASE id \
             WHEN $1 THEN 'epoch'::timestamptz \
             ELSE 'epoch'::timestamptz + interval '1 second' END \
         WHERE id = ANY($2)",
    )
    .bind(older_reservation_id)
    .bind(vec![older_reservation_id, newer_reservation_id])
    .execute(&pool)
    .await
    .expect("set ordered reservation deadlines");
    sqlx::query(
        "UPDATE outbox_jobs SET available_at = CASE aggregate_id \
             WHEN $1 THEN 'epoch'::timestamptz \
             ELSE 'epoch'::timestamptz + interval '1 second' END \
         WHERE aggregate_id = ANY($2)",
    )
    .bind(older_reservation_id)
    .bind(vec![older_reservation_id, newer_reservation_id])
    .execute(&pool)
    .await
    .expect("set ordered outbox deadlines");

    let batch = worker::expire_holds_once(&pool, 1)
        .await
        .expect("process the oldest due queue first");
    assert_eq!(batch.reservations_expired, 1);
    assert_eq!(batch.tickets_released, 1);

    let statuses: Vec<(String, String)> =
        sqlx::query_as("SELECT id, status FROM reservations WHERE id = ANY($1) ORDER BY id")
            .bind(vec![older_reservation_id, newer_reservation_id])
            .fetch_all(&pool)
            .await
            .expect("read reservation states");
    assert_eq!(
        statuses
            .iter()
            .find(|(id, _)| id == older_reservation_id)
            .map(|(_, status)| status.as_str()),
        Some("expired")
    );
    assert_eq!(
        statuses
            .iter()
            .find(|(id, _)| id == newer_reservation_id)
            .map(|(_, status)| status.as_str()),
        Some("held")
    );

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn concurrent_workers_complete_an_expiry_job_and_release_inventory_once() {
    let pool = database().await;
    let _worker_queue_lock = lock_worker_queue_for_test(&pool).await;
    let (event_id, ticket_type_id) = fixture(&pool, 1).await;
    let (_, reservation) = send(
        app::router(pool.clone()),
        request(&event_id, &ticket_type_id, &idempotency_key(), 1),
    )
    .await;
    let reservation_id = reservation["id"].as_str().expect("reservation id");
    sqlx::query("UPDATE reservations SET expires_at = 'epoch' WHERE id = $1")
        .bind(reservation_id)
        .execute(&pool)
        .await
        .expect("expire reservation fixture");
    sqlx::query("UPDATE outbox_jobs SET available_at = 'epoch' WHERE aggregate_id = $1")
        .bind(reservation_id)
        .execute(&pool)
        .await
        .expect("make expiry job due");

    let (first, second) = tokio::join!(
        worker::expire_holds_once(&pool, 1),
        worker::expire_holds_once(&pool, 1)
    );
    let first = first.expect("first worker batch");
    let second = second.expect("second worker batch");
    assert_eq!(first.reservations_expired + second.reservations_expired, 1);
    assert_eq!(first.tickets_released + second.tickets_released, 1);

    let (job_status, attempts): (String, i32) =
        sqlx::query_as("SELECT status, attempts FROM outbox_jobs WHERE aggregate_id = $1")
            .bind(reservation_id)
            .fetch_one(&pool)
            .await
            .expect("read completed expiry job");
    assert_eq!((job_status.as_str(), attempts), ("completed", 1));
    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read released inventory");
    assert_eq!(held_count, 0);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn worker_health_requires_a_recent_successful_expiry_batch() {
    let pool = database().await;
    let instance_id = format!("test-{}", Ulid::new());

    assert!(
        !worker::is_healthy(&pool, &instance_id, 5_000)
            .await
            .expect("check missing worker heartbeat")
    );
    worker::record_heartbeat(&pool, &instance_id, true, Some(1))
        .await
        .expect("record successful expiry batch");
    assert!(
        worker::is_healthy(&pool, &instance_id, 5_000)
            .await
            .expect("check successful worker heartbeat")
    );

    worker::record_heartbeat(&pool, &instance_id, false, Some(1))
        .await
        .expect("record failed expiry batch");
    assert!(
        !worker::is_healthy(&pool, &instance_id, 5_000)
            .await
            .expect("failed batch makes worker unhealthy")
    );
    let failures: i32 = sqlx::query_scalar(
        "SELECT consecutive_failures FROM worker_heartbeats \
         WHERE worker_name = 'reservation-expiry' AND instance_id = $1",
    )
    .bind(&instance_id)
    .fetch_one(&pool)
    .await
    .expect("read consecutive worker failures");
    assert_eq!(failures, 1);

    worker::record_heartbeat(&pool, &instance_id, true, Some(1))
        .await
        .expect("record recovered expiry batch");
    assert!(
        worker::is_healthy(&pool, &instance_id, 5_000)
            .await
            .expect("successful batch restores worker health")
    );
    sqlx::query(
        "UPDATE worker_heartbeats SET last_batch_at = clock_timestamp() - interval '10 seconds' \
         WHERE worker_name = 'reservation-expiry' AND instance_id = $1",
    )
    .bind(&instance_id)
    .execute(&pool)
    .await
    .expect("make worker heartbeat stale");
    assert!(
        !worker::is_healthy(&pool, &instance_id, 5_000)
            .await
            .expect("stale worker heartbeat is unhealthy")
    );
    sqlx::query(
        "DELETE FROM worker_heartbeats \
         WHERE worker_name = 'reservation-expiry' AND instance_id = $1",
    )
    .bind(instance_id)
    .execute(&pool)
    .await
    .expect("clean worker heartbeat fixture");
}

#[tokio::test]
async fn malformed_idempotency_key_is_rejected_without_creating_a_hold() {
    let pool = database().await;
    let (event_id, ticket_type_id) = fixture(&pool, 1).await;
    let router = app::router(pool.clone());
    let response = router
        .oneshot(request(&event_id, &ticket_type_id, "too-short", 1))
        .await
        .expect("route request");
    let request_id = response
        .headers()
        .get("X-Request-ID")
        .expect("response request ID")
        .to_str()
        .expect("request ID header value")
        .to_owned();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let problem: Value = serde_json::from_slice(&body).expect("problem response should be JSON");
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(problem["error"]["code"], "INVALID_IDEMPOTENCY_KEY");
    assert_eq!(problem["request_id"], request_id);

    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read inventory counter");
    assert_eq!(held_count, 0);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn closed_sale_window_rejects_reservation_without_changing_inventory() {
    let pool = database().await;
    let (event_id, ticket_type_id) = fixture(&pool, 1).await;
    sqlx::query("UPDATE ticket_types SET sale_ends_at = now() - interval '1 second' WHERE id = $1")
        .bind(&ticket_type_id)
        .execute(&pool)
        .await
        .expect("close sale fixture");

    let (status, problem) = send(
        app::router(pool.clone()),
        request(&event_id, &ticket_type_id, &idempotency_key(), 1),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(problem["error"]["code"], "SALE_CLOSED");

    let held_count: i64 = sqlx::query_scalar("SELECT held_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read inventory counter");
    assert_eq!(held_count, 0);
    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn public_catalog_returns_only_published_event_data_and_live_ticket_availability() {
    let pool = database().await;
    let _catalog_lock = PUBLIC_CATALOG_TEST_LOCK.lock().await;
    let (event_id, ticket_type_id) = fixture(&pool, 1).await;
    sqlx::query(
        "UPDATE events SET summary = 'A public test event', \
             starts_at = clock_timestamp() + interval '1 day', \
             ends_at = clock_timestamp() + interval '2 days', \
             venue = $2 WHERE id = $1",
    )
    .bind(&event_id)
    .bind(json!({"kind": "physical", "name": "Test Hall", "city": "Colombo", "country_code": "LK"}))
    .execute(&pool)
    .await
    .expect("complete published event fixture");
    sqlx::query("UPDATE ticket_types SET description = 'Includes a day pass' WHERE id = $1")
        .bind(&ticket_type_id)
        .execute(&pool)
        .await
        .expect("describe ticket fixture");

    let router = app::router(pool.clone());
    let (reservation_status, reservation) = send(
        router.clone(),
        request(&event_id, &ticket_type_id, &idempotency_key(), 1),
    )
    .await;
    assert_eq!(reservation_status, StatusCode::CREATED);

    let live_tickets_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/events/{event_id}/ticket-types"))
                .body(Body::empty())
                .expect("build live ticket types request"),
        )
        .await
        .expect("list ticket types while the hold is active");
    assert_eq!(live_tickets_response.status(), StatusCode::OK);
    let live_tickets_body = to_bytes(live_tickets_response.into_body(), usize::MAX)
        .await
        .expect("read live ticket types response");
    let live_tickets: Value =
        serde_json::from_slice(&live_tickets_body).expect("live ticket types response JSON");
    assert_eq!(live_tickets["items"][0]["availability"], "sold_out");

    sqlx::query("UPDATE reservations SET expires_at = 'epoch' WHERE id = $1")
        .bind(reservation["id"].as_str().expect("reservation id"))
        .execute(&pool)
        .await
        .expect("expire held reservation fixture");

    let event_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/events/{event_id}"))
                .body(Body::empty())
                .expect("build event request"),
        )
        .await
        .expect("get public event");
    assert_eq!(event_response.status(), StatusCode::OK);
    assert!(event_response.headers().contains_key("X-Request-ID"));
    let event_body = to_bytes(event_response.into_body(), usize::MAX)
        .await
        .expect("read event response");
    let event: Value = serde_json::from_slice(&event_body).expect("event response JSON");
    assert_eq!(event["id"], event_id);
    assert_eq!(event["summary"], "A public test event");
    assert_eq!(event["venue"]["kind"], "physical");
    assert_eq!(event["venue"]["country_code"], "LK");

    let tickets_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/events/{event_id}/ticket-types"))
                .body(Body::empty())
                .expect("build ticket types request"),
        )
        .await
        .expect("list public ticket types");
    assert_eq!(tickets_response.status(), StatusCode::OK);
    let tickets_body = to_bytes(tickets_response.into_body(), usize::MAX)
        .await
        .expect("read ticket types response");
    let tickets: Value = serde_json::from_slice(&tickets_body).expect("ticket types JSON");
    assert_eq!(tickets["items"][0]["id"], ticket_type_id);
    assert_eq!(tickets["items"][0]["description"], "Includes a day pass");
    assert_eq!(tickets["items"][0]["price"]["amount"], "1250");
    assert_eq!(tickets["items"][0]["availability"], "available");
    assert!(tickets["items"][0].get("capacity").is_none());
    assert!(tickets["items"][0].get("held_count").is_none());

    sqlx::query("UPDATE events SET visibility = 'draft' WHERE id = $1")
        .bind(&event_id)
        .execute(&pool)
        .await
        .expect("make event private");
    let hidden_response = router
        .oneshot(
            Request::builder()
                .uri(format!("/v1/events/{event_id}"))
                .body(Body::empty())
                .expect("build hidden event request"),
        )
        .await
        .expect("read hidden event response");
    assert_eq!(hidden_response.status(), StatusCode::NOT_FOUND);

    sqlx::query("DELETE FROM events WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("clean fixture");
}

#[tokio::test]
async fn health_endpoints_report_process_and_database_readiness() {
    let pool = database().await;
    let router = app::router(pool);

    let live = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header(
                    "traceparent",
                    "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("liveness request");
    assert_eq!(live.status(), StatusCode::NO_CONTENT);
    let live_request_id = live
        .headers()
        .get("X-Request-ID")
        .expect("liveness request ID")
        .to_str()
        .expect("request ID header value");
    assert!(live_request_id.starts_with("req_"));
    let live_traceparent = live
        .headers()
        .get("traceparent")
        .expect("response trace context")
        .to_str()
        .expect("traceparent header value");
    assert!(live_traceparent.starts_with("00-4bf92f3577b34da6a3ce929d0e0e4736-"));
    assert!(live_traceparent.ends_with("-01"));

    let invalid = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header(
                    "traceparent",
                    "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invalid trace context is ignored");
    let invalid_traceparent = invalid
        .headers()
        .get("traceparent")
        .expect("generated response trace context")
        .to_str()
        .expect("traceparent header value");
    assert!(invalid_traceparent.starts_with("00-"));
    assert!(!invalid_traceparent.contains("00000000000000000000000000000000"));

    let ready = router
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("readiness request");
    assert_eq!(ready.status(), StatusCode::NO_CONTENT);
    let ready_request_id = ready
        .headers()
        .get("X-Request-ID")
        .expect("readiness request ID")
        .to_str()
        .expect("request ID header value");
    assert!(ready_request_id.starts_with("req_"));
    assert_ne!(live_request_id, ready_request_id);
}

#[tokio::test]
async fn metrics_expose_bounded_route_labels_and_postgres_pool_state() {
    let pool = database().await;
    let instance_id = format!("metrics-test-{}", Ulid::new());
    worker::record_heartbeat(&pool, &instance_id, false, Some(1))
        .await
        .expect("record failed worker heartbeat fixture");
    let router = app::router(pool.clone());

    for uri in [
        "/health/live",
        "/v1/events/private@example.com/unrecognized",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("build metrics fixture request"),
            )
            .await
            .expect("record metrics fixture request");
        assert!(matches!(
            response.status(),
            StatusCode::NO_CONTENT | StatusCode::NOT_FOUND
        ));
    }

    let response = router
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("build scrape request"),
        )
        .await
        .expect("scrape metrics");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4")
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read Prometheus metrics");
    let metrics = String::from_utf8(body.to_vec()).expect("Prometheus text is UTF-8");
    assert!(metrics.contains(
        "zeno_http_requests_total{method=\"GET\",route=\"/health/live\",status_code=\"204\"} 1"
    ));
    assert!(metrics.contains(
        "zeno_http_requests_total{method=\"GET\",route=\"unmatched\",status_code=\"404\"} 1"
    ));
    assert!(metrics.contains("zeno_database_pool_connections "));
    assert!(metrics.contains("zeno_database_pool_idle_connections "));
    assert!(metrics.contains(&format!(
        "zeno_worker_last_batch_succeeded{{instance_id=\"{instance_id}\",worker_name=\"reservation-expiry\"}} 0"
    )));
    assert!(metrics.contains(&format!(
        "zeno_worker_consecutive_failures{{instance_id=\"{instance_id}\",worker_name=\"reservation-expiry\"}} 1"
    )));
    assert!(metrics.contains("zeno_outbox_due_jobs "));
    let oldest_due_age = metrics
        .lines()
        .find_map(|line| line.strip_prefix("zeno_outbox_oldest_due_job_age_seconds "))
        .and_then(|value| value.parse::<f64>().ok())
        .expect("oldest due job age metric");
    assert!(oldest_due_age >= 0.0);
    assert!(!metrics.contains("private@example.com"));

    sqlx::query("DELETE FROM worker_heartbeats WHERE instance_id = $1")
        .bind(instance_id)
        .execute(&pool)
        .await
        .expect("clean worker heartbeat fixture");
}
