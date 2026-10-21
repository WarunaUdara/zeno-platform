use std::{net::SocketAddr, str::FromStr};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tower::ServiceExt;
use ulid::Ulid;
use zeno_api::{app, auth::NeonJwtVerifier};

const TEST_SEED: [u8; 32] = [73; 32];
const KEY_ID: &str = "organization-integration-key";

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    sub: &'a str,
    exp: usize,
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
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()),
        "refusing to run destructive integration tests against a non-local database"
    );
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("connect to local test Postgres");
    sqlx::migrate!().run(&pool).await.expect("run migrations");
    pool
}

fn signing_key() -> (String, EncodingKey) {
    let pair = Ed25519KeyPair::from_seed_unchecked(&TEST_SEED).expect("test seed is valid");
    let public_key = URL_SAFE_NO_PAD.encode(pair.public_key().as_ref());
    let jwks = serde_json::json!({
        "keys": [{
            "kty": "OKP",
            "crv": "Ed25519",
            "alg": "EdDSA",
            "use": "sig",
            "kid": KEY_ID,
            "x": public_key
        }]
    });
    (
        jwks.to_string(),
        EncodingKey::from_ed_der(&private_key_der()),
    )
}

fn private_key_der() -> Vec<u8> {
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(&TEST_SEED);
    der
}

fn bearer_token(key: &EncodingKey, issuer: &str, subject: &str) -> String {
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(KEY_ID.to_owned());
    encode(
        &header,
        &Claims {
            iss: issuer,
            sub: subject,
            exp: jsonwebtoken::get_current_timestamp() as usize + 60,
        },
        key,
    )
    .expect("integration bearer token signs")
}

/// Walk the public catalog until an event id is found.
///
/// The catalog is a cursor-paginated list ordered by start time and bounded to 50
/// results per page, and it is not filtered by organization. Assuming a given
/// event lands on the first page makes the assertion depend on whatever else the
/// shared test database happens to contain.
async fn find_in_catalog(router: &axum::Router, token: &str, event_id: &str) -> Option<Value> {
    let mut cursor: Option<String> = None;
    for _ in 0..40 {
        let uri = match &cursor {
            Some(cursor) => format!("/v1/events?limit=50&cursor={cursor}"),
            None => "/v1/events?limit=50".to_owned(),
        };
        let (status, page) = test_request(router, "GET", &uri, token, None).await;
        assert_eq!(status, StatusCode::OK);
        let found = page["items"]
            .as_array()
            .expect("catalog items is an array")
            .iter()
            .find(|event| event["id"] == event_id)
            .cloned();
        if found.is_some() {
            return found;
        }
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return None,
        }
    }
    None
}

async fn test_request(
    router: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = router
        .clone()
        .oneshot(builder.body(body).expect("request is valid"))
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024)
        .await
        .expect("response body is bounded");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response body is JSON")
    };
    (status, value)
}

async fn test_ticket_request(
    router: &axum::Router,
    organization_id: &str,
    event_id: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/organizations/{organization_id}/events/{event_id}/ticket-types"
                ))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("Idempotency-Key", "ticket-key-0123456789abc")
                .body(Body::from(body.to_string()))
                .expect("ticket request is valid"),
        )
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024)
        .await
        .expect("ticket response is bounded");
    (
        status,
        serde_json::from_slice(&bytes).expect("ticket response is JSON"),
    )
}

async fn start_jwks(jwks: String) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("JWKS listener binds to loopback");
    let address = listener.local_addr().expect("listener address exists");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let jwks = jwks.clone();
            tokio::spawn(async move {
                let mut request = [0_u8; 2048];
                let _ = stream.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    jwks.len(),
                    jwks
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    address
}

#[tokio::test]
async fn organization_creation_is_retry_safe_and_membership_scoped() {
    let pool = database().await;
    let (jwks, signing_key) = signing_key();
    let jwks_address = start_jwks(jwks).await;
    let issuer = format!("http://{jwks_address}");
    let verifier = NeonJwtVerifier::new(&format!("{issuer}/auth"), &format!("{issuer}/jwks"))
        .expect("loopback Neon Auth configuration is valid");
    let router = app::router_with_auth(pool.clone(), Some(verifier));
    let owner = bearer_token(&signing_key, &issuer, &format!("owner-{}", Ulid::new()));
    let other_identity = bearer_token(&signing_key, &issuer, &format!("other-{}", Ulid::new()));
    let slug = format!("club-{}", Ulid::new().to_string().to_lowercase());
    let create_body = json!({"name":"Campus Arts Society", "slug":slug});

    let (status, created) = test_request(
        &router,
        "POST",
        "/v1/organizations",
        &owner,
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["role"], "owner");
    let organization_id = created["id"].as_str().expect("public organization id");

    let (status, replayed) = test_request(
        &router,
        "POST",
        "/v1/organizations",
        &owner,
        Some(create_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replayed["id"], organization_id);

    let second_slug = format!("club-{}", Ulid::new().to_string().to_lowercase());
    let (status, _) = test_request(
        &router,
        "POST",
        "/v1/organizations",
        &owner,
        Some(json!({"name":"Student Media Circle", "slug":second_slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, owner_page) =
        test_request(&router, "GET", "/v1/organizations?limit=1", &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(owner_page["items"].as_array().unwrap().len(), 1);
    assert!(owner_page["next_cursor"].is_string());
    let cursor = owner_page["next_cursor"].as_str().unwrap();
    let (status, owner_page_two) = test_request(
        &router,
        "GET",
        &format!("/v1/organizations?limit=1&cursor={cursor}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(owner_page_two["items"].as_array().unwrap().len(), 1);
    assert_ne!(
        owner_page["items"][0]["id"],
        owner_page_two["items"][0]["id"]
    );
    assert_eq!(owner_page["items"][0]["role"], "owner");
    assert_eq!(owner_page_two["items"][0]["role"], "owner");
    assert!(owner_page_two["next_cursor"].is_null());

    let (status, other_page) =
        test_request(&router, "GET", "/v1/organizations", &other_identity, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(other_page["items"], json!([]));

    let (status, conflict) = test_request(
        &router,
        "POST",
        "/v1/organizations",
        &other_identity,
        Some(json!({"name":"Campus Arts Society", "slug":slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"]["code"], "ORGANIZATION_SLUG_UNAVAILABLE");
}

#[tokio::test]
async fn event_authoring_is_tenant_scoped_retry_safe_and_publish_gated() {
    let pool = database().await;
    let (jwks, signing_key) = signing_key();
    let jwks_address = start_jwks(jwks).await;
    let issuer = format!("http://{jwks_address}");
    let verifier = NeonJwtVerifier::new(&format!("{issuer}/auth"), &format!("{issuer}/jwks"))
        .expect("loopback Neon Auth configuration is valid");
    let router = app::router_with_auth(pool.clone(), Some(verifier));
    let owner = bearer_token(&signing_key, &issuer, &format!("owner-{}", Ulid::new()));
    let outsider = bearer_token(&signing_key, &issuer, &format!("outsider-{}", Ulid::new()));

    let (_, owner_org) = test_request(
        &router,
        "POST",
        "/v1/organizations",
        &owner,
        Some(json!({"name":"Event Owner Org", "slug":format!("owner-{}", Ulid::new().to_string().to_lowercase())})),
    )
    .await;
    let owner_org_id = owner_org["id"].as_str().unwrap();
    let (_, outsider_org) = test_request(
        &router,
        "POST",
        "/v1/organizations",
        &outsider,
        Some(json!({"name":"Different Org", "slug":format!("outsider-{}", Ulid::new().to_string().to_lowercase())})),
    )
    .await;
    let outsider_org_id = outsider_org["id"].as_str().unwrap();

    let event_body = json!({
        "slug":"campus-summit-2027",
        "title":"Campus Summit",
        "summary":"A practical summit for students.",
        "starts_at":"2027-04-10T04:00:00Z",
        "ends_at":"2027-04-10T12:00:00Z",
        "timezone":"Asia/Colombo",
        "venue":{"kind":"physical","name":"City Hall","city":"Colombo","country_code":"LK"},
        "accessibility_info":"Step-free entrance and accessible restroom.",
        "cancellation_policy":"Contact the organizer at least 48 hours before the event.",
        "support_email":"events@example.com"
    });
    let event_path = format!("/v1/organizations/{owner_org_id}/events");
    let (status, created_event) = test_request(
        &router,
        "POST",
        &event_path,
        &owner,
        Some(event_body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created_event["visibility"], "draft");
    assert_eq!(
        created_event["accessibility_info"],
        "Step-free entrance and accessible restroom."
    );
    let event_id = created_event["id"].as_str().unwrap();

    let (status, upload) = test_request(
        &router,
        "POST",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/cover-uploads"),
        &owner,
        Some(json!({"content_type":"image/webp"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cover_key = upload["key"].as_str().unwrap();
    assert!(cover_key.starts_with(&format!("events/{owner_org_id}/{event_id}/")));
    assert!(cover_key.ends_with(".webp"));

    let (status, unauthorized_upload) = test_request(
        &router,
        "POST",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/cover-uploads"),
        &outsider,
        Some(json!({"content_type":"image/webp"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(unauthorized_upload["error"]["code"], "NOT_FOUND");

    let (status, unsupported_upload) = test_request(
        &router,
        "POST",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/cover-uploads"),
        &owner,
        Some(json!({"content_type":"image/svg+xml"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        unsupported_upload["error"]["code"],
        "INVALID_AUTHORING_INPUT"
    );

    let (status, event_with_cover) = test_request(
        &router,
        "PUT",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/cover-image"),
        &owner,
        Some(json!({"key":cover_key,"alt":"A speaker presenting at Campus Summit"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(event_with_cover["cover_image_key"], cover_key);

    let (status, cross_event_cover) = test_request(
        &router,
        "PUT",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/cover-image"),
        &owner,
        Some(json!({
            "key":cover_key.replace(event_id, "zeno_01J9QY7B5C6D7E8F9G0H1J2K3P"),
            "alt":"A speaker presenting at Campus Summit"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        cross_event_cover["error"]["code"],
        "INVALID_AUTHORING_INPUT"
    );

    let (status, missing_cover_fields) = test_request(
        &router,
        "PUT",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/cover-image"),
        &owner,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing_cover_fields["error"]["code"], "INVALID_REQUEST");

    let (status, invalid_disclosure) = test_request(
        &router,
        "POST",
        &event_path,
        &owner,
        Some(json!({
            "slug":format!("invalid-email-{}", Ulid::new().to_string().to_lowercase()),
            "title":"Invalid disclosure event",
            "timezone":"Asia/Colombo",
            "support_email":"not-an-email"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        invalid_disclosure["error"]["code"],
        "INVALID_AUTHORING_INPUT"
    );

    let (status, same_start_event) = test_request(
        &router,
        "POST",
        &event_path,
        &owner,
        Some(json!({
            "slug":format!("same-start-{}", Ulid::new().to_string().to_lowercase()),
            "title":"Same start event",
            "starts_at":"2027-04-10T04:00:00Z",
            "ends_at":null,
            "timezone":"Asia/Colombo",
            "venue":{"kind":"online"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let same_start_event_id = same_start_event["id"].as_str().unwrap();

    let (status, unscheduled_event) = test_request(
        &router,
        "POST",
        &event_path,
        &owner,
        Some(json!({
            "slug":format!("unscheduled-{}", Ulid::new().to_string().to_lowercase()),
            "title":"Unscheduled event",
            "starts_at":null,
            "ends_at":null,
            "timezone":"Asia/Colombo",
            "venue":{"kind":"online"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let unscheduled_event_id = unscheduled_event["id"].as_str().unwrap();

    let (status, replayed_event) =
        test_request(&router, "POST", &event_path, &owner, Some(event_body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replayed_event["id"], event_id);

    let (status, updated_event) = test_request(
        &router,
        "PUT",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}"),
        &owner,
        Some(json!({
            "title":"Campus Summit, revised",
            "summary":"Updated details.",
            "starts_at":"2027-04-10T04:00:00Z",
            "ends_at":"2027-04-10T12:00:00Z",
            "timezone":"Asia/Colombo",
            "venue":{"kind":"physical","name":"City Hall","city":"Colombo","country_code":"LK"},
            "accessibility_info":"Step-free entrance and accessible restroom.",
            "cancellation_policy":"Contact the organizer at least 48 hours before the event.",
            "support_email":"events@example.com"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated_event["title"], "Campus Summit, revised");

    let mut page_ids = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = match cursor.as_deref() {
            Some(cursor) => format!("{event_path}?limit=1&cursor={cursor}"),
            None => format!("{event_path}?limit=1"),
        };
        let (status, page) = test_request(&router, "GET", &uri, &owner, None).await;
        assert_eq!(status, StatusCode::OK, "event page: {page}");
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        if page["items"][0]["id"] == event_id {
            assert!(page["items"][0].get("support_email").is_none());
        }
        page_ids.push(page["items"][0]["id"].as_str().unwrap().to_owned());
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        assert!(page_ids.len() < 4, "pagination must terminate");
    }
    assert_eq!(page_ids.len(), 3);
    assert_eq!(page_ids[2], unscheduled_event_id);
    assert!(page_ids.contains(&event_id.to_owned()));
    assert!(page_ids.contains(&same_start_event_id.to_owned()));

    let many_event_ids = (0..105)
        .map(|_| format!("zeno_{}", Ulid::new()))
        .collect::<Vec<_>>();
    sqlx::query(
        "INSERT INTO events (id, organization_id, slug, title, visibility, timezone, starts_at) \
         SELECT id, $1, lower('page-' || substr(id, 6)), 'Paginated test event', 'draft', 'Asia/Colombo', '2028-04-10T04:00:00Z' \
         FROM UNNEST($2::text[]) AS generated(id)",
    )
    .bind(owner_org_id)
    .bind(&many_event_ids)
    .execute(&pool)
    .await
    .expect("insert more than one hundred test events");

    let mut cursor: Option<String> = None;
    let mut paged_ids = std::collections::HashSet::new();
    loop {
        let uri = match cursor.as_deref() {
            Some(cursor) => format!("{event_path}?limit=50&cursor={cursor}"),
            None => format!("{event_path}?limit=50"),
        };
        let (status, page) = test_request(&router, "GET", &uri, &owner, None).await;
        assert_eq!(status, StatusCode::OK, "event page: {page}");
        let items = page["items"].as_array().unwrap();
        assert!(!items.is_empty());
        assert!(items.len() <= 50);
        for event in items {
            assert!(paged_ids.insert(event["id"].as_str().unwrap().to_owned()));
        }
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        assert!(paged_ids.len() < 109, "pagination must terminate");
    }
    assert_eq!(paged_ids.len(), 108);
    assert!(many_event_ids.iter().all(|id| paged_ids.contains(id)));

    let (status, _) = test_request(
        &router,
        "GET",
        &format!("{event_path}?limit=51"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, direct_event) = test_request(
        &router,
        "GET",
        &format!("{event_path}/{event_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(direct_event["id"], event_id);
    assert_eq!(direct_event["support_email"], "events@example.com");
    assert_eq!(direct_event["cover_image_key"], cover_key);
    assert_eq!(
        direct_event["cover_image_alt"],
        "A speaker presenting at Campus Summit"
    );

    let (status, _) = test_request(
        &router,
        "GET",
        &format!("/v1/organizations/{outsider_org_id}/events/{event_id}"),
        &outsider,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, not_ready) = test_request(
        &router,
        "POST",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/publish"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(not_ready["error"]["code"], "EVENT_NOT_READY");

    let ticket_body = json!({
        "name":"Student pass",
        "description":"Admission for enrolled students.",
        "amount":"1250.50",
        "currency":"LKR",
        "capacity":100,
        "sale_starts_at":"2026-11-01T00:00:00Z",
        "sale_ends_at":"2027-04-10T04:00:00Z"
    });
    let (status, ticket) =
        test_ticket_request(&router, owner_org_id, event_id, &owner, ticket_body.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "ticket response: {ticket}");
    assert_eq!(ticket["amount"], "1250.5");
    let mismatched_ticket_id = format!("zeno_{}", Ulid::new());
    let tenant_mismatch = sqlx::query(
        "INSERT INTO ticket_types (id, organization_id, event_id, name, unit_amount, currency, capacity, sale_starts_at, sale_ends_at) \
         VALUES ($1, $2, $3, 'Invalid tenant ticket', 1, 'LKR', 1, now(), now() + interval '1 day')",
    )
    .bind(&mismatched_ticket_id)
    .bind(outsider_org_id)
    .bind(event_id)
    .execute(&pool)
    .await;
    assert!(
        tenant_mismatch.is_err(),
        "database must reject tenant mismatch"
    );
    let (status, ticket_replay) =
        test_ticket_request(&router, owner_org_id, event_id, &owner, ticket_body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ticket_replay["id"], ticket["id"]);
    let (status, conflict) = test_ticket_request(
        &router,
        owner_org_id,
        event_id,
        &owner,
        json!({"name":"Changed pass","description":null,"amount":"1250.50","currency":"LKR","capacity":100,"sale_starts_at":"2026-11-01T00:00:00Z","sale_ends_at":"2027-04-10T04:00:00Z"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"]["code"], "AUTHORING_CONFLICT");

    let (status, organizer_tickets) = test_request(
        &router,
        "GET",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/ticket-types"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(organizer_tickets["items"].as_array().unwrap().len(), 1);
    assert_eq!(organizer_tickets["items"][0]["id"], ticket["id"]);
    let (status, updated_ticket) = test_request(
        &router,
        "PUT",
        &format!(
            "/v1/organizations/{owner_org_id}/events/{event_id}/ticket-types/{}",
            ticket["id"].as_str().unwrap()
        ),
        &owner,
        Some(json!({
            "name":"Student early bird",
            "description":"Updated ticket details.",
            "amount":"1400.00",
            "currency":"LKR",
            "capacity":120,
            "sale_starts_at":"2026-11-01T00:00:00Z",
            "sale_ends_at":"2027-04-10T04:00:00Z"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated_ticket["name"], "Student early bird");
    assert_eq!(updated_ticket["capacity"], 120);

    let (status, outsider_events) = test_request(
        &router,
        "GET",
        &format!("/v1/organizations/{outsider_org_id}/events"),
        &outsider,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(outsider_events["items"], json!([]));
    let (status, _) = test_request(
        &router,
        "GET",
        &format!("/v1/organizations/{outsider_org_id}/events/{event_id}/ticket-types"),
        &outsider,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = test_request(
        &router,
        "PUT",
        &format!("/v1/organizations/{outsider_org_id}/events/{event_id}"),
        &outsider,
        Some(json!({
            "title":"Unauthorized edit",
            "timezone":"Asia/Colombo",
            "summary":null,
            "starts_at":"2027-04-10T04:00:00Z",
            "ends_at":"2027-04-10T12:00:00Z",
            "venue":{"kind":"online"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) =
        test_ticket_request(&router, outsider_org_id, event_id, &outsider, ticket_body).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, published) = test_request(
        &router,
        "POST",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/publish"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(published.is_null());
    let (status, published_retry) = test_request(
        &router,
        "POST",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}/publish"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(published_retry.is_null());
    let (status, public_event) = test_request(
        &router,
        "GET",
        &format!("/v1/events/{event_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public_event["cover_image_key"], cover_key);
    assert_eq!(
        public_event["cover_image_alt"],
        "A speaker presenting at Campus Summit"
    );
    let (status, public_event) = test_request(
        &router,
        "GET",
        &format!("/v1/events/{event_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        public_event["accessibility_info"],
        "Step-free entrance and accessible restroom."
    );
    assert_eq!(
        public_event["cancellation_policy"],
        "Contact the organizer at least 48 hours before the event."
    );
    assert_eq!(public_event["support_email"], "events@example.com");
    let (status, _) = test_request(
        &router,
        "PUT",
        &format!("/v1/organizations/{owner_org_id}/events/{event_id}"),
        &owner,
        Some(json!({
            "title":"Attempt to edit published event",
            "timezone":"Asia/Colombo",
            "summary":null,
            "starts_at":"2027-04-10T04:00:00Z",
            "ends_at":"2027-04-10T12:00:00Z",
            "venue":{"kind":"online"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = test_request(
        &router,
        "PUT",
        &format!(
            "/v1/organizations/{owner_org_id}/events/{event_id}/ticket-types/{}",
            ticket["id"].as_str().unwrap()
        ),
        &owner,
        Some(json!({
            "name":"Attempt to change published ticket",
            "amount":"1400.00",
            "currency":"LKR",
            "capacity":120,
            "sale_starts_at":"2026-11-01T00:00:00Z",
            "sale_ends_at":"2027-04-10T04:00:00Z"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, public_event) = test_request(
        &router,
        "GET",
        &format!("/v1/events/{event_id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public_event["visibility"], "public");
    assert_eq!(public_event["title"], "Campus Summit, revised");
    assert_eq!(
        public_event["cancellation_policy"],
        "Contact the organizer at least 48 hours before the event."
    );
    assert_eq!(public_event["support_email"], "events@example.com");
    let catalog_event = find_in_catalog(&router, &owner, event_id)
        .await
        .expect("published event is in the catalog");
    assert!(catalog_event.get("support_email").is_none());

    let racing_event = json!({
        "slug":"campus-summit-race-2027",
        "title":"Race-safe event",
        "timezone":"Asia/Colombo",
        "starts_at":"2027-05-10T04:00:00Z",
        "ends_at":"2027-05-10T12:00:00Z",
        "venue":{"kind":"online"}
    });
    let (_, racing_event_response) = test_request(
        &router,
        "POST",
        &event_path,
        &owner,
        Some(racing_event.clone()),
    )
    .await;
    let racing_event_id = racing_event_response["id"].as_str().unwrap().to_owned();
    let racing_ticket = json!({
        "name":"Race ticket",
        "amount":"1.00",
        "currency":"LKR",
        "capacity":10,
        "sale_starts_at":"2026-11-01T00:00:00Z",
        "sale_ends_at":"2027-05-10T04:00:00Z"
    });
    let racing_publish_path =
        format!("/v1/organizations/{owner_org_id}/events/{racing_event_id}/publish");
    let (ticket_result, publish_result) = tokio::join!(
        test_ticket_request(
            &router,
            owner_org_id,
            &racing_event_id,
            &owner,
            racing_ticket,
        ),
        test_request(&router, "POST", &racing_publish_path, &owner, None,)
    );
    assert_eq!(ticket_result.0, StatusCode::CREATED);
    assert!(matches!(
        publish_result.0,
        StatusCode::NO_CONTENT | StatusCode::CONFLICT
    ));
    let racing_state: (String, i64) = sqlx::query_as(
        "SELECT e.visibility, count(t.id) FROM events e LEFT JOIN ticket_types t ON t.organization_id = e.organization_id AND t.event_id = e.id WHERE e.id = $1 GROUP BY e.id",
    )
    .bind(&racing_event_id)
    .fetch_one(&pool)
    .await
    .expect("read state after concurrent ticket and publish");
    assert_eq!(racing_state.1, 1);
    assert!(matches!(racing_state.0.as_str(), "draft" | "public"));

    sqlx::query("DELETE FROM events WHERE organization_id = ANY($1)")
        .bind(vec![owner_org_id.to_owned(), outsider_org_id.to_owned()])
        .execute(&pool)
        .await
        .expect("remove test events");
    sqlx::query("DELETE FROM organizations WHERE id = $1 OR id = $2")
        .bind(owner_org_id)
        .bind(outsider_org_id)
        .execute(&pool)
        .await
        .expect("remove test organizations");
}
