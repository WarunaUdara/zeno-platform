use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Extension, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use ulid::Ulid;

use crate::{
    auth::{AuthenticatedIdentity, NeonJwtVerifier, TokenError},
    authoring::{
        self, AuthoringError, CoverImageRequest, CreateCoverUploadRequest, CreateEventRequest,
        CreateTicketRequest, OrganizerEventCursor, UpdateEventRequest,
    },
    catalog::{self, PublishedEvent, PublishedEventPage, PublishedTicketType},
    checkin::{self, CheckInError},
    checkout::{CheckoutError, CheckoutService, validate_idempotency_key},
    credential,
    forms::{self, FieldType, SubmittedAnswer},
    metrics::{self, ApiMetrics},
    model::CreateReservationRequest,
    orders::{self, ConfirmOrderCommand, OrderError},
    organizations::{self, CreateOrganizationRequest, OrganizationCursor, OrganizationError},
    postgres::PostgresInventoryRepository,
    telemetry,
};

#[derive(Clone)]
pub(super) struct AppState {
    pub(super) pool: PgPool,
    checkout: CheckoutService<PostgresInventoryRepository>,
    pub(super) metrics: ApiMetrics,
    auth: Option<NeonJwtVerifier>,
    /// Absent when `ZENO_TICKET_CREDENTIAL_KEY` is unset. Confirming an order
    /// needs it to seal credentials, so order confirmation fails closed without it
    /// rather than issuing tickets whose QR codes could never be shown again.
    pub(super) credential_key: Option<credential::CredentialKey>,
}

#[derive(Clone)]
struct RequestId(String);

#[derive(Clone)]
struct TraceContext {
    trace_id: String,
    parent_span_id: String,
    span_id: String,
    flags: String,
}

impl TraceContext {
    fn from_header(value: Option<&HeaderValue>) -> Self {
        let parsed = value
            .and_then(|value| value.to_str().ok())
            .and_then(parse_traceparent);
        let (trace_id, parent_span_id, flags) =
            parsed.unwrap_or_else(|| (random_hex(16), String::new(), "00".to_owned()));
        let span_id = random_hex(8);
        Self {
            trace_id,
            parent_span_id,
            span_id,
            flags,
        }
    }

    fn header_value(&self) -> String {
        format!("00-{}-{}-{}", self.trace_id, self.span_id, self.flags)
    }
}

fn random_hex(byte_count: usize) -> String {
    loop {
        let bytes = Ulid::new().to_bytes();
        let value = bytes[16 - byte_count..]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if value.bytes().any(|byte| byte != b'0') {
            return value;
        }
    }
}

fn parse_traceparent(value: &str) -> Option<(String, String, String)> {
    let parts = value.split('-').collect::<Vec<_>>();
    if parts.len() != 4
        || parts[0] != "00"
        || parts[1].len() != 32
        || parts[2].len() != 16
        || parts[3].len() != 2
        || !parts.iter().all(|part| {
            part.bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        || parts[1].bytes().all(|byte| byte == b'0')
        || parts[2].bytes().all(|byte| byte == b'0')
    {
        return None;
    }
    // Version 00 defines only the sampled bit. Reserved bits must be cleared.
    let flags = u8::from_str_radix(parts[3], 16).ok()? & 1;
    Some((
        parts[1].to_owned(),
        parts[2].to_owned(),
        format!("{flags:02x}"),
    ))
}

pub fn router(pool: PgPool) -> Router {
    router_with_auth(pool, None)
}

pub fn router_with_auth(pool: PgPool, auth: Option<NeonJwtVerifier>) -> Router {
    router_with_credentials(pool, auth, None)
}

/// Build the router with the key used to seal ticket credentials.
///
/// Passing `None` keeps every order-confirming route unavailable rather than
/// letting it issue tickets whose QR codes could not be re-displayed.
pub fn router_with_credentials(
    pool: PgPool,
    auth: Option<NeonJwtVerifier>,
    credential_key: Option<credential::CredentialKey>,
) -> Router {
    let checkout = CheckoutService::new(PostgresInventoryRepository::new(pool.clone()));
    let metrics = ApiMetrics::new();
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/metrics", get(metrics::render_metrics))
        .route("/v1/auth/me", get(authenticated_identity))
        .route(
            "/v1/organizations",
            get(list_organizations).post(create_organization),
        )
        .route(
            "/v1/events/{event_id}/registration-form",
            get(public_registration_form),
        )
        .route(
            "/v1/events/{event_id}/reservations/{reservation_id}/confirm",
            post(confirm_reservation),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/registration-form",
            put(publish_registration_form),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/check-ins",
            post(scan_check_ins),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/attendance",
            get(event_attendance),
        )
        .route(
            "/v1/organizations/{organization_id}/events",
            get(list_organizer_events).post(create_organizer_event),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/ticket-types",
            get(list_organizer_tickets).post(create_organizer_ticket),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/ticket-types/{ticket_type_id}",
            put(update_organizer_ticket),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}",
            get(get_organizer_event).put(update_organizer_event),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/cover-uploads",
            post(create_cover_upload),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/cover-image",
            put(update_organizer_cover_image),
        )
        .route(
            "/v1/organizations/{organization_id}/events/{event_id}/publish",
            post(publish_organizer_event),
        )
        .route("/v1/events", get(list_published_events))
        .route("/v1/events/{event_id}", get(get_published_event))
        .route(
            "/v1/events/{event_id}/ticket-types",
            get(list_published_ticket_types),
        )
        .route(
            "/v1/events/{event_id}/ticket-types/{ticket_type_id}/reservations",
            post(create_reservation),
        )
        .route(
            "/v1/events/{event_id}/reservations/{reservation_id}",
            delete(release_reservation),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(
            metrics.clone(),
            metrics::record_request,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<Body>| {
                    let request_id = request
                        .extensions()
                        .get::<RequestId>()
                        .map(|id| id.0.as_str())
                        .unwrap_or("unavailable");
                    let route =
                        metrics::route_label(request.method().as_str(), request.uri().path());
                    // Health probes run every 5-10 seconds on every instance. Left
                    // in, they were 40 of 45 spans in a sampled window: the trace
                    // list became a liveness-check log and the traffic worth
                    // looking at was the exception. The metric still counts them.
                    if route.starts_with("/health/") {
                        return tracing::Span::none();
                    }
                    let trace = request.extensions().get::<TraceContext>();
                    let trace_id = trace
                        .map(|trace| trace.trace_id.as_str())
                        .unwrap_or("unavailable");
                    let span_id = trace
                        .map(|trace| trace.span_id.as_str())
                        .unwrap_or("unavailable");
                    let parent_span_id = trace
                        .map(|trace| trace.parent_span_id.as_str())
                        .unwrap_or("");
                    let span = tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        route = %route,
                        request_id = %request_id,
                        trace_id = %trace_id,
                        span_id = %span_id,
                        parent_span_id = %parent_span_id,
                    );
                    // Adopt the identifiers the JSON logs already carry, so the
                    // span reaching Cloud Trace and the log line for this request
                    // share a trace id. A request with no upstream span falls back
                    // to its own id, which keeps root work (the expiry worker, the
                    // migration job) correlated as well.
                    if let Some(trace) = trace {
                        let parent = if trace.parent_span_id.is_empty() {
                            &trace.span_id
                        } else {
                            &trace.parent_span_id
                        };
                        if let Some(context) = telemetry::parent_context(&trace.trace_id, parent) {
                            // Only fails if the span already carries a parent,
                            // which a span created one line above cannot.
                            let _ = span.set_parent(context);
                        }
                    }
                    span
                })
                .on_request(DefaultOnRequest::new().level(tracing::Level::INFO))
                .on_response(DefaultOnResponse::new().level(tracing::Level::INFO)),
        )
        .layer(middleware::from_fn(attach_request_id))
        .with_state(AppState {
            pool,
            checkout,
            metrics,
            auth,
            credential_key,
        })
}

#[derive(Serialize)]
struct IdentityResponse {
    subject: String,
}

async fn authenticated_identity(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<IdentityResponse>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    Ok(Json(IdentityResponse {
        subject: identity.subject,
    }))
}

async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    request_id: &RequestId,
) -> Result<AuthenticatedIdentity, ApiError> {
    let verifier = state.auth.as_ref().ok_or_else(|| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        title: "Authentication unavailable",
        detail: "Authentication has not been configured for this service.",
        code: "AUTH_NOT_CONFIGURED",
        request_id: Some(request_id.0.clone()),
    })?;
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, token)| scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty())
        .map(|(_, token)| token)
        .ok_or_else(|| ApiError::unauthenticated(request_id))?;

    match verifier.verify(token).await {
        Ok(identity) => Ok(identity),
        Err(TokenError::Invalid) => Err(ApiError::unauthenticated(request_id)),
        Err(TokenError::Unavailable) => Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            title: "Authentication unavailable",
            detail: "The authentication provider could not be reached.",
            code: "AUTH_PROVIDER_UNAVAILABLE",
            request_id: Some(request_id.0.clone()),
        }),
    }
}

#[derive(Deserialize)]
struct ListOrganizationsQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list_organizations(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<ListOrganizationsQuery>, QueryRejection>,
) -> Result<Json<organizations::OrganizationPage>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Query(query) =
        query.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    let limit = query.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(ApiError::invalid_request().with_request_id(&request_id));
    }
    let cursor = match query.cursor.as_deref() {
        Some(value) => Some(
            OrganizationCursor::decode(value)
                .ok_or_else(|| ApiError::invalid_request().with_request_id(&request_id))?,
        ),
        None => None,
    };
    organizations::list(&state.pool, &identity, cursor, limit)
        .await
        .map(Json)
        .map_err(|_| ApiError::service_unavailable("ORGANIZATION_UNAVAILABLE", &request_id))
}

async fn create_organization(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    payload: Result<Json<CreateOrganizationRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !organizations::is_valid_name(&payload.name) || !organizations::is_valid_slug(&payload.slug)
    {
        return Err(ApiError::invalid_organization().with_request_id(&request_id));
    }
    let (organization, was_created) = organizations::create(
        &state.pool,
        &identity,
        payload,
        format!("zeno_{}", Ulid::new()),
    )
    .await
    .map_err(|error| match error {
        OrganizationError::SlugUnavailable => ApiError::organization_slug_unavailable(&request_id),
        OrganizationError::Unavailable => {
            ApiError::service_unavailable("ORGANIZATION_UNAVAILABLE", &request_id)
        }
    })?;
    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(organization)).into_response())
}

#[derive(Deserialize)]
struct ListOrganizerEventsQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list_organizer_events(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    query: Result<Query<ListOrganizerEventsQuery>, QueryRejection>,
) -> Result<Json<authoring::OrganizerEventPage>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    if !crate::model::is_public_id(&organization_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    let Query(query) =
        query.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    let limit = query.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(ApiError::invalid_request().with_request_id(&request_id));
    }
    let cursor = match query.cursor.as_deref() {
        Some(value) => Some(
            OrganizerEventCursor::decode(value)
                .ok_or_else(|| ApiError::invalid_request().with_request_id(&request_id))?,
        ),
        None => None,
    };
    authoring::list_events(&state.pool, &identity, &organization_id, cursor, limit)
        .await
        .map(Json)
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))
}

async fn get_organizer_event(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<authoring::OrganizerEvent>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    if !crate::model::is_public_id(&organization_id) || !crate::model::is_public_id(&event_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    let event = authoring::get_event(&state.pool, &identity, &organization_id, &event_id)
        .await
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    event
        .map(Json)
        .ok_or_else(|| ApiError::from(AuthoringError::NotFound).with_request_id(&request_id))
}

async fn create_organizer_event(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(organization_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<CreateEventRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !crate::model::is_public_id(&organization_id) || !authoring::is_valid_event(&payload) {
        return Err(ApiError::from(AuthoringError::InvalidInput).with_request_id(&request_id));
    }
    let (event, was_created) = authoring::create_event(
        &state.pool,
        &identity,
        &organization_id,
        payload,
        format!("zeno_{}", Ulid::new()),
    )
    .await
    .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(event)).into_response())
}

async fn update_organizer_event(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Result<Json<UpdateEventRequest>, JsonRejection>,
) -> Result<Json<authoring::OrganizerEvent>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !crate::model::is_public_id(&organization_id)
        || !crate::model::is_public_id(&event_id)
        || !authoring::is_valid_update(&payload)
    {
        return Err(ApiError::from(AuthoringError::InvalidInput).with_request_id(&request_id));
    }
    authoring::update_event(&state.pool, &identity, &organization_id, &event_id, payload)
        .await
        .map(Json)
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))
}

#[derive(Serialize)]
struct CoverUploadResponse {
    key: String,
    content_type: String,
}

async fn create_cover_upload(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Result<Json<CreateCoverUploadRequest>, JsonRejection>,
) -> Result<Json<CoverUploadResponse>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    let Some(extension) = authoring::cover_image_extension(&payload.content_type) else {
        return Err(ApiError::from(AuthoringError::InvalidInput).with_request_id(&request_id));
    };
    if !crate::model::is_public_id(&organization_id) || !crate::model::is_public_id(&event_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    authoring::authorize_cover_upload(&state.pool, &identity, &organization_id, &event_id)
        .await
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    Ok(Json(CoverUploadResponse {
        key: format!(
            "events/{organization_id}/{event_id}/{}.{}",
            Ulid::new(),
            extension
        ),
        content_type: payload.content_type,
    }))
}

async fn update_organizer_cover_image(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Result<Json<CoverImageRequest>, JsonRejection>,
) -> Result<Json<authoring::OrganizerEvent>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !crate::model::is_public_id(&organization_id) || !crate::model::is_public_id(&event_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    if payload.key.is_some() != payload.alt.is_some() {
        return Err(ApiError::from(AuthoringError::InvalidInput).with_request_id(&request_id));
    }
    authoring::update_cover_image(&state.pool, &identity, &organization_id, &event_id, payload)
        .await
        .map(Json)
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))
}

async fn create_organizer_ticket(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Result<Json<CreateTicketRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let idempotency_key = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            (22..=128).contains(&value.len())
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
        .ok_or_else(|| ApiError::from(AuthoringError::InvalidInput).with_request_id(&request_id))?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !crate::model::is_public_id(&organization_id)
        || !crate::model::is_public_id(&event_id)
        || !authoring::is_valid_ticket(&payload)
    {
        return Err(ApiError::from(AuthoringError::InvalidInput).with_request_id(&request_id));
    }
    let (ticket, was_created) = authoring::create_ticket(
        &state.pool,
        &identity,
        &organization_id,
        &event_id,
        idempotency_key,
        payload,
        format!("zeno_{}", Ulid::new()),
    )
    .await
    .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(ticket)).into_response())
}

async fn list_organizer_tickets(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<OrganizerTickets>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    if !crate::model::is_public_id(&organization_id) || !crate::model::is_public_id(&event_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    authoring::list_tickets(&state.pool, &identity, &organization_id, &event_id)
        .await
        .map(|items| Json(OrganizerTickets { items }))
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))
}

#[derive(Serialize)]
struct OrganizerTickets {
    items: Vec<authoring::OrganizerTicket>,
}

async fn update_organizer_ticket(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id, ticket_type_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    payload: Result<Json<CreateTicketRequest>, JsonRejection>,
) -> Result<Json<authoring::OrganizerTicket>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !crate::model::is_public_id(&organization_id)
        || !crate::model::is_public_id(&event_id)
        || !crate::model::is_public_id(&ticket_type_id)
        || !authoring::is_valid_ticket(&payload)
    {
        return Err(ApiError::from(AuthoringError::InvalidInput).with_request_id(&request_id));
    }
    authoring::update_ticket(
        &state.pool,
        &identity,
        &organization_id,
        &event_id,
        &ticket_type_id,
        payload,
    )
    .await
    .map(Json)
    .map_err(|error| ApiError::from(error).with_request_id(&request_id))
}

async fn publish_organizer_event(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    if !crate::model::is_public_id(&organization_id) || !crate::model::is_public_id(&event_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    authoring::publish_event(&state.pool, &identity, &organization_id, &event_id)
        .await
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Orders, registration forms and admission
// ---------------------------------------------------------------------------

/// Collapse a database failure into a safe 503 while recording what actually
/// happened on the current span.
///
/// Keeps the two halves of the rule in one place: the caller may never learn why
/// the database failed, and an operator must always be able to find out.
fn collapse_database_error<'a>(
    operation: &'static str,
    request_id: &'a RequestId,
) -> impl Fn(sqlx::Error) -> ApiError + 'a {
    move |error| {
        crate::db_error::record(error, operation);
        ApiError::from(OrderError::Unavailable).with_request_id(request_id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmOrderRequest {
    purchaser_name: String,
    purchaser_email: String,
    #[serde(default)]
    answers: Vec<SubmittedAnswerWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmittedAnswerWire {
    field_key: String,
    value: String,
}

async fn confirm_reservation(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((event_id, reservation_id)): Path<(String, String)>,
    payload: Result<Json<ConfirmOrderRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(key) = state.credential_key.as_ref() else {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            title: "Ordering unavailable",
            detail: "Order confirmation is not configured on this deployment.",
            code: "ORDERING_UNAVAILABLE",
            request_id: Some(request_id.0.clone()),
        });
    };
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;

    let command = ConfirmOrderCommand {
        event_id,
        reservation_id,
        purchaser_name: payload.purchaser_name,
        purchaser_email: payload.purchaser_email,
        answers: payload
            .answers
            .into_iter()
            .map(|answer| SubmittedAnswer {
                field_key: answer.field_key,
                value: answer.value,
            })
            .collect(),
    };
    let order = orders::confirm(&state.pool, key, &command)
        .await
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    Ok(Json(serde_json::to_value(order).map_err(|_| {
        ApiError::from(OrderError::Unavailable).with_request_id(&request_id)
    })?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishFormField {
    field_key: String,
    label: String,
    #[serde(default)]
    help_text: Option<String>,
    field_type: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    options: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishFormRequest {
    name: String,
    fields: Vec<PublishFormField>,
}

#[derive(Serialize)]
struct PublicFormField {
    field_key: String,
    label: String,
    help_text: Option<String>,
    field_type: String,
    required: bool,
    options: Vec<String>,
}

#[derive(Serialize)]
struct PublicForm {
    name: String,
    version: i32,
    fields: Vec<PublicFormField>,
}

async fn public_registration_form(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(event_id): Path<String>,
) -> Result<Json<PublicForm>, ApiError> {
    let form = sqlx::query_as::<_, (String, i32)>(
        "SELECT f.name, f.version FROM registration_forms f \
         JOIN events e ON e.id = f.event_id \
         WHERE f.event_id = $1 AND f.status = 'published' \
           AND e.visibility = 'public' AND e.starts_at IS NOT NULL AND e.venue IS NOT NULL",
    )
    .bind(&event_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(collapse_database_error(
        "public_registration_form",
        &request_id,
    ))?;

    let Some((name, version)) = form else {
        return Err(ApiError::from(OrderError::NotFound).with_request_id(&request_id));
    };

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            String,
            bool,
            serde_json::Value,
        ),
    >(
        "SELECT field_key, label, help_text, field_type, required, options \
         FROM registration_form_fields WHERE form_id = ( \
             SELECT id FROM registration_forms WHERE event_id = $1 AND status = 'published') \
         ORDER BY position, id",
    )
    .bind(&event_id)
    .fetch_all(&state.pool)
    .await
    .map_err(collapse_database_error(
        "public_registration_form",
        &request_id,
    ))?;

    Ok(Json(PublicForm {
        name,
        version,
        fields: rows
            .into_iter()
            .map(
                |(field_key, label, help_text, field_type, required, options)| PublicFormField {
                    field_key,
                    label,
                    help_text,
                    field_type,
                    required,
                    options: options
                        .as_array()
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|value| value.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default(),
                },
            )
            .collect(),
    }))
}

/// Publish a new form version for an event.
///
/// Each save creates a new version rather than editing the live one, because
/// confirmed bookings already point at a version and must keep reading the labels
/// and options they were validated against.
async fn publish_registration_form(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Result<Json<PublishFormRequest>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !crate::model::is_public_id(&organization_id) || !crate::model::is_public_id(&event_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    if payload.fields.len() > forms::MAX_FIELDS_PER_FORM {
        return Err(ApiError::from(OrderError::InvalidInput).with_request_id(&request_id));
    }
    let mut seen = std::collections::HashSet::new();
    for field in &payload.fields {
        let Some(field_type) = FieldType::parse(&field.field_type) else {
            return Err(ApiError::from(OrderError::InvalidInput).with_request_id(&request_id));
        };
        if !forms::validate_field(
            &field.field_key,
            &field.label,
            field_type,
            field.required,
            &field.options,
        ) || !seen.insert(field.field_key.as_str())
        {
            return Err(ApiError::from(OrderError::InvalidInput).with_request_id(&request_id));
        }
    }

    let mut transaction = state.pool.begin().await.map_err(collapse_database_error(
        "publish_registration_form",
        &request_id,
    ))?;

    let manageable = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events e JOIN organization_memberships m \
           ON m.organization_id = e.organization_id \
         WHERE e.organization_id = $1 AND e.id = $2 AND e.visibility = 'draft' \
           AND m.identity_issuer = $3 AND m.identity_subject = $4 \
           AND m.role IN ('owner', 'admin', 'event_manager'))",
    )
    .bind(&organization_id)
    .bind(&event_id)
    .bind(&identity.issuer)
    .bind(&identity.subject)
    .fetch_one(&mut *transaction)
    .await
    .map_err(collapse_database_error(
        "publish_registration_form",
        &request_id,
    ))?;
    if !manageable {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }

    let next_version: i32 = sqlx::query_scalar(
        "SELECT COALESCE(max(version), 0) + 1 FROM registration_forms WHERE event_id = $1",
    )
    .bind(&event_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| ApiError::from(OrderError::Unavailable))?;

    let form_id = format!("zeno_{}", Ulid::new());
    sqlx::query(
        "INSERT INTO registration_forms (id, organization_id, event_id, name, status, version, published_at) \
         VALUES ($1, $2, $3, $4, 'published', $5, clock_timestamp())",
    )
    .bind(&form_id)
    .bind(&organization_id)
    .bind(&event_id)
    .bind(payload.name.trim())
    .bind(next_version)
    .execute(&mut *transaction)
    .await
    .map_err(collapse_database_error(
        "publish_registration_form",
        &request_id,
    ))?;

    // Retire the previous version rather than deleting it: past answers must stay
    // readable.
    sqlx::query(
        "UPDATE registration_forms SET status = 'retired' \
         WHERE event_id = $1 AND status = 'published'",
    )
    .bind(&event_id)
    .execute(&mut *transaction)
    .await
    .map_err(|_| ApiError::from(OrderError::Unavailable))?;

    for (index, field) in payload.fields.iter().enumerate() {
        sqlx::query(
            "INSERT INTO registration_form_fields (id, form_id, position, field_key, label, \
                 help_text, field_type, required, options) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb)",
        )
        .bind(format!("zeno_{}", Ulid::new()))
        .bind(&form_id)
        .bind(index as i32)
        .bind(field.field_key.trim())
        .bind(field.label.trim())
        .bind(field.help_text.as_deref().map(str::trim))
        .bind(&field.field_type)
        .bind(field.required)
        .bind(serde_json::to_string(&field.options).unwrap_or_else(|_| "[]".to_owned()))
        .execute(&mut *transaction)
        .await
        .map_err(|_| ApiError::from(OrderError::Unavailable))?;
    }

    transaction
        .commit()
        .await
        .map_err(|_| ApiError::from(OrderError::Unavailable))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanRequest {
    codes: Vec<String>,
    scanner_action_id: String,
}

async fn scan_check_ins(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Result<Json<ScanRequest>, JsonRejection>,
) -> Result<Json<Vec<checkin::ScanResult>>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    if !crate::model::is_public_id(&organization_id) || !crate::model::is_public_id(&event_id) {
        return Err(ApiError::from(AuthoringError::NotFound).with_request_id(&request_id));
    }
    if !matches!(payload.scanner_action_id.as_bytes(),
        bytes if (8..=64).contains(&bytes.len())
            && bytes.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
    {
        return Err(ApiError::from(OrderError::InvalidInput).with_request_id(&request_id));
    }

    let results = checkin::scan(
        &state.pool,
        &identity,
        &organization_id,
        &event_id,
        &payload.scanner_action_id,
        &payload.codes,
    )
    .await
    .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    Ok(Json(results))
}

async fn event_attendance(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((organization_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<checkin::Attendance>, ApiError> {
    let identity = authenticate(&state, &headers, &request_id).await?;
    let attendance =
        checkin::attendance_for_event(&state.pool, &identity, &organization_id, &event_id)
            .await
            .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    Ok(Json(attendance))
}

async fn attach_request_id(mut request: Request<Body>, next: Next) -> Response {
    let request_id = RequestId(format!("req_{}", Ulid::new()));
    let trace_context = TraceContext::from_header(request.headers().get("traceparent"));
    request.extensions_mut().insert(request_id.clone());
    request.extensions_mut().insert(trace_context.clone());
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&request_id.0) {
        response.headers_mut().insert("X-Request-ID", value);
    }
    if let Ok(value) = HeaderValue::from_str(&trace_context.header_value()) {
        response.headers_mut().insert("traceparent", value);
    }
    response
}

async fn liveness() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn readiness(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .map_err(|error| {
            crate::db_error::record(error, "readiness");
            ApiError::from(CheckoutError::Unavailable).with_request_id(&request_id)
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_published_event(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(event_id): Path<String>,
) -> Result<Json<PublishedEvent>, ApiError> {
    let event = catalog::get_published_event(&state.pool, &event_id)
        .await
        .map_err(|error| {
            crate::db_error::record(error, "get_published_event");
            ApiError::from(CheckoutError::Unavailable).with_request_id(&request_id)
        })?;
    event
        .map(Json)
        .ok_or_else(|| ApiError::from(CheckoutError::NotFound).with_request_id(&request_id))
}

#[derive(Deserialize)]
struct ListEventsQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list_published_events(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<ListEventsQuery>, QueryRejection>,
) -> Result<Json<PublishedEventPage>, ApiError> {
    let Query(query) =
        query.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    let limit = query.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err(ApiError::invalid_request().with_request_id(&request_id));
    }
    let cursor = match query.cursor.as_deref() {
        Some(cursor) => Some(
            catalog::decode_cursor(cursor)
                .ok_or_else(|| ApiError::invalid_request().with_request_id(&request_id))?,
        ),
        None => None,
    };
    catalog::list_published_events(&state.pool, cursor, limit)
        .await
        .map(Json)
        .map_err(|error| {
            crate::db_error::record(error, "list_published_events");
            ApiError::from(CheckoutError::Unavailable).with_request_id(&request_id)
        })
}

async fn list_published_ticket_types(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(event_id): Path<String>,
) -> Result<Json<PublishedTicketTypes>, ApiError> {
    let items = catalog::list_published_ticket_types(&state.pool, &event_id)
        .await
        .map_err(|error| {
            crate::db_error::record(error, "list_published_ticket_types");
            ApiError::from(CheckoutError::Unavailable).with_request_id(&request_id)
        })?;
    items
        .map(|items| Json(PublishedTicketTypes { items }))
        .ok_or_else(|| ApiError::from(CheckoutError::NotFound).with_request_id(&request_id))
}

#[derive(Serialize)]
struct PublishedTicketTypes {
    items: Vec<PublishedTicketType>,
}

async fn create_reservation(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((event_id, ticket_type_id)): Path<(String, String)>,
    headers: HeaderMap,
    payload: Result<Json<CreateReservationRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let key = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::from(CheckoutError::InvalidIdempotencyKey).with_request_id(&request_id)
        })?;
    let Json(payload) =
        payload.map_err(|_| ApiError::invalid_request().with_request_id(&request_id))?;
    let result = state
        .checkout
        .reserve(&event_id, &ticket_type_id, key, payload)
        .await
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;

    let mut response = (StatusCode::CREATED, Json(result.reservation)).into_response();
    response.headers_mut().insert(
        "Idempotency-Key-Expires-At",
        HeaderValue::from_str(
            &result
                .replay_until
                .to_rfc3339_opts(SecondsFormat::Millis, true),
        )
        .map_err(|_| ApiError::from(CheckoutError::Unavailable))?,
    );
    Ok(response)
}

async fn release_reservation(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path((event_id, reservation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<crate::model::ReleaseReservationResult>, ApiError> {
    if !crate::model::is_public_id(&event_id) || !crate::model::is_public_id(&reservation_id) {
        return Err(ApiError::from(CheckoutError::ReservationNotFound).with_request_id(&request_id));
    }
    let key = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::from(CheckoutError::InvalidIdempotencyKey).with_request_id(&request_id)
        })?;
    validate_idempotency_key(key)
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    let result = crate::postgres::release_reservation(&state.pool, &event_id, &reservation_id, key)
        .await
        .map_err(|error| ApiError::from(error).with_request_id(&request_id))?;
    Ok(Json(result))
}

#[derive(Serialize)]
struct Problem {
    #[serde(rename = "type")]
    problem_type: &'static str,
    title: &'static str,
    status: u16,
    detail: &'static str,
    error: ProblemError,
    request_id: String,
}

#[derive(Serialize)]
struct ProblemError {
    code: &'static str,
}

struct ApiError {
    status: StatusCode,
    title: &'static str,
    detail: &'static str,
    code: &'static str,
    request_id: Option<String>,
}

impl ApiError {
    fn invalid_organization() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            title: "Invalid organization",
            detail: "Provide a name of 2–120 characters and a lowercase slug of 3–62 letters, digits, or hyphens.",
            code: "INVALID_ORGANIZATION",
            request_id: None,
        }
    }

    fn organization_slug_unavailable(request_id: &RequestId) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            title: "Organization slug unavailable",
            detail: "That slug is already in use. Choose a different slug.",
            code: "ORGANIZATION_SLUG_UNAVAILABLE",
            request_id: Some(request_id.0.clone()),
        }
    }

    fn service_unavailable(code: &'static str, request_id: &RequestId) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            title: "Service unavailable",
            detail: "The organization service is temporarily unavailable.",
            code,
            request_id: Some(request_id.0.clone()),
        }
    }

    fn unauthenticated(request_id: &RequestId) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            title: "Authentication required",
            detail: "Provide a valid bearer token to access this resource.",
            code: "UNAUTHENTICATED",
            request_id: Some(request_id.0.clone()),
        }
    }

    fn invalid_request() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            title: "Invalid request",
            detail: "The request body is invalid or contains unsupported fields.",
            code: "INVALID_REQUEST",
            request_id: None,
        }
    }

    fn with_request_id(mut self, request_id: &RequestId) -> Self {
        self.request_id = Some(request_id.0.clone());
        self
    }
}

impl From<CheckoutError> for ApiError {
    fn from(error: CheckoutError) -> Self {
        match error {
            CheckoutError::InvalidPublicId => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                title: "Invalid identifier",
                detail: "Event and ticket type identifiers must be valid public IDs.",
                code: "INVALID_ID",
                request_id: None,
            },
            CheckoutError::InvalidIdempotencyKey => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                title: "Invalid idempotency key",
                detail: "Provide a base64url key containing at least 128 bits of random data.",
                code: "INVALID_IDEMPOTENCY_KEY",
                request_id: None,
            },
            CheckoutError::InvalidQuantity => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                title: "Invalid quantity",
                detail: "Quantity must be between 1 and 100.",
                code: "INVALID_QUANTITY",
                request_id: None,
            },
            CheckoutError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                title: "Resource not found",
                detail: "The event or ticket type is not publicly available.",
                code: "NOT_FOUND",
                request_id: None,
            },
            CheckoutError::ReservationNotFound => Self {
                status: StatusCode::NOT_FOUND,
                title: "Reservation not found",
                detail: "The reservation could not be found.",
                code: "NOT_FOUND",
                request_id: None,
            },
            CheckoutError::SaleClosed => Self {
                status: StatusCode::CONFLICT,
                title: "Sale closed",
                detail: "Reservations are not available during this sale window.",
                code: "SALE_CLOSED",
                request_id: None,
            },
            CheckoutError::InventoryUnavailable => Self {
                status: StatusCode::CONFLICT,
                title: "Inventory unavailable",
                detail: "The requested quantity is no longer available.",
                code: "INVENTORY_UNAVAILABLE",
                request_id: None,
            },
            CheckoutError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                title: "Idempotency key conflict",
                detail: "This key was already used for a different request.",
                code: "IDEMPOTENCY_KEY_REUSED",
                request_id: None,
            },
            CheckoutError::Unavailable => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                title: "Service unavailable",
                detail: "The reservation service is temporarily unavailable.",
                code: "SERVICE_UNAVAILABLE",
                request_id: None,
            },
        }
    }
}

impl From<AuthoringError> for ApiError {
    fn from(error: AuthoringError) -> Self {
        match error {
            AuthoringError::InvalidInput => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                title: "Invalid event or ticket",
                detail: "Check the provided values and their documented limits.",
                code: "INVALID_AUTHORING_INPUT",
                request_id: None,
            },
            AuthoringError::Conflict => Self {
                status: StatusCode::CONFLICT,
                title: "Request conflicts with existing data",
                detail: "That event slug or idempotency key was already used with different values.",
                code: "AUTHORING_CONFLICT",
                request_id: None,
            },
            AuthoringError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                title: "Resource not found",
                detail: "The requested organization or event is unavailable.",
                code: "NOT_FOUND",
                request_id: None,
            },
            AuthoringError::NotReady => Self {
                status: StatusCode::CONFLICT,
                title: "Event is not ready to publish",
                detail: "Add a future start time, a valid venue and timezone, and a ticket type with a future sale end before publishing.",
                code: "EVENT_NOT_READY",
                request_id: None,
            },
            AuthoringError::Unavailable => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                title: "Service unavailable",
                detail: "The event service is temporarily unavailable.",
                code: "EVENT_SERVICE_UNAVAILABLE",
                request_id: None,
            },
        }
    }
}

impl From<OrderError> for ApiError {
    fn from(error: OrderError) -> Self {
        match error {
            OrderError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                title: "Reservation not found",
                detail: "That reservation could not be found.",
                code: "RESERVATION_NOT_FOUND",
                request_id: None,
            },
            // The hold ran out. Distinct wording from HoldUnavailable because the
            // next step differs: return to availability rather than retry.
            OrderError::HoldExpired => Self {
                status: StatusCode::CONFLICT,
                title: "Reservation expired",
                detail: "This reservation has expired. Check availability and try again.",
                code: "RESERVATION_EXPIRED",
                request_id: None,
            },
            OrderError::HoldUnavailable => Self {
                status: StatusCode::CONFLICT,
                title: "Reservation no longer available",
                detail: "This reservation is no longer available to confirm.",
                code: "RESERVATION_UNAVAILABLE",
                request_id: None,
            },
            OrderError::InvalidAnswers => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                title: "Check your answers",
                detail: "Some answers were missing or not valid for this event.",
                code: "INVALID_REGISTRATION_ANSWERS",
                request_id: None,
            },
            OrderError::InvalidInput => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                title: "Invalid request",
                detail: "Check the name and email address you provided.",
                code: "INVALID_REQUEST",
                request_id: None,
            },
            OrderError::Unavailable => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                title: "Service unavailable",
                detail: "The reservation service is temporarily unavailable.",
                code: "SERVICE_UNAVAILABLE",
                request_id: None,
            },
        }
    }
}

impl From<CheckInError> for ApiError {
    fn from(error: CheckInError) -> Self {
        match error {
            CheckInError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                title: "Not found",
                detail: "That resource is unavailable.",
                code: "NOT_FOUND",
                request_id: None,
            },
            // A ticket outside the caller's organization is reported as not found,
            // never as forbidden, so a scanner cannot probe for ticket ids.
            CheckInError::ScopeMismatch => Self {
                status: StatusCode::NOT_FOUND,
                title: "Not found",
                detail: "That resource is unavailable.",
                code: "NOT_FOUND",
                request_id: None,
            },
            CheckInError::NotAdmissible => Self {
                status: StatusCode::CONFLICT,
                title: "Ticket cannot be admitted",
                detail: "This ticket has already been used or was voided.",
                code: "TICKET_NOT_ADMISSIBLE",
                request_id: None,
            },
            CheckInError::InvalidInput => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                title: "Invalid request",
                detail: "Provide between 1 and 50 codes to scan.",
                code: "INVALID_REQUEST",
                request_id: None,
            },
            CheckInError::Unavailable => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                title: "Service unavailable",
                detail: "The check-in service is temporarily unavailable.",
                code: "SERVICE_UNAVAILABLE",
                request_id: None,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = self
            .request_id
            .unwrap_or_else(|| format!("req_{}", Ulid::new()));
        let body = Problem {
            problem_type: "about:blank",
            title: self.title,
            status: self.status.as_u16(),
            detail: self.detail,
            error: ProblemError { code: self.code },
            request_id: request_id.clone(),
        };
        let mut response = (self.status, Json(body)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("X-Request-ID", value);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::{parse_traceparent, router, router_with_auth};
    use crate::auth::NeonJwtVerifier;

    #[test]
    fn traceparent_parser_accepts_v00_and_clears_reserved_flags() {
        assert_eq!(
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-ff"),
            Some((
                "4bf92f3577b34da6a3ce929d0e0e4736".to_owned(),
                "00f067aa0ba902b7".to_owned(),
                "01".to_owned(),
            ))
        );
        assert!(
            parse_traceparent("00-00000000000000000000000000000000-00f067aa0ba902b7-01").is_none()
        );
        assert!(
            parse_traceparent("00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01").is_none()
        );
    }

    #[tokio::test]
    async fn request_context_returns_the_server_span_and_preserves_request_id() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://zeno:zeno@127.0.0.1/zeno")
            .expect("lazy pool URL is valid");
        let response = router(pool)
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/me")
                    .header(
                        "traceparent",
                        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                    )
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");
        let traceparent = response
            .headers()
            .get("traceparent")
            .expect("response trace context")
            .to_str()
            .expect("trace context is ASCII");
        assert!(traceparent.starts_with("00-4bf92f3577b34da6a3ce929d0e0e4736-"));
        assert!(traceparent.ends_with("-01"));
        assert!(response.headers().contains_key("x-request-id"));
    }

    #[tokio::test]
    async fn auth_identity_reports_unconfigured_auth_without_exposing_details() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://zeno:zeno@127.0.0.1/zeno")
            .expect("lazy pool URL is valid");
        let response = router(pool)
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/me")
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");

        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/problem+json"
        );
        assert!(response.headers().contains_key("x-request-id"));
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("problem body is readable");
        let body = String::from_utf8(body.to_vec()).expect("problem body is UTF-8");
        assert!(body.contains("AUTH_NOT_CONFIGURED"));
        assert!(!body.contains("NEON_AUTH"));
    }

    #[tokio::test]
    async fn auth_identity_requires_a_bearer_token_before_calling_jwks() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://zeno:zeno@127.0.0.1/zeno")
            .expect("lazy pool URL is valid");
        let verifier = NeonJwtVerifier::new(
            "https://auth.example.test",
            "https://auth.example.test/jwks",
        )
        .expect("test verifier configuration is valid");
        let response = router_with_auth(pool, Some(verifier))
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/me")
                    .body(Body::empty())
                    .expect("request is valid"),
            )
            .await
            .expect("router responds");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key("x-request-id"));
    }

    #[tokio::test]
    async fn organization_reads_and_writes_require_identity() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://zeno:zeno@127.0.0.1/zeno")
            .expect("lazy pool URL is valid");
        let verifier = NeonJwtVerifier::new(
            "https://auth.example.test",
            "https://auth.example.test/jwks",
        )
        .expect("test verifier configuration is valid");
        let router = router_with_auth(pool, Some(verifier));

        for (method, body) in [
            ("GET", Body::empty()),
            (
                "POST",
                Body::from(r#"{"name":"Test Org","slug":"test-org"}"#),
            ),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/v1/organizations")
                        .body(body)
                        .expect("request is valid"),
                )
                .await
                .expect("router responds");
            assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        }
    }
}
