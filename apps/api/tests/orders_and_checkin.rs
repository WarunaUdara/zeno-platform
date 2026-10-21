//! End-to-end behaviour of confirming an order and admitting guests at the door.
//!
//! These tests exist because the failure modes they cover are invisible in unit
//! tests: two gates scanning one guest in the same second, a checkout retried on a
//! dropped connection, and a company whose ten colleagues arrive at different
//! times. All of them are normal operating conditions, not edge cases.

use std::{net::IpAddr, str::FromStr};

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use ulid::Ulid;
use zeno_api::{
    auth::AuthenticatedIdentity,
    checkin::{self, ScanOutcome},
    credential::CredentialKey,
    orders::{self, ConfirmOrderCommand},
};

fn credential_key() -> CredentialKey {
    CredentialKey::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
        .expect("the test key is valid base64url of 32 bytes")
}

fn identity(issuer: &str, subject: &str) -> AuthenticatedIdentity {
    AuthenticatedIdentity {
        issuer: issuer.to_owned(),
        subject: subject.to_owned(),
    }
}

async fn database() -> PgPool {
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must point to local test Postgres");
    let host = PgConnectOptions::from_str(&url)
        .expect("DATABASE_URL must be a valid PostgreSQL connection URL")
        .get_host()
        .to_owned();
    assert!(
        host.eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
        "refusing to run destructive integration tests against a non-local database"
    );
    // Enough connections that a concurrency test is not serialised by the pool
    // itself, which would hide the races these tests are looking for.
    let pool = PgPoolOptions::new()
        .max_connections(32)
        .connect(&url)
        .await
        .expect("connect to local test Postgres");
    sqlx::migrate!().run(&pool).await.expect("run migrations");
    pool
}

/// A published event with one ticket type, plus an organization the organizer
/// belongs to.
async fn published_event(pool: &PgPool, capacity: i64) -> (String, String) {
    let organization_id = format!("zeno_{}", Ulid::new());
    let event_id = format!("zeno_{}", Ulid::new());
    let ticket_type_id = format!("zeno_{}", Ulid::new());
    let slug = format!("slug-{}", Ulid::new()).to_lowercase();

    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
        .bind(&organization_id)
        .bind("Test organizer")
        .bind(&slug)
        .execute(pool)
        .await
        .expect("insert organization");

    sqlx::query(
        "INSERT INTO organization_memberships (organization_id, identity_issuer, identity_subject, role) \
         VALUES ($1, 'https://auth.test', 'organizer-1', 'owner')",
    )
    .bind(&organization_id)
    .execute(pool)
    .await
    .expect("insert membership");

    sqlx::query(
        "INSERT INTO events (id, organization_id, title, visibility, timezone, starts_at, venue) \
         VALUES ($1, $2, 'Door test event', 'public', 'Asia/Colombo', \
                 clock_timestamp() + interval '30 days', '{\"kind\":\"online\"}')",
    )
    .bind(&event_id)
    .bind(&organization_id)
    .execute(pool)
    .await
    .expect("insert event");

    sqlx::query(
        "INSERT INTO ticket_types (id, organization_id, event_id, name, unit_amount, currency, \
             capacity, sale_starts_at, sale_ends_at) \
         VALUES ($1, $2, $3, 'General', 2500, 'LKR', $4, \
                 clock_timestamp() - interval '1 hour', clock_timestamp() + interval '30 days')",
    )
    .bind(&ticket_type_id)
    .bind(&organization_id)
    .bind(&event_id)
    .bind(capacity)
    .execute(pool)
    .await
    .expect("insert ticket type");

    (event_id, ticket_type_id)
}

/// Reserve `quantity` tickets and return the reservation id.
async fn reserve(pool: &PgPool, event_id: &str, ticket_type_id: &str, quantity: i16) -> String {
    let reservation_id = format!("zeno_{}", Ulid::new());
    let amount = rust_decimal::Decimal::from(2500i64) * rust_decimal::Decimal::from(quantity);

    sqlx::query("UPDATE ticket_types SET held_count = held_count + $2 WHERE id = $1")
        .bind(ticket_type_id)
        .bind(quantity)
        .execute(pool)
        .await
        .expect("hold inventory");

    sqlx::query(
        "INSERT INTO reservations (id, event_id, ticket_type_id, quantity, amount_due, currency, \
             status, expires_at) \
         VALUES ($1, $2, $3, $4, $5, 'LKR', 'held', clock_timestamp() + interval '15 minutes')",
    )
    .bind(&reservation_id)
    .bind(event_id)
    .bind(ticket_type_id)
    .bind(quantity)
    .bind(amount)
    .execute(pool)
    .await
    .expect("insert reservation");

    sqlx::query(
        "INSERT INTO outbox_jobs (job_type, aggregate_id, payload, available_at) \
         VALUES ('reservation.expire', $1, jsonb_build_object('ticket_type_id', $2), \
                 clock_timestamp() + interval '15 minutes')",
    )
    .bind(&reservation_id)
    .bind(ticket_type_id)
    .execute(pool)
    .await
    .expect("insert expiry job");

    reservation_id
}

fn confirm_command(event_id: &str, reservation_id: &str) -> ConfirmOrderCommand {
    ConfirmOrderCommand {
        event_id: event_id.to_owned(),
        reservation_id: reservation_id.to_owned(),
        purchaser_name: "Campus Arts Society".to_owned(),
        purchaser_email: "society@example.com".to_owned(),
        answers: Vec::new(),
    }
}

#[tokio::test]
async fn confirming_a_hold_issues_one_ticket_per_unit_and_commits_the_inventory() {
    let pool = database().await;
    let key = credential_key();
    let (event_id, ticket_type_id) = published_event(&pool, 10).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 4).await;

    let order = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect("confirm succeeds");

    // Four units, four independently addressable tickets.
    assert_eq!(order.quantity, 4);
    assert_eq!(order.tickets.len(), 4);
    let sequences: Vec<i16> = order.tickets.iter().map(|t| t.sequence).collect();
    assert_eq!(sequences, vec![1, 2, 3, 4]);
    let credentials: std::collections::HashSet<&String> =
        order.tickets.iter().map(|t| &t.credential).collect();
    assert_eq!(credentials.len(), 4, "each ticket needs its own credential");

    let counts: (i64, i64, String) = sqlx::query_as(
        "SELECT held_count, sold_count, (SELECT status FROM reservations WHERE id = $2) \
         FROM ticket_types WHERE id = $1",
    )
    .bind(&ticket_type_id)
    .bind(&reservation_id)
    .fetch_one(&pool)
    .await
    .expect("read inventory");
    assert_eq!(counts.0, 0, "the hold must leave held_count");
    assert_eq!(counts.1, 4, "and become sold");
    assert_eq!(counts.2, "confirmed");
}

#[tokio::test]
async fn confirming_twice_returns_the_same_order_rather_than_a_second_one() {
    // The response to a checkout that timed out on a flaky network must be safe to
    // retry. Without the unique reservation_id this would issue two orders and two
    // sets of tickets for one purchase.
    let pool = database().await;
    let key = credential_key();
    let (event_id, ticket_type_id) = published_event(&pool, 5).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 2).await;

    let first = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect("first confirm succeeds");
    let second = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect("retry succeeds by replaying the existing order");

    assert_eq!(first.id, second.id, "retry must not create a second order");
    assert_eq!(first.tickets.len(), second.tickets.len());
    // Credentials are re-rendered so a guest who never saw the first response
    // still has working QR codes.
    assert_eq!(first.tickets[0].credential, second.tickets[0].credential);

    let order_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM orders WHERE reservation_id = $1")
            .bind(&reservation_id)
            .fetch_one(&pool)
            .await
            .expect("count orders");
    assert_eq!(order_count, 1);

    let sold: i64 = sqlx::query_scalar("SELECT sold_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read sold count");
    assert_eq!(sold, 2, "a retry must not sell the inventory twice");
}

#[tokio::test]
async fn concurrent_confirms_produce_exactly_one_order() {
    let pool = database().await;
    let key = credential_key();
    let (event_id, ticket_type_id) = published_event(&pool, 5).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 2).await;

    // Ten simultaneous attempts at the same confirm, as a double-submitting form
    // or a retry storm would produce.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let pool = pool.clone();
        let key = key.clone();
        let command = confirm_command(&event_id, &reservation_id);
        handles.push(tokio::spawn(async move {
            orders::confirm(&pool, &key, &command).await
        }));
    }

    let mut successes = 0;
    for handle in handles {
        if let Ok(Ok(_)) = handle.await {
            successes += 1;
        }
    }
    assert_eq!(successes, 10, "every caller sees a successful outcome");

    let orders: i64 = sqlx::query_scalar("SELECT count(*) FROM orders WHERE reservation_id = $1")
        .bind(&reservation_id)
        .fetch_one(&pool)
        .await
        .expect("count orders");
    assert_eq!(orders, 1, "ten concurrent confirms must yield one order");

    let tickets: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tickets t \
         JOIN orders o ON o.id = t.order_id WHERE o.reservation_id = $1",
    )
    .bind(&reservation_id)
    .fetch_one(&pool)
    .await
    .expect("count tickets");
    assert_eq!(tickets, 2, "and exactly the tickets that were paid for");
}

#[tokio::test]
async fn confirming_an_expired_hold_is_refused_and_leaves_no_order() {
    let pool = database().await;
    let key = credential_key();
    let (event_id, ticket_type_id) = published_event(&pool, 5).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 1).await;

    // The hold elapsed while the attendee was typing their name and email.
    sqlx::query("UPDATE reservations SET expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(&reservation_id)
        .execute(&pool)
        .await
        .expect("expire the hold");

    let error = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect_err("an expired hold must not confirm");
    assert_eq!(error, orders::OrderError::HoldExpired);

    let order_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM orders WHERE reservation_id = $1")
            .bind(&reservation_id)
            .fetch_one(&pool)
            .await
            .expect("count orders");
    assert_eq!(order_rows, 0, "an expired hold must leave no order behind");

    let sold: i64 = sqlx::query_scalar("SELECT sold_count FROM ticket_types WHERE id = $1")
        .bind(&ticket_type_id)
        .fetch_one(&pool)
        .await
        .expect("read sold count");
    assert_eq!(sold, 0, "and must not consume inventory");
}

#[tokio::test]
async fn a_group_can_arrive_over_time_and_every_guest_is_admitted_once() {
    // The motivating case: a company buys ten tickets, six colleagues arrive at
    // the door, the other four turn up an hour later.
    let pool = database().await;
    let key = credential_key();
    let organizer = identity("https://auth.test", "organizer-1");
    let (event_id, ticket_type_id) = published_event(&pool, 10).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 10).await;

    let order = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect("confirm succeeds");
    assert_eq!(order.tickets.len(), 10);

    let credentials: Vec<String> = order
        .tickets
        .iter()
        .map(|ticket| ticket.credential.clone())
        .collect();

    // First wave: six of them, scanned together in one request.
    let first_wave = credentials[..6].to_vec();
    let results = checkin::scan(
        &pool,
        &organizer,
        &order.organization_id,
        &event_id,
        &format!("scan-{}", Ulid::new()),
        &first_wave,
    )
    .await
    .expect("first wave scans");
    assert_eq!(results.len(), 6);
    assert!(
        results
            .iter()
            .all(|r| matches!(r.outcome, ScanOutcome::Admitted { .. })),
        "all six should be admitted"
    );

    let midway =
        checkin::attendance_for_event(&pool, &organizer, &order.organization_id, &event_id)
            .await
            .expect("attendance");
    assert_eq!(midway.issued, 10);
    assert_eq!(midway.admitted, 6);
    assert_eq!(midway.awaiting, 4);

    // Second wave: the remaining four, hours later, with no coordination.
    let second_wave = credentials[6..].to_vec();
    let late = checkin::scan(
        &pool,
        &organizer,
        &order.organization_id,
        &event_id,
        &format!("scan-{}", Ulid::new()),
        &second_wave,
    )
    .await
    .expect("second wave scans");
    assert!(
        late.iter()
            .all(|r| matches!(r.outcome, ScanOutcome::Admitted { .. }))
    );

    let final_counts =
        checkin::attendance_for_event(&pool, &organizer, &order.organization_id, &event_id)
            .await
            .expect("attendance");
    assert_eq!(final_counts.admitted, 10);
    assert_eq!(final_counts.awaiting, 0);

    // Every admission left exactly one ledger row, so the audit trail matches the
    // headcount. Scoped to this event: the suite shares one database and runs in
    // parallel, so a global count would count other tests' admissions.
    let ledger: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM check_ins WHERE action = 'admit' AND event_id = $1",
    )
    .bind(&event_id)
    .fetch_one(&pool)
    .await
    .expect("count ledger rows");
    assert_eq!(ledger, 10);
}

#[tokio::test]
async fn two_gates_scanning_the_same_guest_admit_exactly_one() {
    // The core race. Both scanners read the ticket as valid, both try to admit, and
    // only the row lock on the conditional update may stop a double admission.
    let pool = database().await;
    let key = credential_key();
    let organizer = identity("https://auth.test", "organizer-1");
    let (event_id, ticket_type_id) = published_event(&pool, 5).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 1).await;

    let order = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect("confirm succeeds");
    let credential = order.tickets[0].credential.clone();
    let organization_id = order.organization_id;

    let mut handles = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let organizer = organizer.clone();
        let credential = credential.clone();
        let organization_id = organization_id.clone();
        let event_id = event_id.clone();
        handles.push(tokio::spawn(async move {
            checkin::scan(
                &pool,
                &organizer,
                &organization_id,
                &event_id,
                &format!("scan-{}", Ulid::new()),
                std::slice::from_ref(&credential),
            )
            .await
        }));
    }

    let mut admitted = 0;
    let mut already = 0;
    for handle in handles {
        let results = handle.await.expect("scan task").expect("scan succeeds");
        match &results[0].outcome {
            ScanOutcome::Admitted { .. } => admitted += 1,
            ScanOutcome::AlreadyAdmitted { .. } => already += 1,
            other => panic!("unexpected scan outcome: {other:?}"),
        }
    }
    assert_eq!(admitted, 1, "exactly one gate may admit a guest");
    assert_eq!(already, 7);

    let admissions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM check_ins WHERE ticket_id = $1 AND action = 'admit'",
    )
    .bind(&order.tickets[0].id)
    .fetch_one(&pool)
    .await
    .expect("count admissions");
    assert_eq!(admissions, 1, "and exactly one ledger row");
}

#[tokio::test]
async fn a_repeated_scan_does_not_write_a_second_ledger_row() {
    // A scanner on bad venue Wi-Fi retries the same scan. It must receive the
    // original outcome and leave one record.
    let pool = database().await;
    let key = credential_key();
    let organizer = identity("https://auth.test", "organizer-1");
    let (event_id, ticket_type_id) = published_event(&pool, 5).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 1).await;

    let order = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect("confirm succeeds");
    let credential = order.tickets[0].credential.clone();
    let organization_id = order.organization_id;
    let scanner_action_id = format!("scan-{}", Ulid::new());

    let first = checkin::scan(
        &pool,
        &organizer,
        &organization_id,
        &event_id,
        &scanner_action_id,
        std::slice::from_ref(&credential),
    )
    .await
    .expect("first scan");
    assert!(matches!(first[0].outcome, ScanOutcome::Admitted { .. }));
    assert!(!first[0].replayed);

    let second = checkin::scan(
        &pool,
        &organizer,
        &organization_id,
        &event_id,
        &scanner_action_id,
        std::slice::from_ref(&credential),
    )
    .await
    .expect("retried scan");
    assert!(
        second[0].replayed,
        "the retry must be recognised as a replay"
    );

    let ledger: i64 = sqlx::query_scalar("SELECT count(*) FROM check_ins WHERE ticket_id = $1")
        .bind(&order.tickets[0].id)
        .fetch_one(&pool)
        .await
        .expect("count ledger");
    assert_eq!(ledger, 1, "one guest, one ledger row");
}

#[tokio::test]
async fn a_ticket_from_another_organization_is_unknown_rather_than_forbidden() {
    // Reporting "forbidden" would confirm a ticket id exists somewhere else and let
    // a scanner probe for one.
    let pool = database().await;
    let key = credential_key();
    let organizer = identity("https://auth.test", "organizer-1");
    let stranger = identity("https://auth.test", "organizer-2");

    let (event_id, ticket_type_id) = published_event(&pool, 5).await;
    let reservation_id = reserve(&pool, &event_id, &ticket_type_id, 1).await;
    let order = orders::confirm(&pool, &key, &confirm_command(&event_id, &reservation_id))
        .await
        .expect("confirm succeeds");
    let credential = order.tickets[0].credential.clone();
    let real_organization = order.organization_id;

    // A member of a different organization presents this event's ticket.
    let (other_event, other_ticket) = published_event(&pool, 5).await;
    let _ = (other_event, other_ticket);

    let results = checkin::scan(
        &pool,
        &stranger,
        &real_organization,
        &event_id,
        &format!("scan-{}", Ulid::new()),
        std::slice::from_ref(&credential),
    )
    .await;
    assert_eq!(
        results.err(),
        Some(checkin::CheckInError::ScopeMismatch),
        "a non-member cannot scan at all"
    );

    // And the legitimate organizer still can.
    let allowed = checkin::scan(
        &pool,
        &organizer,
        &real_organization,
        &event_id,
        &format!("scan-{}", Ulid::new()),
        std::slice::from_ref(&credential),
    )
    .await
    .expect("the owning organizer may scan");
    assert!(matches!(allowed[0].outcome, ScanOutcome::Admitted { .. }));
}

#[tokio::test]
async fn a_code_from_a_different_event_cannot_be_admitted() {
    // The credential is scoped to its event, so a ticket from tonight's event must
    // not admit someone at tomorrow's.
    let pool = database().await;
    let key = credential_key();
    let organizer = identity("https://auth.test", "organizer-1");

    let (event_a, ticket_a) = published_event(&pool, 5).await;
    let reservation_a = reserve(&pool, &event_a, &ticket_a, 1).await;
    let order_a = orders::confirm(&pool, &key, &confirm_command(&event_a, &reservation_a))
        .await
        .expect("confirm succeeds");
    let credential_a = order_a.tickets[0].credential.clone();

    let (event_b, ticket_b) = published_event(&pool, 5).await;
    let _ = reserve(&pool, &event_b, &ticket_b, 1).await;

    let results = checkin::scan(
        &pool,
        &organizer,
        &order_b_organization(&pool, &event_b).await,
        &event_b,
        &format!("scan-{}", Ulid::new()),
        std::slice::from_ref(&credential_a),
    )
    .await
    .expect("scan succeeds");
    assert!(
        matches!(results[0].outcome, ScanOutcome::Unknown),
        "event A's credential must be unknown at event B"
    );
}

#[tokio::test]
async fn an_action_id_from_another_tenant_is_not_replayed() {
    // The action id comes from the caller and the ledger's uniqueness is global.
    // If the replay lookup were not tenant-scoped, a member of organization B
    // could reuse an id from organization A and read A's admitted ticket.
    let pool = database().await;
    let key = credential_key();
    let organizer_a = identity("https://auth.test", "organizer-1");
    let organizer_b = identity("https://auth.test", "organizer-2");

    let (event_a, ticket_a) = published_event(&pool, 5).await;
    let reservation_a = reserve(&pool, &event_a, &ticket_a, 1).await;
    let order_a = orders::confirm(&pool, &key, &confirm_command(&event_a, &reservation_a))
        .await
        .expect("confirm succeeds");

    let (event_b, _ticket_b) = published_event(&pool, 5).await;
    let organization_b = order_b_organization(&pool, &event_b).await;
    sqlx::query(
        "INSERT INTO organization_memberships (organization_id, identity_issuer, identity_subject, role) \
         VALUES ($1, 'https://auth.test', 'organizer-2', 'owner')",
    )
    .bind(&organization_b)
    .execute(&pool)
    .await
    .expect("insert second organizer membership");

    let shared_action_id = format!("scan-{}", Ulid::new());
    let first = checkin::scan(
        &pool,
        &organizer_a,
        &order_a.organization_id,
        &event_a,
        &shared_action_id,
        std::slice::from_ref(&order_a.tickets[0].credential),
    )
    .await
    .expect("first tenant scans");
    assert!(matches!(first[0].outcome, ScanOutcome::Admitted { .. }));

    let second = checkin::scan(
        &pool,
        &organizer_b,
        &organization_b,
        &event_b,
        &shared_action_id,
        &["not-a-real-credential".to_owned()],
    )
    .await
    .expect("second tenant scans");
    assert!(
        !second[0].replayed,
        "another tenant's action id must not be treated as a replay"
    );
    assert!(
        matches!(second[0].outcome, ScanOutcome::Unknown),
        "the response must not disclose the first tenant's ticket"
    );
}

async fn order_b_organization(pool: &PgPool, event_id: &str) -> String {
    sqlx::query_scalar("SELECT organization_id FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .expect("read organization")
}
